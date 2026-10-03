use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

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

// ---------------------------------------------------------------------
// Side-panel shortcut
// ---------------------------------------------------------------------

/// The user's side-panel shortcut, stored canonical ("Ctrl+Alt+P"). Unset
/// means `DEFAULT_PANEL_SHORTCUT`.
pub const SETTING_PANEL_SHORTCUT: &str = "panel_shortcut";

/// Same Ctrl+Alt family as the HUD toggle, so both are learnt together.
pub const DEFAULT_PANEL_SHORTCUT: &str = "Ctrl+Alt+P";

/// The fixed HUD toggle (`table_track::toggle_hud_for_foreground_table`).
/// Not configurable, so the panel shortcut may never take it.
pub const HUD_TOGGLE_SHORTCUT: &str = "Ctrl+Alt+H";

/// A shortcut that passed `parse_shortcut`: what to register, and the
/// canonical text to store and show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedShortcut {
    pub shortcut: Shortcut,
}

impl ParsedShortcut {
    /// "Ctrl+Alt+Shift+Key", modifiers always in that order.
    pub fn canonical(&self) -> String {
        let mods = self.shortcut.mods;
        let mut parts: Vec<&str> = Vec::new();
        if mods.contains(Modifiers::CONTROL) {
            parts.push("Ctrl");
        }
        if mods.contains(Modifiers::ALT) {
            parts.push("Alt");
        }
        if mods.contains(Modifiers::SHIFT) {
            parts.push("Shift");
        }
        parts.push(key_name(self.shortcut.key));
        parts.join("+")
    }
}

/// The keys a shortcut may use: letters, digits and F1-F12. Anything else
/// (arrows, Space, Tab, punctuation) is either layout-dependent or already
/// means something in every app.
const KEYS: &[(&str, Code)] = &[
    ("A", Code::KeyA), ("B", Code::KeyB), ("C", Code::KeyC), ("D", Code::KeyD),
    ("E", Code::KeyE), ("F", Code::KeyF), ("G", Code::KeyG), ("H", Code::KeyH),
    ("I", Code::KeyI), ("J", Code::KeyJ), ("K", Code::KeyK), ("L", Code::KeyL),
    ("M", Code::KeyM), ("N", Code::KeyN), ("O", Code::KeyO), ("P", Code::KeyP),
    ("Q", Code::KeyQ), ("R", Code::KeyR), ("S", Code::KeyS), ("T", Code::KeyT),
    ("U", Code::KeyU), ("V", Code::KeyV), ("W", Code::KeyW), ("X", Code::KeyX),
    ("Y", Code::KeyY), ("Z", Code::KeyZ),
    ("0", Code::Digit0), ("1", Code::Digit1), ("2", Code::Digit2), ("3", Code::Digit3),
    ("4", Code::Digit4), ("5", Code::Digit5), ("6", Code::Digit6), ("7", Code::Digit7),
    ("8", Code::Digit8), ("9", Code::Digit9),
    ("F1", Code::F1), ("F2", Code::F2), ("F3", Code::F3), ("F4", Code::F4),
    ("F5", Code::F5), ("F6", Code::F6), ("F7", Code::F7), ("F8", Code::F8),
    ("F9", Code::F9), ("F10", Code::F10), ("F11", Code::F11), ("F12", Code::F12),
];

fn key_name(code: Code) -> &'static str {
    KEYS.iter()
        .find(|(_, c)| *c == code)
        .map(|(name, _)| *name)
        .unwrap_or("?")
}

/// The fixed HUD toggle, as registered in `lib.rs`.
pub fn hud_toggle_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyH)
}

