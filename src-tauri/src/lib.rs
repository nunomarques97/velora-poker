mod commands;
mod settings;
mod state;
mod watcher;

pub mod db;
pub mod import;
pub mod parser;
pub mod stats;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_players,
            commands::get_import_status,
            commands::set_hand_history_dir,
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
