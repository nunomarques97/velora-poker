//! Windows-only table detection & window-following.
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
//! ## Why window-following once fired zero updates
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
//! (cross-integrity-level hook/message blocking) plays any role on a given
//! machine — `debug_snapshot` below reports both processes'
//! integrity level so that can be read directly from Settings instead of
//! guessed at.
//!
//! ## One tracked table became a registry of all of them
//!
//! Originally this module held exactly one table's worth of state in
//! process-wide statics — `TRACKED_HWND`, `LAST_RECT`, `LAST_TABLE_NAME` —
//! and `try_acquire_and_sync` picked `enumerate_table_windows().first()`,
//! discarding every other real table window it had just found. Everything
//! above it inherited that shape: one overlay window, one hit-test instance,
//! one "active table" for every DB query. Confirmed live: with two real
//! tables open, one got a HUD and the other got nothing.
//!
//! The statics are now a single `TABLES` registry holding every open table
//! window, each with its own id, rect and parsed name. The poll loop
//! reconciles that registry against the live enumeration every tick — new
//! windows are added, closed ones removed, survivors re-read — and every
//! change is handed to `overlay::manager`, which owns one overlay window per
//! entry. `win_event_proc` looks its HWND up in the registry instead of
//! comparing it against the single tracked handle.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
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
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, CHILDID_SELF, EVENT_OBJECT_DESTROY,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART,
    EVENT_SYSTEM_MOVESIZEEND, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
};

use super::{
    extract_table_name, table_for_hwnd, DebugSnapshot, ResyncLogEntry, TrackedTable, WindowRect,
    TRACKED_TABLES_EVENT,
};
use crate::db::{now_iso, now_played_at};
use crate::overlay::manager;

/// Confirmed against a live PokerStars client via a read-only `EnumWindows`
/// dump: real table windows report class `GLFW30`. That's a generic
/// GLFW-library class name shared by any GLFW-based window, so it alone
/// over-matches — see `LOGGED_IN_TITLE_MARKER` below for the check that
/// actually scopes this to a real table.
const POKERSTARS_TABLE_CLASS_CANDIDATES: &[&str] = &["GLFW30"];

/// Every confirmed real table title contains this marker, e.g. "Session:
/// 05:11 - Aegle IV - No Limit Hold'em \u{20ac}0.01/\u{20ac}0.02 EUR - Logged
/// In as <screen name>". No lobby, tournament-lobby, or dialog title does
/// (confirmed via the same live-client dump). A positive requirement
/// beats an exclusion list here: it doesn't need extending every time
/// PokerStars ships a new non-table window type.
const LOGGED_IN_TITLE_MARKER: &str = " - Logged In as ";

/// `EnumWindows` reads a callback's return value as "keep going": zero stops
/// the enumeration. Named because the difference between the two is one
/// character and, in `enum_windows_proc`, the whole multi-table count.
const CONTINUE_ENUMERATION: BOOL = BOOL(1);

/// True if `title` belongs to a real, logged-in PokerStars table window.
fn is_table_title(title: &str) -> bool {
    title.contains(LOGGED_IN_TITLE_MARKER)
}

fn log(msg: impl AsRef<str>) {
    eprintln!("[table_track] {}", msg.as_ref());
}

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
// Keeps the hooks alive for the process's lifetime; never unhooked (the app
// only ever installs these once, at startup, and runs until process exit).
static HOOKS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// Every real, logged-in PokerStars table window currently open, in the
/// `EnumWindows` order they were last enumerated in. This replaced
/// `TRACKED_HWND`/`LAST_RECT`/`LAST_TABLE_NAME`, which between them could
/// describe exactly one table.
///
/// A `Vec` rather than the `HashMap<isize, _>` the shape suggests: a real
/// multi-tabling session is a handful of windows, so the lookups this does per
/// event are already free, and a `Vec` both keeps the enumeration's own
/// ordering (stable for the UI's table list) and is `const`-initialisable, so
/// the registry needs no `LazyLock` wrapper to live in a plain `static`.
static TABLES: Mutex<Vec<TrackedTable>> = Mutex::new(Vec::new());

