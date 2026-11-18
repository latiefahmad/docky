//! Open windows and the apps they belong to.
//!
//! An app identifier is a shell name that can be opened and resolved to an icon
//! and display name: the lowercase executable path or, for packaged (Store) apps,
//! `shell:AppsFolder\<AppUserModelID>`.

use std::collections::HashMap;

use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT},
        Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
        Storage::{
            EnhancedStorage::PKEY_AppUserModel_ID, Packaging::Appx::GetApplicationUserModelId,
        },
        System::Threading::{
            GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
        UI::{
            Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
            WindowsAndMessaging::{
                EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetForegroundWindow,
                GetWindow, GetWindowLongW, GetWindowPlacement, GetWindowRect, GetWindowTextLengthW,
                GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed,
                SM_CXPADDEDBORDER, SM_CXSIZEFRAME, WINDOWPLACEMENT, WPF_RESTORETOMAXIMIZED,
                WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_MAXIMIZE, WS_THICKFRAME,
            },
        },
    },
    core::{BOOL, PWSTR},
};

const APPS_FOLDER: &str = "shell:AppsFolder\\";
const FRAME_HOST_CLASS: &str = "ApplicationFrameWindow";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackedWindow {
    pub hwnd: isize,
    pub app_id: String,
    pub minimized: bool,
}

#[must_use]
pub fn normalize_id(id: &str) -> String {
    if id.starts_with(APPS_FOLDER) {
        id.to_owned()
    } else {
        id.to_lowercase()
    }
}

/// Lists app windows and tracks which app each window belongs to.
///
/// Identifying a window's app requires opening its process or querying shell
/// properties, which makes each lookup expensive. The cache limits this to once
/// per window, when it appears.
#[derive(Debug, Default)]
pub struct WindowTracker {
    app_ids: HashMap<isize, String>,
}

impl WindowTracker {
    pub fn list(&mut self) -> Vec<TrackedWindow> {
        unsafe extern "system" fn collect(hwnd: HWND, list: LPARAM) -> BOOL {
            // SAFETY: `list` points to the Vec passed by `list` and remains
            // alive for the entire synchronous enumeration.
            unsafe { (*(list.0 as *mut Vec<HWND>)).push(hwnd) };
            true.into()
        }

        let mut all: Vec<HWND> = Vec::new();
        // SAFETY: The Vec pointer remains valid until EnumWindows returns.
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM(&raw mut all as isize));
        }

        let windows: Vec<TrackedWindow> = all
            .into_iter()
            .filter(|&hwnd| is_app_window(hwnd))
            .filter_map(|hwnd| {
                let key = hwnd.0 as isize;
                let app_id = match self.app_ids.get(&key) {
                    Some(id) => id.clone(),
                    // The newly created UWP app has not declared an identifier
                    // yet; do not cache it so the lookup can be retried.
                    None => {
                        let id = app_id(hwnd)?;
                        self.app_ids.insert(key, id.clone());
                        id
                    }
                };
                Some(TrackedWindow {
                    hwnd: key,
                    app_id,
                    minimized: unsafe { IsIconic(hwnd).as_bool() },
                })
            })
            .collect();

        // Remove closed windows because Windows reuses their handles.
        self.app_ids
            .retain(|hwnd, _| windows.iter().any(|window| window.hwnd == *hwnd));
        windows
    }
}

