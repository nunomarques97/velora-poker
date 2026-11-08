//! Keeps every shown overlay inside the always-on-top band.
//!
//! Overlays are built `alwaysOnTop` (the prototype in `tauri.conf.json`), and
//! that used to be the end of it. A live session on 2026-11-08 showed it is
//! not: after the player busted from one tournament, both remaining overlays
//! were still visible, positioned on their tables and painting their chips,
//! but they sat *below* the PokerStars table windows in the z-order — still
//! carrying `WS_EX_TOPMOST`, yet enumerated after non-topmost windows — so the
//! tables covered them and the HUD looked gone. It happened again nine
//! minutes later without any table closing. Re-asserting `HWND_TOPMOST` from
//! outside the process brought both HUDs straight back, which is what this
//! module now does from inside it. The trigger itself was not pinned down.
//!
//! The check is cheap and only acts when something is wrong: an overlay is
//! out of the band exactly when some *visible, non-topmost* window sits above
//! it, because Windows keeps every window of the topmost band above every
//! window outside it. When that is the case the overlay is put back at the top
//! of the band with `SWP_ASYNCWINDOWPOS`, so the caller (the hit-test thread,
//! which does not own the window) never waits on the event loop — the same
//! reason overlay windows are only ever built on the manager's own thread
//! (see `manager`). No window is created or destroyed here, and nothing
//! moves, resizes or takes focus.

use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindow, GetWindowLongPtrW, IsWindowVisible, SetWindowPos, GWL_EXSTYLE, GW_HWNDPREV,
    HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
    WS_EX_TOPMOST,
};

/// Upper bound on how far up the z-order one check walks. A desktop has a few
/// hundred top-level windows at most; the bound only guards against a z-order
/// that changes under the walk and never reaches the top.
const MAX_WINDOWS_ABOVE: usize = 4096;

/// How many times an overlay had to be put back on top this run. Diagnostics.
static RESTORES: AtomicU64 = AtomicU64::new(0);

pub fn restore_count() -> u64 {
    RESTORES.load(Ordering::SeqCst)
}

/// One window above the overlay, as far as the band check cares.
#[derive(Debug, Clone, Copy)]
pub struct Above {
    pub visible: bool,
    pub topmost: bool,
}

/// Whether an overlay with these windows above it has fallen out of the
/// always-on-top band. Only a visible window counts: hidden ones (idle pooled
/// overlays, the never-shown prototype, tool windows) cover nothing.
pub fn out_of_topmost_band(above: impl IntoIterator<Item = Above>) -> bool {
    above.into_iter().any(|w| w.visible && !w.topmost)
}

fn windows_above(hwnd: HWND) -> Vec<Above> {
    let mut above = Vec::new();
    let mut current = hwnd;
    while above.len() < MAX_WINDOWS_ABOVE {
        current = match unsafe { GetWindow(current, GW_HWNDPREV) } {
            Ok(prev) if !prev.0.is_null() => prev,
            _ => break,
        };
        let ex = unsafe { GetWindowLongPtrW(current, GWL_EXSTYLE) };
        above.push(Above {
            visible: unsafe { IsWindowVisible(current) }.as_bool(),
            topmost: ex & WS_EX_TOPMOST.0 as isize != 0,
        });
    }
    above
}

/// Whether this overlay is currently below a visible window outside the band.
pub fn is_out_of_band(hwnd_raw: isize) -> bool {
    out_of_topmost_band(windows_above(HWND(hwnd_raw as *mut _)))
}

/// Puts one visible overlay back on top if it has fallen out of the band.
/// Returns whether it had to.
pub fn keep_on_top(hwnd_raw: isize) -> bool {
    if hwnd_raw == 0 || !is_out_of_band(hwnd_raw) {
        return false;
    }
    let hwnd = HWND(hwnd_raw as *mut _);
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
        )
    };
    RESTORES.fetch_add(1, Ordering::SeqCst);
    eprintln!("[overlay] hwnd {hwnd_raw} had dropped below a table; put back on top");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, ShowWindow, HWND_BOTTOM, SW_SHOWNOACTIVATE,
        WINDOW_EX_STYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    fn above(visible: bool, topmost: bool) -> Above {
        Above { visible, topmost }
    }

    #[test]
    fn only_topmost_windows_above_is_inside_the_band() {
        assert!(!out_of_topmost_band([]));
        assert!(!out_of_topmost_band([above(true, true), above(true, true)]));
    }

    #[test]
    fn a_visible_normal_window_above_means_the_overlay_fell_out() {
        assert!(out_of_topmost_band([above(true, true), above(true, false)]));
    }

    #[test]
    fn hidden_windows_above_do_not_count() {
        assert!(!out_of_topmost_band([above(false, false), above(true, true)]));
    }

    /// A small, off-screen, non-activating popup that shows up in the z-order
    /// without ever being seen or taking focus — the test runs on a desktop
    /// someone may be using.
    fn test_window(ex: WINDOW_EX_STYLE) -> HWND {
        let hwnd = unsafe {
            CreateWindowExW(
                ex | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("velora zorder test"),
                WS_POPUP,
                -20000,
                -20000,
                8,
                8,
                None,
                None,
                None,
                None,
            )
        }
        .expect("create test window");
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        hwnd
    }

    /// The 2026-11-08 failure, reproduced with real windows: an always-on-top
    /// window pushed below an ordinary one is detected, put back on top, and
    /// left alone once it is.
    #[test]
    fn an_overlay_pushed_below_a_table_is_put_back_on_top() {
        let overlay = test_window(WS_EX_TOPMOST);
        let table = test_window(WINDOW_EX_STYLE(0));

        assert!(!is_out_of_band(overlay.0 as isize), "a new topmost window starts inside the band");

        unsafe {
            SetWindowPos(overlay, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)
                .expect("push the overlay to the bottom");
        }
        assert!(is_out_of_band(overlay.0 as isize), "the table now covers the overlay");

        assert!(keep_on_top(overlay.0 as isize), "the guard acts on a covered overlay");
        assert!(!is_out_of_band(overlay.0 as isize), "the overlay is back above the table");
        assert!(!keep_on_top(overlay.0 as isize), "and nothing happens once it is");

        unsafe {
            let _ = DestroyWindow(overlay);
            let _ = DestroyWindow(table);
        }
    }
}
