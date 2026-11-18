//! Registering the dock as an *AppBar* reserves a bottom strip of the screen
//! so maximized windows end above the dock, and notifies the dock when an app
//! enters or exits full-screen mode.

use std::mem::size_of;

use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::Shell::{
        ABE_BOTTOM, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
        SHAppBarMessage,
    },
};

pub const ABN_FULLSCREENAPP: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
}

pub struct AppBar {
    hwnd: HWND,
    reserved: Option<RECT>,
}

impl AppBar {
    /// Registers `hwnd` as an AppBar. Notifications are sent to that window
    /// using `callback`, with the notification code in `wParam`.
    #[must_use]
    pub fn register(hwnd: HWND, callback: u32) -> Option<Self> {
        let mut data = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: hwnd,
            uCallbackMessage: callback,
            ..Default::default()
        };
        // SAFETY: `data` is a valid APPBARDATA with its size initialized.
        let registered = unsafe { SHAppBarMessage(ABM_NEW, &mut data) } != 0;
        registered.then_some(Self {
            hwnd,
            reserved: None,
        })
    }

    /// Reserves a strip `thickness` pixels deep along `edge` of monitor
    /// `screen`. Calls the system only when the strip changes because each
    /// reservation triggers a settings-change notification; repeating it would
    /// create a loop.
    pub fn reserve(&mut self, edge: Edge, screen: RECT, thickness: i32) {
        let wanted = match edge {
            Edge::Top => RECT {
                bottom: screen.top + thickness,
                ..screen
            },
            Edge::Bottom => RECT {
                top: screen.bottom - thickness,
                ..screen
            },
        };
        if self.reserved == Some(wanted) {
            return;
        }

        let mut data = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: self.hwnd,
            uEdge: match edge {
                Edge::Top => ABE_TOP,
                Edge::Bottom => ABE_BOTTOM,
            },
            rc: wanted,
            ..Default::default()
        };
        // SAFETY: `data` has its size initialized and remains valid for both synchronous calls.
        unsafe {
            SHAppBarMessage(ABM_QUERYPOS, &mut data);
            // Another bar on the same edge (the Windows taskbar, before it is
            // hidden) pushes the strip inward, so preserve its thickness.
            match edge {
                Edge::Top => data.rc.bottom = data.rc.top + thickness,
                Edge::Bottom => data.rc.top = data.rc.bottom - thickness,
            }
            SHAppBarMessage(ABM_SETPOS, &mut data);
        }
        // Treat the reservation as complete only if the system granted the
        // requested area; otherwise, retry on the next call.
        self.reserved = (data.rc == wanted).then_some(wanted);
    }
}

impl Drop for AppBar {
    fn drop(&mut self) {
        let mut data = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: self.hwnd,
            ..Default::default()
        };
        // SAFETY: `data` has its size initialized and remains valid for the synchronous call.
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut data) };
    }
}
