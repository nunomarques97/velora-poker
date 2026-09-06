//! Windows-only table detection & window-following (Phase E).
//!
//! `EnumWindows` finds the PokerStars table window once; `SetWinEventHook`
//! (`EVENT_OBJECT_LOCATIONCHANGE` / `EVENT_SYSTEM_MOVESIZEEND`) then keeps
//! the overlay window glued to it without polling. A low-frequency polling
//! fallback re-acquires the table window if it closes and a new one opens
//! (a fresh hand can open a brand-new table window with a new HWND), and
//! covers the case where PokerStars wasn't running yet when Velora started.
//!
//! WinEvent callbacks are a plain C function pointer with no closure
//! capture, so the tracked HWND and the `AppHandle` needed to reposition the
//! overlay live in process-wide statics, set once by `install_tracking`.
//!
//! ##  debug brief (2026-08-29): window-following fired zero updates
//!
//! Confirmed with a real table: detection worked (Settings showed
//! "Detected"), but dragging the table left the overlay exactly where it
//! was — not delayed, motionless. Two real bugs found by re-reading this
//! file against Win32 semantics (no live client needed to see either one):
//!
//! 1. `SetWinEventHook`'s `eventMin`/`eventMax` parameters are an *inclusive
//!    numeric range* (`eventMin <= event_id <= eventMax`) — they are not
//!    "here are two events I want." The old code called
//!    `SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_MOVESIZEEND, ...)`.
//!    `EVENT_OBJECT_LOCATIONCHANGE` is `0x8000_000B`-range (`0x800B`) and
//!    `EVENT_SYSTEM_MOVESIZEEND` is `0x000B` — so `eventMin` (0x800B) was
//!    *larger* than `eventMax` (0x000B), an inverted, empty range that
//!    matches no event ID at all. The hook installs "successfully" (a
//!    non-null `HWINEVENTHOOK`) but the callback can never fire for either
//!    event — a silent failure mode that matches the reported symptom
//!    exactly. Fixed by registering one hook per event with
//!    `eventMin == eventMax`, the documented way to hook a single event.
//! 2. The "1.5s polling fallback" never actually re-synced the overlay's
//!    position on a tick where the tracked HWND was still valid — it only
//!    reacted when the window handle became invalid (closed/replaced). So
//!    even with bug 1 unfixed, nothing else was moving the overlay either.
//!    Fixed to unconditionally re-read the tracked window's rect and
//!    re-sync the overlay every tick, independent of whether the event
//!    hook fires — this is the fallback the doc comment always claimed to
//!    be.
//!
//! Both are root-caused from source + documented Win32 behavior, not
//! guessed. What's still a hypothesis, not evidence: whether Windows UIPI
//! (cross-integrity-level hook/message blocking) plays any role on the
//! user's machine — `debug_snapshot` below reports both processes'
//! integrity level so that can be read directly from Settings instead of
//! guessed at.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, RECT};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
    IsWindowVisible, CHILDID_SELF, EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_MOVESIZEEND,
    OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
};

use super::{extract_table_name, overlay_bounds_for_table, DebugSnapshot, ResyncLogEntry, WindowRect};
use crate::db::now_iso;
use crate::overlay::OVERLAY_LABEL;

/// Confirmed against the user's live client via a read-only `EnumWindows`
/// dump: real table windows report class `GLFW30`. That's a generic
/// GLFW-library class name shared by any GLFW-based window, so it alone
/// over-matches — see `LOGGED_IN_TITLE_MARKER` below for the check that
/// actually scopes this to a real table.
const POKERSTARS_TABLE_CLASS_CANDIDATES: &[&str] = &["GLFW30"];

/// Every confirmed real table title contains this marker, e.g. "Session:
/// 05:11 - Aegle IV - No Limit Hold'em \u{20ac}0.01/\u{20ac}0.02 EUR - Logged
/// In as TourneyHero". No lobby, tournament-lobby, or dialog title does
/// (confirmed via the same live-client dump). A positive requirement
/// beats an exclusion list here: it doesn't need extending every time
/// PokerStars ships a new non-table window type.
const LOGGED_IN_TITLE_MARKER: &str = " - Logged In as ";

/// True if `title` belongs to a real, logged-in PokerStars table window.
fn is_table_title(title: &str) -> bool {
    title.contains(LOGGED_IN_TITLE_MARKER)
}

fn log(msg: impl AsRef<str>) {
    eprintln!("[table_track] {}", msg.as_ref());
}

static TRACKED_HWND: AtomicIsize = AtomicIsize::new(0);
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static LAST_RECT: Mutex<Option<WindowRect>> = Mutex::new(None);
// Keeps the hooks alive for the process's lifetime; never unhooked (the app
// only ever installs these once, at startup, and runs until process exit).
static HOOKS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

