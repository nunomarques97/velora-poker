use serde::Serialize;
use tauri::{Emitter, State};

use crate::classification::{self, ClassificationResult};
use crate::db;
use crate::hud::{self, HudProfile};
use crate::import;
use crate::overlay;
use crate::sessions::{self, SessionSummary, SessionsTodaySummary};
use crate::settings::{self, DetectedDir, DirValidation};
use crate::state::AppState;
use crate::stats::{self, PlayerStats};
use crate::table_track;
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
    /// Free-text note the user wrote about this player (Phase E), or
    /// `None` when none has been written. Rendered in the player profile
    /// drawer and the Players list only — deliberately *not* on the HUD
    /// overlay in v1, to protect overlay compactness (spec,  precedent).
    pub note: Option<String>,
    /// This player's seat number at the currently active table — `None` for
    /// the all-time roster (`get_players`), populated for
    /// `get_active_table_players` (Phase E seat mapping).
    pub seat: Option<i64>,
}

/// `include_note` controls whether the player's free-text note is fetched
/// (Phase E). It is `false` for exactly one caller,
/// `get_active_table_players`, and that is deliberate: the overlay never
/// renders notes, so fetching one there would add a per-player query to the
/// hot overlay-refresh path for a field nothing reads. That path is also under
/// a live-verification embargo, so its query surface must stay
/// exactly as it was before . Every other caller passes `true`.
fn build_player_payload(
    conn: &rusqlite::Connection,
    rules: &[classification::ClassificationRule],
    id: i64,
    name: String,
    hands: i64,
    seat: Option<i64>,
    include_note: bool,
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

    let note = if include_note {
        db::get_player_note(conn, id).map_err(|e| e.to_string())?
    } else {
        None
    };

    Ok(PlayerPayload {
        id: id.to_string(),
        name,
        hands,
        stats: player_stats,
        classification: classification_result,
        snapshot,
        note,
        seat,
    })
}

#[tauri::command]
pub fn get_players(state: State<AppState>) -> Result<Vec<PlayerPayload>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let rows = db::list_players(&conn).map_err(|e| e.to_string())?;
    let rules = classification::list_rules(&conn).map_err(|e| e.to_string())?;

    let mut players = Vec::with_capacity(rows.len());
    for row in rows {
        players.push(build_player_payload(
            &conn, &rules, row.id, row.name, row.hands, None, true,
        )?);
    }

    Ok(players)
}

/// Players seated in the most recently imported hand — what the live
/// overlay should render, as opposed to `get_players`' full all-time roster
/// (used by the Players list and HUD profile preview/management views).
///
/// scoped to the table window the overlay is actually tracking
/// (`table_track::active_table_name()`), when one is tracked, so that
/// multi-tabling (e.g. the four simultaneous tournament lobbies confirmed
/// on the user's machine in) can't have a hand completing on a
/// *different* open table flip which players' cards render on this one —
/// see the notes  and `db::list_active_table_players_with_seats`.
/// Falls back to the pre- global-latest-hand behavior when no table is
/// currently tracked (cold start, non-Windows, or the title didn't parse),
/// so the existing single-table experience is unchanged in that case.
#[tauri::command]
pub fn get_active_table_players(state: State<AppState>) -> Result<Vec<PlayerPayload>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let scope = table_track::active_table_name();
    let rows = db::list_active_table_players_with_seats(&conn, scope.as_deref())
        .map_err(|e| e.to_string())?;
    let rules = classification::list_rules(&conn).map_err(|e| e.to_string())?;

    let mut players = Vec::with_capacity(rows.len());
    for row in rows {
        // `include_note: false` — the overlay never renders notes, so this
        // path must not gain a per-player note lookup.
        players.push(build_player_payload(
            &conn, &rules, row.id, row.name, row.hands, row.seat, false,
        )?);
    }

    let active_hand = db::active_hand_info(&conn, scope.as_deref()).ok().flatten();
    let (hand_table_name, hand_id) = match active_hand {
        Some(h) => (h.table_name, Some(h.hand_id)),
        None => (None, None),
    };
    drop(conn);
    if let Ok(mut log) = state.refresh_log.lock() {
        if log.len() >= crate::state::REFRESH_LOG_CAP {
            log.pop_front();
        }
        log.push_back(crate::state::RefreshLogEntry {
            at: db::now_iso(),
            table_name: scope.or(hand_table_name),
            hand_id,
            player_count: players.len(),
        });
    }

    Ok(players)
}

