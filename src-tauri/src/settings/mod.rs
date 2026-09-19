use std::path::{Path, PathBuf};

use serde::Serialize;

pub const SETTING_HAND_HISTORY_DIR: &str = "hand_history_dir";
pub const SETTING_POKER_ROOM: &str = "poker_room";
pub const SETTING_ONBOARDING_COMPLETE: &str = "onboarding_complete";
pub const SETTING_OVERLAY_ENABLED: &str = "overlay_enabled";
/// Whether the user has confirmed PokerStars' own "Auto-Center" table
/// option is enabled — the seat-mapping template system requires it (hero always bottom-middle, other seats in a fixed rotation)
/// and has no way to detect it itself, so this is a user-declared setting,
/// documented as a prerequisite in Settings. Defaults to unset/false: without
/// this confirmation, seat mapping falls back to today's manual per-card
/// placement (still relative-to-table, so window-following still applies).
pub const SETTING_AUTO_CENTER_ENABLED: &str = "auto_center_enabled";

/// A candidate PokerStars hand-history folder found during auto-detection,
/// ranked by how much real evidence it holds that it's the right one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedDir {
    pub path: String,
    pub hand_file_count: i64,
    pub screen_names: Vec<String>,
}

/// Best-effort, ranked detection of PokerStars hand-history folders across
/// common Windows install locations (`%LOCALAPPDATA%`/`%APPDATA%`, any
/// folder starting with `PokerStars` — covers regional installs like
/// `PokerStars.PT`/`PokerStars.FR`/`PokerStars.ES`). Purely filesystem
/// checks: no OCR, no process inspection, no registry reads. Always
/// user-overridable in Settings/onboarding.
pub fn detect_candidates() -> Vec<DetectedDir> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        bases.push(PathBuf::from(local));
    }
    if let Ok(roaming) = std::env::var("APPDATA") {
        bases.push(PathBuf::from(roaming));
    }

    let mut candidates = Vec::new();
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
                    let (hand_file_count, screen_names) = scan_hand_history_dir(&candidate);
                    candidates.push(DetectedDir {
                        path: candidate.to_string_lossy().to_string(),
                        hand_file_count,
                        screen_names,
                    });
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.hand_file_count.cmp(&a.hand_file_count));
    candidates
}

/// Convenience wrapper returning just the top-ranked candidate, used at app
/// startup to pre-fill a default before the user has configured anything.
pub fn detect_default_hand_history_dir() -> Option<PathBuf> {
    detect_candidates().into_iter().next().map(|c| PathBuf::from(c.path))
}

fn scan_hand_history_dir(dir: &Path) -> (i64, Vec<String>) {
    let mut count = 0i64;
    let mut screen_names = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return (0, screen_names),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            screen_names.push(entry.file_name().to_string_lossy().to_string());
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                count += sub_entries
                    .flatten()
                    .filter(|e| e.path().extension().map_or(false, |ext| ext == "txt"))
                    .count() as i64;
            }
        } else if path.extension().map_or(false, |ext| ext == "txt") {
            count += 1;
        }
    }

    (count, screen_names)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirValidation {
    pub is_valid: bool,
    pub hand_file_count: i64,
    pub message: String,
}

/// Validates a user-chosen (or auto-detected) directory: it must exist, and
/// contain at least one `.txt` file either directly or under a
/// per-screen-name subfolder — matching what the recursive scanner/watcher
/// will actually pick up.
pub fn validate_hand_history_dir(path: &Path) -> DirValidation {
    if !path.is_dir() {
        return DirValidation {
            is_valid: false,
            hand_file_count: 0,
            message: "That folder doesn't exist.".to_string(),
        };
    }

    let (count, _) = scan_hand_history_dir(path);
    let direct_txt = std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "txt"))
                .count() as i64
        })
        .unwrap_or(0);
    let total = count + direct_txt;

    if total == 0 {
        DirValidation {
            is_valid: true,
            hand_file_count: 0,
            message: "Folder is valid but no hand history files were found yet.".to_string(),
        }
    } else {
        DirValidation {
            is_valid: true,
            hand_file_count: total,
            message: format!("Found {total} hand history file(s)."),
        }
    }
}
