//! Power and memory usage (plan section 8).
//!
//! - **EcoQoS** (Windows 11 efficiency mode): the scheduler runs the thread on
//!   efficiency cores at a lower frequency. This suits background dock work
//!   (window tracking and recomposition), but not animations, so it is toggled
//!   as needed.
//! - **Memory trimming**: an idle dock rarely touches most of its pages, so
//!   returning them to the system frees RAM for other apps. If needed again,
//!   they can be restored from the standby list in microseconds.

use std::{ffi::c_void, mem::size_of};

use windows::Win32::System::{
    ProcessStatus::EmptyWorkingSet,
    Threading::{
        GetCurrentProcess, GetCurrentThread, IDLE_PRIORITY_CLASS, MEMORY_PRIORITY_INFORMATION,
        MEMORY_PRIORITY_LOW, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
        ProcessMemoryPriority, ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
        SetThreadInformation, THREAD_POWER_THROTTLING_CURRENT_VERSION,
        THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
        ThreadPowerThrottling,
    },
};

pub fn set_thread_efficiency(enabled: bool) {
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: if enabled {
            THREAD_POWER_THROTTLING_EXECUTION_SPEED
        } else {
            0
        },
    };
    // SAFETY: The structure has its size initialized; if unsupported, the call
    // fails without effect.
    unsafe {
        let _ = SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            (&raw const state).cast::<c_void>(),
            size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

pub fn trim_memory() {
    // SAFETY: Trims this process's own working set.
    unsafe {
        let _ = EmptyWorkingSet(GetCurrentProcess());
    }
}

/// Configures a process that only waits (the watchdog) with EcoQoS, minimum
/// CPU priority, and low-priority memory that the system can reclaim first.
pub fn run_as_background_process() {
    let throttling = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    };
    let memory = MEMORY_PRIORITY_INFORMATION {
        MemoryPriority: MEMORY_PRIORITY_LOW,
    };
    // SAFETY: The structures have their sizes initialized and target this
    // process; a failure leaves the default settings unchanged.
    unsafe {
        let process = GetCurrentProcess();
        let _ = SetProcessInformation(
            process,
            ProcessPowerThrottling,
            (&raw const throttling).cast::<c_void>(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
        let _ = SetProcessInformation(
            process,
            ProcessMemoryPriority,
            (&raw const memory).cast::<c_void>(),
            size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
        );
        let _ = SetPriorityClass(process, IDLE_PRIORITY_CLASS);
    }
    trim_memory();
}

/// Tells a window, with `WM_POWERBROADCAST` / `PBT_POWERSETTINGCHANGE`, when
/// energy saver (battery saver on Windows 10) turns on or off: Windows then
/// stops or resumes see-through effects, and glass must follow. Plugging the
/// charger in or out already arrives as `PBT_APMPOWERSTATUSCHANGE`, but
/// turning the saver on by hand does not always.
///
/// Event-driven: nothing runs until Windows sends the change. The
/// subscription ends when this is dropped.
pub struct SaverWatch(Vec<windows::Win32::System::Power::HPOWERNOTIFY>);

impl SaverWatch {
    #[must_use]
    pub fn new(hwnd: isize) -> Self {
        use windows::{
            Win32::{
                Foundation::HANDLE,
                System::{
                    Power::RegisterPowerSettingNotification,
                    SystemServices::GUID_POWER_SAVING_STATUS,
                },
                UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE,
            },
            core::GUID,
        };
        /// Energy saver, Windows 11 22H2 and later.
        const GUID_ENERGY_SAVER_STATUS: GUID =
            GUID::from_u128(0x550e8400_e29b_41d4_a716_446655440000);
        let handles = [GUID_ENERGY_SAVER_STATUS, GUID_POWER_SAVING_STATUS]
            .iter()
            .filter_map(|setting| {
                // SAFETY: `hwnd` is the caller's live window; an unknown
                // setting on an older Windows just fails.
                unsafe {
                    RegisterPowerSettingNotification(
                        HANDLE(hwnd as *mut c_void),
                        setting,
                        DEVICE_NOTIFY_WINDOW_HANDLE,
                    )
                }
                .ok()
            })
            .collect();
        Self(handles)
    }
}

impl Drop for SaverWatch {
    fn drop(&mut self) {
        for handle in self.0.drain(..) {
            // SAFETY: Registered in `new` and released once.
            unsafe {
                let _ = windows::Win32::System::Power::UnregisterPowerSettingNotification(handle);
            }
        }
    }
}