/// The current active table's max-players count (2/6/9-max) — the key the
/// seat-mapping template is looked up by (Phase E). `None` until at
/// least one hand has been imported. Same  table-name scoping as
/// `get_active_table_players`.
#[tauri::command]
pub fn get_active_table_max_players(state: State<AppState>) -> Result<Option<i64>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let scope = table_track::active_table_name();
    db::active_table_max_players(&conn, scope.as_deref()).map_err(|e| e.to_string())
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
    build_player_payload(&conn, &rules, id, name, hands, None, true)
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
    build_player_payload(&conn, &rules, id, name, hands, None, true)
}

/// Saves the free-text note for one player (Phase E) and returns the
/// refreshed player, so the caller can update its list/drawer in place the
/// same way the colour-override commands above do. A blank note clears it.
#[tauri::command]
pub fn set_player_note(
    state: State<AppState>,
    player_id: String,
    note: String,
) -> Result<PlayerPayload, String> {
    let id: i64 = player_id.parse().map_err(|_| "invalid player id".to_string())?;
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::set_player_note(&conn, id, &note).map_err(|e| e.to_string())?;

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
    build_player_payload(&conn, &rules, id, name, hands, None, true)
}

// ---------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSummaryPayload {
    pub hands_played: i64,
    pub players_tracked: i64,
    pub current_hud_profile: String,
    /// `None` when no session has been played today — the Dashboard hides
    /// the "Sessions Today" card in that case rather than showing zeros.
    pub sessions_today: Option<SessionsTodaySummary>,
}

#[tauri::command]
pub fn get_dashboard_summary(state: State<AppState>) -> Result<DashboardSummaryPayload, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let hands_played = db::count_hands(&conn).map_err(|e| e.to_string())?;
    let players_tracked = db::list_players(&conn).map_err(|e| e.to_string())?.len() as i64;
    let current_hud_profile = hud::get_active_profile(&conn).map_err(|e| e.to_string())?.name;
    let sessions_today = sessions::sessions_today(&conn, chrono::Local::now().date_naive())
        .map_err(|e| e.to_string())?;

    Ok(DashboardSummaryPayload {
        hands_played,
        players_tracked,
        current_hud_profile,
        sessions_today,
    })
}

// ---------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------

#[tauri::command]
pub fn get_sessions(state: State<AppState>) -> Result<Vec<SessionSummary>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    sessions::list_sessions(&conn).map_err(|e| e.to_string())
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
    /// user-declared confirmation that PokerStars' own "Auto-Center"
    /// table option is on — required for automatic seat-mapping templates
    /// (Phase E); cannot be detected, only asked for in Settings.
    pub auto_center_enabled: bool,
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
    let auto_center_enabled =
        db::get_setting(&conn, settings::SETTING_AUTO_CENTER_ENABLED).map_err(|e| e.to_string())?
            == Some("true".to_string());

    Ok(AppSettingsPayload {
        onboarding_complete,
        poker_room,
        overlay_enabled,
        auto_center_enabled,
    })
}

#[tauri::command]
pub fn set_auto_center_enabled(state: State<AppState>, enabled: bool) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::set_setting(
        &conn,
        settings::SETTING_AUTO_CENTER_ENABLED,
        if enabled { "true" } else { "false" },
    )
    .map_err(|e| e.to_string())
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

// ---------------------------------------------------------------------
// Seat-mapping templates (Phase E)
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatTemplatePayload {
    pub seat: i64,
    pub x: f64,
    pub y: f64,
}