/// Source of `TrackedTable::id`. Monotonic and never reset: an id is not
/// reused after its table closes, so a recycled HWND landing on a new table
/// cannot be mistaken for the old one by an overlay that hasn't caught up yet.
static NEXT_TABLE_ID: AtomicU32 = AtomicU32::new(1);

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

/// Every table window currently tracked, newest-enumerated order. One overlay
/// window exists per entry.
pub fn tracked_tables() -> Vec<TrackedTable> {
    TABLES.lock().map(|t| t.clone()).unwrap_or_default()
}

/// One tracked table by its id, or `None` once its window has closed.
pub fn table_for(table_id: u32) -> Option<TrackedTable> {
    TABLES
        .lock()
        .ok()
        .and_then(|t| t.iter().find(|t| t.id == table_id).cloned())
}

/// The table name parsed from that table window's own title, which is
/// what every DB query for this table is scoped by. `None` when the table has
/// closed or its title didn't parse — see
/// `commands::get_active_table_players` for what the second case renders.
pub fn table_name_for(table_id: u32) -> Option<String> {
    table_for(table_id).and_then(|t| t.name)
}

/// The global-hotkey handler (default `Ctrl+Alt+H`, registered in
/// `lib.rs`'s `setup()`): resolves "the" table to act on from OS foreground
/// focus, not any Velora-side "active table" notion, so it always acts on
/// whichever table the user is actually looking at with several open.
/// Toggles that table's own dismissed state through the exact same
/// `OverlayRequest` plumbing the in-overlay Hide button and the HUD
/// Profiles page's Show button already use — this is a new trigger for that
/// mechanism, not a new one.
///
/// A no-op, not an error, when the foreground window isn't a tracked table
/// (PokerStars' own lobby, a different application, or nothing recognized)
/// — the hotkey is meant to be pressed at any time without looking, so
/// silently doing nothing is the only sane behavior outside a table.
pub fn toggle_hud_for_foreground_table() {
    let foreground = unsafe { GetForegroundWindow() };
    let hwnd = foreground.0 as isize as i64;
    let tables = tracked_tables();
    let Some(table) = table_for_hwnd(&tables, hwnd) else {
        return;
    };
    if manager::is_dismissed(table.id) {
        manager::request(manager::OverlayRequest::Show { table_id: table.id });
    } else {
        manager::request(manager::OverlayRequest::Dismiss { table_id: table.id });
    }
}

/// Last `RESYNC_LOG_CAP` overlay resync events (window acquired, WinEvent
/// callback fired, or a poll tick), newest last — diagnostics evidence for
/// multi-table debugging, so "is the overlay actually tracking a table
/// window, and how often" is something the user can read from Settings
/// instead of a claim to take on faith.
pub fn resync_log() -> Vec<ResyncLogEntry> {
    RESYNC_LOG
        .lock()
        .map(|g| g.iter().cloned().collect())
        .unwrap_or_default()
}

pub fn debug_snapshot() -> DebugSnapshot {
    // Any tracked table answers the UIPI question equally well — they are all
    // windows of the same PokerStars process.
    let table_integrity_level = tracked_tables().first().and_then(|t| {
        table_process_pid(HWND(t.hwnd as isize as *mut _))
            .and_then(process_integrity_level_for_pid)
            .map(integrity_level_name)
    });
    DebugSnapshot {
        hooks_installed: HOOKS_INSTALLED.load(Ordering::SeqCst),
        event_callbacks_total: EVENT_CALLBACKS_TOTAL.load(Ordering::SeqCst),
        event_callbacks_matched: EVENT_CALLBACKS_MATCHED.load(Ordering::SeqCst),
        poll_ticks: POLL_TICKS.load(Ordering::SeqCst),
        app_integrity_level: current_process_integrity_level().map(integrity_level_name),
        table_integrity_level,
    }
}

