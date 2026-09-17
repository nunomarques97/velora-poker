use serde::Serialize;
use tauri::{Emitter, State};

use crate::classification::{self, ClassificationResult};
use crate::db;
use crate::description_rules::{self, RuleResult};
use crate::hud::{self, HudProfile};
use crate::import;
use crate::overlay::{self, HotZone, OverlayMode};
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
    /// Structured rule results for the player profile drawer's
    /// TENDENCIES/EXPLOITS/CONFIDENCE sections (Phase 0+1), sorted by
    /// confidence descending. Empty when the `strategic-analysis` build flag
    /// is off, or when no rule cleared its opportunity floor — the drawer
    /// shows each section's own empty state for either case, never a blank
    /// or fabricated line. Independent of `classification` below, which is
    /// gated by the separate `auto-classification` flag.
    pub descriptions: Vec<RuleResult>,
    pub classification: ClassificationResult,
    pub snapshot: Option<PlayerSnapshotPayload>,
    /// Free-text note the user wrote about this player (Phase E), or
    /// `None` when none has been written. Rendered in the player profile
    /// drawer and the Players list only — deliberately *not* on the HUD
    /// overlay in v1, to protect overlay compactness (spec,  precedent).
    pub note: Option<String>,
    /// This player's absolute PokerStars seat number at the currently active
    /// table — `None` for the all-time roster (`get_players`), populated for
    /// `get_active_table_players` (Phase E seat mapping).
    pub seat: Option<i64>,
    /// The same seat rotated so the hero sits at offset 0 — the key a
    /// saved HUD card position is actually stored under, because
    /// "Auto-Center me" makes the hero, not any absolute seat number, the
    /// fixed screen anchor. `None` when the active hand has no recorded hero
    /// or table size, in which case no seat template can be resolved.
    pub seat_offset: Option<i64>,
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
    seat_offset: Option<i64>,
    include_note: bool,
) -> Result<PlayerPayload, String> {
    let (player_stats, opportunities) =
        stats::compute_player_stats_with_opportunities(conn, id).map_err(|e| e.to_string())?;
    let descriptions = description_rules::descriptions_for_player(&player_stats, &opportunities);
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
        descriptions,
        classification: classification_result,
        snapshot,
        note,
        seat,
        seat_offset,
    })
}

/// Full, unpaginated roster. `build_player_payload` runs several queries per
/// row (stats, opportunities, classification, snapshot, note), so this is
/// only safe at the small player counts this project started with — at the
/// thousands a bulk import produces it takes seconds and holds `AppState`'s
/// single `Mutex<Connection>` the whole time, which can stall the live
/// overlay's own per-hand refresh on every open table. No frontend view calls
/// this any more (`PlayersView`/`HudProfilesView` use `get_players_page`);
/// kept for any future caller that genuinely needs the entire set at once.
#[tauri::command]
pub fn get_players(state: State<AppState>) -> Result<Vec<PlayerPayload>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let rows = db::list_players(&conn).map_err(|e| e.to_string())?;
    let rules = classification::list_rules(&conn).map_err(|e| e.to_string())?;

    let mut players = Vec::with_capacity(rows.len());
    for row in rows {
        players.push(build_player_payload(
            &conn, &rules, row.id, row.name, row.hands, None, None, true,
        )?);
    }

    Ok(players)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayersPagePayload {
    pub players: Vec<PlayerPayload>,
    /// Total players matching `search` (or the whole roster when `search` is
    /// `None`) — the denominator for a "showing N of TOTAL" / "load more" UI,
    /// computed with a `COUNT(*)` rather than `players.len()` on the full set.
    pub total: i64,
}

