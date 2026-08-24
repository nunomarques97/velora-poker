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

pub struct AppState {
    pub conn: Mutex<Connection>,
    pub import: Mutex<ImportState>,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

impl AppState {
    pub fn new(db_path: std::path::PathBuf) -> rusqlite::Result<Self> {
        let conn = db::open(&db_path)?;

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
        })
    }
}
