use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[cfg(windows)]
pub mod hittest;
pub mod manager;
pub mod plan;

/// Label of the overlay window declared statically in `tauri.conf.json`. It
/// is a *prototype*, never shown: every per-table overlay window is
/// cloned from its config with a new label and URL — see `manager` for why it
/// is still declared at all.
pub const OVERLAY_PROTOTYPE_LABEL: &str = "overlay";

/// The window label for one pool slot. Slots are numbered from 1 and outlive
/// the tables they serve, so this is *not* a table id — `manager` owns the
/// slot-to-table mapping, and `manager::label_for_table` is the only way to go
/// from a table to its window.
pub fn overlay_label(slot: u32) -> String {
    format!("{OVERLAY_PROTOTYPE_LABEL}{slot}")
}

/// Which table an overlay event is about, and what changed.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayVisibility {
    pub table_id: u32,
    pub visible: bool,
}

/// Which table's HUD *content* was collapsed or restored by hand — see
/// `OVERLAY_DISMISSED_EVENT`. Deliberately its own type, not a reuse of
/// `OverlayVisibility`: the two describe different things (a window actually
/// on/off screen vs. a deliberate per-table content toggle) and conflating
/// them would let a window-level event — restoring a minimized table, say —
/// incorrectly un-collapse a HUD the user dismissed on purpose.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayDismissed {
    pub table_id: u32,
    pub dismissed: bool,
}

/// The `AppHandle`, so code reached from the tracking layer can find windows
/// without one being threaded through every call. Set once during `setup`.
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

pub fn app_handle() -> Option<AppHandle> {
    APP_HANDLE.get().cloned()
}

/// Broadcast (payload: `OverlayVisibility`) whenever one table's overlay
/// window is shown or hidden. Emitted from here, rather than from the command
/// layer, so that *every* path which changes overlay visibility notifies every
/// window. The main window's "Open/Close Overlay" button previously kept a
/// local React flag that went stale the moment the overlay was closed from
/// its own in-overlay "Close" button, so the label advertised the opposite of
/// what the next click would do. The payload carries the table id because,
/// with one overlay per table, "the overlay closed" is only half
/// an answer.
pub const OVERLAY_VISIBILITY_EVENT: &str = "overlay-visibility-changed";

/// Broadcast (payload: `OverlayDismissed`) whenever one table's HUD content is
/// collapsed or restored by hand — `overlay::manager`'s `Dismiss`/`Show`
/// requests, however they were triggered (the overlay's own "Hide"/"Show"
/// pill, the main window's table list, or the global hotkey). The overlay
/// itself is this event's main subscriber: it is what lets clicking "Show" on
/// *any* of those three restore the HUD on the actual window, which stays up
/// and on-screen the whole time rather than being hidden and rebuilt.
pub const OVERLAY_DISMISSED_EVENT: &str = "overlay-dismissed-changed";

// There is deliberately no overlay "mode". A whole-window Reposition mode
// once cleared click-through for an entire overlay so a card could be
// dragged, and while it lasted the table's Fold/Call/Raise were unreachable.
// Chips are now dragged straight from their own hot zone, and the hit test
// (`hittest::wants_click_through`) looks at nothing but those zones.

/// Records the `AppHandle` so the tracking layer can reach windows without
/// one being threaded through every native callback. Called once from
/// `setup()`, before the overlay thread starts.
pub fn install(app_handle: &AppHandle) {
    let _ = APP_HANDLE.set(app_handle.clone());
}

/// One always-clickable rectangle, in CSS pixels relative to the overlay
/// webview's own viewport — the coordinate space `getBoundingClientRect()`
/// hands the frontend, so it reports exactly what it measured and nothing
/// has to reason about DPI on the JavaScript side.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotZone {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Replaces one overlay's set of always-clickable rectangles, scaling the
/// frontend's CSS pixels by that window's own current scale factor into the
/// client pixels a cursor point lands in after `ScreenToClient`. The frontend
/// re-reports on every layout change (chips moving or being dragged, its
/// tracked table resizing), so a rect never outlives the control it describes.
///
/// Scoped per overlay: with one global list, the last table to
/// report would decide where every *other* table's overlay stopped swallowing
/// clicks.
pub fn set_hot_zones(
    app_handle: &AppHandle,
    table_id: u32,
    zones: &[HotZone],
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let Some(label) = manager::label_for_table(table_id) else {
            return Ok(());
        };
        let scale = app_handle
            .get_webview_window(&label)
            .and_then(|w| w.scale_factor().ok())
            .unwrap_or(1.0);
        let converted = zones
            .iter()
            .map(|z| hittest::ClientRect {
                x: (z.x * scale).round() as i32,
                y: (z.y * scale).round() as i32,
                width: (z.width * scale).round().max(0.0) as i32,
                height: (z.height * scale).round().max(0.0) as i32,
            })
            .collect();
        hittest::set_hot_zones(&label, converted);
    }
    #[cfg(not(windows))]
    {
        let _ = (app_handle, table_id, zones);
    }
    Ok(())
}

/// Live state of the click-through tracker, per overlay window — its
/// label and HWND, how many hot zones it currently claims and
/// whether clicks are passing through it right now — plus the shared tracker's
/// tick and style-write counters. Diagnostics only; see `hittest` for why each
/// of these is worth having.
pub struct HitTestProbe {
    pub overlays: Vec<OverlayProbe>,
    pub ticks: u64,
    pub style_writes: u64,
}

pub struct OverlayProbe {
    pub label: String,
    pub hwnd: isize,
    pub hot_zones: usize,
    pub click_through: bool,
}

pub fn hit_test_probe() -> HitTestProbe {
    #[cfg(windows)]
    {
        let (probes, ticks, style_writes) = hittest::probe();
        HitTestProbe {
            overlays: probes
                .into_iter()
                .map(|p| OverlayProbe {
                    label: p.label,
                    hwnd: p.hwnd,
                    hot_zones: p.hot_zones,
                    click_through: p.click_through,
                })
                .collect(),
            ticks,
            style_writes,
        }
    }
    #[cfg(not(windows))]
    {
        HitTestProbe {
            overlays: Vec::new(),
            ticks: 0,
            style_writes: 0,
        }
    }
}

/// Whether one table's overlay window exists and is on screen right now.
pub fn is_open(app_handle: &AppHandle, table_id: u32) -> bool {
    manager::label_for_table(table_id)
        .and_then(|label| app_handle.get_webview_window(&label))
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}
