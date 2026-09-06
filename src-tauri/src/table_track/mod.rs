//! Phase E (table auto-detection) — window-following.
//!
//! Two halves, deliberately kept separate:
//!
//! 1. Pure, OS-independent math (`to_relative`/`to_absolute`,
//!    `overlay_bounds_for_table`) — unit-testable without a real window,
//!    per the spec's exit criteria.
//! 2. Windows-only detection/tracking (`win` submodule, `#[cfg(windows)]`)
//!    that finds the PokerStars table window and keeps the overlay window
//!    glued to it.
//!
//! The table window class name (`POKERSTARS_TABLE_CLASS_CANDIDATES` in
//! `win.rs`) and the title check (`LOGGED_IN_TITLE_MARKER`) are confirmed
//! against a read-only `EnumWindows` dump of the user's live client
//! — see the notes decision .

use serde::{Deserialize, Serialize};

/// A window's bounds in absolute screen coordinates, in the same units
/// `GetWindowRect`/Tauri's `PhysicalPosition`/`PhysicalSize` use.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Converts an absolute screen point into a fraction (0..1) of `table`'s
/// bounds — the representation `hud_positions`/`seat_templates` now store,
/// so a saved position means the same seat-relative spot regardless of
/// where the table window currently sits or how large it currently is.
pub fn to_relative(table: WindowRect, abs_x: f64, abs_y: f64) -> (f64, f64) {
    let width = (table.width as f64).max(1.0);
    let height = (table.height as f64).max(1.0);
    (
        (abs_x - table.x as f64) / width,
        (abs_y - table.y as f64) / height,
    )
}

/// Inverse of `to_relative`: expands a stored fraction back into an
/// absolute screen point for `table`'s *current* bounds.
pub fn to_absolute(table: WindowRect, rel_x: f64, rel_y: f64) -> (f64, f64) {
    (
        table.x as f64 + rel_x * table.width as f64,
        table.y as f64 + rel_y * table.height as f64,
    )
}

/// What the overlay window's own bounds should become when the table window
/// moves/resizes to `table`. v1 keeps the overlay a 1:1 mirror of the table
/// window (so a stored fraction maps directly onto overlay viewport
/// percentages client-side, no further conversion needed there) — kept as
/// its own named function rather than inlined so the "table moved -> overlay
/// repositioned" behavior has one obvious, testable seam.
pub fn overlay_bounds_for_table(table: WindowRect) -> WindowRect {
    table
}

/// Debug evidence for the  window-following brief, surfaced via
/// `get_table_detection_status` so it's reachable from Settings without
/// needing to read a terminal: whether the WinEvent hooks registered, how
/// often the callback has fired at all vs. specifically for the tracked
/// table window, how many 1.5s poll ticks have run, and each process's
/// Windows integrity level (to rule the UIPI hypothesis in or out).
#[derive(Debug, Clone, Default)]
pub struct DebugSnapshot {
    pub hooks_installed: u32,
    pub event_callbacks_total: u64,
    pub event_callbacks_matched: u64,
    pub poll_ticks: u64,
    pub app_integrity_level: Option<String>,
    pub table_integrity_level: Option<String>,
}

/// One entry in the  diagnostics resync log — every time the tracked
/// table window's rect was (re-)read and the overlay window repositioned to
/// match, whichever of the three call sites triggered it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResyncLogEntry {
    pub at: String,
    pub source: &'static str,
    pub rect: WindowRect,
}

/// Extracts the PokerStars table name from a table window's title, so the
///  multi-table investigation can scope "the active table" to whichever
/// table window the overlay is actually tracking, instead of a global
/// most-recently-imported-hand query that has no notion of *which* table a
/// hand came from (see `db::list_active_table_players_with_seats`).
///
/// Confirmed real title (, a cash table): `"Session: 05:11 - Aegle IV -
/// No Limit Hold'em \u{20ac}0.01/\u{20ac}0.02 EUR - Logged In as
/// TourneyHero"` — table name `"Aegle IV"` sits between a leading session
/// timer and a trailing game-description segment, right before the fixed
/// `" - Logged In as "` suffix (`LOGGED_IN_TITLE_MARKER` in `win.rs`).
/// Splitting on `" - "` and taking the second-to-last segment (before the
/// game description) recovers it without hardcoding the "Session: mm:ss -"
/// prefix, so it degrades gracefully if that prefix is absent.
///
/// **Unconfirmed for tournament tables** — no real tournament *table*
/// window title has been captured yet ( only captured tournament
/// *lobby* titles, which this function is never called on since they fail
/// `is_table_title` first). If a tournament table's title doesn't follow
/// this same "name segment before the description segment" shape, this
/// returns a value that won't match any `hands.table_name`, and the
/// active-table query safely falls back to showing no players for that
/// table (never another table's players) until this is confirmed and
/// fixed with real data, per the same discipline as /.
pub fn extract_table_name(title: &str) -> Option<String> {
    let prefix = title.split(" - Logged In as ").next()?;
    let parts: Vec<&str> = prefix.split(" - ").collect();
    match parts.len() {
        0 => None,
        1 => None,
        _ => {
            let name = parts[parts.len() - 2].trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        }
    }
}