// the table *name* of whichever window is currently
// tracked, extracted from its title (see `extract_table_name`) so the
// active-table DB query can be scoped to this specific table instead of
// "whatever hand was imported most recently across every open table" — see
// the notes  and `db::list_active_table_players_with_seats`.
// `None` whenever no window is tracked, or its title didn't parse.
static LAST_TABLE_NAME: Mutex<Option<String>> = Mutex::new(None);

const RESYNC_LOG_CAP: usize = 20;
static RESYNC_LOG: Mutex<VecDeque<ResyncLogEntry>> = Mutex::new(VecDeque::new());

// Debug counters surfaced via `debug_snapshot` / the
// `get_table_detection_status` command, so the evidence needed to confirm
// or deny "is the hook firing" / "is the poll loop alive" is reachable from
// Settings, not just a terminal running `npm run tauri dev`.
static HOOKS_INSTALLED: AtomicU32 = AtomicU32::new(0);
static EVENT_CALLBACKS_TOTAL: AtomicU64 = AtomicU64::new(0);
static EVENT_CALLBACKS_MATCHED: AtomicU64 = AtomicU64::new(0);
static POLL_TICKS: AtomicU64 = AtomicU64::new(0);

pub fn last_known_table_rect() -> Option<WindowRect> {
    LAST_RECT.lock().ok().and_then(|g| *g)
}

/// The table name extracted from the currently-tracked window's title, if
/// any — see the `LAST_TABLE_NAME` doc comment above. Used both to scope
/// the active-table DB query and surfaced in the diagnostics report
/// so the user can see exactly which table Velora thinks is active and
/// that it came from window tracking, not a guess.
pub fn active_table_name() -> Option<String> {
    LAST_TABLE_NAME.lock().ok().and_then(|g| g.clone())
}

/// Last `RESYNC_LOG_CAP` overlay resync events (window acquired, WinEvent
/// callback fired, or a poll tick), newest last — diagnostics evidence for
/// the  investigation, so "is the overlay actually tracking a table
/// window, and how often" is something the user can read from Settings
/// instead of a claim to take on faith.
pub fn resync_log() -> Vec<ResyncLogEntry> {
    RESYNC_LOG
        .lock()
        .map(|g| g.iter().cloned().collect())
        .unwrap_or_default()
}

pub fn debug_snapshot() -> DebugSnapshot {
    let tracked = TRACKED_HWND.load(Ordering::SeqCst);
    let table_integrity_level = if tracked != 0 {
        table_process_pid(HWND(tracked as *mut _))
            .and_then(process_integrity_level_for_pid)
            .map(integrity_level_name)
    } else {
        None
    };
    DebugSnapshot {
        hooks_installed: HOOKS_INSTALLED.load(Ordering::SeqCst),
        event_callbacks_total: EVENT_CALLBACKS_TOTAL.load(Ordering::SeqCst),
        event_callbacks_matched: EVENT_CALLBACKS_MATCHED.load(Ordering::SeqCst),
        poll_ticks: POLL_TICKS.load(Ordering::SeqCst),
        app_integrity_level: current_process_integrity_level().map(integrity_level_name),
        table_integrity_level,
    }
}