/// Every WinEvent hook `install_tracking` registers, one entry per hook — the
/// single source of truth for "how many hooks should there be", so a
/// diagnostics message reporting that count can be derived from this list's
/// length instead of a second hardcoded number that can silently drift out
/// of sync the next time a hook is added or removed.
const TRACKED_EVENTS: &[(&str, u32)] = &[
    ("EVENT_OBJECT_LOCATIONCHANGE", EVENT_OBJECT_LOCATIONCHANGE),
    ("EVENT_SYSTEM_MOVESIZEEND", EVENT_SYSTEM_MOVESIZEEND),
    ("EVENT_OBJECT_DESTROY", EVENT_OBJECT_DESTROY),
    ("EVENT_SYSTEM_MINIMIZESTART", EVENT_SYSTEM_MINIMIZESTART),
    ("EVENT_SYSTEM_MINIMIZEEND", EVENT_SYSTEM_MINIMIZEEND),
];

/// How many WinEvent hooks `install_tracking` is expected to register —
/// `TRACKED_EVENTS.len()`, so the diagnostics report can show "installed
/// N/expected" without a hardcoded expectation of its own.
pub fn expected_hook_count() -> u32 {
    TRACKED_EVENTS.len() as u32
}

/// Installs the WinEvent hooks and starts the low-frequency re-acquisition
/// poll. Call once, during Tauri's `setup()`, on the main thread (the same
/// thread already pumping the window message loop `SetWinEventHook`'s
/// `WINEVENT_OUTOFCONTEXT` callback needs).
pub fn install_tracking(app_handle: AppHandle) {
    let _ = APP_HANDLE.set(app_handle.clone());

    // One hook per event, each with eventMin == eventMax (the documented
    // way to hook exactly one event) — see the module doc comment above for
    // why a single hook spanning both events was wrong.
    for (name, event) in TRACKED_EVENTS.iter().copied() {
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

    // Reconcile immediately so tables that were already open when Velora
    // started get their overlays without waiting for the first poll tick,
    // then keep reconciling for tables that open and close later.
    reconcile_tables("startup");

    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_millis(1500));
        let tick = POLL_TICKS.fetch_add(1, Ordering::SeqCst) + 1;
        reconcile_tables("poll");

        // The actual polling fallback: unconditionally re-sync every
        // tracked table's overlay to its window's current rect, regardless of
        // whether the WinEvent hook has fired since the last tick.
        // `reconcile_tables` above has already re-read those rects and pushed
        // the ones that moved; this line is the periodic evidence that the
        // loop itself is alive.
        if tick % 20 == 0 {
            log(format!(
                "poll tick {tick}: tracking {} table(s) hooks_installed={} \
                 event_callbacks_total={} event_callbacks_matched={}",
                TABLES.lock().map(|t| t.len()).unwrap_or(0),
                HOOKS_INSTALLED.load(Ordering::SeqCst),
                EVENT_CALLBACKS_TOTAL.load(Ordering::SeqCst),
                EVENT_CALLBACKS_MATCHED.load(Ordering::SeqCst),
            ));
        }
    });
}

