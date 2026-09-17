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

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Broadcast (payload: `Vec<TrackedTable>`) whenever the set of tracked
/// PokerStars table windows changes — a table opened, closed, was renamed, or
/// was minimized/restored. Emitted from the tracking loop so every window's
/// view of "which tables am I following" updates live.
///
///  replaced 's `table-count-changed` (payload: a bare `u32`). The count
/// existed only to power an honest "N tables open, only one gets a HUD"
/// notice; now every table gets its own overlay, so what the UI needs is the
/// list itself, not a number to apologise for.
pub const TRACKED_TABLES_EVENT: &str = "tracked-tables-changed";

/// One real, logged-in PokerStars table window Velora is currently following
///. Before  this was three process-wide statics holding exactly one
/// table's worth of state (`TRACKED_HWND`/`LAST_RECT`/`LAST_TABLE_NAME`), and
/// every extra table window found by the enumeration was discarded.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedTable {
    /// Stable identity for this table *for this run of the app*, assigned when
    /// the window is first seen and never reused once it closes. This — not
    /// the HWND — is what the overlay window's label and URL are built from
    /// and what every table-scoped command takes, so a recycled HWND can never
    /// silently re-point an existing overlay at a different table.
    pub id: u32,
    /// The table window's own HWND, as an `isize`. Diagnostics and the
    /// registry's own lookups; never crosses into the frontend as an identity.
    pub hwnd: i64,
    /// Table name parsed out of the window title (`extract_table_name`), and
    /// the key every DB query for this table is scoped by. `None` when the
    /// title didn't parse — see `commands::get_active_table_players` for what
    /// that means for the roster.
    pub name: Option<String>,
    /// Where the table window is on screen right now; the overlay for this
    /// table mirrors it 1:1 (`overlay_bounds_for_table`).
    pub rect: WindowRect,
    /// Minimized tables keep their registry entry (they are still open, and
    /// come back where they were) but their overlay is hidden — Windows parks
    /// a minimized window at roughly (-32000, -32000), and an overlay dutifully
    /// following it there is just an invisible window burning a resync per
    /// tick.
    pub minimized: bool,
    /// When this table window was first registered, `db::now_played_at()`
    /// -stamped, set once and never updated for the rest of this table's
    /// life. Not `db::now_iso()` — see that function's own doc
    /// comment for why the clock has to match `played_at`'s.
    /// PokerStars reuses table *names* from a pool, so `table_name = ?` alone
    /// can resolve to a hand played hours or days ago at a different sitting
    /// of the same name — indistinguishable from a fresh one without a
    /// recency bound. Every query that resolves "the active hand" for this
    /// table passes this as a `since` floor, so a hand older than the table
    /// itself can never render as if it were live.
    pub first_seen_at: String,
}

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

