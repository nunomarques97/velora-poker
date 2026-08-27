use serde::Serialize;
use tauri::{Emitter, State};

use crate::classification::{self, ClassificationResult};
use crate::db;
use crate::hud::{self, HudProfile};
use crate::import;
use crate::overlay;
use crate::settings::{self, DetectedDir, DirValidation};
use crate::state::AppState;
use crate::stats::{self, PlayerStats};
use crate::watcher;

// ---------------------------------------------------------------------
// Players
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshotPayload {
    pub stack: f64,
    pub big_blind: f64,
    pub stack_bb: f64,
    pub currency: String,
    pub format: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPayload {
    pub id: String,
    pub name: String,
    pub hands: i64,
    pub stats: PlayerStats,
    pub classification: ClassificationResult,
    pub snapshot: Option<PlayerSnapshotPayload>,
}

fn build_player_payload(
    conn: &rusqlite::Connection,
    rules: &[classification::ClassificationRule],
    id: i64,
    name: String,
    hands: i64,
) -> Result<PlayerPayload, String> {
    let player_stats = stats::compute_player_stats(conn, id).map_err(|e| e.to_string())?;
    let classification_result =
        classification::resolve_for_player(conn, rules, id, hands, &player_stats)
            .map_err(|e| e.to_string())?;
    let snapshot = db::latest_player_snapshot(conn, id)
        .map_err(|e| e.to_string())?
        .map(|s| PlayerSnapshotPayload {
            stack: s.starting_stack,
            big_blind: s.big_blind,
            stack_bb: if s.big_blind > 0.0 {
                (s.starting_stack / s.big_blind * 10.0).round() / 10.0
            } else {
                0.0
            },
            currency: s.currency,
            format: s.format,
        });

    Ok(PlayerPayload {
        id: id.to_string(),
        name,
        hands,
        stats: player_stats,
        classification: classification_result,
        snapshot,
    })
}

#[tauri::command]
pub fn get_players(state: State<AppState>) -> Result<Vec<PlayerPayload>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let rows = db::list_players(&conn).map_err(|e| e.to_string())?;
    let rules = classification::list_rules(&conn).map_err(|e| e.to_string())?;

    let mut players = Vec::with_capacity(rows.len());
    for row in rows {
        players.push(build_player_payload(&conn, &rules, row.id, row.name, row.hands)?);
    }

    Ok(players)
}

