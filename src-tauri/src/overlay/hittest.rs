//! Per-point overlay click-through — Windows only.
//!
//! ## Why this exists
//!
//! Originally the overlay had exactly one knob for "can the user click through
//! me": `set_ignore_cursor_events`, applied to the whole window and left there
//! until something toggled it back. That is all-or-nothing, and both of its
//! states are wrong for real play:
//!
//! - ignore_cursor_events(true) — the table underneath is clickable, but
//!   nothing of Velora's is: the overlay's own control bar became unreachable
//!   by a real click, which locked the user out mid-session, and the HUD cards' pagination dots were dead too.
//! - ignore_cursor_events(false) — the dots and the control bar work, but the
//!   overlay swallows every click across its whole footprint, including the
//!   fully transparent areas over Fold/Call/Raise.
//!
//! What is needed is that same decision made *per point*: click-through
//! everywhere except a handful of small rectangles the frontend keeps up to
//! date (its HUD chips, its "Show" pill and an open detail panel).
//!
//! ## Why not WM_NCHITTEST
//!
//! The textbook mechanism is subclassing the overlay's window procedure and
//! answering `WM_NCHITTEST` with `HTTRANSPARENT` outside those rectangles.
//! That was implemented first and **measured not to work here**, which is
//! worth writing down so nobody spends the afternoon on it again:
//!
//! `SetWindowSubclass` on the overlay's top-level HWND succeeded, and the
//! subclass received messages (a counter proved it), but across a full run of
//! cursor moves and real clicks over the overlay it was asked to hit-test
//! **zero** times. Windows resolves a mouse point by descending the window
//! tree and asking the *deepest* window under it first, bubbling up only
//! while a window answers `HTTRANSPARENT`. The overlay's client area is
//! completely covered by WebView2's own child window, which answers
//! `HTCLIENT` and ends the search — and that window lives in the
//! `msedgewebview2` process, so it cannot be subclassed from here at all.
//! The top-level's own hit test is never reached. `WM_MOUSEACTIVATE` never
//! arrived either, for the same reason.
//!
//! The extended-style bits do not have that problem: they are checked
//! structurally during the descent, and a window carrying them is skipped
//! together with its entire child tree — which is precisely why the old
//! whole-window flag worked, WebView2 child and all.
//!
//! ## What this does instead
//!
//! A dedicated thread reads `GetCursorPos` at ~60Hz, maps it into the
//! overlay's client area, and sets or clears `CLICK_THROUGH_BITS` so the
//! style always matches the point the cursor is actually on: cleared over a
//! registered hot zone, set everywhere else. `wants_click_through` is that
//! whole decision, and it depends on nothing but the zones and the point:
//! there is no mode that makes a whole window capture the mouse. (There used
//! to be — a "Reposition" mode that cleared click-through for the entire
//! overlay so a card could be dragged, which took Fold/Call/Raise away from
//! the player until they found the control to leave it. Chips are now
//! dragged straight from their own hot zone.) The style is only written when
//! the answer changes, so the steady state is one `GetCursorPos` per frame and
//! nothing else.
//!
//! Same class of native work as `table_track::win`'s `SetWinEventHook`: one
//! polling thread started once at startup, driven by process-wide statics.
//!
//! ## One instance became a registry
//!
//! This started as a single global instance — one HWND, one hot-zone list —
//! because there was only ever one overlay window to hit-test against. With
//! one overlay per tracked table that shape would have every overlay sharing
//! the last one's hot zones. Each registered overlay now owns its own entry; the cursor is
//! still read once per tick for all of them, since only one window can be
//! under it anyway and the rest simply resolve to "not over a hot zone",
//! which is the right answer for them regardless.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, IsWindowVisible, SetWindowLongPtrW, GWL_EXSTYLE,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
};

