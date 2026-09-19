//! One overlay window per tracked PokerStars table.
//!
//! ## Why window creation lives on its own thread
//!
//! Velora's single worst bug to date was the overlay window existing as a
//! native window while its WebView2 controller never finished initialising —
//! no renderer process, nothing ever painted. The first fix was to stop
//! building the window with `WebviewWindowBuilder` from inside the `open_overlay` **command
//! handler** and declare it statically in `tauri.conf.json` instead, so Tauri
//! creates it during its own startup bootstrap.
//!
//! Multi-table support needs windows created after startup, which looked
//! like the broken pattern. So the mechanism was checked against the actual
//! Tauri source rather than assumed, and "not after startup" turned out to be
//! one step short of the real rule. Tauri
//! 2.11.5's own doc comment on `WebviewWindowBuilder::new` says:
//!
//! > On Windows, this function deadlocks when used in a synchronous command
//! > and event handlers, see [the Webview2 issue].
//! > You should use `async` commands and separate threads when creating
//! > windows.
//!
//! (`tauri-2.11.5/src/webview/webview_window.rs`, linking wry#583.) The
//! offending ingredient was never "after startup" — it was "from a
//! synchronous command handler", which is exactly what `open_overlay` was.
//! The three patterns Tauri documents as safe are the `setup` hook, an `async`
//! command, and **a separate thread**; the last is what this module is.
//!
//! Nothing else in the codebase may build an overlay window: `win_event_proc`
//! (main thread, an event handler) only ever repositions windows that already
//! exist, via `sync_overlay_bounds`.
//!
//! ## Why overlay windows are pooled and never destroyed
//!
//! The first version of this module created a window when a table opened and
//! destroyed it when the table closed. That was **measured to deadlock**, and
//! the measurement is the reason the pool exists, so it is worth recording
//! precisely: driving four simulated table windows through repeated
//! open-all/close-all cycles, the 24th window build wedged. The overlay thread
//! stopped consuming its queue, and — the telling part — the tracker's
//! `event_callbacks_total` counter froze at the same moment, so the main
//! thread's message loop had stopped pumping too. The event loop and the
//! overlay thread were waiting on each other.
//!
//! The mechanism fits: creating a WebView2 webview runs a *nested* message
//! pump inside the event loop while it waits for the controller-created
//! callback, and that nested pump dispatches whatever else is queued —
//! including another window create or destroy. Rapid create/destroy churn
//! therefore re-enters window management inside window management.
//!
//! So overlay windows are now created once and reused. A table closing
//! releases its window back to the pool (hidden, table-less) instead of
//! destroying it; the next table to open takes it and is pointed at its own
//! `overlay.html?table=<id>` with `navigate`. Window *creation* is capped at
//! one per reconciliation pass, and only ever happens when more tables are
//! open at once than at any earlier point in the session — a handful of times
//! per run, never in a churn.
//!
//! That is also simply better behaviour: each overlay owns a WebView2 renderer
//! process, and a player who opens and closes tables all session would
//! otherwise spend the whole session tearing them down and rebuilding them.
//!
//! There is exactly one `destroy` in this module, and it is not part of that
//! lifecycle: a window that was built but turned out to have no HWND is
//! unusable (it could never be hit-tested, so it could only ever eat the
//! table's clicks), never enters the pool, and is destroyed on the spot rather
//! than left behind for the life of the process. A *pooled*
//! window is still never destroyed. See `create_window` for why that one call
//! cannot deadlock this thread.
//!
//! ## The prototype window
//!
//! `tauri.conf.json` still declares the `overlay` window statically, hidden,
//! and it is never shown. It survives as the single source of truth for every
//! overlay's window properties — transparent, undecorated, always-on-top,
//! skip-taskbar, no shadow — which each pooled window is cloned from via
//! `WebviewWindowBuilder::from_config`, so those properties cannot drift
//! between the config and the code. It also means the process still creates
//! one overlay webview through Tauri's own startup bootstrap, a path known to
//! be reliable, before any dynamic one is built.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};

use tauri::utils::config::WindowConfig;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use super::plan::{self, Assignment, SlotState};
use super::{
    OverlayDismissed, OverlayMode, OverlayVisibility, OVERLAY_DISMISSED_EVENT,
    OVERLAY_PROTOTYPE_LABEL, OVERLAY_VISIBILITY_EVENT,
};
use crate::table_track::{self, TrackedTable, WindowRect};

