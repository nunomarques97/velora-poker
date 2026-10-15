use std::path::{Path, PathBuf};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{Emitter, Manager};

use crate::db;
use crate::import;
use crate::state::AppState;

/// Starts watching `dir` recursively for created/modified `.txt` files,
/// importing any newly appearing hands as they are written by PokerStars.
/// Recursive so that per-screen-name subdirectories (e.g.
/// `HandHistory\<ScreenName>\*.txt`, including ones created after watching
/// starts) are picked up without the caller needing to know the screen
/// name(s) in advance. The returned watcher must be kept alive (e.g. stored
/// in [`AppState`]) for watching to continue.
pub fn start_watching(
    app_handle: tauri::AppHandle,
    dir: PathBuf,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let event = match res {
            Ok(event) => event,
            Err(err) => {
                // Surfaced in Settings instead of dropped: a watcher that
                // stopped delivering events leaves every HUD frozen.
                eprintln!("hand history watcher error: {err}");
                let state = app_handle.state::<AppState>();
                if let Ok(mut import_state) = state.import.lock() {
                    import_state.parser_status = import::status_for_watcher_error(&err.to_string());
                }
                return;
            }
        };
        if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
            return;
        }
        for path in &event.paths {
            if path.extension().map_or(false, |ext| ext == "txt") {
                handle_file_event(&app_handle, path);
            }
        }
    })?;

    watcher.watch(&dir, RecursiveMode::Recursive)?;
    Ok(watcher)
}

fn handle_file_event(app_handle: &tauri::AppHandle, path: &Path) {
    let state = app_handle.state::<AppState>();
    let summary = {
        // A lock poisoned by a panic elsewhere is recovered: returning here
        // would silently stop every live import for the rest of the session.
        let mut conn = state
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match import::import_file(&mut conn, path) {
            Ok(summary) => summary,
            Err(err) => {
                eprintln!("import error for {path:?}: {err}");
                return;
            }
        }
    };

    if let Ok(mut import_state) = state.import.lock() {
        if let Some(status) = import::status_after_watched_import(&import_state.parser_status) {
            import_state.parser_status = status;
        }
        if summary.hands_imported > 0 {
            import_state.last_import_at = Some(db::now_iso());
        }
    }

    if summary.hands_imported > 0 {
        // Event-driven refresh: the frontend (main window and overlay) listen
        // for this instead of polling SQLite on a timer.
        let _ = app_handle.emit("hands-imported", summary.hands_imported);
    }
}
