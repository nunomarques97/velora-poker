use serde::Serialize;
use tauri::State;

use crate::db;
use crate::import;
use crate::settings;
use crate::state::AppState;
use crate::stats::{self, PlayerStats};
use crate::watcher;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPayload {
    pub id: String,
    pub name: String,
    pub hands: i64,
    pub stats: PlayerStats,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStatusPayload {
    pub configured_dir: Option<String>,
    pub hands_imported: i64,
    pub last_import_at: Option<String>,
    pub parser_status: String,
}

fn build_status_payload(state: &AppState) -> Result<ImportStatusPayload, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let hands_imported = db::count_hands(&conn).map_err(|e| e.to_string())?;
    drop(conn);

    let import_state = state.import.lock().map_err(|e| e.to_string())?.clone();

    Ok(ImportStatusPayload {
        configured_dir: import_state.configured_dir,
        hands_imported,
        last_import_at: import_state.last_import_at,
        parser_status: import_state.parser_status,
    })
}

#[tauri::command]
pub fn get_players(state: State<AppState>) -> Result<Vec<PlayerPayload>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let rows = db::list_players(&conn).map_err(|e| e.to_string())?;

    let mut players = Vec::with_capacity(rows.len());
    for row in rows {
        let player_stats =
            stats::compute_player_stats(&conn, row.id).map_err(|e| e.to_string())?;
        players.push(PlayerPayload {
            id: row.id.to_string(),
            name: row.name,
            hands: row.hands,
            stats: player_stats,
        });
    }

    Ok(players)
}

#[tauri::command]
pub fn get_import_status(state: State<AppState>) -> Result<ImportStatusPayload, String> {
    build_status_payload(&state)
}

#[tauri::command]
pub fn set_hand_history_dir(
    app_handle: tauri::AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<ImportStatusPayload, String> {
    let dir_path = std::path::PathBuf::from(&path);
    if !dir_path.is_dir() {
        return Err(format!("'{path}' is not a directory"));
    }

    {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        db::set_setting(&conn, settings::SETTING_HAND_HISTORY_DIR, &path)
            .map_err(|e| e.to_string())?;
    }

    // Drop any previous watcher before starting a new one.
    *state.watcher.lock().map_err(|e| e.to_string())? = None;

    let summary = {
        let mut conn = state.conn.lock().map_err(|e| e.to_string())?;
        import::import_directory(&mut conn, &dir_path)?
    };

    {
        let mut import_state = state.import.lock().map_err(|e| e.to_string())?;
        import_state.configured_dir = Some(path.clone());
        import_state.parser_status = "watching".to_string();
        if summary.hands_imported > 0 {
            import_state.last_import_at = Some(db::now_iso());
        }
    }

    match watcher::start_watching(app_handle, dir_path) {
        Ok(w) => {
            *state.watcher.lock().map_err(|e| e.to_string())? = Some(w);
        }
        Err(err) => {
            let mut import_state = state.import.lock().map_err(|e| e.to_string())?;
            import_state.parser_status = format!("error: {err}");
        }
    }

    build_status_payload(&state)
}