/// Installs the WinEvent hooks and starts the low-frequency re-acquisition
/// poll. Call once, during Tauri's `setup()`, on the main thread (the same
/// thread already pumping the window message loop `SetWinEventHook`'s
/// `WINEVENT_OUTOFCONTEXT` callback needs).
pub fn install_tracking(app_handle: AppHandle) {
    let _ = APP_HANDLE.set(app_handle.clone());

    // One hook per event, each with eventMin == eventMax (the documented
    // way to hook exactly one event) — see the  doc comment above for
    // why a single hook spanning both events was wrong.
    for (name, event) in [
        ("EVENT_OBJECT_LOCATIONCHANGE", EVENT_OBJECT_LOCATIONCHANGE),
        ("EVENT_SYSTEM_MOVESIZEEND", EVENT_SYSTEM_MOVESIZEEND),
    ] {
        let hook: HWINEVENTHOOK = unsafe {
            SetWinEventHook(
                event,
                event,
                None,
                Some(win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if hook.0.is_null() {
            log(format!("SetWinEventHook FAILED to register for {name}"));
            continue;
        }
        log(format!("SetWinEventHook registered for {name}"));
        HOOKS_INSTALLED.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut slot) = HOOKS.lock() {
            slot.push(hook.0 as isize);
        }
    }

    if let Some(level) = current_process_integrity_level() {
        log(format!(
            "Velora process integrity level: {}",
            integrity_level_name(level)
        ));
    } else {
        log("could not determine Velora's own process integrity level");
    }

    // Try an immediate acquisition so the overlay is already aligned if the
    // table window already exists when Velora starts, then fall back to
    // periodic re-checks for tables that open/close later.
    try_acquire_and_sync();

    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_millis(1500));
        let tick = POLL_TICKS.fetch_add(1, Ordering::SeqCst) + 1;

        let tracked = TRACKED_HWND.load(Ordering::SeqCst);
        let still_valid = tracked != 0 && unsafe { IsWindow(HWND(tracked as *mut _)) }.as_bool();

        if !still_valid {
            if tracked != 0 {
                log(format!(
                    "poll tick {tick}: tracked window {tracked:#x} no longer valid, reacquiring"
                ));
            }
            TRACKED_HWND.store(0, Ordering::SeqCst);
            if let Ok(mut slot) = LAST_TABLE_NAME.lock() {
                *slot = None;
            }
            try_acquire_and_sync();
            continue;
        }

        // The actual polling fallback: unconditionally re-read the tracked
        // window's current rect and re-sync the overlay to it, regardless
        // of whether the WinEvent hook has fired since the last tick. This
        // is what makes this a genuine fallback rather than something that
        // only ever does work when the hook is already broken in a
        // different way (window replaced).
        if let Some(rect) = window_rect(HWND(tracked as *mut _)) {
            let moved = last_known_table_rect() != Some(rect);
            log(format!(
                "poll tick {tick}: tracked={tracked:#x} rect={rect:?} moved_since_last_sync={moved} \
                 hooks_installed={} event_callbacks_total={} event_callbacks_matched={}",
                HOOKS_INSTALLED.load(Ordering::SeqCst),
                EVENT_CALLBACKS_TOTAL.load(Ordering::SeqCst),
                EVENT_CALLBACKS_MATCHED.load(Ordering::SeqCst),
            ));
            sync_overlay_to_table(rect, "poll");
        }
    });
}

fn try_acquire_and_sync() {
    if let Some(hwnd) = find_table_window() {
        log(format!("acquired table window {:#x}", hwnd.0 as isize));
        TRACKED_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

        let table_name = extract_table_name(&window_title(hwnd));
        log(format!("extracted table name: {table_name:?}"));
        if let Ok(mut slot) = LAST_TABLE_NAME.lock() {
            *slot = table_name;
        }

        if let Some(rect) = window_rect(hwnd) {
            sync_overlay_to_table(rect, "acquire");
        }
    } else if let Ok(mut slot) = LAST_TABLE_NAME.lock() {
        *slot = None;
    }
}

fn record_resync(rect: WindowRect, source: &'static str) {
    if let Ok(mut log) = RESYNC_LOG.lock() {
        if log.len() >= RESYNC_LOG_CAP {
            log.pop_front();
        }
        log.push_back(ResyncLogEntry { at: now_iso(), source, rect });
    }
}

fn sync_overlay_to_table(table_rect: WindowRect, source: &'static str) {
    record_resync(table_rect, source);
    if let Ok(mut slot) = LAST_RECT.lock() {
        *slot = Some(table_rect);
    }
    let Some(app_handle) = APP_HANDLE.get() else {
        log("sync_overlay_to_table: no AppHandle installed yet");
        return;
    };
    let Some(win) = app_handle.get_webview_window(OVERLAY_LABEL) else {
        log(format!(
            "sync_overlay_to_table: overlay window '{OVERLAY_LABEL}' not found"
        ));
        return;
    };
    let bounds = overlay_bounds_for_table(table_rect);
    let _ = win.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
        x: bounds.x,
        y: bounds.y,
    }));
    let _ = win.set_size(tauri::Size::Physical(tauri::PhysicalSize {
        width: bounds.width.max(0) as u32,
        height: bounds.height.max(0) as u32,
    }));
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    EVENT_CALLBACKS_TOTAL.fetch_add(1, Ordering::SeqCst);

    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
        return;
    }
    let tracked = TRACKED_HWND.load(Ordering::SeqCst);
    if tracked == 0 || hwnd.0 as isize != tracked {
        return;
    }
    EVENT_CALLBACKS_MATCHED.fetch_add(1, Ordering::SeqCst);
    if let Some(rect) = window_rect(hwnd) {
        sync_overlay_to_table(rect, "event");
    }
}

fn window_rect(hwnd: HWND) -> Option<WindowRect> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some(WindowRect {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    })
}

/// `GetWindowTextW` wrapped as an owned `String` — shared by the
/// `EnumWindows` title check and `try_acquire_and_sync`'s table-name
/// extraction so there's one place that owns the buffer-sizing detail.
fn window_title(hwnd: HWND) -> String {
    let mut title_buf = [0u16; 512];
    let title_len = unsafe { GetWindowTextW(hwnd, &mut title_buf) };
    String::from_utf16_lossy(&title_buf[..title_len.max(0) as usize])
}