/// Work for the overlay thread. Deliberately payload-free where it can be:
/// `Reconcile` re-reads `table_track`'s registry itself, so there is no
/// second copy of "which tables exist" to drift out of date.
pub enum OverlayRequest {
    /// Make the set of shown overlay windows match the tracked-table registry.
    Reconcile,
    /// Global HUD kill switch (persisted as `overlay_enabled`).
    SetEnabled(bool),
    /// Collapse one table's HUD content to a "Show" pill, without stopping
    /// tracking or touching the window itself — the overlay's own
    /// "Hide" button. Comes back on the next `SetEnabled(true)`, when that
    /// table window is closed and reopened, or on a `Show` for that table.
    Dismiss { table_id: u32 },
    /// Un-dismiss one table, restoring its full HUD content without touching
    /// any other table or the global switch — the overlay's own "Show" pill,
    /// for a table dismissed by hand.
    Show { table_id: u32 },
}

/// One pooled overlay window: a native window that outlives the tables it
/// serves, and whichever table currently owns it.
struct PoolSlot {
    /// Stable Tauri window label, `overlay1`, `overlay2`, … by pool position.
    /// Not a table id — a slot is re-pointed at a new table over its life.
    label: String,
    /// The table this window is currently showing, or `None` when idle.
    table_id: Option<u32>,
}

static SENDER: OnceLock<Sender<OverlayRequest>> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(true);
static POOL: Mutex<Vec<PoolSlot>> = Mutex::new(Vec::new());

/// Source of pool-slot numbers, and monotonic on purpose. Numbering from
/// `pool.len() + 1` would collide the moment a slot is ever dropped (the
/// "window vanished" path): a pool of overlay1/overlay3 would try to build
/// "overlay3" again, and Tauri refuses a duplicate label, so that slot could
/// never be rebuilt.
static NEXT_SLOT: AtomicU32 = AtomicU32::new(1);

/// Tables whose HUD *content* the user collapsed by hand. Not
/// consulted by `reconcile`'s window-assignment pass at all — a dismissed
/// table's window stays up and positioned exactly like any other tracked
/// table's, so the overlay's own frontend can render a "Show" pill on it
/// instead of the full HUD. This set exists purely so `is_dismissed` can
/// answer "which state should this table's overlay render in", both for the
/// overlay itself on mount and for `OVERLAY_VISIBILITY_EVENT` broadcasts.
static DISMISSED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Diagnostics: how many overlay windows this process has built, how many
/// times a pooled one was handed to a different table, and how many builds
/// failed. Dynamic window creation is the historically fragile part, so the counts are in the diagnostics report rather than
/// only in a terminal.
static WINDOWS_CREATED: AtomicU64 = AtomicU64::new(0);
static WINDOWS_REUSED: AtomicU64 = AtomicU64::new(0);
static CREATE_FAILURES: AtomicU64 = AtomicU64::new(0);

fn log(msg: impl AsRef<str>) {
    eprintln!("[overlay] {}", msg.as_ref());
}

/// Starts the overlay thread. Called once from `setup()`, with the persisted
/// kill-switch state.
pub fn start(app_handle: AppHandle, enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);

    let (tx, rx) = mpsc::channel::<OverlayRequest>();
    if SENDER.set(tx).is_err() {
        return;
    }

    std::thread::spawn(move || {
        for request in rx {
            match request {
                OverlayRequest::Reconcile => reconcile(&app_handle),
                OverlayRequest::SetEnabled(enabled) => {
                    ENABLED.store(enabled, Ordering::SeqCst);
                    if enabled {
                        // Turning the HUD back on clears every by-hand
                        // dismissal: the switch means "show my HUDs", and
                        // leaving one table silently opted out of that would
                        // be a silent gap in HUD coverage.
                        if let Ok(mut dismissed) = DISMISSED.lock() {
                            dismissed.clear();
                        }
                    }
                    reconcile(&app_handle);
                }
                OverlayRequest::Dismiss { table_id } => {
                    if let Ok(mut dismissed) = DISMISSED.lock() {
                        if !dismissed.contains(&table_id) {
                            dismissed.push(table_id);
                        }
                    }
                    // The window itself is untouched by a dismissal
                    // — `wanted_table_ids` no longer excludes it, so there is
                    // nothing for `reconcile` to do here. Just tell every
                    // window (this table's own overlay included) that its
                    // content should collapse.
                    emit_dismissed(&app_handle, table_id, true);
                }
                OverlayRequest::Show { table_id } => {
                    if let Ok(mut dismissed) = DISMISSED.lock() {
                        dismissed.retain(|id| *id != table_id);
                    }
                    emit_dismissed(&app_handle, table_id, false);
                }
            }
        }
    });
}