/// Which tracked table (if any) owns the window `hwnd` — the pure half of
/// the  global-hotkey feature, kept separate from the actual
/// `GetForegroundWindow` call so "does a foreground HWND resolve to the
/// right table, or correctly resolve to nothing" is unit-testable without a
/// real window or a global-hotkey test harness. `hwnd` is compared against
/// each table's own `TrackedTable::hwnd`; a HWND that belongs to no tracked
/// table (PokerStars lobby, a different app entirely, or nothing at all)
/// correctly resolves to `None` rather than guessing.
pub fn table_for_hwnd(tables: &[TrackedTable], hwnd: i64) -> Option<&TrackedTable> {
    tables.iter().find(|t| t.hwnd == hwnd)
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

/// One entry in the  diagnostics resync log — every time a tracked table
/// window's rect was (re-)read and its own overlay window repositioned to
/// match, whichever of the three call sites triggered it. `table_id` was added
/// in with one overlay per table, "which table moved" is the first thing
/// this log has to answer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResyncLogEntry {
    pub at: String,
    pub table_id: u32,
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
/// ## Tournament tables (, a known issue)
///
/// Tournaments do not have a human table name at all. Checked against the
/// user's own real hand histories, PokerStars writes the tournament id and
/// the table number in the `Table '...'` line — `Table '4001646104 38' 8-max
/// Seat #1 is the button` — so `hands.table_name` for a tournament is the
/// literal string `"4001646104 38"`. The cash rule above ("the segment before
/// the game description") cannot produce that, so before  every tournament
/// table parsed to something matching no row, and its overlay rendered empty.
///
/// **Verified against a real captured tournament table window title**
/// (2026-09-13, live session). 's original guess — that the id/table pair
/// sits at the *start* of the title — was wrong: the real title is
/// `"Session: 00:14 - Progressive KO €10 [6-Max] | €3.000 Gtd - 125/250 ante
/// 30 - Tournament 4029218003 Table 24 - Logged In as ..."`, with the pair
/// embedded after several other segments, marked by the literal words
/// "Tournament"/"Table" rather than by position. `tournament_table_name`
/// now searches the whole title for that literal keyword pattern first (see
/// its own doc comment), falling back to the original start-anchored
/// bare-digit shape. A cash table's own description is exceedingly unlikely
/// to contain the literal phrase "Tournament <digits> Table <digits>", so
/// this cannot start misreading cash titles; a table whose name still
/// doesn't resolve renders no players rather than another table's
/// (`commands::table_scope`).
pub fn extract_table_name(title: &str) -> Option<String> {
    let prefix = title.split(" - Logged In as ").next()?;
    if let Some(tournament_table) = tournament_table_name(prefix) {
        return Some(tournament_table);
    }
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

/// `"4001646104 38"` out of a tournament title, or `None`. Tries the
/// keyword-anchored match first (confirmed against a real captured title,
/// see below), falling back to the older start-anchored bare-digit shape.
fn tournament_table_name(prefix: &str) -> Option<String> {
    tournament_table_name_from_keywords(prefix).or_else(|| tournament_table_name_from_leading_digits(prefix))
}

/// Finds `Tournament <id> Table <table>` **anywhere** in the title — not
/// anchored to the start — and returns `"<id> <table>"`, matching the exact
/// format already used in `hands.table_name` for tournament hands.
///
/// 's original rule (`tournament_table_name_from_leading_digits` below)
/// assumed the id/table pair sat at the very start of the title. Live
/// tournament testing (2026-09-13, a known issue) confirmed that
/// assumption was simply wrong for real tournament table windows: the real
/// captured title is
/// `"Session: 00:14 - Progressive KO €10 [6-Max] | €3.000 Gtd - 125/250 ante
/// 30 - Tournament 4029218003 Table 24 - Logged In as ..."` — the id/table
/// pair sits after several other " - "-separated segments, preceded by the
/// literal words "Tournament"/"Table". Anchoring on those literal keywords
/// (rather than digit shape/position) is what makes searching the whole
/// string safe: a cash title's own description is exceedingly unlikely to
/// contain the literal phrase "Tournament <digits> Table <digits>", so this
/// cannot start misreading cash titles the way a looser digit-shape-only
/// search anywhere in the string could.
fn tournament_table_name_from_keywords(prefix: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"Tournament\s+(\d+)\s+Table\s+(\d+)").expect("valid tournament keyword regex")
    });
    let caps = re.captures(prefix)?;
    Some(format!("{} {}", &caps[1], &caps[2]))
}

/// `"4001646104 38"` out of a title that begins with it, or `None`.
///
/// Deliberately anchored to the start and to two all-digit words: anything
/// looser would start reinterpreting cash table names that happen to contain
/// numbers, and a wrong name is worse than no name — it would scope an overlay
/// to another table's hands. Kept as a fallback behind the keyword-anchored
/// match above: this shape was never confirmed against a real captured
/// window title ( in the notes), but nothing establishes it's
/// impossible either, and removing it isn't in scope for this fix.
fn tournament_table_name_from_leading_digits(prefix: &str) -> Option<String> {
    let mut words = prefix.split_whitespace();
    let tournament_id = words.next()?;
    let table_number = words.next()?;
    let all_digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if !all_digits(tournament_id) || !all_digits(table_number) {
        return None;
    }
    // Tournament ids are long; a table number is short. Requiring that shape
    // keeps a cash title like "12 34 - ..." from being read as a tournament.
    if tournament_id.len() < 6 || table_number.len() > 4 {
        return None;
    }
    Some(format!("{tournament_id} {table_number}"))
}

#[cfg(windows)]
pub mod win;

#[cfg(windows)]
pub use win::{
    debug_snapshot, expected_hook_count, install_tracking, resync_log, table_for, table_name_for,
    toggle_hud_for_foreground_table, tracked_tables,
};

#[cfg(not(windows))]
pub fn install_tracking(_app_handle: tauri::AppHandle) {}

