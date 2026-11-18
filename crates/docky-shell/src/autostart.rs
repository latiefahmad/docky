//! Starting Docky when the user signs in, with the `Run` registry key: what
//! most desktop apps use, and what security software expects from them.
//!
//! Versions under the old name (Muelle) 0.1.7 and 0.1.8 used a logon
//! scheduled task instead, created by running `schtasks.exe` hidden. That
//! looks like a trojan setting itself up to persist, and some antivirus
//! programs deleted the dock for it. Those tasks are moved back to `Run` once
//! (`migrate_legacy`), deleting the task through the Task Scheduler API
//! rather than another hidden process. The same migration picks up users of
//! the old name: their marker moves to `Software\Docky` and their `Run` value
//! now points at the renamed program.
//!
//! Whether autostart is on is also written to `HKCU\Software\Docky`, so the
//! watchdog knows it instantly while Windows shuts down.

use std::env;

use windows::{
    Win32::{
        Foundation::WIN32_ERROR,
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
            RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteValueW,
            RegGetValueW, RegOpenKeyExW, RegSetValueExW,
        },
    },
    core::{PCWSTR, w},
};

const RUN_VALUE: PCWSTR = w!("Docky");
/// The `Run` value written by versions under the old name (Muelle).
const LEGACY_RUN_VALUE: PCWSTR = w!("Muelle");
const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const DOCKY_KEY: PCWSTR = w!(r"Software\Docky");
/// Where the old name (Muelle) kept its autostart marker.
const LEGACY_KEY: PCWSTR = w!(r"Software\Muelle");
const MARKER_VALUE: PCWSTR = w!("Autostart");

/// Starts `exe` at every sign-in, or stops doing so (`None`).
pub fn set(exe: Option<&str>) -> windows::core::Result<()> {
    match exe {
        Some(exe) => set_run_value(exe)?,
        None => {
            delete_run_value()?;
            // A task left by old versions must not keep starting the dock.
            clear_legacy();
        }
    }
    set_marker(exe.is_some())
}

/// `true` if Docky is set to start when the user signs in. Reads only the
/// registry, so it is instant and works while Windows shuts down.
#[must_use]
pub fn is_enabled() -> bool {
    marker() || run_value_exists()
}

/// Picks up where previous versions left off: autostart was on (the marker,
/// under either name) but there is no `Docky` `Run` value. Updates keep the
/// user's choice and never re-register it, so the dock does it once.
pub fn migrate_legacy() {
    if run_value_exists() {
        // Already configured: only remove what the old name left behind.
        clear_legacy();
        return;
    }
    if !(marker() || legacy_marker()) {
        return;
    }
    let Ok(exe) = env::current_exe() else {
        return;
    };
    if set_run_value(&exe.to_string_lossy()).is_ok() {
        clear_legacy();
    }
}

/// Removes what versions under the old name (Muelle) left behind: their `Run`
/// value, their autostart marker and their logon task. Every part fails
/// harmlessly when there is nothing to remove.
fn clear_legacy() {
    delete_legacy_task();
    let _ = delete_run_value_named(LEGACY_RUN_VALUE);
    clear_legacy_marker();
}

/// Removes the logon task old versions created, through the Task Scheduler's
/// own API. Fails harmlessly if there is none.
fn delete_legacy_task() {
    use windows::{
        Win32::System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            TaskScheduler::{ITaskService, TaskScheduler},
            Variant::VARIANT,
        },
        core::BSTR,
    };
    // One task per user, since task names are shared by the whole PC.
    let name = format!("Muelle ({})", env::var("USERNAME").unwrap_or_default());
    // SAFETY: COM is initialized for this call and released after; the
    // service and folder are released before.
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let deleted = (|| -> windows::core::Result<()> {
            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
            let none = VARIANT::default();
            service.Connect(&none, &none, &none, &none)?;
            service
                .GetFolder(&BSTR::from("\\"))?
                .DeleteTask(&BSTR::from(name), 0)
        })();
        let _ = deleted;
        if initialized {
            CoUninitialize();
        }
    }
}

fn marker() -> bool {
    read_marker(DOCKY_KEY)
}

/// The autostart marker written by versions under the old name (Muelle).
fn legacy_marker() -> bool {
    read_marker(LEGACY_KEY)
}

fn read_marker(key_name: PCWSTR) -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `value` and `size` describe a valid 4-byte buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key_name,
            MARKER_VALUE,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    status.is_ok() && value != 0
}

fn set_marker(on: bool) -> windows::core::Result<()> {
    let mut key = HKEY::default();
    let value = u32::from(on).to_le_bytes();
    // SAFETY: The key is closed before returning; `value` lives through the call.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            DOCKY_KEY,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()?;
        let result = RegSetValueExW(key, MARKER_VALUE, None, REG_DWORD, Some(&value));
        let _ = RegCloseKey(key);
        result.ok()
    }
}

/// Deletes the old name's autostart marker, leaving the rest of its key
/// (a last-update timestamp) alone. Fails harmlessly when it is absent.
fn clear_legacy_marker() {
    delete_value(LEGACY_KEY, MARKER_VALUE);
}

fn run_value_exists() -> bool {
    // SAFETY: Only asks whether the value exists; no buffers are passed.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
        .is_ok()
    }
}

fn set_run_value(exe: &str) -> windows::core::Result<()> {
    with_run_key(|key| {
        let command: Vec<u16> = format!("\"{exe}\"").encode_utf16().chain(Some(0)).collect();
        // SAFETY: `command` lives through the call.
        unsafe {
            let bytes = std::slice::from_raw_parts(command.as_ptr().cast(), command.len() * 2);
            RegSetValueExW(key, RUN_VALUE, None, REG_SZ, Some(bytes))
        }
    })
}

fn delete_run_value() -> windows::core::Result<()> {
    with_run_key(|key| delete_value_result(key, RUN_VALUE))
}

fn delete_run_value_named(value: PCWSTR) -> windows::core::Result<()> {
    with_run_key(|key| delete_value_result(key, value))
}

/// Removes a value of the open key. Removing a value that does not exist is
/// not an error.
fn delete_value_result(key: HKEY, value: PCWSTR) -> WIN32_ERROR {
    // SAFETY: `key` is open and `value` is a null-terminated constant.
    match unsafe { RegDeleteValueW(key, value) } {
        WIN32_ERROR(2) => WIN32_ERROR(0),
        other => other,
    }
}

/// Deletes a value under `HKCU\key_name`, failing harmlessly when either the
/// key or the value is missing.
fn delete_value(key_name: PCWSTR, value: PCWSTR) {
    let mut key = HKEY::default();
    // SAFETY: The key is closed before returning.
    let opened = unsafe {
        RegOpenKeyExW(HKEY_CURRENT_USER, key_name, None, KEY_SET_VALUE, &mut key).is_ok()
    };
    if !opened {
        return;
    }
    // SAFETY: The key is open and `value` is a null-terminated constant.
    unsafe {
        let _ = RegDeleteValueW(key, value);
        let _ = RegCloseKey(key);
    }
}

fn with_run_key(action: impl FnOnce(HKEY) -> WIN32_ERROR) -> windows::core::Result<()> {
    let mut key = HKEY::default();
    // SAFETY: The key is closed before returning.
    unsafe {
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_SET_VALUE, &mut key).ok()?;
        let result = action(key);
        let _ = RegCloseKey(key);
        result.ok()
    }
}