/// Queues work for the overlay thread. A no-op before `start`, which is only
/// reachable in tests and on non-Windows builds.
pub fn request(request: OverlayRequest) {
    if let Some(sender) = SENDER.get() {
        let _ = sender.send(request);
    }
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Whether the user collapsed this table's HUD content by hand (its own
/// "Hide" button) and hasn't shown it again since. Read by the overlay
/// itself on mount (`is_overlay_dismissed`) to decide whether to paint the
/// full HUD or the "Show" pill in the same spot, and by the main window's
/// table list for its status label — both derived from this one flag.
pub fn is_dismissed(table_id: u32) -> bool {
    DISMISSED
        .lock()
        .map(|d| d.contains(&table_id))
        .unwrap_or(false)
}

/// The window label currently serving one table, or `None` when that table has
/// no overlay. Every table-scoped overlay command resolves through this: the
/// label belongs to a pool slot, not to the table.
pub fn label_for_table(table_id: u32) -> Option<String> {
    POOL.lock().ok().and_then(|pool| {
        pool.iter()
            .find(|slot| slot.table_id == Some(table_id))
            .map(|slot| slot.label.clone())
    })
}

/// `(windows built, pooled windows re-pointed at a new table, failed builds)`.
pub fn window_lifecycle_counts() -> (u64, u64, u64) {
    (
        WINDOWS_CREATED.load(Ordering::SeqCst),
        WINDOWS_REUSED.load(Ordering::SeqCst),
        CREATE_FAILURES.load(Ordering::SeqCst),
    )
}

/// Brings the set of *shown* overlay windows in line with the tracked-table
/// registry: one window per table, sitting exactly on it; nothing shown for a
/// table that has closed or is minimized. A dismissed table's window is
/// still shown — only its *content* collapses, decided by the overlay's
/// own frontend from `manager::is_dismissed`, not by this function.
///
/// A full reconciliation rather than a stream of create/destroy deltas, so a
/// window build that failed is simply retried on the next pass instead of
/// leaving that one table permanently HUD-less.
///
/// Every *decision* this pass makes lives in `overlay::plan`, which is pure and
/// unit-tested; this function is only the part that has to touch real windows.
fn reconcile(app_handle: &AppHandle) {
    let tables = table_track::tracked_tables();
    let tracked_ids: Vec<u32> = tables.iter().map(|t| t.id).collect();
    let wanted = plan::wanted_table_ids(&tracked_ids, is_enabled());

    let plan = {
        let Ok(pool) = POOL.lock() else {
            return;
        };
        let slots: Vec<SlotState> = pool
            .iter()
            .map(|slot| SlotState {
                label: slot.label.clone(),
                table_id: slot.table_id,
            })
            .collect();
        plan::plan_reconcile(&slots, &wanted, NEXT_SLOT.load(Ordering::SeqCst))
    };

    // Release first, so a table that just closed frees its window for a table
    // that just opened instead of forcing a new one to be built. The planner
    // has already counted on that having happened.
    if !plan.released.is_empty() {
        // Captured before each slot's `table_id` is cleared. This fires
        // only for a table that actually closed or for the kill switch
        // going off — a plain per-table dismissal no longer reaches here at
        // all (see `OverlayRequest::Dismiss`, which emits this same event
        // itself, directly, since the window it's about is never released).
        let mut released_table_ids: Vec<u32> = Vec::new();
        if let Ok(mut pool) = POOL.lock() {
            for slot in pool.iter_mut() {
                if plan.released.iter().any(|label| *label == slot.label) {
                    if let Some(table_id) = slot.table_id.take() {
                        released_table_ids.push(table_id);
                    }
                }
            }
        }
        for label in &plan.released {
            release_window(app_handle, label);
        }
        for table_id in released_table_ids {
            emit_visibility(app_handle, table_id, false);
        }
    }

    // Claim the slot number the plan spent before building anything with it:
    // the counter must never go backwards, even if the build below fails.
    NEXT_SLOT.store(plan.next_slot, Ordering::SeqCst);

    for assignment in &plan.assignments {
        let Some(table) = tables.iter().find(|t| t.id == assignment.table_id()) else {
            continue;
        };
        match assignment {
            Assignment::Sync { label, .. } => {
                if let Some(window) = app_handle.get_webview_window(label) {
                    apply_bounds(&window, table);
                }
            }
            Assignment::Repoint { label, .. } => repoint_window(app_handle, label, table),
            Assignment::Create { label, .. } => create_window(app_handle, label, table),
            // Nothing to do: at most one window is built per pass, and the
            // next poll tick picks this table up.
            Assignment::Defer { .. } => {}
        }
    }
}

/// Hands an idle pooled window to a table: points it at that table's own
/// overlay URL and drops everything the previous table left behind.
fn repoint_window(app_handle: &AppHandle, label: &str, table: &TrackedTable) {
    let Some(window) = app_handle.get_webview_window(label) else {
        // The window is gone but the pool still lists it — drop the slot, and
        // the hit tester's entry with it, so the 60Hz tracker stops writing
        // styles to a HWND that no longer exists and the next pass builds a
        // replacement.
        log(format!("pooled overlay '{label}' has vanished; dropping its slot"));
        #[cfg(windows)]
        super::hittest::remove(label);
        if let Ok(mut pool) = POOL.lock() {
            pool.retain(|slot| slot.label != label);
        }
        return;
    };

    // Hot zones and mode describe the previous table's cards. Cleared before
    // this window is claimed, so it can never spend a frame claiming clicks
    // over a point where the new table's HUD draws nothing.
    #[cfg(windows)]
    {
        super::hittest::set_hot_zones(label, Vec::new());
        super::hittest::set_mode(label, OverlayMode::Normal);
    }

    // Re-pointing by URL, not by an event, keeps the window's own URL the one
    // source of truth for which table it belongs to — the frontend reads it at
    // mount and never has to be told again.
    match window.url() {
        Ok(mut url) => {
            url.set_query(Some(&format!("table={}", table.id)));
            if let Err(err) = window.navigate(url) {
                log(format!("failed to re-point '{label}' at table {}: {err}", table.id));
                return;
            }
        }
        Err(err) => {
            log(format!("cannot read '{label}' url: {err}"));
            return;
        }
    }
    let _ = window.set_title(&window_title_for(table));

    if let Ok(mut pool) = POOL.lock() {
        if let Some(slot) = pool.iter_mut().find(|slot| slot.label == label) {
            slot.table_id = Some(table.id);
        }
    }
    WINDOWS_REUSED.fetch_add(1, Ordering::SeqCst);
    log(format!(
        "re-pointed overlay '{label}' at table {} ({:?})",
        table.id, table.name
    ));

    apply_bounds(&window, table);
}

/// Builds a new pooled overlay window for `table`. **Only ever called from the
/// overlay thread** — see this module's header for why that matters on
/// Windows.
fn create_window(app_handle: &AppHandle, label: &str, table: &TrackedTable) {
    let label = label.to_string();

    let Some(mut config) = prototype_config(app_handle) else {
        log(format!(
            "cannot create overlay for table {}: no '{OVERLAY_PROTOTYPE_LABEL}' window declared in tauri.conf.json",
            table.id
        ));
        CREATE_FAILURES.fetch_add(1, Ordering::SeqCst);
        return;
    };

    config.label = label.clone();
    // The table id travels in the URL so the overlay's own frontend knows
    // which table it is showing before it makes a single backend call.
    config.url = WebviewUrl::App(format!("overlay.html?table={}", table.id).into());
    // Built hidden and shown only once it has been moved onto its table:
    // created at the prototype's own coordinates it would flash at 80,80 first.
    config.visible = false;
    // Neither flag is cosmetic. An overlay appearing must never take the
    // foreground away from the table underneath it — with several tables
    // opening at once that would be a burst of focus changes mid-hand, the
    // same class of bug as a focus-stealing click on a pagination dot. `focus: false` makes the
    // first show a `SW_SHOWNOACTIVATE`, and `focusable: false` is what makes
    // tao itself put `WS_EX_NOACTIVATE` in the window's computed style — so
    // tao *keeps* the bit when it re-applies styles, instead of dropping one
    // written out of band, which is exactly what `hittest::apply_style`'s own
    // comment records happening before.
    config.focus = false;
    config.focusable = false;
    config.title = window_title_for(table);

    let window = match WebviewWindowBuilder::from_config(app_handle, &config) {
        Ok(builder) => match builder.build() {
            Ok(window) => window,
            Err(err) => {
                log(format!("failed to build overlay '{label}': {err}"));
                CREATE_FAILURES.fetch_add(1, Ordering::SeqCst);
                return;
            }
        },
        Err(err) => {
            log(format!("failed to configure overlay '{label}': {err}"));
            CREATE_FAILURES.fetch_add(1, Ordering::SeqCst);
            return;
        }
    };

    // From here on `WS_EX_TRANSPARENT` belongs to the hit-test tracker alone —
    // nothing else may write it, or the two would fight over the same bit.
    let _ = window.set_ignore_cursor_events(false);

    #[cfg(windows)]
    {
        match window.hwnd() {
            Ok(hwnd) => {
                super::hittest::install(&label, hwnd.0 as isize);
                super::hittest::set_mode(&label, OverlayMode::Normal);
            }
            // Without hit-testing this window would capture every click across
            // its whole footprint, including the ones meant for Fold/Call/Raise
            // — locking the user out of the table. So it is never shown. It must not be *kept*
            // either: it is not in the pool, and since nothing here destroys a
            // pooled window, an unpooled one would never be destroyed by
            // anything else and would sit on the process for the rest of the
            // run. It is destroyed here instead, and the next
            // pass builds a fresh one under a new label.
            //
            // This does not reopen the create/destroy deadlock, for three
            // reasons that are about *this* call rather than destroys in
            // general:
            //
            // 1. It cannot block this thread. `WebviewWindow::destroy` is the
            //    one dispatcher call that deliberately does not go through
            //    `send_user_message` — it posts `WindowMessage::Destroy` to the
            //    event-loop proxy and returns (tauri-runtime-wry 2.11.4,
            //    `WindowDispatcher::destroy`, whose own comment says destroy
            //    "cannot use the `send_user_message` function"). Off the main
            //    thread nothing waits for a reply, so the overlay thread and
            //    the event loop cannot end up waiting on each other, which is
            //    what the churn deadlock was.
            // 2. It is not nested inside a window creation. `build()` has
            //    already returned, so the webview's nested message pump — the
            //    mechanism that made churn re-enter window management — has
            //    finished before this runs.
            // 3. It is not churn. At most one window is built per
            //    reconciliation pass, so this can happen at most once per pass,
            //    and only on a failure path that has never yet been observed.
            //
            // `NEXT_SLOT` has already moved past this label (`reconcile` stores
            // the plan's counter before executing it), so the destroyed label is
            // never requested again and cannot collide with the retry.
            Err(err) => {
                log(format!(
                    "overlay '{label}' has no HWND ({err}); destroying it rather than leaving a click-eating window behind"
                ));
                if let Err(err) = window.destroy() {
                    log(format!(
                        "failed to destroy unusable overlay '{label}': {err}"
                    ));
                }
                CREATE_FAILURES.fetch_add(1, Ordering::SeqCst);
                return;
            }
        }
    }

    if let Ok(mut pool) = POOL.lock() {
        pool.push(PoolSlot {
            label: label.clone(),
            table_id: Some(table.id),
        });
    }
    WINDOWS_CREATED.fetch_add(1, Ordering::SeqCst);
    log(format!(
        "created overlay '{label}' for table {} ({:?}) at {:?}",
        table.id, table.name, table.rect
    ));

    apply_bounds(&window, table);
}

/// Hides a pooled window and clears everything specific to the table it was
/// serving, leaving it ready for the next one. The window itself stays alive —
/// see this module's header for the deadlock that destroying it caused.
fn release_window(app_handle: &AppHandle, label: &str) {
    #[cfg(windows)]
    {
        // Hot zones describe cards that are no longer on screen; a stale list
        // would keep claiming those points on a window that is about to be
        // handed to a different table.
        super::hittest::set_hot_zones(label, Vec::new());
        super::hittest::set_mode(label, OverlayMode::Normal);
    }

    if let Some(window) = app_handle.get_webview_window(label) {
        hide_window(&window);
        let _ = window.set_title("Velora Overlay (idle)");
    }
    log(format!("released overlay '{label}' back to the pool"));
}

/// Distinctive per-table window title. Invisible to the user (overlays are
/// undecorated and skip the taskbar) and exact for diagnostics and tests,
/// which is the only reason it carries the table id.
fn window_title_for(table: &TrackedTable) -> String {
    format!(
        "Velora Overlay {} - {}",
        table.id,
        table.name.as_deref().unwrap_or("unnamed table")
    )
}

/// Moves one existing overlay onto its table's current bounds. Safe to call
/// from the main thread (Tauri runs these inline when it is already on the
/// event-loop thread), which is what `win_event_proc` relies on.
pub fn sync_overlay_bounds(table: &TrackedTable) {
    let Some(app_handle) = crate::overlay::app_handle() else {
        return;
    };
    let Some(label) = label_for_table(table.id) else {
        return;
    };
    let Some(window) = app_handle.get_webview_window(&label) else {
        return;
    };
    apply_bounds(&window, table);
}

/// The overlay mirrors its table window 1:1, so a position saved as a fraction
/// of the table maps straight onto overlay viewport percentages with no
/// further conversion (`table_track::overlay_bounds_for_table`).
fn apply_bounds(window: &WebviewWindow, table: &TrackedTable) {
    if table.minimized {
        // Windows parks a minimized window at roughly (-32000, -32000);
        // following it there would leave a stack of invisible overlays
        // off-screen and burn a resync per tick on each.
        if window.is_visible().unwrap_or(false) {
            hide_window(window);
            emit_visibility_from(window, table.id, false);
        }
        return;
    }

    let bounds: WindowRect = table_track::overlay_bounds_for_table(table.rect);
    let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
        x: bounds.x,
        y: bounds.y,
    }));
    let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize {
        width: bounds.width.max(1) as u32,
        height: bounds.height.max(1) as u32,
    }));

    if !window.is_visible().unwrap_or(false) {
        show_without_activating(window);
        emit_visibility_from(window, table.id, true);
    }
}

