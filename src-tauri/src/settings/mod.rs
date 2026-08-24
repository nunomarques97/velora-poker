use std::path::{Path, PathBuf};

pub const SETTING_HAND_HISTORY_DIR: &str = "hand_history_dir";

/// Best-effort detection of a PokerStars hand-history folder in common
/// Windows install locations. This is only a starting suggestion — the
/// directory is always user-configurable in Settings.
pub fn detect_default_hand_history_dir() -> Option<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        bases.push(PathBuf::from(local));
    }
    if let Ok(roaming) = std::env::var("APPDATA") {
        bases.push(PathBuf::from(roaming));
    }

    for base in bases {
        let entries = match std::fs::read_dir(&base) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() && name.starts_with("PokerStars") {
                let candidate = path.join("HandHistory");
                if candidate.is_dir() {
                    if let Some(resolved) = resolve_hand_history_dir(&candidate) {
                        return Some(resolved);
                    }
                }
            }
        }
    }

    None
}

/// If `dir` directly contains `.txt` files, use it as-is. Otherwise PokerStars
/// commonly nests hand histories one level deeper under a per-screen-name
/// folder, so fall back to the first subdirectory found.
fn resolve_hand_history_dir(dir: &Path) -> Option<PathBuf> {
    if contains_txt_files(dir) {
        return Some(dir.to_path_buf());
    }

    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            return Some(path);
        }
    }

    None
}

fn contains_txt_files(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.path().extension().map_or(false, |ext| ext == "txt"))
        })
        .unwrap_or(false)
}
