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
pub mod stats;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_players,
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
                    let state = app_handle.state::<AppState>();

                    {
                        let mut conn = state.conn.lock().expect("db lock poisoned");
                        match import::import_directory(&mut conn, &dir_path) {
                            Ok(summary) if summary.hands_imported > 0 => {
                                drop(conn);
                                state
                                    .import
                                    .lock()
                                    .expect("import state lock poisoned")
                                    .last_import_at = Some(db::now_iso());
                            }
                            Ok(_) => {}
                            Err(err) => {
                                eprintln!("initial hand history import failed: {err}");
                            }
                        }
                    }

                    match watcher::start_watching(app_handle.clone(), dir_path) {
                        Ok(w) => {
                            *state.watcher.lock().expect("watcher lock poisoned") = Some(w);
                        }
                        Err(err) => {
                            eprintln!("failed to start hand history watcher: {err}");
                            state
                                .import
                                .lock()
                                .expect("import state lock poisoned")
                                .parser_status = format!("error: {err}");
                        }
                    }
                }
            }

            // The overlay is intentionally NOT auto-reopened here. Creating a
            // second WebviewWindow synchronously inside `setup()` — before
            // the main window's own WebView2 environment has finished
            // initializing — reliably crashed the whole process on Windows
            // (observed as `velora-poker.exe` exiting with 0xcfffffff a few
            // seconds after launch). The overlay is only ever created by an
            // explicit user action (the "Open Overlay" command), which is
            // stable; see `overlay::open`.

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