#[must_use]
pub fn title(hwnd: isize) -> String {
    let mut buffer = [0u16; 256];
    // SAFETY: The buffer remains alive during the call, which returns the number of characters written.
    let len = unsafe { GetWindowTextW(HWND(hwnd as *mut _), &mut buffer) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

/// Restored-state rectangle of a window, also valid while it is minimized.
/// Workspace coordinates match screen coordinates here: the work area starts at
/// the screen's top-left corner because the dock reserves only the bottom edge.
#[must_use]
pub fn normal_rect(hwnd: isize) -> Option<RECT> {
    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: The structure has its size initialized and lives for the call.
    unsafe { GetWindowPlacement(HWND(hwnd as *mut _), &mut placement) }.ok()?;
    // Placement uses workspace coordinates, whose origin is the top-left
    // corner of the work area: below the menu bar when it is shown.
    let (dx, dy) = crate::system::primary_monitor().map_or((0, 0), |monitor| {
        (
            monitor.work_left - monitor.screen.left,
            monitor.work_top - monitor.screen.top,
        )
    });
    let local = placement.rcNormalPosition;
    let rect = RECT {
        left: local.left + dx,
        top: local.top + dy,
        right: local.right + dx,
        bottom: local.bottom + dy,
    };
    (rect.right > rect.left && rect.bottom > rect.top).then_some(rect)
}

/// Where a window is on screen now, and whether it is maximized; `None`
/// while it is minimized. Taken whenever the dock looks at its windows, so a
/// minimize knows where a snapped window was: Windows keeps a snapped
/// window's pre-snap rectangle as its "normal" one.
#[must_use]
pub fn screen_rect(hwnd: isize) -> Option<(RECT, bool)> {
    let window = HWND(hwnd as *mut _);
    // SAFETY: Read-only queries on a window handle.
    unsafe {
        if IsIconic(window).as_bool() {
            return None;
        }
        let mut rect = RECT::default();
        GetWindowRect(window, &mut rect).ok()?;
        Some((rect, IsZoomed(window).as_bool()))
    }
}

/// Where a window that has just started minimizing was on screen, which by
/// then it no longer says itself: it is already at -32000.
///
/// Returns the whole window, invisible resize borders included (where its
/// live thumbnail goes), and the part that was visible on screen (what the
/// genie copies and flies). `last_seen` is the [`screen_rect`] taken while
/// it was not maximized, if any.
///
/// A maximized window fills its monitor's work area, plus its borders: its
/// "normal" rectangle is the smaller one it returns to when un-maximized,
/// and flying from it made the genie jump first.
#[must_use]
pub fn rect_before_minimize(hwnd: isize, last_seen: Option<RECT>) -> Option<(RECT, RECT)> {
    use windows::Win32::{
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        },
        UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
    };
    let window = HWND(hwnd as *mut _);
    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: The structure has its size initialized and lives for the call.
    unsafe { GetWindowPlacement(window, &mut placement) }.ok()?;
    if placement.flags.0 & WPF_RESTORETOMAXIMIZED.0 == 0 {
        let rect = last_seen.or_else(|| normal_rect(hwnd))?;
        return Some((rect, rect));
    }
    // SAFETY: Read-only monitor, DPI and style queries.
    unsafe {
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let work = info.rcWork;
        // Windows maximizes a resizable window with its borders past the
        // work area's edges, so that only the window itself shows.
        let resizable = GetWindowLongW(window, GWL_STYLE) as u32 & WS_THICKFRAME.0 != 0;
        let border = if resizable {
            let dpi = GetDpiForWindow(window);
            GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi)
                + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        } else {
            0
        };
        let whole = RECT {
            left: work.left - border,
            top: work.top - border,
            right: work.right + border,
            bottom: work.bottom + border,
        };
        Some((whole, work))
    }
}

#[must_use]
pub fn foreground_window() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// Returns `true` if the foreground window covers the entire `screen`
/// (for example, video, presentation, or game). The desktop does not count,
/// and neither does a maximized window: when nothing reserves screen space
/// (auto-hide), it covers the screen too, but it still has its sizing frame,
/// which apps drop when they go full screen.
#[must_use]
pub fn foreground_is_fullscreen(screen: RECT) -> bool {
    // SAFETY: Read-only queries on the foreground window.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() || is_desktop(hwnd) || is_task_switcher(hwnd) {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return false;
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let maximized = WS_MAXIMIZE.0 | WS_THICKFRAME.0;
        if style & maximized == maximized {
            return false;
        }
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).is_ok() && covers(rect, screen)
    }
}