/// Shows an overlay without letting it take the foreground.
///
/// Tauri's own `show()` goes through tao, which issues a plain `SW_SHOW` —
/// activating — on every show after the first (its "don't focus" marker is
/// consumed by the initial one). That would pull focus off the poker table
/// every time a pooled window is handed to a new table or a minimized one is
/// restored. `SW_SHOWNOACTIVATE` is the same call without that side effect.
///
/// Visibility is then this module's to own end to end — see `hide_window`.
fn show_without_activating(window: &WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};

        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                let _ = ShowWindow(HWND(hwnd.0 as *mut _), SW_SHOWNOACTIVATE);
            }
            return;
        }
    }
    let _ = window.show();
}

/// Hides an overlay, at the same level `show_without_activating` shows it.
///
/// Tauri's `hide()` cannot be mixed with that: tao decides what to do from the
/// *diff* against its own cached window flags, and a window shown behind its
/// back still reads as hidden there — so `hide()` computes an empty diff and
/// returns without ever issuing `SW_HIDE`. Measured, not deduced: released
/// pool windows stayed on screen as "Velora Overlay (idle)". Reading
/// visibility is unaffected either way, because tao answers `is_visible` from
/// `IsWindowVisible` rather than from its cache.
fn hide_window(window: &WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                let _ = ShowWindow(HWND(hwnd.0 as *mut _), SW_HIDE);
            }
            return;
        }
    }
    let _ = window.hide();
}