/// A hot-zone rectangle in the overlay window's *client* pixels (already
/// multiplied by the window's scale factor — the frontend reports CSS pixels).
#[derive(Debug, Clone, Copy)]
pub struct ClientRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ClientRect {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Everything the tracker needs to decide one overlay window's click-through
/// state, and nothing shared with any other overlay.
struct OverlayHitState {
    /// The overlay's own label, as `overlay::overlay_label(table_id)` built it
    /// — the key every command addresses this entry by.
    label: String,
    hwnd: isize,
    zones: Vec<ClientRect>,
    /// Mirror of the style last written for this window, so the steady state
    /// costs no `GetWindowLongPtrW`.
    click_through: bool,
}

/// The click-through bits, set and cleared together.
///
/// `WS_EX_TRANSPARENT` on its own does **not** make a window transparent to
/// the mouse — measured, not assumed: with only that bit set, the tracker
/// reported click-through while `WindowFromPoint` still returned the
/// overlay's webview and the window underneath received nothing. It is the
/// layered-plus-transparent pair that Windows honours, which is also exactly
/// what Tauri's own `set_ignore_cursor_events` writes — the call the
/// overlay's old whole-window Lock used, and the one part of that mechanism
/// which was always working.
const CLICK_THROUGH_BITS: isize = (WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0) as isize;

/// How often the cursor is sampled. 16ms is one frame at 60Hz: fast enough
/// that the style is already correct by the time a human who has moved onto a
/// 4px dot presses the button, and cheap enough to leave running (one
/// `GetCursorPos` per tick, and a style write only when the answer changes).
const CURSOR_POLL: Duration = Duration::from_millis(16);

/// Every overlay window currently registered, one entry each. A `Vec`
/// for the same reasons as `table_track::win`'s table registry: a handful of
/// entries, ordered, and `const`-initialisable so it needs no `LazyLock`.
static OVERLAYS: Mutex<Vec<OverlayHitState>> = Mutex::new(Vec::new());
static TRACKER_STARTED: AtomicBool = AtomicBool::new(false);

// Diagnostics, surfaced in the diagnostics report. "Is the tracker alive, and what is
// it deciding" is the first question any regression here starts with, and
// behaviour alone cannot answer it — an overlay that swallows clicks looks
// identical whether the tracker died, the hot zones went stale, or the style
// write failed.
static TICKS: AtomicU64 = AtomicU64::new(0);
static STYLE_WRITES: AtomicU64 = AtomicU64::new(0);

/// One overlay's live hit-test state, for the diagnostics report.
pub struct Probe {
    pub label: String,
    pub hwnd: isize,
    pub hot_zones: usize,
    pub click_through: bool,
}

/// Every registered overlay's state, plus the tracker's own counters.
pub fn probe() -> (Vec<Probe>, u64, u64) {
    let probes = OVERLAYS
        .lock()
        .map(|overlays| {
            overlays
                .iter()
                .map(|o| Probe {
                    label: o.label.clone(),
                    hwnd: o.hwnd,
                    hot_zones: o.zones.len(),
                    click_through: o.click_through,
                })
                .collect()
        })
        .unwrap_or_default();
    (
        probes,
        TICKS.load(Ordering::SeqCst),
        STYLE_WRITES.load(Ordering::SeqCst),
    )
}

pub fn set_hot_zones(label: &str, zones: Vec<ClientRect>) {
    if let Ok(mut overlays) = OVERLAYS.lock() {
        if let Some(overlay) = overlays.iter_mut().find(|o| o.label == label) {
            overlay.zones = zones;
        }
    }
    // A card that just moved out from under the cursor, or a newly reported
    // control bar, changes the answer for the point the cursor is on right now.
    apply_for_current_cursor();
}

/// Registers one overlay window and starts the shared cursor tracker, which
/// owns that window's extended style from here on. Idempotent per label: a
/// re-registered label keeps its zones but takes the new HWND.
pub fn install(label: &str, hwnd_raw: isize) {
    if hwnd_raw == 0 {
        return;
    }
    if let Ok(mut overlays) = OVERLAYS.lock() {
        match overlays.iter_mut().find(|o| o.label == label) {
            Some(existing) => existing.hwnd = hwnd_raw,
            None => overlays.push(OverlayHitState {
                label: label.to_string(),
                hwnd: hwnd_raw,
                zones: Vec::new(),
                click_through: false,
            }),
        }
    }

    if !TRACKER_STARTED.swap(true, Ordering::SeqCst) {
        std::thread::spawn(|| loop {
            std::thread::sleep(CURSOR_POLL);
            TICKS.fetch_add(1, Ordering::SeqCst);
            apply_for_current_cursor();
        });
    }
}

/// Forgets one overlay, called before its window is destroyed so the tracker
/// can never write a style to a HWND that is being torn down.
pub fn remove(label: &str) {
    if let Ok(mut overlays) = OVERLAYS.lock() {
        overlays.retain(|o| o.label != label);
    }
}

#[cfg(test)]
fn is_installed(label: &str) -> bool {
    OVERLAYS
        .lock()
        .map(|overlays| overlays.iter().any(|o| o.label == label))
        .unwrap_or(false)
}

/// Decides, for wherever the cursor is right now, whether each registered
/// overlay should be letting clicks through, and writes the styles that
/// changed. One `GetCursorPos` serves every overlay: the cursor can only be
/// over one of them, and for the rest `ScreenToClient` lands outside every hot
/// zone, which is the answer they want anyway.
fn apply_for_current_cursor() {
    let mut cursor = POINT::default();
    let cursor_ok = unsafe { GetCursorPos(&mut cursor) }.is_ok();

    let Ok(mut overlays) = OVERLAYS.lock() else {
        return;
    };
    for overlay in overlays.iter_mut() {
        let hwnd = HWND(overlay.hwnd as *mut _);
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            continue;
        }

        let point = if cursor_ok {
            let mut pt = cursor;
            unsafe { ScreenToClient(hwnd, &mut pt) }
                .as_bool()
                .then_some((pt.x, pt.y))
        } else {
            None
        };
        let want_click_through = wants_click_through(&overlay.zones, point);

        overlay.click_through = want_click_through;
        apply_style(hwnd, want_click_through);
    }
}