#[cfg(not(windows))]
pub fn tracked_tables() -> Vec<TrackedTable> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn table_for(_table_id: u32) -> Option<TrackedTable> {
    None
}

#[cfg(not(windows))]
pub fn table_name_for(_table_id: u32) -> Option<String> {
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

#[cfg(not(windows))]
pub fn expected_hook_count() -> u32 {
    0
}

#[cfg(not(windows))]
pub fn toggle_hud_for_foreground_table() {}

/// How many real PokerStars table windows are being tracked right now — each
/// one has its own overlay .
pub fn table_window_count() -> u32 {
    tracked_tables().len() as u32
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

    /// . The expected values here are not invented: they
    /// are the exact `hands.table_name` strings in the user's own real
    /// tournament hand histories (`Table '4001646104 38' 8-max ...`), which is
    /// what a tournament overlay has to match to find its own players.
    #[test]
    fn extract_table_name_recovers_a_tournament_id_and_table_number() {
        assert_eq!(
            extract_table_name(
                "4001646104 38 - Blinds 100/200 - Hold'em No Limit - Logged In as TourneyHero"
            ),
            Some("4001646104 38".to_string())
        );
        assert_eq!(
            extract_table_name("4022791123 4 - Logged In as TourneyHero"),
            Some("4022791123 4".to_string())
        );
    }

    /// a known issue follow-up (2026-09-13): live tournament testing found
    /// 's start-anchored assumption was wrong — the real title carries the
    /// id/table pair after several other segments, marked by the literal
    /// words "Tournament"/"Table" rather than by position. This is the exact
    /// title captured live tonight.
    #[test]
    fn extract_table_name_finds_the_tournament_pair_anywhere_in_the_title() {
        assert_eq!(
            extract_table_name(
                "Session: 00:14 - Progressive KO \u{20ac}10 [6-Max] | \u{20ac}3.000 Gtd - 125/250 ante 30 - Tournament 4029218003 Table 24 - Logged In as TourneyHero"
            ),
            Some("4029218003 24".to_string())
        );
    }

    /// The tournament rule must never fire on a cash table: a wrong name is
    /// worse than none, because it would scope that overlay to another table's
    /// hands rather than simply showing nothing.
    #[test]
    fn the_tournament_rule_leaves_cash_titles_alone() {
        assert_eq!(
            extract_table_name(
                "Session: 05:11 - Aegle IV - No Limit Hold'em \u{20ac}0.01/\u{20ac}0.02 EUR - Logged In as TourneyHero"
            ),
            Some("Aegle IV".to_string())
        );
        // Two short numbers are not a tournament id and a table number.
        assert_eq!(
            extract_table_name("12 34 - No Limit Hold'em - Logged In as TourneyHero"),
            Some("12 34".to_string()),
            "falls through to the cash rule, which reads the whole first segment"
        );
        assert_eq!(tournament_table_name("12 34 - No Limit Hold'em"), None);
        assert_eq!(tournament_table_name("Session: 05:11 - Aegle IV"), None);
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

    /// Minimal `TrackedTable` fixture for `table_for_hwnd` — only `id` and
    /// `hwnd` matter to that lookup, so every other field gets an arbitrary
    /// but valid value.
    fn fake_table(id: u32, hwnd: i64) -> TrackedTable {
        TrackedTable {
            id,
            hwnd,
            name: Some(format!("Table {id}")),
            rect: TABLE,
            minimized: false,
            first_seen_at: "2026-09-11T00:00:00".to_string(),
        }
    }

    #[test]
    fn table_for_hwnd_finds_the_table_whose_window_has_foreground_focus() {
        let tables = vec![fake_table(1, 0x1001), fake_table(2, 0x1002)];
        let found = table_for_hwnd(&tables, 0x1002).expect("hwnd 0x1002 is tracked");
        assert_eq!(found.id, 2);
    }

    #[test]
    fn table_for_hwnd_is_none_for_a_foreground_window_that_is_not_a_tracked_table() {
        let tables = vec![fake_table(1, 0x1001), fake_table(2, 0x1002)];
        // A real HWND (some other app, or the PokerStars lobby) that just
        // isn't one of the currently tracked table windows.
        assert!(table_for_hwnd(&tables, 0x9999).is_none());
    }

    #[test]
    fn table_for_hwnd_is_none_when_nothing_is_tracked() {
        assert!(table_for_hwnd(&[], 0x1001).is_none());
    }
}