/// Bounded page of the player roster, most-played-first, optionally filtered
/// by a case-insensitive substring of the name. Runs `build_player_payload`
/// (the expensive part — stats, classification, snapshot, note) only for the
/// `limit` rows actually returned, not the whole `players` table, so a tab
/// open with a huge roster costs the same regardless of how large the table
/// has grown.
#[tauri::command]
pub fn get_players_page(
    state: State<AppState>,
    offset: i64,
    limit: i64,
    search: Option<String>,
) -> Result<PlayersPagePayload, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let search = search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let total = db::count_players(&conn, search).map_err(|e| e.to_string())?;
    let rows =
        db::list_players_page(&conn, offset, limit, search).map_err(|e| e.to_string())?;
    let rules = classification::list_rules(&conn).map_err(|e| e.to_string())?;

    let mut players = Vec::with_capacity(rows.len());
    for row in rows {
        players.push(build_player_payload(
            &conn, &rules, row.id, row.name, row.hands, None, None, true,
        )?);
    }

    Ok(PlayersPagePayload { players, total })
}

/// How one overlay's DB queries are scoped to its own table.
///
/// Each overlay window passes the id of the table it was created for, and its
/// roster comes from that table's parsed name and nothing else. The one case
/// that needs a decision is a tracked table whose window title didn't parse
/// (`extract_table_name` returned `None` — the open tournament-title risk,
/// a known issue):
///
/// - With a single table tracked, the pre- global "latest hand anywhere"
///   fallback is kept. There is no other table for it to leak from, and
///   showing the right players beats showing none.
/// - With two or more, that fallback becomes the  bug itself: every
///   unparsed table would render whichever table played the most recent hand.
///   Those tables render empty instead, which is 's designed-safe failure
///   — never another table's players.
fn table_scope(table_id: u32) -> Option<TableScope> {
    match table_track::table_name_for(table_id) {
        Some(name) => Some(TableScope::Named(name)),
        None => {
            let tables = table_track::tracked_tables();
            let is_tracked = tables.iter().any(|t| t.id == table_id);
            match (is_tracked, tables.len()) {
                (true, 1) => Some(TableScope::GlobalLatest),
                _ => None,
            }
        }
    }
}

enum TableScope {
    Named(String),
    GlobalLatest,
}

impl TableScope {
    fn name(&self) -> Option<&str> {
        match self {
            TableScope::Named(name) => Some(name.as_str()),
            TableScope::GlobalLatest => None,
        }
    }
}

/// Players seated in the most recently imported hand *at this overlay's own
/// table* — what the live overlay renders, as opposed to `get_players`' full
/// all-time roster (used by the Players list and HUD profile views).
///
///  scoped this to the tracked table window's name so that a hand
/// completing on a *different* open table could not flip which players' cards
/// render here.  makes that scope per-overlay rather than process-wide:
/// `table_id` identifies the caller's own table, so N overlays each show N
/// different rosters. See `table_scope` for the unparsed-title case.
#[tauri::command]
pub fn get_active_table_players(
    state: State<AppState>,
    table_id: u32,
) -> Result<Vec<PlayerPayload>, String> {
    let Some(table_scope) = table_scope(table_id) else {
        return Ok(Vec::new());
    };
    let scope = table_scope.name().map(|s| s.to_string());
    // bounds "the active hand" to no earlier than this table window's
    // own first appearance, so a reused table name can never resolve to a
    // hand from an earlier sitting. `None` only if the table closed between
    // `table_scope` above and this lookup — vanishingly rare, and the
    // pre- unbounded behavior for that instant is harmless since the
    // overlay for a closed table is being torn down anyway.
    let since = table_track::table_for(table_id).map(|t| t.first_seen_at);
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let rows = db::list_active_table_players_with_seats(&conn, scope.as_deref(), since.as_deref())
        .map_err(|e| e.to_string())?;
    let rules = classification::list_rules(&conn).map_err(|e| e.to_string())?;

    let mut players = Vec::with_capacity(rows.len());
    for row in rows {
        // `include_note: false` — the overlay never renders notes, so this
        // path must not gain a per-player note lookup.
        players.push(build_player_payload(
            &conn,
            &rules,
            row.id,
            row.name,
            row.hands,
            row.seat,
            row.seat_offset,
            false,
        )?);
    }

    let active_hand = db::active_hand_info(&conn, scope.as_deref(), since.as_deref())
        .ok()
        .flatten();
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
            table_id,
            table_name: scope.or(hand_table_name),
            hand_id,
            player_count: players.len(),
        });
    }

    Ok(players)
}

