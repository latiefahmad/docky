use std::mem::size_of;

use windows::Win32::{
    Foundation::HWND,
    UI::Shell::{
        Common::ITEMIDLIST, FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, SHCNE_CREATE, SHCNE_DELETE,
        SHCNE_RENAMEITEM, SHCNE_UPDATEDIR, SHCNE_UPDATEIMAGE, SHCNE_UPDATEITEM,
        SHCNRF_InterruptLevel, SHCNRF_ShellLevel, SHChangeNotifyDeregister, SHChangeNotifyEntry,
        SHChangeNotifyRegister, SHEmptyRecycleBinW, SHGetKnownFolderIDList, SHQUERYRBINFO,
        SHQueryRecycleBinW,
    },
};

pub const PARSING_NAME: &str = "::{645FF040-5081-101B-9F08-00AA002F954E}";

#[must_use]
pub fn has_items() -> bool {
    let mut info = SHQUERYRBINFO {
        cbSize: size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: The structure has its size initialized; `None` means all drives.
    unsafe { SHQueryRecycleBinW(None, &mut info) }.is_ok() && info.i64NumItems > 0
}

/// Empties the recycle bin using the native confirmation and progress UI.
pub fn empty(owner: HWND) {
    // SAFETY: `owner` is an optional owner window; cancellation is handled by the shell.
    unsafe {
        let _ = SHEmptyRecycleBinW(Some(owner), None, 0);
    }
}

pub struct BinWatch(u32);

impl BinWatch {
    /// Sends `message` to `hwnd` whenever the recycle bin contents change.
    /// Notifications arrive in bursts, so receivers should coalesce them.
    #[must_use]
    pub fn register(hwnd: HWND, message: u32) -> Option<Self> {
        // SAFETY: The identifier list is valid for this registration call, which copies it.
        unsafe {
            let pidl: *mut ITEMIDLIST =
                SHGetKnownFolderIDList(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT.0 as u32, None)
                    .ok()?;
            let entry = SHChangeNotifyEntry {
                pidl,
                fRecursive: true.into(),
            };
            let events = SHCNE_CREATE
                | SHCNE_DELETE
                | SHCNE_RENAMEITEM
                | SHCNE_UPDATEDIR
                | SHCNE_UPDATEITEM
                | SHCNE_UPDATEIMAGE;
            let id = SHChangeNotifyRegister(
                hwnd,
                SHCNRF_ShellLevel | SHCNRF_InterruptLevel,
                events.0 as i32,
                message,
                1,
                &entry,
            );
            windows::Win32::System::Com::CoTaskMemFree(Some(pidl.cast()));
            (id != 0).then_some(Self(id))
        }
    }
}

impl Drop for BinWatch {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid registration returned by SHChangeNotifyRegister.
        unsafe {
            let _ = SHChangeNotifyDeregister(self.0);
        }
    }
}
