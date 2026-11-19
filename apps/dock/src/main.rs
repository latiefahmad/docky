//! Docky: a macOS-style dock for Windows 11.
//!
//! Usage:
//! - `docky`                     starts the dock (does nothing if already running)
//! - `docky --quit`              closes the running dock
//! - `docky --restore`           restores the Windows taskbar
//! - `docky --autostart on|off`  enables or disables startup with Windows
//! - `docky --guard <pid>`       internal watchdog process
//! - `docky --usage-worker`      internal one-shot quota reader
//! - `docky --shell-worker`      internal one-shot shell action

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod assets;
mod drop;
mod flight;
mod gfx;
mod guard;
mod menu;
mod menubar;
mod scene;
mod shell_work;
mod strings;
mod thumbnails;
mod usage;

use std::env;

use anyhow::{Context, Result, bail};
use docky_shell::{autostart, taskbar};
use windows::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
        System::{
            Threading::CreateMutexW,
            WinRT::{
                CreateDispatcherQueueController, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT,
                DispatcherQueueOptions,
            },
        },
        UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
    },
    core::w,
};

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => run_dock(),
        ["--quit"] => {
            app::request_quit();
            Ok(())
        }
        ["--restore"] => {
            app::request_quit();
            taskbar::give_back();
            Ok(())
        }
        ["--autostart", "on"] => {
            let exe = env::current_exe().context("could not locate docky.exe")?;
            Ok(autostart::set(Some(&exe.to_string_lossy()))?)
        }
        ["--autostart", "off"] => Ok(autostart::set(None)?),
        [guard::FLAG, pid] => guard::run(pid.parse().context("invalid pid")?),
        ["--usage-worker"] => usage::worker_main(),
        [shell_work::FLAG] => shell_work::worker_main(),
        _ => bail!("usage: docky [--quit | --restore | --autostart on|off]"),
    }
}

fn run_dock() -> Result<()> {
    // SAFETY: The static name is valid; the handle is released when the process exits.
    let _single = unsafe { CreateMutexW(None, false, w!("Local\\Docky.Dock"))? };
    // SAFETY: Reads the thread's last error immediately after the call.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        eprintln!("docky: a dock is already running");
        return Ok(());
    }

    // SAFETY: Sets DPI awareness and initializes the dispatcher before creating windows.
    let _queue = unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        // The compositor requires a dispatcher queue on its thread; this also
        // initializes COM, which icon extraction requires.
        CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_STA,
        })?
    };

    // Start the watchdog first so it can restore the taskbar if anything fails later.
    guard::spawn()?;
    // Moves an older `Run` autostart to the faster logon task, off the UI thread.
    std::thread::spawn(autostart::migrate_legacy);
    let result = app::run();
    taskbar::give_back_on_exit();
    result
}