/// This overlay's own table's max-players count (2/6/9-max) — the key the
/// seat-mapping template is looked up by (Phase E). `None` until at
/// least one hand has been imported at that table. Same per-table scoping as
/// `get_active_table_players`.
#[tauri::command]
pub fn get_active_table_max_players(
    state: State<AppState>,
    table_id: u32,
) -> Result<Option<i64>, String> {
    let Some(scope) = table_scope(table_id) else {
        return Ok(None);
    };
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::active_table_max_players(&conn, scope.name()).map_err(|e| e.to_string())
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
    build_player_payload(&conn, &rules, id, name, hands, None, None, true)
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
    build_player_payload(&conn, &rules, id, name, hands, None, None, true)
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
    build_player_payload(&conn, &rules, id, name, hands, None, None, true)
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
    // unset means on. HUD overlays are automatic now — the setting is a
    // kill switch, not the record of an "Open Overlay" click — so a fresh
    // install must show HUDs, not wait to be asked.
    let overlay_enabled = db::get_setting(&conn, settings::SETTING_OVERLAY_ENABLED)
        .map_err(|e| e.to_string())?
        .map(|value| value != "false")
        .unwrap_or(true);
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
// Classification rules (read-only)
// ---------------------------------------------------------------------

/// Serialisable view of `classification::ClassificationRule`.
///
/// The engine's own struct deliberately stays free of a `Serialize` derive —
/// this payload exists so the Settings page can explain the rules without
/// `classification/mod.rs` having to know a frontend exists. Field-for-field
/// copy: the UI renders the thresholds itself rather than receiving a
/// pre-formatted sentence, so a future change to the rule table shows up in
/// the UI automatically instead of going stale against hardcoded copy.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationRulePayload {
    pub id: String,
    pub label: String,
    pub color: String,
    pub priority: i64,
    pub min_hands: i64,
    pub vpip_min: Option<f64>,
    pub vpip_max: Option<f64>,
    pub pfr_min: Option<f64>,
    pub pfr_max: Option<f64>,
    pub three_bet_min: Option<f64>,
    pub three_bet_max: Option<f64>,
}

/// The archetype rules as the engine will actually apply them, already in
/// evaluation order (`list_rules` orders by ascending priority, and
/// `classification::classify` takes the first match).
#[tauri::command]
pub fn get_classification_rules(state: State<AppState>) -> Result<Vec<ClassificationRulePayload>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let rules = classification::list_rules(&conn).map_err(|e| e.to_string())?;
    Ok(rules
        .into_iter()
        .map(|r| ClassificationRulePayload {
            id: r.id,
            label: r.label,
            color: r.color,
            priority: r.priority,
            min_hands: r.min_hands,
            vpip_min: r.vpip_min,
            vpip_max: r.vpip_max,
            pfr_min: r.pfr_min,
            pfr_max: r.pfr_max,
            three_bet_min: r.three_bet_min,
            three_bet_max: r.three_bet_max,
        })
        .collect())
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

/// Live state of the HUD overlays as a whole: the global kill switch,
/// and every table currently tracked with whether its own overlay window is up.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayStatusPayload {
    /// The global on/off switch. When off, no table gets a HUD regardless of
    /// how many are open.
    pub enabled: bool,
    pub tables: Vec<TrackedTableStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedTableStatus {
    pub id: u32,
    pub name: Option<String>,
    pub minimized: bool,
    /// Whether this table's own overlay window exists and is on screen.
    pub overlay_visible: bool,
    /// Whether the user dismissed this table's overlay by hand (its own
    /// "Hide" button) and hasn't brought it back since — distinct from
    /// `overlay_visible: false` while a window is merely still starting up
    ///.
    pub dismissed: bool,
}

