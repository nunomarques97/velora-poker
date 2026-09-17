use std::collections::VecDeque;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::db;
use crate::settings;

/// Tracks the pieces of import state that cannot be derived by a plain
/// database query (hand counts are always read live from SQLite instead).
#[derive(Debug, Clone, Default)]
pub struct ImportState {
    pub configured_dir: Option<String>,
    pub last_import_at: Option<String>,
    pub parser_status: String,
}

/// One entry in the  diagnostics refresh log — every time an overlay asked
/// for its table's players, what table it resolved to, and how many players
/// came back. Kept so a user's "Copy Diagnostics" dump shows exactly what
/// each overlay has been rendering and when, without needing him to
/// characterize the bug himself. `table_id` was added in with several
/// overlays refreshing into one log, the entries interleave and the table's
/// own id is what tells them apart.
#[derive(Debug, Clone)]
pub struct RefreshLogEntry {
    pub at: String,
    pub table_id: u32,
    pub table_name: Option<String>,
    pub hand_id: Option<String>,
    pub player_count: usize,
}

pub const REFRESH_LOG_CAP: usize = 20;

pub struct AppState {
    pub conn: Mutex<Connection>,
    pub import: Mutex<ImportState>,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    pub refresh_log: Mutex<VecDeque<RefreshLogEntry>>,
}

impl AppState {
    pub fn new(db_path: std::path::PathBuf) -> rusqlite::Result<Self> {
        let conn = db::open(&db_path)?;

        // The overlay kill switch is a "not right now" for the current
        // session, not a permanent off — a past session left toggled off must
        // never silently suppress every HUD on a future launch with no
        // visible sign why. Forced back to enabled here, before the Tauri
        // builder exists and before any window or command can read the
        // setting, so there is no earlier point where a stale "off" could be
        // observed.
        db::set_setting(&conn, settings::SETTING_OVERLAY_ENABLED, "true")?;

        let configured_dir = db::get_setting(&conn, settings::SETTING_HAND_HISTORY_DIR)?.or_else(|| {
            settings::detect_default_hand_history_dir().map(|p| p.to_string_lossy().to_string())
        });

        let parser_status = if configured_dir.is_some() {
            "watching".to_string()
        } else {
            "not_configured".to_string()
        };

        let import = ImportState {
            configured_dir,
            last_import_at: None,
            parser_status,
        };

        Ok(Self {
            conn: Mutex::new(conn),
            import: Mutex::new(import),
            watcher: Mutex::new(None),
            refresh_log: Mutex::new(VecDeque::new()),
        })
    }
}
