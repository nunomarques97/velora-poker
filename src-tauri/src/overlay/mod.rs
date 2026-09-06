use tauri::{AppHandle, Emitter, Manager};

pub const OVERLAY_LABEL: &str = "overlay";

/// Broadcast (payload: `bool` = "is now visible") whenever the overlay window
/// is shown or hidden. Emitted from here, rather than from the command layer,
/// so that *every* path which changes overlay visibility notifies every
/// window. The main window's "Open/Close Overlay" button previously kept a
/// local React flag that went stale the moment the overlay was closed from
/// its own in-overlay "Close" button, so the label advertised the opposite of
/// what the next click would do (Phase E polish, a known issue).
pub const OVERLAY_VISIBILITY_EVENT: &str = "overlay-visibility-changed";

/// Shows the native HUD overlay window: transparent, borderless,
/// always-on-top, excluded from the taskbar. Its content is a separate
/// Vite entry point (`overlay.html`) rendering positioned/draggable player
/// HUD cards — a real OS-level window, not an in-app mockup.
///
/// the window is now declared *statically* in
/// `tauri.conf.json` (`visible: false`) instead of built dynamically here
/// via `WebviewWindowBuilder`. Rationale: a statically-declared window's
/// WebView2 controller is created and awaited during Tauri's own startup
/// sequence, which is guaranteed to pump the async controller-ready
/// handshake correctly; a window built on-demand from inside a command
/// handler (the previous approach) depends on that same async completion
/// being pumped correctly *after* startup, which was never verified and
/// matches's process-level evidence (native window exists,
/// WebView2 controller never finishes initializing, no renderer process
/// ever spawns) better than anything ruled out in. This
/// function now just grabs the pre-existing window and shows it.
///
/// See the notes for the full diagnosis history before
/// attempting another fix here — through #7 already ruled out
/// `transparent`/`decorations`/`always_on_top` (individually and
/// combined), a resize-nudge workaround, devtools inspection, and a
/// minimal-repro isolation attempt (inconclusive for unrelated reasons).
pub fn open(app_handle: &AppHandle) -> Result<(), String> {
    let window = app_handle
        .get_webview_window(OVERLAY_LABEL)
        .ok_or_else(|| "overlay window is not declared in tauri.conf.json".to_string())?;

    // WebView2 on Windows sometimes never establishes render bounds for a
    // webview until the host window receives a resize event, leaving it
    // blank/"(Not Responding)" indefinitely. Forcing a trivial resize is
    // the standard workaround for this exact symptom (kept from;
    // tauri-apps/tauri#8632, #12975).
    if let Ok(size) = window.inner_size() {
        let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize {
            width: size.width + 1,
            height: size.height,
        }));
        let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize {
            width: size.width,
            height: size.height,
        }));
    }

    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    let _ = app_handle.emit(OVERLAY_VISIBILITY_EVENT, true);
    Ok(())
}

pub fn close(app_handle: &AppHandle) -> Result<(), String> {
    if let Some(win) = app_handle.get_webview_window(OVERLAY_LABEL) {
        win.hide().map_err(|e| e.to_string())?;
    }
    let _ = app_handle.emit(OVERLAY_VISIBILITY_EVENT, false);
    Ok(())
}

/// Toggles whether mouse events pass through the overlay to the window
/// underneath it (the poker client), vs. being captured so the user can
/// drag HUD cards to reposition them.
pub fn set_click_through(app_handle: &AppHandle, enabled: bool) -> Result<(), String> {
    let win = app_handle
        .get_webview_window(OVERLAY_LABEL)
        .ok_or_else(|| "overlay window is not open".to_string())?;
    win.set_ignore_cursor_events(enabled).map_err(|e| e.to_string())
}

pub fn is_open(app_handle: &AppHandle) -> bool {
    // the window is always declared (visible: false at startup),
    // so "open" now means "visible," not "exists."
    app_handle
        .get_webview_window(OVERLAY_LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}