/// Every tracked table plus whether its HUD is up — what the HUD Profiles page
/// renders now that there is no single "Open Overlay" button to reflect.
#[tauri::command]
pub fn get_overlay_status(app_handle: tauri::AppHandle) -> OverlayStatusPayload {
    OverlayStatusPayload {
        enabled: overlay::manager::is_enabled(),
        tables: table_track::tracked_tables()
            .into_iter()
            .map(|t| TrackedTableStatus {
                id: t.id,
                name: t.name,
                minimized: t.minimized,
                overlay_visible: overlay::is_open(&app_handle, t.id),
                dismissed: overlay::manager::is_dismissed(t.id),
            })
            .collect(),
    }
}

/// The global HUD kill switch. Overlays are automatic — one appears for
/// every real table window and disappears with it, no click needed — so the
/// only thing left to press is "not right now" for the rest of *this*
/// session. Persisted within a session (e.g. surviving a hand refresh), but
/// `AppState::new` forces the setting back to enabled on every app startup —
/// this switch was never meant to be a permanent off with no visible sign
/// why the HUD stopped appearing.
#[tauri::command]
pub fn set_overlays_enabled(state: State<AppState>, enabled: bool) -> Result<(), String> {
    {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        db::set_setting(
            &conn,
            settings::SETTING_OVERLAY_ENABLED,
            if enabled { "true" } else { "false" },
        )
        .map_err(|e| e.to_string())?;
    }
    overlay::manager::request(overlay::manager::OverlayRequest::SetEnabled(enabled));
    Ok(())
}

/// Collapses one table's HUD content from the overlay's own "Hide" button,
/// leaving every other table alone. Not the kill switch, and —  —
/// not a window hide either: the window stays up and positioned, only its
/// content collapses to a "Show" pill in the same spot, so restoring it
/// never requires leaving the overlay. Comes back when the switch is cycled,
/// that table window is closed and reopened, or `show_overlay` is called.
#[tauri::command]
pub fn close_overlay(table_id: u32) {
    overlay::manager::request(overlay::manager::OverlayRequest::Dismiss { table_id });
}

/// Restores one table's full HUD content after it was collapsed by hand — the
/// overlay's own "Show" pill, and still reachable from the main
/// window's table list or the global hotkey. Leaves every other table and the
/// global switch untouched.
#[tauri::command]
pub fn show_overlay(table_id: u32) {
    overlay::manager::request(overlay::manager::OverlayRequest::Show { table_id });
}

#[tauri::command]
pub fn is_overlay_open(app_handle: tauri::AppHandle, table_id: u32) -> bool {
    overlay::is_open(&app_handle, table_id)
}

/// Whether this table's HUD content is currently collapsed — read by
/// the overlay itself on mount so it knows whether to paint the full HUD or
/// the "Show" pill from the very first render, before any event has fired.
#[tauri::command]
pub fn is_overlay_dismissed(table_id: u32) -> bool {
    overlay::manager::is_dismissed(table_id)
}

/// Switches one table's overlay between its table-interactive default and
/// reposition mode. Replaces the old `set_overlay_click_through`, whose
/// single window-wide flag could not express "the table is clickable *and* so
/// are the overlay's own controls" — see `overlay::hittest`.
#[tauri::command]
pub fn set_overlay_mode(
    app_handle: tauri::AppHandle,
    table_id: u32,
    mode: OverlayMode,
) -> Result<(), String> {
    overlay::set_mode(&app_handle, table_id, mode)
}