/// Brings `TABLES` in line with the table windows that actually exist right
/// now, and hands each difference to the overlay manager.
///
/// This is the whole multi-table story in one function: a window that appeared
/// gets an id and an overlay, a window that vanished loses its overlay, and a
/// window that moved, resized, was renamed or was minimized has its own
/// overlay — and only its own — brought back into line. It runs on the poll
/// thread, never on the main thread, because overlay *creation* must not
/// happen inside an event handler (see `overlay::manager`).
fn reconcile_tables(source: &'static str) {
    let live = enumerate_table_windows();

    let mut opened: Vec<TrackedTable> = Vec::new();
    let mut closed: Vec<TrackedTable> = Vec::new();
    let mut moved: Vec<TrackedTable> = Vec::new();
    let changed;

    {
        let Ok(mut tables) = TABLES.lock() else {
            return;
        };
        let before = tables.clone();

        tables.retain(|tracked| {
            let still_open = live.iter().any(|w| w.hwnd as i64 == tracked.hwnd);
            if !still_open {
                closed.push(tracked.clone());
            }
            still_open
        });

        for window in &live {
            match tables.iter_mut().find(|t| t.hwnd == window.hwnd as i64) {
                Some(existing) => {
                    let name = extract_table_name(&window.title);
                    let table_changed = existing.rect != window.rect
                        || existing.minimized != window.minimized
                        || existing.name != name;
                    if table_changed {
                        existing.rect = window.rect;
                        existing.minimized = window.minimized;
                        existing.name = name;
                        moved.push(existing.clone());
                    }
                }
                None => {
                    let table = TrackedTable {
                        id: NEXT_TABLE_ID.fetch_add(1, Ordering::SeqCst),
                        hwnd: window.hwnd as i64,
                        name: extract_table_name(&window.title),
                        rect: window.rect,
                        minimized: window.minimized,
                        // Stamped once, here, and never touched again —
                        // the floor every "active hand for this table" query
                        // is bound by, so a hand from an earlier sitting of a
                        // reused table name can never resolve as this one's.
                        // `now_played_at()`, not `now_iso()`: it has to be on
                        // the same clock as `played_at` for the comparison to
                        // mean anything — see that function's doc comment.
                        first_seen_at: now_played_at(),
                    };
                    log(format!(
                        "table {} opened ({source}): hwnd={:#x} name={:?} rect={:?}",
                        table.id, window.hwnd, table.name, table.rect
                    ));
                    tables.push(table.clone());
                    opened.push(table);
                }
            }
        }

        changed = *tables != before;
    }

    for table in &closed {
        log(format!(
            "table {} closed: hwnd={:#x} name={:?}",
            table.id, table.hwnd, table.name
        ));
    }
    // A table that only *moved* is repositioned straight away rather than
    // waiting behind the overlay thread's queue — repositioning an existing
    // window is safe from any thread, unlike creating one.
    for table in &moved {
        record_resync(table, source);
        manager::sync_overlay_bounds(table);
    }
    // Every tick, not only when something changed: `reconcile` is a full
    // create/destroy/reposition pass against this registry, so a window build
    // that failed once is retried on the next tick instead of leaving that one
    // table permanently without a HUD.
    manager::request(manager::OverlayRequest::Reconcile);

    if changed {
        broadcast_tracked_tables();
    }
}

/// Tells every window which tables are being followed right now, so the UI's
/// table list updates live as tables open and close. Replaces an older bare count
/// broadcast: the list is what a UI showing one HUD per table actually needs.
fn broadcast_tracked_tables() {
    let tables = tracked_tables();
    log(format!("tracked tables changed: {} open", tables.len()));
    if let Some(app_handle) = APP_HANDLE.get() {
        let _ = app_handle.emit(TRACKED_TABLES_EVENT, tables);
    }
}

