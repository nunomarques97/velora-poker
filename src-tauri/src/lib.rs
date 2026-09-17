// `pub` (not private) so integration tests in `src-tauri/tests/` can call the
// pure onboarding-readiness computation (, `commands::onboarding_readiness`)
// directly, without going through a live Tauri `State`.
pub mod commands;
mod overlay;
mod settings;
mod state;
mod watcher;

pub mod classification;
pub mod db;
pub mod description_rules;
pub mod hud;
pub mod import;
pub mod parser;
pub mod sessions;
pub mod stats;
pub mod table_track;

use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use state::AppState;

/// Resolves the same per-user data directory Tauri's own
/// `PathResolver::app_data_dir()` returns on Windows — `%APPDATA%\<identifier>`
/// — but without needing an `AppHandle`, so the database can be opened before
/// the Tauri app exists. The identifier is read from the generated context
/// rather than hardcoded, so it cannot drift from `tauri.conf.json`.
fn app_data_dir(identifier: &str) -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .expect("resolve %APPDATA%");
    base.join(identifier)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();

    // `AppState` is created here and handed to
    // `Builder::manage` *before* the builder runs, rather than being
    // `app.manage(...)`-ed from inside `setup()`. Windows declared in
    // `tauri.conf.json` start loading their frontend as soon as Tauri creates
    // them, and in a release bundle (assets embedded in the binary, no Vite
    // dev-server round-trip) that is fast enough that the frontend's first IPC
    // calls — `get_app_settings`, `get_dashboard_summary` — can land before
    // `setup()` ever reaches `app.manage(...)`. Those calls then fail with
    // "state not managed for field `state`", which is not just a cosmetic
    // error: `App.tsx` treats a failed `get_app_settings` as "already
    // onboarded", so a first-time user on a fresh install would silently skip
    // onboarding and never be asked for their hand-history folder. This only
    // ever reproduced in an installed build, never under `npm run tauri dev`.
    // `Builder::manage` populates the state map before any window exists, so
    // the race cannot occur at all.
    let db_path = app_data_dir(&context.config().identifier).join("velora.db");
    let app_state = AppState::new(db_path).expect("failed to initialize database");

    tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // global HUD toggle hotkey. One shortcut is ever registered, so
        // the handler doesn't need to discriminate by which one fired — see
        // `table_track::toggle_hud_for_foreground_table` for what it does
        // and why resolving "the" table from OS foreground focus, not any
        // Velora-side concept of an active table, is the whole point.
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|_app, _shortcut, event| {
                    // A shortcut delivers both press and release; acting
                    // only on `Pressed` is what makes this "press once,
                    // toggle once" instead of firing twice per press.
                    if event.state() == ShortcutState::Pressed {
                        table_track::toggle_hud_for_foreground_table();
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::get_players,
            commands::get_players_page,
            commands::get_active_table_players,
            commands::set_player_color_override,
            commands::clear_player_color_override,
            commands::set_player_note,
            commands::get_dashboard_summary,
            commands::get_import_status,
            commands::set_hand_history_dir,
            commands::detect_pokerstars_dirs,
            commands::validate_hand_history_dir,
            commands::pick_folder_dialog,
            commands::get_app_settings,
            commands::complete_onboarding,
            commands::reset_onboarding,
            commands::get_classification_rules,
            commands::get_hud_profiles,
            commands::get_active_hud_profile,
            commands::set_active_hud_profile,
            commands::set_hud_profile_min_hands,
            commands::get_overlay_status,
            commands::set_overlays_enabled,
            commands::close_overlay,
            commands::show_overlay,
            commands::is_overlay_open,
            commands::is_overlay_dismissed,
            commands::set_overlay_mode,
            commands::set_all_overlay_modes,
            commands::get_overlay_mode,
            commands::get_any_overlay_mode,
            commands::set_overlay_hot_zones,
            commands::save_hud_position,
            commands::get_hud_positions,
            commands::get_sessions,
            commands::get_active_table_max_players,
            commands::set_auto_center_enabled,
            commands::get_seat_templates,
            commands::save_seat_template,
            commands::get_table_detection_status,
            commands::get_diagnostics_report,
            commands::get_onboarding_readiness,
            commands::get_ingestion_health,
            commands::get_app_version,
        ])
        .setup(|app| {
            let app_handle = app.handle().clone();
            let configured_dir = app
                .state::<AppState>()
                .import
                .lock()
                .expect("import state lock poisoned")
                .configured_dir
                .clone();

            if let Some(dir) = configured_dir {
                let dir_path = std::path::PathBuf::from(&dir);
                if dir_path.is_dir() {
                    // this backlog import used to run
                    // right here, synchronously, on the same thread that's
                    // still inside `setup()` — for a hand-history folder
                    // built up over real play that can take the better part
                    // of a minute, during which the whole app is unresponsive
                    // (the main thread never gets back to pumping window
                    // messages) and every IPC command blocks on `state.conn`'s
                    // mutex, held for the entire scan. Moved to a background
                    // thread so window creation and the event loop are never
                    // blocked. The frontend's `players` state starts empty
                    // and renders no cards until this thread's
                    // "hands-imported" emit lands — never a stale or
                    // partially-computed stat value in the meantime.
                    let thread_handle = app_handle.clone();
                    std::thread::spawn(move || {
                        let state = thread_handle.state::<AppState>();

                        let summary = {
                            let mut conn = match state.conn.lock() {
                                Ok(conn) => conn,
                                Err(_) => return,
                            };
                            match import::import_directory(&mut conn, &dir_path) {
                                Ok(summary) => summary,
                                Err(err) => {
                                    eprintln!("initial hand history import failed: {err}");
                                    if let Ok(mut import_state) = state.import.lock() {
                                        import_state.parser_status = format!("error: {err}");
                                    }
                                    return;
                                }
                            }
                        };

                        if summary.hands_imported > 0 {
                            if let Ok(mut import_state) = state.import.lock() {
                                import_state.last_import_at = Some(db::now_iso());
                            }
                            let _ = thread_handle.emit("hands-imported", summary.hands_imported);
                        }

                        match watcher::start_watching(thread_handle.clone(), dir_path) {
                            Ok(w) => {
                                if let Ok(mut watcher_slot) = state.watcher.lock() {
                                    *watcher_slot = Some(w);
                                }
                            }
                            Err(err) => {
                                eprintln!("failed to start hand history watcher: {err}");
                                if let Ok(mut import_state) = state.import.lock() {
                                    import_state.parser_status = format!("error: {err}");
                                }
                            }
                        }
                    });
                }
            }

            // / the `overlay` window
            // declared in `tauri.conf.json` is created here by Tauri's own
            // batch window-bootstrap and stays hidden forever. Since  it is
            // a *prototype*: every real per-table overlay is cloned from its
            // config at runtime by `overlay::manager`, on that module's own
            // thread. Building windows from a plain thread is one of the three
            // patterns Tauri documents as safe on Windows; building them from
            // a *synchronous command handler* — which is what the old
            // `open_overlay` did — is the one it documents as deadlocking, and
            // is what spent eight rounds diagnosing. See
            // `overlay::manager`'s header for the citation.

            // PHASE E (2026-08-28): table window-following. Installs a
            // `SetWinEventHook` on the main thread — the same thread that
            // runs Tauri's window message loop, which is what pumps the
            // hook's `WINEVENT_OUTOFCONTEXT` callback — plus a low-frequency
            // polling fallback that reconciles the tracked-table registry
            // against the table windows that actually exist (app started
            // before the tables opened, tables opened or closed since).
            // an overlay decides click-through per-pixel from a list of
            // hot zones instead of one window-wide WS_EX_TRANSPARENT flag, so
            // the table stays clickable while the overlay's own control bar
            // and pagination dots never stop being reachable. per
            // overlay window, since there are now as many as there are tables.
            overlay::install(&app_handle);

            let app_state = app.state::<AppState>();
            let overlays_enabled = {
                let conn = app_state.conn.lock().expect("db lock poisoned");
                // HUDs are automatic, so the default for a fresh install
                // is on — the setting only exists as a kill switch. Before
                //  this was written by an explicit "Open Overlay" click and
                // defaulted to off, which as a default now would mean a new
                // user opens a table and sees nothing.
                db::get_setting(&conn, settings::SETTING_OVERLAY_ENABLED)
                    .ok()
                    .flatten()
                    .map(|v| v != "false")
                    .unwrap_or(true)
            };
            overlay::manager::start(app_handle.clone(), overlays_enabled);

            table_track::install_tracking(app_handle.clone());

            // default, hardcoded for now — no settings UI to change it
            // yet. Registration failure (e.g. another app already owns this
            // combination) is logged, not fatal: every other feature works
            // fine without the hotkey, so it must not block startup.
            let hotkey = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyH);
            if let Err(err) = app_handle.global_shortcut().register(hotkey) {
                eprintln!("[table_track] failed to register global hotkey Ctrl+Alt+H: {err}");
            }

            Ok(())
        })
        .run(context)
        .expect("error while running tauri application");
}