/// Puts *every* overlay in one mode — the main window's single Reposition
/// control, which has no one table to act on. Reports the first failure but
/// only after trying them all: leaving half the tables in reposition mode is
/// the state that stops clicks reaching a real-money table.
#[tauri::command]
pub fn set_all_overlay_modes(
    app_handle: tauri::AppHandle,
    mode: OverlayMode,
) -> Result<(), String> {
    let mut first_error = None;
    for table in table_track::tracked_tables() {
        // A table with no overlay right now — dismissed, or the HUDs are
        // switched off — is skipped, not an error. Reporting one would fail the
        // whole call and leave the button's label describing a mode change that
        // did happen on every table that has an overlay.
        if !overlay::is_open(&app_handle, table.id) {
            continue;
        }
        if let Err(err) = overlay::set_mode(&app_handle, table.id, mode) {
            first_error.get_or_insert(err);
        }
    }
    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// One table's overlay interaction mode, read from the native hit-test state
/// rather than any window's React flag — this is what a freshly mounted
/// overlay seeds itself from.
#[tauri::command]
pub fn get_overlay_mode(table_id: u32) -> OverlayMode {
    overlay::current_mode(table_id)
}

/// `reposition` if any overlay is repositioning — what the main window's
/// single control for every table reads its label from.
#[tauri::command]
pub fn get_any_overlay_mode() -> OverlayMode {
    overlay::any_reposition_mode()
}

/// Registers the rectangles that stay clickable in `Normal` mode on one
/// overlay: that overlay's own control bar and each of its visible cards'
/// pagination-dot clusters. Reported by each overlay frontend in CSS pixels
/// relative to its own viewport, and re-reported on every layout change — a
/// rect must never outlive the control it describes, or the overlay would keep
/// swallowing table clicks at a point where nothing of Velora's is drawn any
/// more.
#[tauri::command]
pub fn set_overlay_hot_zones(
    app_handle: tauri::AppHandle,
    table_id: u32,
    zones: Vec<HotZone>,
) -> Result<(), String> {
    overlay::set_hot_zones(&app_handle, table_id, &zones)
}

/// Saves one player's manually-dragged card position.
///
/// Deliberately *not* table-scoped, unlike its neighbours above. A
/// `hud_positions` row is keyed by player and holds a fraction of the overlay
/// window, which mirrors whichever table that player is sitting at — there is
/// no table-specific component to store. The same player showing up at two
/// tables at once gets the same card position at both, which is the same
/// deliberate sharing `seat_templates` already has across same-sized tables
///.
///
/// `x`/`y` are clamped to 0..1 on write. One real row had drifted to
/// `x = 1.104`, which would have parked that player's card
/// off the right edge of every overlay it ever appeared on.
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
    /// Seat offset from the hero, not an absolute PokerStars seat
    /// number — see `db::relative_seat`.
    pub seat_offset: i64,
    pub x: f64,
    pub y: f64,
}

/// One max-players size's calibrated seat layout, `x`/`y` as fractions
/// (0..1) of the overlay/table window — same coordinate scheme as
/// `hud_positions`. Keyed by hero-relative seat offset .
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
                .map(|r| SeatTemplatePayload { seat_offset: r.seat_offset, x: r.x, y: r.y })
                .collect()
        })
}

/// Broadcast (payload: the `max_players` size) whenever a seat template is
/// calibrated, so every *other* overlay on a table of that size picks the new
/// layout up.
///
/// A seat template has always been shared across same-sized tables, but
/// with one overlay this was invisible: the window you dragged in was the only
/// one there was. With one overlay per table, a drag on table A must move the
/// same seat's card on tables B and C — and nothing else refreshes them, since
/// their own `refresh()` only runs on mount, on a new hand, or on being shown.
pub const SEAT_TEMPLATES_EVENT: &str = "seat-templates-changed";

#[tauri::command]
pub fn save_seat_template(
    app_handle: tauri::AppHandle,
    state: State<AppState>,
    max_players: i64,
    seat_offset: i64,
    x: f64,
    y: f64,
) -> Result<(), String> {
    {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        db::set_seat_template(&conn, max_players, seat_offset, x, y).map_err(|e| e.to_string())?;
    }
    let _ = app_handle.emit(SEAT_TEMPLATES_EVENT, max_players);
    Ok(())
}