fn record_resync(table: &TrackedTable, source: &'static str) {
    if let Ok(mut log) = RESYNC_LOG.lock() {
        if log.len() >= RESYNC_LOG_CAP {
            log.pop_front();
        }
        log.push_back(ResyncLogEntry {
            at: now_iso(),
            table_id: table.id,
            source,
            rect: table.rect,
        });
    }
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
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
    let raw = hwnd.0 as isize;

    if event == EVENT_OBJECT_LOCATIONCHANGE || event == EVENT_SYSTEM_MOVESIZEEND {
        // Matched against every tracked table, not one global handle, and
        // the update is pushed to that table's own overlay alone. This callback
        // runs on the main thread (a `WINEVENT_OUTOFCONTEXT` hook is delivered
        // through the message loop of the thread that installed it), so it may
        // reposition an existing window — Tauri runs those calls inline when it
        // is already on the event-loop thread — but must never *create* one:
        // building a webview from inside an event handler is the documented
        // Windows deadlock, and the reason overlay creation lives on
        // `overlay::manager`'s own thread.
        let updated = {
            let Ok(mut tables) = TABLES.lock() else {
                return;
            };
            let Some(table) = tables.iter_mut().find(|t| t.hwnd == raw as i64) else {
                return;
            };
            let Some(rect) = window_rect(hwnd) else {
                return;
            };
            let minimized = IsIconic(hwnd).as_bool();
            if table.rect == rect && table.minimized == minimized {
                return;
            }
            table.rect = rect;
            table.minimized = minimized;
            table.clone()
        };

        EVENT_CALLBACKS_MATCHED.fetch_add(1, Ordering::SeqCst);
        record_resync(&updated, "event");
        manager::sync_overlay_bounds(&updated);
        return;
    }

    // EVENT_OBJECT_DESTROY / EVENT_SYSTEM_MINIMIZESTART / EVENT_SYSTEM_MINIMIZEEND:
    // installed with no process/thread filter, so this fires for every window on
    // the whole system — cheaply check whether `hwnd` is even one of ours before
    // doing anything else. A destroyed window's rect can't be read any more, and
    // a mid-minimize window is more reliably resolved by a fresh full
    // enumeration than by hand-rolled incremental logic for this one case, so
    // this reuses the poll's already-tested `reconcile_tables` path instead of
    // patching a single field in place.
    let is_tracked = TABLES
        .lock()
        .map(|tables| tables.iter().any(|t| t.hwnd == raw as i64))
        .unwrap_or(false);
    if !is_tracked {
        return;
    }

    EVENT_CALLBACKS_MATCHED.fetch_add(1, Ordering::SeqCst);
    reconcile_tables("event");
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

/// One real table window as the enumeration found it — everything
/// `reconcile_tables` needs to decide whether it is new, unchanged or moved,
/// read once per window per tick rather than re-queried per comparison.
struct EnumeratedWindow {
    hwnd: isize,
    title: String,
    rect: WindowRect,
    minimized: bool,
}

/// Every real, logged-in PokerStars table window currently open, in
/// `EnumWindows` z-order (topmost first) — one tracked table, and one
/// overlay, per entry.
fn enumerate_table_windows() -> Vec<EnumeratedWindow> {
    let mut found: Vec<EnumeratedWindow> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut found as *mut Vec<EnumeratedWindow> as isize),
        );
    }
    found
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let found = &mut *(lparam.0 as *mut Vec<EnumeratedWindow>);

    if !IsWindowVisible(hwnd).as_bool() {
        return CONTINUE_ENUMERATION;
    }

    let mut class_buf = [0u16; 256];
    let class_len = GetClassNameW(hwnd, &mut class_buf);
    let class_name = String::from_utf16_lossy(&class_buf[..class_len.max(0) as usize]);

    let class_matches = POKERSTARS_TABLE_CLASS_CANDIDATES
        .iter()
        .any(|c| c.eq_ignore_ascii_case(&class_name));
    if !class_matches {
        return CONTINUE_ENUMERATION;
    }

    let title = window_title(hwnd);

    if !is_table_title(&title) {
        return CONTINUE_ENUMERATION;
    }

    let Some(rect) = window_rect(hwnd) else {
        return CONTINUE_ENUMERATION;
    };

    found.push(EnumeratedWindow {
        hwnd: hwnd.0 as isize,
        title,
        rect,
        minimized: IsIconic(hwnd).as_bool(),
    });
    // Keep enumerating: every table is needed, not just the first match.
    // This used to return `BOOL(0)`, which stops `EnumWindows` — with that,
    // only one table could ever be found.
    CONTINUE_ENUMERATION
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
/// the UIPI hypothesis: a lower-integrity Velora process can have its
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
            "Session: 05:11 - Aegle IV - No Limit Hold'em \u{20ac}0.01/\u{20ac}0.02 EUR - Logged In as HeroName"
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

    /// Regression test for the window-following root cause: `SetWinEventHook`'s
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