#[tauri::command]
pub fn set_player_color_override(
    state: State<AppState>,
    player_id: String,
    color: String,
    label: Option<String>,
) -> Result<PlayerPayload, String> {
    let id: i64 = player_id.parse().map_err(|_| "invalid player id".to_string())?;
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::set_player_color_override(&conn, id, &color, label.as_deref())
        .map_err(|e| e.to_string())?;

    let name: String = conn
        .query_row("SELECT name FROM players WHERE id = ?1", [id], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    let hands: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM player_hands WHERE player_id = ?1",
            [id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let rules = classification::list_rules(&conn).map_err(|e| e.to_string())?;
    build_player_payload(&conn, &rules, id, name, hands)
}

#[tauri::command]
pub fn clear_player_color_override(
    state: State<AppState>,
    player_id: String,
) -> Result<PlayerPayload, String> {
    let id: i64 = player_id.parse().map_err(|_| "invalid player id".to_string())?;
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::clear_player_color_override(&conn, id).map_err(|e| e.to_string())?;

    let name: String = conn
        .query_row("SELECT name FROM players WHERE id = ?1", [id], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    let hands: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM player_hands WHERE player_id = ?1",
            [id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let rules = classification::list_rules(&conn).map_err(|e| e.to_string())?;
    build_player_payload(&conn, &rules, id, name, hands)
}

// ---------------------------------------------------------------------
// Hand history import status / configuration
// ---------------------------------------------------------------------

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

    match watcher::start_watching(app_handle.clone(), dir_path) {
        Ok(w) => {
            *state.watcher.lock().map_err(|e| e.to_string())? = Some(w);
        }
        Err(err) => {
            let mut import_state = state.import.lock().map_err(|e| e.to_string())?;
            import_state.parser_status = format!("error: {err}");
        }
    }

    if summary.hands_imported > 0 {
        let _ = app_handle.emit("hands-imported", summary.hands_imported);
    }

    build_status_payload(&state)
}

#[tauri::command]
pub fn detect_pokerstars_dirs() -> Vec<DetectedDir> {
    settings::detect_candidates()
}

#[tauri::command]
pub fn validate_hand_history_dir(path: String) -> DirValidation {
    settings::validate_hand_history_dir(std::path::Path::new(&path))
}

#[tauri::command]
pub async fn pick_folder_dialog(app_handle: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app_handle
        .dialog()
        .file()
        .blocking_pick_folder()
        .map(|p| p.to_string())
}

// ---------------------------------------------------------------------
// App / onboarding settings
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsPayload {
    pub onboarding_complete: bool,
    pub poker_room: Option<String>,
    pub overlay_enabled: bool,
}

#[tauri::command]
pub fn get_app_settings(state: State<AppState>) -> Result<AppSettingsPayload, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let onboarding_complete =
        db::get_setting(&conn, settings::SETTING_ONBOARDING_COMPLETE).map_err(|e| e.to_string())?
            == Some("true".to_string());
    let poker_room = db::get_setting(&conn, settings::SETTING_POKER_ROOM).map_err(|e| e.to_string())?;
    let overlay_enabled =
        db::get_setting(&conn, settings::SETTING_OVERLAY_ENABLED).map_err(|e| e.to_string())?
            == Some("true".to_string());

    Ok(AppSettingsPayload {
        onboarding_complete,
        poker_room,
        overlay_enabled,
    })
}

#[tauri::command]
pub fn complete_onboarding(state: State<AppState>, poker_room: String) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::set_setting(&conn, settings::SETTING_POKER_ROOM, &poker_room).map_err(|e| e.to_string())?;
    db::set_setting(&conn, settings::SETTING_ONBOARDING_COMPLETE, "true").map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn reset_onboarding(state: State<AppState>) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::set_setting(&conn, settings::SETTING_ONBOARDING_COMPLETE, "false").map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------
// HUD profiles
// ---------------------------------------------------------------------

#[tauri::command]
pub fn get_hud_profiles(state: State<AppState>) -> Result<Vec<HudProfile>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    hud::list_profiles(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_active_hud_profile(state: State<AppState>) -> Result<HudProfile, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    hud::get_active_profile(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_active_hud_profile(state: State<AppState>, id: String) -> Result<HudProfile, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    hud::set_active_profile(&conn, &id).map_err(|e| e.to_string())?;
    hud::get_active_profile(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_hud_profile_min_hands(
    state: State<AppState>,
    id: String,
    min_hands: i64,
) -> Result<HudProfile, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    hud::set_profile_min_hands(&conn, &id, min_hands).map_err(|e| e.to_string())?;
    hud::get_profile(&conn, &id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "profile not found".to_string())
}

// ---------------------------------------------------------------------
// HUD overlay window + positions
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudPositionPayload {
    pub player_id: String,
    pub x: f64,
    pub y: f64,
}

#[tauri::command]
pub fn open_overlay(app_handle: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        db::set_setting(&conn, settings::SETTING_OVERLAY_ENABLED, "true").map_err(|e| e.to_string())?;
    }
    overlay::open(&app_handle)
}

#[tauri::command]
pub fn close_overlay(app_handle: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        db::set_setting(&conn, settings::SETTING_OVERLAY_ENABLED, "false").map_err(|e| e.to_string())?;
    }
    overlay::close(&app_handle)
}

#[tauri::command]
pub fn is_overlay_open(app_handle: tauri::AppHandle) -> bool {
    overlay::is_open(&app_handle)
}

#[tauri::command]
pub fn set_overlay_click_through(app_handle: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    overlay::set_click_through(&app_handle, enabled)
}

#[tauri::command]
pub fn save_hud_position(state: State<AppState>, player_id: String, x: f64, y: f64) -> Result<(), String> {
    let id: i64 = player_id.parse().map_err(|_| "invalid player id".to_string())?;
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::set_hud_position(&conn, id, x, y).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_hud_positions(state: State<AppState>) -> Result<Vec<HudPositionPayload>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT player_id, x, y FROM hud_positions")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(HudPositionPayload {
                player_id: row.get::<_, i64>(0)?.to_string(),
                x: row.get(1)?,
                y: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}