/// Parses and validates a user-typed shortcut such as "ctrl + shift + p".
/// Case and spaces around `+` don't matter; everything else is rejected with
/// a message the Settings page shows as is. The rules keep a global shortcut
/// from stealing a key from every other app: Ctrl or Alt is required, a
/// letter or digit needs two modifiers (Ctrl+C alone would break copying
/// everywhere), the Windows key and Alt+F4 are left to Windows, and the HUD
/// toggle is taken.
pub fn parse_shortcut(input: &str) -> Result<ParsedShortcut, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(format!("Type a shortcut, for example {DEFAULT_PANEL_SHORTCUT}."));
    }

    let mut mods = Modifiers::empty();
    let mut key: Option<(&str, Code)> = None;
    for token in trimmed.split('+').map(str::trim) {
        if token.is_empty() {
            return Err(format!(
                "\"{trimmed}\" has an empty part. Join the keys with +, for example {DEFAULT_PANEL_SHORTCUT}."
            ));
        }
        let modifier = match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Modifiers::CONTROL),
            "alt" => Some(Modifiers::ALT),
            "shift" => Some(Modifiers::SHIFT),
            "win" | "windows" | "super" | "meta" | "cmd" => {
                return Err(
                    "The Windows key is left to the shortcuts of Windows itself. Use Ctrl, Alt or Shift."
                        .to_string(),
                );
            }
            _ => None,
        };
        if let Some(modifier) = modifier {
            if key.is_some() {
                return Err(format!(
                    "Put the modifiers before the key, for example {DEFAULT_PANEL_SHORTCUT}."
                ));
            }
            if mods.contains(modifier) {
                return Err(format!("\"{token}\" appears twice in \"{trimmed}\"."));
            }
            mods.insert(modifier);
            continue;
        }
        if key.is_some() {
            return Err(format!(
                "Use one key with its modifiers, for example {DEFAULT_PANEL_SHORTCUT}."
            ));
        }
        let upper = token.to_ascii_uppercase();
        match KEYS.iter().find(|(name, _)| *name == upper) {
            Some(found) => key = Some(*found),
            None => {
                return Err(format!(
                    "\"{token}\" isn't a key Velora can use. Use a letter, a digit or F1 to F12."
                ));
            }
        }
    }

    let Some((name, code)) = key else {
        return Err(format!(
            "Add a key after the modifiers, for example {DEFAULT_PANEL_SHORTCUT}."
        ));
    };
    if mods.is_empty() {
        return Err(format!(
            "Add a modifier: a shortcut needs Ctrl or Alt, for example {DEFAULT_PANEL_SHORTCUT}."
        ));
    }
    if !mods.intersects(Modifiers::CONTROL | Modifiers::ALT) {
        return Err(
            "Add Ctrl or Alt: with Shift alone the key would stop typing in every other app."
                .to_string(),
        );
    }
    let is_function_key = name.len() > 1 && name.starts_with('F');
    if !is_function_key && mods.bits().count_ones() < 2 {
        return Err(format!(
            "Use two modifiers with a letter or digit, for example {DEFAULT_PANEL_SHORTCUT}: with one, that key would stop working in every other app."
        ));
    }

    let shortcut = Shortcut::new(Some(mods), code);
    if shortcut == hud_toggle_shortcut() {
        return Err(format!(
            "{HUD_TOGGLE_SHORTCUT} already shows and hides the HUD of the table in front. Pick another combination."
        ));
    }
    if mods == Modifiers::ALT && code == Code::F4 {
        return Err("Alt+F4 closes windows. Pick another combination.".to_string());
    }
    Ok(ParsedShortcut { shortcut })
}

/// The shortcut to register at startup: the saved one, or the default when
/// nothing is saved. A saved value that no longer validates falls back to the
/// default, with the reason to show in Settings.
pub fn startup_panel_shortcut(saved: Option<&str>) -> (ParsedShortcut, Option<String>) {
    let default = || parse_shortcut(DEFAULT_PANEL_SHORTCUT).expect("default panel shortcut is valid");
    match saved {
        None => (default(), None),
        Some(saved) => match parse_shortcut(saved) {
            Ok(parsed) => (parsed, None),
            Err(reason) => (
                default(),
                Some(format!(
                    "The saved shortcut \"{saved}\" is not valid ({reason}) Using {DEFAULT_PANEL_SHORTCUT} instead."
                )),
            ),
        },
    }
}

