//! Hides the Windows taskbar while the dock is active and restores its previous state.
//!
//! Auto-hide (which frees the work area) is combined with making the taskbar
//! window invisible and click-through. `ShowWindow(SW_HIDE)` does not work on
//! Windows 11 because Explorer shows the taskbar again.
//!
//! The original auto-hide setting is saved the first time the taskbar is taken
//! over and deleted only when it is restored. If the dock exits unexpectedly
//! and restarts, its own auto-hide setting is not mistaken for the user's.

use std::{env, fs, mem::size_of, path::PathBuf};

use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM},
        UI::{
            Shell::{ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage},
            WindowsAndMessaging::{
                FindWindowExW, FindWindowW, GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA,
                SetLayeredWindowAttributes, SetWindowLongPtrW, WS_EX_LAYERED, WS_EX_TRANSPARENT,
            },
        },
    },
    core::{PCWSTR, w},
};

const PRIMARY: PCWSTR = w!("Shell_TrayWnd");
const SECONDARY: PCWSTR = w!("Shell_SecondaryTrayWnd");
const GHOST_STYLES: isize = (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) as isize;

fn state_file() -> PathBuf {
    PathBuf::from(env::var_os("LOCALAPPDATA").unwrap_or_default())
        .join("Docky")
        .join("taskbar-original")
}

/// Hides the taskbar on all monitors.
///
/// Idempotent; can be called again after Explorer restarts or Windows redraws
/// the taskbar.
pub fn take_over() {
    let file = state_file();
    if !file.exists() {
        let original = if autohide_enabled() { "1" } else { "0" };
        let saved = file
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&file, original));
        if let Err(error) = saved {
            // Never hide the system taskbar unless the guard can restore its
            // original state after a crash or hang.
            eprintln!("docky: no se pudo guardar el estado de la barra: {error}");
            return;
        }
    }
    set_autohide(true);
    for hwnd in taskbars() {
        set_ghost(hwnd, true);
    }
}

/// What the dock and its watchdog do when the dock stops: give the taskbar
/// back, except while Windows shuts down, restarts or signs out with Docky
/// set to start again. Then the taskbar stays auto-hidden for the next
/// sign-in; given back, Windows showed it in full at every start until the
/// dock took it over again. Turning autostart off, quitting or uninstalling
/// still gives it back.
pub fn give_back_on_exit() {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SHUTTINGDOWN};
    // SAFETY: Read-only system query.
    let session_ending = unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) } != 0;
    if session_ending && crate::autostart::is_enabled() {
        return;
    }
    give_back();
}

/// Restores the taskbar to its state before `take_over`. Has no effect if the
/// dock did not take it over.
pub fn give_back() {
    let file = state_file();
    let Ok(original) = fs::read_to_string(&file) else {
        // No saved state; make sure the taskbar is visible anyway.
        for hwnd in taskbars() {
            set_ghost(hwnd, false);
        }
        return;
    };
    for hwnd in taskbars() {
        set_ghost(hwnd, false);
    }
    set_autohide(original.trim() == "1");
    let _ = fs::remove_file(file);
}

fn taskbars() -> Vec<HWND> {
    let mut found = Vec::new();
    // SAFETY: Read-only calls with valid static strings.
    unsafe {
        if let Ok(hwnd) = FindWindowW(PRIMARY, None) {
            found.push(hwnd);
        }
        let mut previous = None;
        while let Ok(hwnd) = FindWindowExW(None, previous, SECONDARY, None) {
            found.push(hwnd);
            previous = Some(hwnd);
        }
    }
    found
}

fn appbar_data() -> APPBARDATA {
    APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        hWnd: taskbars().first().copied().unwrap_or_default(),
        ..Default::default()
    }
}

fn autohide_enabled() -> bool {
    let mut data = appbar_data();
    // SAFETY: `data` is a valid APPBARDATA with its size initialized.
    let state = unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) };
    state as u32 & ABS_AUTOHIDE != 0
}

fn set_autohide(enabled: bool) {
    let mut data = appbar_data();
    data.lParam = LPARAM(if enabled { ABS_AUTOHIDE as isize } else { 0 });
    // SAFETY: Same as above.
    unsafe { SHAppBarMessage(ABM_SETSTATE, &mut data) };
}

fn set_ghost(hwnd: HWND, ghost: bool) {
    // SAFETY: `hwnd` comes from FindWindow; if the window no longer exists, the calls have no effect.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ghost {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | GHOST_STYLES);
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
        } else if style & GHOST_STYLES != 0 {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !GHOST_STYLES);
        }
    }
}
