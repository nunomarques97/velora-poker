use std::path::{Path, PathBuf};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::Manager;

use crate::db;
use crate::import;
use crate::state::AppState;

/// Starts watching `dir` (non-recursively) for created/modified `.txt`
/// files, importing any newly appearing hands as they are written by
/// PokerStars. The returned watcher must be kept alive (e.g. stored in
/// [`AppState`]) for watching to continue.
pub fn start_watching(
    app_handle: tauri::AppHandle,
    dir: PathBuf,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
            return;
        }
        for path in &event.paths {
            if path.extension().map_or(false, |ext| ext == "txt") {
                handle_file_event(&app_handle, path);
            }
        }
    })?;

    watcher.watch(&dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

fn handle_file_event(app_handle: &tauri::AppHandle, path: &Path) {
    let state = app_handle.state::<AppState>();
    let summary = {
        let mut conn = match state.conn.lock() {
            Ok(conn) => conn,
            Err(_) => return,
        };
        match import::import_file(&mut conn, path) {
            Ok(summary) => summary,
            Err(err) => {
                eprintln!("import error for {path:?}: {err}");
                return;
            }
        }
    };

    if summary.hands_imported > 0 {
        if let Ok(mut import_state) = state.import.lock() {
            import_state.last_import_at = Some(db::now_iso());
        }
    }
}