#[cfg(windows)]
pub mod win;

#[cfg(windows)]
pub use win::{active_table_name, debug_snapshot, install_tracking, last_known_table_rect, resync_log};

#[cfg(not(windows))]
pub fn install_tracking(_app_handle: tauri::AppHandle) {}

#[cfg(not(windows))]
pub fn last_known_table_rect() -> Option<WindowRect> {
    None
}

#[cfg(not(windows))]
pub fn active_table_name() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn resync_log() -> Vec<ResyncLogEntry> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn debug_snapshot() -> DebugSnapshot {
    DebugSnapshot::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: WindowRect = WindowRect {
        x: 100,
        y: 50,
        width: 800,
        height: 600,
    };

    #[test]
    fn to_relative_maps_table_corners_to_unit_square() {
        assert_eq!(to_relative(TABLE, 100.0, 50.0), (0.0, 0.0));
        assert_eq!(to_relative(TABLE, 900.0, 650.0), (1.0, 1.0));
        assert_eq!(to_relative(TABLE, 500.0, 350.0), (0.5, 0.5));
    }

    #[test]
    fn to_absolute_is_the_inverse_of_to_relative() {
        for (abs_x, abs_y) in [(100.0, 50.0), (900.0, 650.0), (327.0, 411.0)] {
            let (rel_x, rel_y) = to_relative(TABLE, abs_x, abs_y);
            let (back_x, back_y) = to_absolute(TABLE, rel_x, rel_y);
            assert!((back_x - abs_x).abs() < 1e-9, "x round-trip: {back_x} vs {abs_x}");
            assert!((back_y - abs_y).abs() < 1e-9, "y round-trip: {back_y} vs {abs_y}");
        }
    }

    #[test]
    fn a_relative_position_follows_the_table_when_it_moves_and_resizes() {
        let (rel_x, rel_y) = to_relative(TABLE, 500.0, 350.0); // table center
        assert_eq!((rel_x, rel_y), (0.5, 0.5));

        // Table moved to a different monitor and resized.
        let moved = WindowRect {
            x: 2000,
            y: 300,
            width: 1000,
            height: 700,
        };
        let (new_abs_x, new_abs_y) = to_absolute(moved, rel_x, rel_y);
        assert_eq!((new_abs_x, new_abs_y), (2500.0, 650.0)); // moved table's own center
    }

    #[test]
    fn extract_table_name_recovers_the_name_from_a_confirmed_real_title() {
        assert_eq!(
            extract_table_name(
                "Session: 05:11 - Aegle IV - No Limit Hold'em \u{20ac}0.01/\u{20ac}0.02 EUR - Logged In as TourneyHero"
            ),
            Some("Aegle IV".to_string())
        );
    }

    #[test]
    fn extract_table_name_returns_none_without_a_recoverable_segment() {
        assert_eq!(extract_table_name("Logged In as TourneyHero"), None);
        assert_eq!(extract_table_name("just one segment"), None);
        assert_eq!(extract_table_name(""), None);
    }

    #[test]
    fn extract_table_name_tolerates_a_missing_session_timer_prefix() {
        // If a table window's title omits the leading "Session: mm:ss -"
        // segment, the name is still the segment right before the game
        // description, one position earlier than the confirmed sample.
        assert_eq!(
            extract_table_name("Orion II - No Limit Hold'em - Logged In as TourneyHero"),
            Some("Orion II".to_string())
        );
    }

    #[test]
    fn overlay_bounds_track_the_table_rect_exactly() {
        let table = WindowRect { x: 10, y: 20, width: 640, height: 480 };
        assert_eq!(overlay_bounds_for_table(table), table);

        let resized = WindowRect { x: 10, y: 20, width: 900, height: 700 };
        assert_eq!(overlay_bounds_for_table(resized), resized);
        assert_ne!(overlay_bounds_for_table(resized), table);
    }
}