/// `true` if `point` is on the minimize, maximize or close button of the
/// window under it. `screen` is the monitor the point is on.
pub fn caption_button_at(point: POINT, screen: RECT) -> bool {
    use windows::Win32::{
        Foundation::WPARAM,
        Graphics::Dwm::{DWMWA_CAPTION_BUTTON_BOUNDS, DwmGetWindowAttribute},
        UI::{
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::{
                GA_ROOT, GetAncestor, GetWindowRect, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
                WM_NCHITTEST, WindowFromPoint,
            },
        },
    };
    const HTMINBUTTON: usize = 8;
    const HTMAXBUTTON: usize = 9;
    const HTCLOSE: usize = 20;
    const HTHELP: usize = 21;
    /// Windows' caption buttons are 46 logical pixels wide, three of them.
    const BUTTONS_WIDTH: i32 = 3 * 46;
    // SAFETY: Read-only queries; the message has a short timeout and is
    // abandoned if the window does not answer.
    unsafe {
        let window = GetAncestor(WindowFromPoint(point), GA_ROOT);
        let mut frame = RECT::default();
        if window.is_invalid() || is_desktop(window) || GetWindowRect(window, &mut frame).is_err() {
            return false;
        }
        // A maximized window's frame overflows the screen by its border, so
        // its buttons reach the screen edge: the corner pixel is "close".
        let visible_right = frame.right.min(screen.right);
        let (x, y) = (point.x - frame.left, point.y - frame.top);

        // The buttons DWM draws for standard title bars.
        let mut buttons = RECT::default();
        if DwmGetWindowAttribute(
            window,
            DWMWA_CAPTION_BUTTON_BOUNDS,
            (&raw mut buttons).cast(),
            size_of::<RECT>() as u32,
        )
        .is_ok()
            && buttons.right > buttons.left
        {
            let right = buttons.right.min(visible_right - frame.left);
            if x >= buttons.left && x <= right && y < buttons.bottom.max(1) {
                return true;
            }
        }
        // Apps that draw their own title bar (browsers, Office) answer with
        // the same hit-test codes, which Windows needs for Snap layouts.
        let position = ((point.y as u32 & 0xFFFF) << 16 | (point.x as u32 & 0xFFFF)) as isize;
        let mut hit = 0usize;
        let answered = SendMessageTimeoutW(
            window,
            WM_NCHITTEST,
            WPARAM(0),
            LPARAM(position),
            SMTO_ABORTIFHUNG,
            50,
            Some(&mut hit),
        );
        if answered.0 != 0 && matches!(hit, HTMINBUTTON | HTMAXBUTTON | HTCLOSE | HTHELP) {
            return true;
        }
        // Some draw them in web content and answer "client area" (Electron
        // apps). Every Windows app keeps them in the top-right corner, so a
        // window touching the top edge keeps that corner free of the bar.
        let width = BUTTONS_WIDTH * GetDpiForWindow(window).max(96) as i32 / 96;
        frame.top >= screen.top - 16 && point.x >= visible_right - width
    }
}

/// The Alt+Tab switcher (and, on Windows 11, Task View and Snap Assist,
/// which share its window class). It covers the screen but is not a
/// full-screen app.
pub fn is_task_switcher(hwnd: HWND) -> bool {
    matches!(
        class_name(hwnd).as_str(),
        "XamlExplorerHostIslandWindow" | "MultitaskingViewFrame"
    )
}

/// A window that is briefly in front while Windows switches between two
/// others (Alt+Tab passes through it), never the user's choice.
pub fn is_switch_transient(hwnd: HWND) -> bool {
    is_task_switcher(hwnd) || class_name(hwnd) == "ForegroundStaging"
}