// ---------------------------------------------------------------------
// Table window detection status (Phase E)
// ---------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDetectionStatusPayload {
    pub detected: bool,
    /// How many real, logged-in PokerStars table windows are open right now.
    /// Since  each one has its own overlay, so this is simply how many HUDs
    /// are running — it is no longer the count behind an apology.
    pub table_window_count: u32,
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

/// Whether any PokerStars table window is currently found and being tracked
/// — surfaced in Settings so the user can see detection is (or isn't)
/// working without having to open the overlay and drag a card to find out.
#[tauri::command]
pub fn get_table_detection_status() -> TableDetectionStatusPayload {
    let debug = table_track::debug_snapshot();
    let tables = table_track::tracked_tables();
    TableDetectionStatusPayload {
        detected: !tables.is_empty(),
        table_window_count: tables.len() as u32,
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
pub fn get_diagnostics_report(
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<String, String> {
    let mut out = String::new();
    out.push_str(&format!(
        "Velora diagnostics report — generated {}\n",
        db::now_iso()
    ));
    out.push_str("========================================\n\n");

    let tables = table_track::tracked_tables();
    let conn = state.conn.lock().map_err(|e| e.to_string())?;

    // Ingestion integrity (work unit 2). Recomputed from the data on every
    // report, so it cannot go stale or claim a clean database that isn't.
    out.push_str("-- Ingestion integrity --\n");
    match db::integrity_counters(&conn) {
        Ok((no_players, orphan_actions, phantoms, no_position)) => {
            out.push_str(&format!(
                "Hands stored with no players: {no_players}\n\
                 Action rows with no matching player row: {orphan_actions}\n\
                 Players never seen in a stored hand: {phantoms}\n\
                 Player rows with no derived position: {no_position}\n"
            ));
            if no_players == 0 && orphan_actions == 0 && phantoms == 0 && no_position == 0 {
                out.push_str("All four counters are zero — no known ingestion corruption.\n");
            } else {
                out.push_str(
                    "One or more counters is non-zero — please send this report to the maintainer.\n",
                );
            }
        }
        Err(err) => out.push_str(&format!("(could not compute: {err})\n")),
    }
    match db::import_problem_summary(&conn) {
        Ok(problems) if problems.is_empty() => {
            out.push_str("No hand has failed the import integrity gate.\n")
        }
        Ok(problems) => {
            out.push_str("Import integrity gate findings (code x count):\n");
            for (severity, code, count, detail) in problems {
                out.push_str(&format!(
                    "  [{severity}] {code} x{count} — most recent: {detail}\n"
                ));
            }
        }
        Err(err) => out.push_str(&format!("(could not list gate findings: {err})\n")),
    }
    out.push('\n');

    out.push_str("-- Tracked tables (one overlay each) --\n");
    if tables.is_empty() {
        out.push_str("(none — no real PokerStars table window is open)\n");
    }
    for table in &tables {
        let scope = table_scope(table.id);
        out.push_str(&format!(
            "Table {} — window {:#x} rect=(x={}, y={}, w={}, h={}){}\n",
            table.id,
            table.hwnd,
            table.rect.x,
            table.rect.y,
            table.rect.width,
            table.rect.height,
            if table.minimized { " MINIMIZED" } else { "" },
        ));
        match (&table.name, &scope) {
            (Some(name), _) => out.push_str(&format!(
                "  Name: \"{name}\" (parsed from this window's own title)\n"
            )),
            (None, Some(TableScope::GlobalLatest)) => out.push_str(
                "  Name: could not be parsed from this window's title — as the only open table \
                 it falls back to the most recently imported hand from any table.\n",
            ),
            (None, _) => out.push_str(
                "  Name: could not be parsed from this window's title, and other tables are \
                 open — this overlay renders NO players rather than risk showing another \
                 table's.\n",
            ),
        }
        out.push_str(&format!(
            "  Overlay window: {}\n",
            if overlay::is_open(&app_handle, table.id) {
                "up"
            } else {
                "not shown"
            }
        ));
        let scope_name = scope.as_ref().and_then(|s| s.name());
        let renders_players = scope.is_some();
        let since = Some(table.first_seen_at.as_str());
        match db::active_hand_info(&conn, scope_name, since).map_err(|e| e.to_string())? {
            Some(h) => out.push_str(&format!(
                "  Resolved active hand: {} (table=\"{}\", tournament={}, played_at={})\n",
                h.hand_id,
                h.table_name.as_deref().unwrap_or("?"),
                h.tournament_id.as_deref().unwrap_or("-"),
                h.played_at.as_deref().unwrap_or("?"),
            )),
            None => out
                .push_str("  Resolved active hand: none (no hands imported for this table yet)\n"),
        }
        let player_count = if renders_players {
            db::list_active_table_players_with_seats(&conn, scope_name, since)
                .map(|rows| rows.len())
                .unwrap_or(0)
        } else {
            0
        };
        out.push_str(&format!("  Players currently rendered: {player_count}\n"));
    }
    out.push('\n');

    out.push_str("-- Overlay data refreshes (last 20) --\n");
    {
        let log = state.refresh_log.lock().map_err(|e| e.to_string())?;
        if log.is_empty() {
            out.push_str("(none yet — no overlay has requested active-table data this run)\n");
        } else {
            for entry in log.iter() {
                out.push_str(&format!(
                    "{}  table#{}  name={}  hand={}  players={}\n",
                    entry.at,
                    entry.table_id,
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
                "{}  table#{}  source={}  rect=(x={}, y={}, w={}, h={})\n",
                entry.at,
                entry.table_id,
                entry.source,
                entry.rect.x,
                entry.rect.y,
                entry.rect.width,
                entry.rect.height
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
        "Detected: {}  hooks_installed={}/{}  event_callbacks_total={}  event_callbacks_matched={}  poll_ticks={}\n",
        detection.detected,
        detection.hooks_installed,
        table_track::expected_hook_count(),
        detection.event_callbacks_total,
        detection.event_callbacks_matched,
        detection.poll_ticks
    ));
    out.push_str(&format!(
        "Integrity level — Velora: {}, table: {}\n",
        detection.app_integrity_level.as_deref().unwrap_or("unknown"),
        detection.table_integrity_level.as_deref().unwrap_or("not tracked"),
    ));
    out.push_str(&format!(
        "Open PokerStars table windows: {} (one overlay each)\n",
        detection.table_window_count
    ));
    let (created, destroyed, failures) = overlay::manager::window_lifecycle_counts();
    out.push_str(&format!(
        "Overlay windows built this run: {created}  destroyed: {destroyed}  failed to build: {failures}\n"
    ));
    out.push_str(&format!(
        "HUD overlays globally: {}\n\n",
        if overlay::manager::is_enabled() {
            "enabled"
        } else {
            "disabled (kill switch)"
        }
    ));

    // the facts that decide whether a click lands on the table or on
    // Velora. "Hot zones" are the rectangles that stay clickable in normal
    // mode — an overlay's control bar plus one per visible card's pagination
    // dots — so a count of 0 while that overlay is up and showing cards is
    // itself the symptom, rather than something to infer from behaviour. Per
    // overlay , because "the mode" is no longer one value.
    out.push_str("-- Overlay interaction --\n");
    let probe = overlay::hit_test_probe();
    if probe.overlays.is_empty() {
        out.push_str("(no overlay windows registered with the hit tester)\n");
    }
    for overlay_probe in &probe.overlays {
        out.push_str(&format!(
            "{}: hwnd={}  mode={:?}  hot zones={}  clicks passing through={}\n",
            overlay_probe.label,
            overlay_probe.hwnd,
            overlay_probe.mode,
            overlay_probe.hot_zones,
            overlay_probe.click_through,
        ));
    }
    out.push_str(&format!(
        "Cursor tracker: ticks={}  style writes={}\n",
        probe.ticks, probe.style_writes
    ));

    Ok(out)
}