/// Replaces the registered shortcut `current` with `next` without ever
/// leaving the user with neither: `next` is registered first and `current`
/// is only released once that worked. A failed registration (another app owns
/// the combination) returns the error and leaves `current` registered.
/// Asking for what is already registered changes nothing.
pub fn swap_shortcut<K: Copy + PartialEq>(
    current: Option<K>,
    next: K,
    register: impl FnOnce(K) -> Result<(), String>,
    unregister: impl FnOnce(K),
) -> Result<(), String> {
    if current == Some(next) {
        return Ok(());
    }
    register(next)?;
    if let Some(old) = current {
        unregister(old);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(input: &str) -> String {
        parse_shortcut(input)
            .map(|p| p.canonical())
            .unwrap_or_else(|e| format!("ERR {e}"))
    }

    #[test]
    fn default_is_ctrl_alt_p() {
        let parsed = parse_shortcut(DEFAULT_PANEL_SHORTCUT).unwrap();
        assert_eq!(
            parsed.shortcut,
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyP)
        );
        assert_eq!(parsed.canonical(), "Ctrl+Alt+P");
    }

    #[test]
    fn parsing_ignores_case_spaces_and_modifier_order() {
        assert_eq!(canonical("ctrl + shift + p"), "Ctrl+Shift+P");
        assert_eq!(canonical("Shift+Control+p"), "Ctrl+Shift+P");
        assert_eq!(canonical("alt+ctrl+7"), "Ctrl+Alt+7");
        assert_eq!(canonical("Ctrl+F9"), "Ctrl+F9");
        assert_eq!(canonical("  Alt+Shift+f12 "), "Alt+Shift+F12");
    }

    #[test]
    fn rejects_no_modifier() {
        let err = parse_shortcut("P").unwrap_err();
        assert!(err.starts_with("Add a modifier"), "{err}");
        assert!(parse_shortcut("F9").unwrap_err().starts_with("Add a modifier"));
    }

    #[test]
    fn rejects_shift_only_and_single_modifier_letters() {
        assert!(parse_shortcut("Shift+F9").unwrap_err().starts_with("Add Ctrl or Alt"));
        assert!(parse_shortcut("Ctrl+C").unwrap_err().starts_with("Use two modifiers"));
        assert!(parse_shortcut("Alt+1").unwrap_err().starts_with("Use two modifiers"));
    }

    #[test]
    fn rejects_unknown_keys() {
        for input in [
            "Ctrl+Alt+Space",
            "Ctrl+Alt+Esc",
            "Ctrl+Alt+F13",
            "Ctrl+Alt+PP",
            "Ctrl+Alt+\u{e9}",
            "Ctrl+Alt+;",
        ] {
            let err = parse_shortcut(input).unwrap_err();
            assert!(err.contains("isn't a key Velora can use"), "{input}: {err}");
        }
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(parse_shortcut("").unwrap_err().starts_with("Type a shortcut"));
        assert!(parse_shortcut("   ").unwrap_err().starts_with("Type a shortcut"));
        assert!(parse_shortcut("Ctrl++P").unwrap_err().contains("empty part"));
        assert!(parse_shortcut("Ctrl+Alt+").unwrap_err().contains("empty part"));
        assert!(parse_shortcut("Ctrl+Alt").unwrap_err().starts_with("Add a key"));
        assert!(parse_shortcut("Ctrl+Alt+P+Q").unwrap_err().starts_with("Use one key"));
        assert!(parse_shortcut("P+Ctrl+Alt").unwrap_err().starts_with("Put the modifiers before"));
        assert!(parse_shortcut("Ctrl+ctrl+P").unwrap_err().contains("appears twice"));
    }

    #[test]
    fn rejects_the_hud_toggle_in_any_spelling() {
        for input in ["Ctrl+Alt+H", "alt + ctrl + h", "CONTROL+ALT+H"] {
            let err = parse_shortcut(input).unwrap_err();
            assert!(
                err.starts_with("Ctrl+Alt+H already shows and hides the HUD"),
                "{input}: {err}"
            );
        }
        // Adding Shift makes it a different combination.
        assert_eq!(canonical("Ctrl+Alt+Shift+H"), "Ctrl+Alt+Shift+H");
    }

    #[test]
    fn rejects_windows_key_and_alt_f4() {
        assert!(parse_shortcut("Win+Alt+P").unwrap_err().contains("Windows key"));
        assert!(parse_shortcut("Super+Ctrl+P").unwrap_err().contains("Windows key"));
        assert!(parse_shortcut("Alt+F4").unwrap_err().starts_with("Alt+F4 closes windows"));
        assert_eq!(canonical("Ctrl+Alt+F4"), "Ctrl+Alt+F4");
    }

    #[test]
    fn startup_uses_saved_value_or_falls_back_to_default() {
        let (parsed, warning) = startup_panel_shortcut(None);
        assert_eq!(parsed.canonical(), DEFAULT_PANEL_SHORTCUT);
        assert!(warning.is_none());

        let (parsed, warning) = startup_panel_shortcut(Some("Ctrl+Shift+F9"));
        assert_eq!(parsed.canonical(), "Ctrl+Shift+F9");
        assert!(warning.is_none());

        let (parsed, warning) = startup_panel_shortcut(Some("Ctrl+Alt+H"));
        assert_eq!(parsed.canonical(), DEFAULT_PANEL_SHORTCUT);
        let warning = warning.unwrap();
        assert!(
            warning.contains("\"Ctrl+Alt+H\" is not valid")
                && warning.ends_with("Using Ctrl+Alt+P instead."),
            "{warning}"
        );
    }

    /// Runs `swap_shortcut` with fakes, against combinations "owned by
    /// another app", and returns the calls it made in order.
    fn swap(
        current: Option<&'static str>,
        next: &'static str,
        owned: &[&str],
    ) -> (Result<(), String>, Vec<String>) {
        let log = std::cell::RefCell::new(Vec::new());
        let result = swap_shortcut(
            current,
            next,
            |k| {
                log.borrow_mut().push(format!("register {k}"));
                if owned.contains(&k) {
                    Err(format!("{k} is already registered"))
                } else {
                    Ok(())
                }
            },
            |k| log.borrow_mut().push(format!("unregister {k}")),
        );
        (result, log.into_inner())
    }

    #[test]
    fn swap_registers_new_before_releasing_old() {
        let (result, log) = swap(Some("Ctrl+Alt+P"), "Ctrl+Shift+P", &[]);
        assert!(result.is_ok());
        assert_eq!(log, ["register Ctrl+Shift+P", "unregister Ctrl+Alt+P"]);
    }

    #[test]
    fn swap_failure_keeps_the_previous_shortcut_registered() {
        let (result, log) = swap(Some("Ctrl+Alt+P"), "Ctrl+Alt+Q", &["Ctrl+Alt+Q"]);
        assert_eq!(result.unwrap_err(), "Ctrl+Alt+Q is already registered");
        assert_eq!(log, ["register Ctrl+Alt+Q"], "the old shortcut must not be released");
    }

    #[test]
    fn swap_to_the_same_shortcut_does_nothing() {
        let (result, log) = swap(Some("Ctrl+Alt+P"), "Ctrl+Alt+P", &["Ctrl+Alt+P"]);
        assert!(result.is_ok());
        assert!(log.is_empty());
    }

    #[test]
    fn swap_with_nothing_registered_retries_registration() {
        // Startup registration failed: saving (even the same text) tries again.
        let (result, log) = swap(None, "Ctrl+Alt+P", &[]);
        assert!(result.is_ok());
        assert_eq!(log, ["register Ctrl+Alt+P"]);
        let (result, _) = swap(None, "Ctrl+Alt+P", &["Ctrl+Alt+P"]);
        assert!(result.is_err());
    }
}