/// The task switcher in front: `Some(true)` once on screen, `Some(false)`
/// while it is still appearing (it comes to the front some 50 ms before it
/// shows) or after it closed, `None` when another window is in front.
pub fn task_switcher_state() -> Option<bool> {
    // SAFETY: Read-only queries on the foreground window.
    unsafe {
        let front = GetForegroundWindow();
        is_task_switcher(front).then(|| IsWindowVisible(front).as_bool())
    }
}

/// Hidden by the system although "visible": on another virtual desktop, or
/// a suspended Store app.
fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    // SAFETY: `cloaked` is a valid 4-byte output.
    unsafe {
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            size_of::<u32>() as u32,
        );
    }
    cloaked != 0
}

/// The desktop and the (hidden) taskbar: shell windows, not apps.
pub fn is_desktop(hwnd: HWND) -> bool {
    matches!(
        class_name(hwnd).as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd"
    )
}

fn covers(window: RECT, screen: RECT) -> bool {
    window.left <= screen.left
        && window.top <= screen.top
        && window.right >= screen.right
        && window.bottom >= screen.bottom
}

fn is_app_window(hwnd: HWND) -> bool {
    // SAFETY: Read-only queries; calls on a destroyed `hwnd` have no effect.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || GetWindowTextLengthW(hwnd) == 0 {
            return false;
        }
        let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let has =
            |flag: windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE| style & flag.0 != 0;
        if has(WS_EX_TOOLWINDOW) {
            return false;
        }
        let owned = GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid());
        if (owned || has(WS_EX_NOACTIVATE)) && !has(WS_EX_APPWINDOW) {
            return false;
        }
        // Windows hidden by the system, such as on other virtual desktops or suspended apps.
        if is_cloaked(hwnd) {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid != GetCurrentProcessId()
    }
}

fn app_id(hwnd: HWND) -> Option<String> {
    if class_name(hwnd) == FRAME_HOST_CLASS {
        return window_aumid(hwnd).map(|aumid| format!("{APPS_FOLDER}{aumid}"));
    }

    // SAFETY: The process handle is closed before leaving the block, and the
    // buffers have the declared lengths.
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let id = if GetApplicationUserModelId(process, &mut len, Some(PWSTR(buffer.as_mut_ptr())))
            .is_ok()
        {
            // `len` includes the null terminator.
            let aumid = String::from_utf16_lossy(&buffer[..len.saturating_sub(1) as usize]);
            Some(format!("{APPS_FOLDER}{aumid}"))
        } else {
            len = buffer.len() as u32;
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut len,
            )
            .ok()
            .map(|()| normalize_id(&String::from_utf16_lossy(&buffer[..len as usize])))
        };
        let _ = CloseHandle(process);
        id
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 128];
    // SAFETY: The buffer remains alive during the call, which returns the number of characters written.
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

fn window_aumid(hwnd: HWND) -> Option<String> {
    // SAFETY: COM calls use pointers that remain valid for the duration of each call.
    let value = unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).ok()?;
        store.GetValue(&PKEY_AppUserModel_ID).ok()?
    };
    let aumid = value.to_string();
    (!aumid.is_empty()).then_some(aumid)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: RECT = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    #[test]
    fn covers_should_accept_window_exactly_the_size_of_the_screen() {
        assert!(covers(SCREEN, SCREEN));
    }

    #[test]
    fn covers_should_reject_maximized_window_above_the_dock() {
        // A maximized window extends a few pixels beyond the sides and top, but
        // ends above the strip reserved by the dock.
        let maximized = RECT {
            left: -8,
            top: -8,
            right: 1928,
            bottom: 985,
        };
        assert!(!covers(maximized, SCREEN));
    }

    #[test]
    fn normalize_id_should_lowercase_executable_paths() {
        assert_eq!(
            normalize_id("C:\\Windows\\Explorer.EXE"),
            "c:\\windows\\explorer.exe"
        );
    }

    #[test]
    fn normalize_id_should_keep_packaged_app_ids_untouched() {
        let id = "shell:AppsFolder\\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App";
        assert_eq!(normalize_id(id), id);
    }
}