/// The whole click-through decision for one overlay, free of any OS call:
/// clicks pass through to the table at every point outside the overlay's own
/// hot zones, and only a point inside one of them stays with the overlay.
/// `None` is a cursor that could not be read or mapped into the window; it
/// passes through too, because guessing "opaque" is the failure that takes
/// the table away from the player.
pub fn wants_click_through(zones: &[ClientRect], point: Option<(i32, i32)>) -> bool {
    match point {
        Some((x, y)) => !zones.iter().any(|z| z.contains(x, y)),
        None => true,
    }
}

/// Writes the overlay's extended style, but only when it does not already say
/// what it should. Read on every tick rather than only when the click-through
/// answer changes, because `WS_EX_NOACTIVATE` below is not ours alone to
/// keep: tao re-applies its own computed styles on show/always-on-top/focus
/// operations and silently drops anything written out of band, which is
/// exactly how the first attempt at this lost the bit (measured: set during
/// `setup()`, absent from the live window afterwards). Re-deriving the whole
/// style each tick makes that self-healing instead of a race.
fn apply_style(hwnd: HWND, click_through: bool) {
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);

        // Clicking the overlay must not pull focus off the table.
        // Measured before this was added: with the table focused, one real
        // click on a (since removed) pagination dot flipped the page *and* made the overlay the
        // foreground window, so the player's own table quietly stopped being
        // the active window mid-hand. `WS_EX_NOACTIVATE` declines that
        // activation while still delivering the click; the overlay takes no
        // keyboard input, so it gives up nothing.
        let mut next = current | WS_EX_NOACTIVATE.0 as isize;
        next = if click_through {
            next | CLICK_THROUGH_BITS
        } else {
            next & !CLICK_THROUGH_BITS
        };

        if next != current {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
            STYLE_WRITES.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registers a bare entry with no real window behind it. Every test below
    /// exercises the decision, never the OS calls, so a HWND that no longer
    /// exists is exactly what is wanted: `apply_for_current_cursor` skips it
    /// on `IsWindowVisible` and touches nothing.
    fn register(label: &str) {
        install(label, 0x1000);
    }

    /// The tracker's own decision for one overlay, asked without a real window
    /// in the way. A poisoned lock answers "no": an overlay that stops eating
    /// table clicks is the safe failure, never the other way round.
    fn point_in_hot_zone(label: &str, x: i32, y: i32) -> bool {
        match OVERLAYS.lock() {
            Ok(overlays) => overlays
                .iter()
                .find(|o| o.label == label)
                .map(|o| !wants_click_through(&o.zones, Some((x, y))))
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    #[test]
    fn a_rect_contains_its_top_left_but_not_its_bottom_right_edge() {
        let r = ClientRect { x: 10, y: 20, width: 30, height: 40 };
        assert!(r.contains(10, 20));
        assert!(r.contains(39, 59));
        assert!(!r.contains(40, 59));
        assert!(!r.contains(39, 60));
        assert!(!r.contains(9, 20));
    }

    /// The tracker's decision, isolated from the OS calls around it: a point
    /// inside any registered zone keeps the overlay clickable, and everything
    /// else lets the click through to the table.
    /// The pure decision the tracker applies: outside every zone the click
    /// goes to the table, inside any zone it stays with the overlay, and an
    /// unreadable cursor never makes the window capture.
    #[test]
    fn click_through_is_decided_only_by_the_hot_zones() {
        let chip = ClientRect { x: 100, y: 200, width: 60, height: 22 };
        let pill = ClientRect { x: 700, y: 10, width: 40, height: 18 };
        let zones = [chip, pill];

        assert!(!wants_click_through(&zones, Some((100, 200))), "chip top-left");
        assert!(!wants_click_through(&zones, Some((159, 221))), "chip bottom-right pixel");
        assert!(!wants_click_through(&zones, Some((720, 20))), "Show pill");
        assert!(wants_click_through(&zones, Some((160, 210))), "just right of the chip");
        assert!(wants_click_through(&zones, Some((400, 300))), "bare table");
        assert!(wants_click_through(&zones, Some((-5, -5))), "outside the window");
        assert!(wants_click_through(&zones, None), "cursor unreadable");
        assert!(wants_click_through(&[], Some((100, 200))), "no zones: all click-through");
    }

    /// No state inverts the decision for a whole window: with a zone covering
    /// one corner, every sampled point outside it is click-through.
    #[test]
    fn no_point_outside_the_zones_ever_captures() {
        let zones = [ClientRect { x: 0, y: 0, width: 50, height: 50 }];
        for x in (0..800).step_by(25) {
            for y in (0..600).step_by(25) {
                let inside = x < 50 && y < 50;
                assert_eq!(wants_click_through(&zones, Some((x, y))), !inside, "({x},{y})");
            }
        }
    }

    #[test]
    fn only_registered_zones_keep_the_overlay_clickable() {
        let label = "overlay901";
        register(label);
        set_hot_zones(
            label,
            vec![
                ClientRect { x: 1250, y: 12, width: 138, height: 29 },
                ClientRect { x: 230, y: 449, width: 36, height: 8 },
            ],
        );
        assert!(point_in_hot_zone(label, 1300, 20), "Show pill");
        assert!(point_in_hot_zone(label, 248, 453), "a chip");
        assert!(!point_in_hot_zone(label, 700, 600), "bare overlay");
        set_hot_zones(label, Vec::new());
        assert!(
            !point_in_hot_zone(label, 1300, 20),
            "cleared zones stop claiming their points"
        );
        remove(label);
    }

    /// The reason the registry exists: one global zone list meant table B's
    /// cards decided whether table A's overlay swallowed a click.
    #[test]
    fn overlays_do_not_share_hot_zones() {
        let (a, b) = ("overlay902", "overlay903");
        register(a);
        register(b);

        set_hot_zones(a, vec![ClientRect { x: 0, y: 0, width: 100, height: 100 }]);
        set_hot_zones(b, vec![ClientRect { x: 500, y: 500, width: 100, height: 100 }]);

        assert!(point_in_hot_zone(a, 50, 50), "A's own zone");
        assert!(!point_in_hot_zone(b, 50, 50), "A's zone must not claim B's points");
        assert!(point_in_hot_zone(b, 550, 550), "B's own zone");
        assert!(!point_in_hot_zone(a, 550, 550), "B's zone must not claim A's points");

        remove(a);
        remove(b);
    }

    /// A destroyed overlay must leave nothing behind for the 60Hz tracker to
    /// walk into, and must not answer for its old label.
    #[test]
    fn removing_an_overlay_forgets_its_zones() {
        let label = "overlay904";
        register(label);
        set_hot_zones(label, vec![ClientRect { x: 0, y: 0, width: 10, height: 10 }]);
        assert!(is_installed(label));

        remove(label);

        assert!(!is_installed(label));
        assert!(!point_in_hot_zone(label, 5, 5));
    }
}
