//! Watchdog process: a minimal second `docky.exe` that waits for the dock to
//! exit (normally, after a failure, or via End Task) and then restores the
//! Windows taskbar.

use std::{env, os::windows::process::CommandExt, process::Command};

use anyhow::{Context, Result};
use docky_shell::{
    dock_link::DOCK_WINDOW_CLASS, power, system::set_window_transitions, taskbar,
    tracker::WindowTracker,
};
use windows::Win32::{
    Foundation::{CloseHandle, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM},
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    UI::WindowsAndMessaging::{
        FindWindowW, GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT,
        SendMessageTimeoutW, WM_NULL,
    },
};

pub const FLAG: &str = "--guard";

/// Starts without a console in its own process group so Ctrl+C in the dock's
/// terminal does not also terminate the watchdog.
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const HEALTH_INTERVAL_MS: u32 = 5_000;
const PING_TIMEOUT_MS: u32 = 250;
const MISSES_BEFORE_RESTORE: u8 = 4;

#[derive(Default)]
struct Health {
    missed: u8,
    restored: bool,
}

impl Health {
    fn observe(&mut self, responsive: bool) -> bool {
        if responsive {
            self.missed = 0;
            self.restored = false;
            return false;
        }
        self.missed = self.missed.saturating_add(1);
        if self.missed >= MISSES_BEFORE_RESTORE && !self.restored {
            self.restored = true;
            return true;
        }
        false
    }
}

pub fn spawn() -> Result<()> {
    let exe = env::current_exe().context("no se pudo localizar docky.exe")?;
    Command::new(exe)
        .args([FLAG, &std::process::id().to_string()])
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .context("could not launch the guardian process")?;
    Ok(())
}

/// Watchdog body: restores the taskbar after exit or a sustained UI hang.
pub fn run(pid: u32) -> Result<()> {
    // The watchdog may wait for weeks, so it should not compete for CPU or memory.
    power::run_as_background_process();
    // A fast startup failure can end the dock before this child opens it.
    // The taskbar still needs to be restored in that case.
    let process = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(process) => process,
        Err(error) => {
            eprintln!("docky: the guardian could not open the dock: {error}");
            restore_desktop();
            return Ok(());
        }
    };
    let mut health = Health::default();
    loop {
        // SAFETY: `process` is a live synchronization handle owned by this guard.
        let wait = unsafe { WaitForSingleObject(process, HEALTH_INTERVAL_MS) };
        if wait == WAIT_OBJECT_0 {
            break;
        }
        if wait != WAIT_TIMEOUT {
            eprintln!("docky: the guardian could not wait for the dock: {wait:?}");
            break;
        }
        if health.observe(ui_responds(pid)) {
            eprintln!("docky: the interface stopped responding; restoring the Windows taskbar");
            // Keep monitoring this process. An unrelated hung app must not
            // hold the guard in window enumeration while it repairs the shell.
            taskbar::give_back_on_exit();
        }
    }
    // SAFETY: The process handle is no longer used after this point.
    unsafe {
        let _ = CloseHandle(process);
    }
    restore_desktop();
    Ok(())
}

fn ui_responds(pid: u32) -> bool {
    // SAFETY: FindWindow returns an HWND owned by the dock's UI thread. WM_NULL
    // requires no marshalling and the timeout bounds an unresponsive peer.
    unsafe {
        let Ok(window) = FindWindowW(DOCK_WINDOW_CLASS, None) else {
            return false;
        };
        let mut owner = 0u32;
        GetWindowThreadProcessId(window, Some(&mut owner));
        if owner != pid {
            return false;
        }
        SendMessageTimeoutW(
            window,
            WM_NULL,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
            PING_TIMEOUT_MS,
            None,
        )
        .0 != 0
    }
}

fn restore_desktop() {
    // If the dock exited cleanly, repeating its restoration is harmless.
    taskbar::give_back_on_exit();
    // The dock turns off window transitions while it runs (genie effect); if
    // it died without restoring them, do it here.
    for window in WindowTracker::default().list() {
        set_window_transitions(window.hwnd, true);
    }
}

#[cfg(test)]
mod tests {
    use super::Health;

    #[test]
    fn intermittent_delay_does_not_restore_the_taskbar() {
        let mut health = Health::default();
        for _ in 0..3 {
            assert!(!health.observe(false));
        }
        assert!(!health.observe(true));
        for _ in 0..3 {
            assert!(!health.observe(false));
        }
    }

    #[test]
    fn sustained_hang_restores_once_until_recovery() {
        let mut health = Health::default();
        for _ in 0..3 {
            assert!(!health.observe(false));
        }
        assert!(health.observe(false));
        assert!(!health.observe(false));
        assert!(!health.observe(true));
        for _ in 0..3 {
            assert!(!health.observe(false));
        }
        assert!(health.observe(false));
    }
}