/// One max-players size's calibrated seat layout, `x`/`y` as fractions
/// (0..1) of the overlay/table window — same coordinate scheme as
/// `hud_positions`.
#[tauri::command]
pub fn get_seat_templates(
    state: State<AppState>,
    max_players: i64,
) -> Result<Vec<SeatTemplatePayload>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::list_seat_templates(&conn, max_players)
        .map_err(|e| e.to_string())
        .map(|rows| {
            rows.into_iter()
                .map(|r| SeatTemplatePayload { seat: r.seat, x: r.x, y: r.y })
                .collect()
        })
}

#[tauri::command]
pub fn save_seat_template(
    state: State<AppState>,
    max_players: i64,
    seat: i64,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::set_seat_template(&conn, max_players, seat, x, y).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------
// Table window detection status (Phase E)
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDetectionStatusPayload {
    pub detected: bool,
    /// Debug evidence added for the  window-following brief — lets the
    /// user confirm from Settings (no terminal needed) whether the
    /// WinEvent hooks registered and are firing, whether the 1.5s poll
    /// fallback loop is alive, and whether the two processes' Windows
    /// integrity levels differ (the UIPI hypothesis).
    pub hooks_installed: u32,
    pub event_callbacks_total: u64,
    pub event_callbacks_matched: u64,
    pub poll_ticks: u64,
    pub app_integrity_level: Option<String>,
    pub table_integrity_level: Option<String>,
}

/// Whether the PokerStars table window is currently found and being tracked
/// — surfaced in Settings so the user can see detection is (or isn't)
/// working without having to open the overlay and drag a card to find out.
#[tauri::command]
pub fn get_table_detection_status() -> TableDetectionStatusPayload {
    let debug = table_track::debug_snapshot();
    TableDetectionStatusPayload {
        detected: table_track::last_known_table_rect().is_some(),
        hooks_installed: debug.hooks_installed,
        event_callbacks_total: debug.event_callbacks_total,
        event_callbacks_matched: debug.event_callbacks_matched,
        poll_ticks: debug.poll_ticks,
        app_integrity_level: debug.app_integrity_level,
        table_integrity_level: debug.table_integrity_level,
    }
}

// ---------------------------------------------------------------------
// Diagnostics — self-serve "Copy Diagnostics" for the user
// ---------------------------------------------------------------------

/// Plain-text diagnostics dump the user can paste back to the maintainer when
/// something "feels wrong" during play without needing to characterize the
/// bug himself first ('s own framing: a non-technical user hitting a
/// reasonable limit on precision). Covers exactly the four things 's
/// brief asked for: which table(s)/hand the overlay currently considers
/// active and where that came from, the last ~20 overlay refresh/resync
/// events, the last ~10 imported hands and their table identifiers, and
/// current watcher/import status.
#[tauri::command]
pub fn get_diagnostics_report(state: State<AppState>) -> Result<String, String> {
    let mut out = String::new();
    out.push_str(&format!(
        "Velora diagnostics report — generated {}\n",
        db::now_iso()
    ));
    out.push_str("========================================\n\n");

    let scope = table_track::active_table_name();
    let tracked_rect = table_track::last_known_table_rect();
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let active_hand = db::active_hand_info(&conn, scope.as_deref()).map_err(|e| e.to_string())?;
    let player_count = db::list_active_table_players_with_seats(&conn, scope.as_deref())
        .map(|rows| rows.len())
        .unwrap_or(0);

    out.push_str("-- Active table --\n");
    match &scope {
        Some(name) => out.push_str(&format!(
            "Tracked table window: \"{name}\" (extracted from the tracked window's title)\n"
        )),
        None => out.push_str(
            "Tracked table window: none — falling back to the globally most-recently-imported \
             hand across ALL open tables. If more than one table was open, this can show a \
             different table's players than the one the overlay is sitting on.\n",
        ),
    }
    match &tracked_rect {
        Some(r) => out.push_str(&format!(
            "Tracked window rect: x={} y={} w={} h={}\n",
            r.x, r.y, r.width, r.height
        )),
        None => out.push_str("Tracked window rect: none\n"),
    }
    match &active_hand {
        Some(h) => out.push_str(&format!(
            "Resolved active hand: {} (table=\"{}\", tournament={}, played_at={})\n",
            h.hand_id,
            h.table_name.as_deref().unwrap_or("?"),
            h.tournament_id.as_deref().unwrap_or("-"),
            h.played_at.as_deref().unwrap_or("?"),
        )),
        None => {
            out.push_str("Resolved active hand: none (no hands imported for this scope yet)\n")
        }
    }
    out.push_str(&format!("Players currently rendered: {player_count}\n\n"));

    out.push_str("-- Overlay data refreshes (last 20) --\n");
    {
        let log = state.refresh_log.lock().map_err(|e| e.to_string())?;
        if log.is_empty() {
            out.push_str("(none yet — the overlay hasn't requested active-table data this run)\n");
        } else {
            for entry in log.iter() {
                out.push_str(&format!(
                    "{}  table={}  hand={}  players={}\n",
                    entry.at,
                    entry.table_name.as_deref().unwrap_or("(global fallback)"),
                    entry.hand_id.as_deref().unwrap_or("-"),
                    entry.player_count,
                ));
            }
        }
    }
    out.push('\n');

    out.push_str("-- Overlay window resyncs (last 20) --\n");
    let resyncs = table_track::resync_log();
    if resyncs.is_empty() {
        out.push_str("(none — no table window has been acquired this run)\n");
    } else {
        for entry in &resyncs {
            out.push_str(&format!(
                "{}  source={}  rect=(x={}, y={}, w={}, h={})\n",
                entry.at, entry.source, entry.rect.x, entry.rect.y, entry.rect.width, entry.rect.height
            ));
        }
    }
    out.push('\n');

    out.push_str("-- Last 10 imported hands --\n");
    let recent = db::recent_hands(&conn, 10).map_err(|e| e.to_string())?;
    if recent.is_empty() {
        out.push_str("(none imported yet)\n");
    } else {
        for h in &recent {
            out.push_str(&format!(
                "{}  table=\"{}\"  tournament={}  format={}  played_at={}\n",
                h.hand_id,
                h.table_name.as_deref().unwrap_or("?"),
                h.tournament_id.as_deref().unwrap_or("-"),
                h.format,
                h.played_at.as_deref().unwrap_or("?"),
            ));
        }
    }
    out.push('\n');
    drop(conn);

    out.push_str("-- Import / watcher status --\n");
    let import_status = build_status_payload(&state)?;
    out.push_str(&format!(
        "Configured folder: {}\n",
        import_status.configured_dir.as_deref().unwrap_or("(not configured)")
    ));
    out.push_str(&format!(
        "Hands imported (all-time): {}\n",
        import_status.hands_imported
    ));
    out.push_str(&format!(
        "Last import at: {}\n",
        import_status.last_import_at.as_deref().unwrap_or("never")
    ));
    out.push_str(&format!("Parser status: {}\n\n", import_status.parser_status));

    out.push_str("-- Table detection / window-following --\n");
    let detection = get_table_detection_status();
    out.push_str(&format!(
        "Detected: {}  hooks_installed={}/2  event_callbacks_total={}  event_callbacks_matched={}  poll_ticks={}\n",
        detection.detected,
        detection.hooks_installed,
        detection.event_callbacks_total,
        detection.event_callbacks_matched,
        detection.poll_ticks
    ));
    out.push_str(&format!(
        "Integrity level — Velora: {}, table: {}\n",
        detection.app_integrity_level.as_deref().unwrap_or("unknown"),
        detection.table_integrity_level.as_deref().unwrap_or("not tracked"),
    ));

    Ok(out)
}