/// The statically-declared `overlay` window's own config — every pooled
/// window is a clone of it with a new label, URL and title.
fn prototype_config(app_handle: &AppHandle) -> Option<WindowConfig> {
    app_handle
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == OVERLAY_PROTOTYPE_LABEL)
        .cloned()
}

fn emit_visibility(app_handle: &AppHandle, table_id: u32, visible: bool) {
    let _ = app_handle.emit(
        OVERLAY_VISIBILITY_EVENT,
        OverlayVisibility { table_id, visible },
    );
}

fn emit_visibility_from(window: &WebviewWindow, table_id: u32, visible: bool) {
    emit_visibility(window.app_handle(), table_id, visible);
}

fn emit_dismissed(app_handle: &AppHandle, table_id: u32, dismissed: bool) {
    let _ = app_handle.emit(
        OVERLAY_DISMISSED_EVENT,
        OverlayDismissed { table_id, dismissed },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::overlay_label;

    /// Pool slots are numbered from 1 and their labels must never collide with
    /// the statically-declared prototype, which is a real window in the same
    /// namespace — a collision would make `from_config` fail on every build.
    #[test]
    fn pool_slot_labels_are_distinct_from_the_prototype() {
        for slot in 1u32..=8 {
            assert_ne!(overlay_label(slot), OVERLAY_PROTOTYPE_LABEL);
        }
        assert_eq!(overlay_label(1), "overlay1");
        assert_eq!(overlay_label(2), "overlay2");
    }

    /// The pool starts empty, so nothing claims to be serving a table before
    /// any window has been built — `label_for_table` is what every table-scoped
    /// overlay command resolves through, and a wrong answer here would point a
    /// command at another table's window.
    #[test]
    fn no_label_resolves_for_a_table_with_no_overlay() {
        assert_eq!(label_for_table(4_294_967_295), None);
    }
}