fn find_table_window() -> Option<HWND> {
    let mut found: Option<HWND> = None;
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut found as *mut Option<HWND> as isize),
        );
    }
    found
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let found = &mut *(lparam.0 as *mut Option<HWND>);

    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }

    let mut class_buf = [0u16; 256];
    let class_len = GetClassNameW(hwnd, &mut class_buf);
    let class_name = String::from_utf16_lossy(&class_buf[..class_len.max(0) as usize]);

    let class_matches = POKERSTARS_TABLE_CLASS_CANDIDATES
        .iter()
        .any(|c| c.eq_ignore_ascii_case(&class_name));
    if !class_matches {
        return BOOL(1);
    }

    let title = window_title(hwnd);

    if !is_table_title(&title) {
        return BOOL(1);
    }

    *found = Some(hwnd);
    BOOL(0) // stop enumeration — found it
}

/// PID of the process that owns `hwnd`, if it can be determined.
fn table_process_pid(hwnd: HWND) -> Option<u32> {
    let mut pid = 0u32;
    let result = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if result == 0 || pid == 0 {
        None
    } else {
        Some(pid)
    }
}

/// Windows integrity-level RID for the given open process handle (does not
/// take ownership; caller closes `process` if it owns it). Used to check
/// the  UIPI hypothesis: a lower-integrity Velora process can have its
/// `SetWinEventHook`/message delivery silently blocked by a
/// higher-integrity (e.g. elevated "Run as Administrator") target process.
fn process_integrity_level(process: HANDLE) -> Option<u32> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;

        let mut len = 0u32;
        // Expected to fail with ERROR_INSUFFICIENT_BUFFER; we only want `len`.
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut len);
        if len == 0 {
            let _ = CloseHandle(token);
            return None;
        }

        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr() as *mut _),
            len,
            &mut len,
        );
        let _ = CloseHandle(token);
        ok.ok()?;

        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return None;
        }
        let rid = *GetSidSubAuthority(sid, (count as u32) - 1);
        Some(rid)
    }
}

fn current_process_integrity_level() -> Option<u32> {
    process_integrity_level(unsafe { GetCurrentProcess() })
}

fn process_integrity_level_for_pid(pid: u32) -> Option<u32> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let level = process_integrity_level(handle);
        let _ = CloseHandle(handle);
        level
    }
}

fn integrity_level_name(rid: u32) -> String {
    match rid {
        0x0000 => "Untrusted".to_string(),
        0x1000 => "Low".to_string(),
        0x2000 => "Medium".to_string(),
        0x2100 => "Medium Plus".to_string(),
        0x3000 => "High".to_string(),
        0x4000 => "System".to_string(),
        other => format!("Unknown ({other:#x})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_table_title_matches() {
        assert!(is_table_title(
            "Session: 05:11 - Aegle IV - No Limit Hold'em \u{20ac}0.01/\u{20ac}0.02 EUR - Logged In as TourneyHero"
        ));
    }

    #[test]
    fn lobby_and_tournament_lobby_titles_do_not_match() {
        let non_table_titles = [
            "PokerStars Lobby - Last Login: 2026/08/24 12:52:05 WET",
            "Tournament 4025640801 Lobby",
            "Tournament 4025638047 Lobby",
        ];
        for title in non_table_titles {
            assert!(!is_table_title(title), "should not match: {title}");
        }
    }

    #[test]
    fn table_class_candidates_contain_glfw30_case_insensitively() {
        assert!(POKERSTARS_TABLE_CLASS_CANDIDATES
            .iter()
            .any(|c| c.eq_ignore_ascii_case("GLFW30")));
        assert!(POKERSTARS_TABLE_CLASS_CANDIDATES
            .iter()
            .any(|c| c.eq_ignore_ascii_case("glfw30")));
    }

    /// Regression test for the  root cause: `SetWinEventHook`'s
    /// eventMin/eventMax form an inclusive numeric range, so passing them
    /// in the wrong order silently creates a hook that never fires for
    /// either event. `install_tracking` now hooks each event individually
    /// with eventMin == eventMax, which is order-safe by construction, but
    /// this pins down the actual event ID relationship so nobody
    /// "simplifies" this back into a single min/max call without noticing
    /// they're inverted.
    #[test]
    fn location_change_and_movesize_end_event_ids_are_not_naturally_ordered() {
        assert!(
            EVENT_OBJECT_LOCATIONCHANGE > EVENT_SYSTEM_MOVESIZEEND,
            "if this ever flips, double-check any code that hooks both events with a single \
             SetWinEventHook(min, max) call instead of one hook per event"
        );
    }
}
