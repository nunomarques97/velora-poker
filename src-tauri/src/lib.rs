mod commands;
mod overlay;
mod settings;
mod state;
mod watcher;

pub mod classification;
pub mod db;
pub mod hud;
pub mod import;
pub mod parser;
pub mod sessions;
pub mod stats;

use tauri::{Emitter, Manager};

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_players,
            commands::get_active_table_players,
            commands::set_player_color_override,
            commands::clear_player_color_override,
            commands::get_dashboard_summary,
            commands::get_import_status,
            commands::set_hand_history_dir,
            commands::detect_pokerstars_dirs,
            commands::validate_hand_history_dir,
            commands::pick_folder_dialog,
            commands::get_app_settings,
            commands::complete_onboarding,
            commands::reset_onboarding,
            commands::get_hud_profiles,
            commands::get_active_hud_profile,
            commands::set_active_hud_profile,
            commands::set_hud_profile_min_hands,
            commands::open_overlay,
            commands::close_overlay,
            commands::is_overlay_open,
            commands::set_overlay_click_through,
            commands::save_hud_position,
            commands::get_hud_positions,
            commands::get_sessions,
        ])
        .setup(|app| {
            let app_handle = app.handle().clone();
            let data_dir = app_handle
                .path()
                .app_data_dir()
                .expect("resolve app data dir");
            let db_path = data_dir.join("velora.db");

            let app_state = AppState::new(db_path).expect("failed to initialize database");
            let configured_dir = app_state
                .import
                .lock()
                .expect("import state lock poisoned")
                .configured_dir
                .clone();

            app.manage(app_state);

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

            // the overlay window is now declared
            // statically in `tauri.conf.json` (`visible: false`), so Tauri
            // creates it as part of its own normal batch window-bootstrap —
            // the same mechanism the main window has always used without
            // issue — rather than via a manual `WebviewWindowBuilder::build()`
            // call from inside this closure. That manual, out-of-band
            // approach (building a second window synchronously in `setup()`,
            // before the main window's own WebView2 environment had finished
            // initializing) is what reliably crashed the whole process on
            // Windows historically (`velora-poker.exe` exiting with
            // 0xcfffffff a few seconds after launch) — this static
            // declaration does not do that. It stays hidden until the user
            // clicks "Open Overlay"; see `overlay::open`.

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
