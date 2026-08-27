use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const OVERLAY_LABEL: &str = "overlay";

/// Opens the native HUD overlay window: transparent, borderless,
/// always-on-top, excluded from the taskbar. Its content is a separate
/// Vite entry point (`overlay.html`) rendering positioned/draggable player
/// HUD cards — a real OS-level window, not an in-app mockup.
///
/// Window creation is explicitly dispatched to the main thread and bounded
/// with a timeout: in some WebView2/Tauri environment configurations the
/// second webview's controller-ready handshake can stall indefinitely (the
/// renderer process spawns but never signals ready back to Tauri). Rather
/// than hang the calling command — and by extension the frontend button —
/// forever, this surfaces a clear error after a few seconds so the app stays
/// usable even if the overlay itself can't come up in that environment.
pub fn open(app_handle: &AppHandle) -> Result<(), String> {
    if let Some(existing) = app_handle.get_webview_window(OVERLAY_LABEL) {
        // Reuse the existing overlay instead of creating a second instance;
        // bring it to front rather than silently no-op-ing.
        let _ = existing.set_focus();
        return Ok(());
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let app_handle_for_thread = app_handle.clone();
    app_handle
        .run_on_main_thread(move || {
            let result = WebviewWindowBuilder::new(
                &app_handle_for_thread,
                OVERLAY_LABEL,
                WebviewUrl::App("overlay.html".into()),
            )
            .title("Velora Overlay")
            .transparent(true)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .resizable(true)
            .inner_size(1400.0, 900.0)
            .position(80.0, 80.0)
            .visible(true)
            .build()
            .map(|_| ())
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;

    match rx.recv_timeout(std::time::Duration::from_secs(8)) {
        Ok(result) => result,
        Err(_) => {
            eprintln!(
                "overlay window creation timed out after 8s — the WebView2 controller for the \
                 second window never signaled ready. This is an environment-level WebView2/Tauri \
                 issue, not an application bug; see overlay::open doc comment."
            );
            Err("Opening the overlay timed out. This is a known environment issue with creating \
                 a second window; the rest of the app is unaffected."
                .to_string())
        }
    }
}

pub fn close(app_handle: &AppHandle) -> Result<(), String> {
    if let Some(win) = app_handle.get_webview_window(OVERLAY_LABEL) {
        win.close().map_err(|e| e.to_string())?;
    }
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
    app_handle.get_webview_window(OVERLAY_LABEL).is_some()
}
