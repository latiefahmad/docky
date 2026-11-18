//! System status for the menu bar: the network, the output volume, and the
//! buttons the user has turned on for the Windows taskbar.
//!
//! Nothing here polls. Windows reports network and volume changes through
//! callbacks on its own threads; they only post `message` to the bar's window,
//! which then reads the new state on its UI thread.

use windows::{
    Networking::Connectivity::{
        NetworkConnectivityLevel, NetworkInformation, NetworkStatusChangedEventHandler,
    },
    Win32::{
        Foundation::{HWND, LPARAM, PROPERTYKEY, WPARAM},
        Media::Audio::{
            AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole,
            Endpoints::{
                IAudioEndpointVolume, IAudioEndpointVolumeCallback,
                IAudioEndpointVolumeCallback_Impl,
            },
            IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
            MMDeviceEnumerator, eConsole, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree},
            Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        },
        UI::WindowsAndMessaging::PostMessageW,
    },
    core::{HSTRING, PCWSTR, Result, implement, w},
};

fn notify(window: isize, message: u32) {
    // SAFETY: Only posts a message; a window that no longer exists ignores it.
    unsafe {
        let _ = PostMessageW(Some(HWND(window as *mut _)), message, WPARAM(0), LPARAM(0));
    }
}

/* ---------------------------------------------------------------- Network */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Offline,
    Ethernet,
    /// Signal strength from 1 to 4 bars.
    Wifi {
        bars: u8,
    },
}

/// The connection Windows uses to reach the internet.
#[must_use]
pub fn network() -> Network {
    let Ok(profile) = NetworkInformation::GetInternetConnectionProfile() else {
        return Network::Offline;
    };
    let connected = profile
        .GetNetworkConnectivityLevel()
        .is_ok_and(|level| level != NetworkConnectivityLevel::None);
    if !connected {
        return Network::Offline;
    }
    if profile.IsWlanConnectionProfile().unwrap_or(false) {
        // Windows reports 0 to 5 bars; the glyphs have 4 steps.
        let reported = profile
            .GetSignalBars()
            .and_then(|bars| bars.Value())
            .unwrap_or(5);
        let bars = (u32::from(reported.min(5)) * 4).div_ceil(5).max(1) as u8;
        Network::Wifi { bars }
    } else {
        Network::Ethernet
    }
}

/// Posts `message` to `window` whenever the network changes.
pub struct NetworkWatch {
    token: i64,
}

impl NetworkWatch {
    #[must_use]
    pub fn new(window: isize, message: u32) -> Option<Self> {
        let handler = NetworkStatusChangedEventHandler::new(move |_| {
            notify(window, message);
            Ok(())
        });
        NetworkInformation::NetworkStatusChanged(&handler)
            .ok()
            .map(|token| Self { token })
    }
}

impl Drop for NetworkWatch {
    fn drop(&mut self) {
        let _ = NetworkInformation::RemoveNetworkStatusChanged(self.token);
    }
}

/* ----------------------------------------------------------------- Volume */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume {
    pub muted: bool,
    /// 0 (silent) to 3 (loud): the steps the speaker glyph has.
    pub level: u8,
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeChanged {
    window: isize,
    message: u32,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeChanged_Impl {
    fn OnNotify(&self, _data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        notify(self.window, self.message);
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceChanged {
    window: isize,
    message: u32,
}

impl IMMNotificationClient_Impl for DeviceChanged_Impl {
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _id: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            notify(self.window, self.message);
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// The default speakers' volume, followed across device changes.
pub struct AudioWatch {
    enumerator: IMMDeviceEnumerator,
    devices: IMMNotificationClient,
    /// Current default device, its volume control and our callback on it.
    endpoint: Option<(String, IAudioEndpointVolume, IAudioEndpointVolumeCallback)>,
    window: isize,
    message: u32,
}

impl AudioWatch {
    /// Must be created on a thread with COM initialized (the UI thread).
    #[must_use]
    pub fn new(window: isize, message: u32) -> Option<Self> {
        // SAFETY: COM is initialized on this thread; the enumerator and the
        // registered client are released in `Drop`.
        let (enumerator, devices) = unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let devices: IMMNotificationClient = DeviceChanged { window, message }.into();
            enumerator
                .RegisterEndpointNotificationCallback(&devices)
                .ok()?;
            (enumerator, devices)
        };
        Some(Self {
            enumerator,
            devices,
            endpoint: None,
            window,
            message,
        })
    }

    /// Current volume of the default speakers, or `None` without any.
    pub fn volume(&mut self) -> Option<Volume> {
        self.follow_default();
        let (_, endpoint, _) = self.endpoint.as_ref()?;
        // SAFETY: Plain queries on a live COM object.
        let (level, muted) = unsafe {
            (
                endpoint.GetMasterVolumeLevelScalar().ok()?,
                endpoint.GetMute().ok()?.as_bool(),
            )
        };
        let level = match level {
            l if l < 0.01 => 0,
            l if l < 0.34 => 1,
            l if l < 0.67 => 2,
            _ => 3,
        };
        Some(Volume { muted, level })
    }

    /// Moves the volume callback to the default device if it changed.
    fn follow_default(&mut self) {
        // SAFETY: COM calls on live objects; the id string is freed with
        // CoTaskMemFree as the API requires.
        unsafe {
            let Ok(device) = self.enumerator.GetDefaultAudioEndpoint(eRender, eConsole) else {
                self.unbind();
                return;
            };
            let Ok(raw) = device.GetId() else {
                return;
            };
            let id = raw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(raw.0.cast()));
            if self
                .endpoint
                .as_ref()
                .is_some_and(|(current, ..)| *current == id)
            {
                return;
            }
            self.unbind();
            let Ok(endpoint) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) else {
                return;
            };
            let callback: IAudioEndpointVolumeCallback = VolumeChanged {
                window: self.window,
                message: self.message,
            }
            .into();
            if endpoint.RegisterControlChangeNotify(&callback).is_ok() {
                self.endpoint = Some((id, endpoint, callback));
            }
        }
    }

    fn unbind(&mut self) {
        if let Some((_, endpoint, callback)) = self.endpoint.take() {
            // SAFETY: Unregisters the callback registered on this endpoint.
            unsafe {
                let _ = endpoint.UnregisterControlChangeNotify(&callback);
            }
        }
    }
}

impl Drop for AudioWatch {
    fn drop(&mut self) {
        self.unbind();
        // SAFETY: Unregisters the client registered in `new`.
        unsafe {
            let _ = self
                .enumerator
                .UnregisterEndpointNotificationCallback(&self.devices);
        }
    }
}

/* ------------------------------------------------------ Taskbar buttons */

/// The optional buttons the user has turned on in Windows Settings →
/// Personalization → Taskbar, so the menu bar offers the same ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskbarButtons {
    pub search: bool,
    pub task_view: bool,
    pub widgets: bool,
}

const ADVANCED: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced");
const SEARCH: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Search");

fn dword(key: PCWSTR, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `data` and `size` live for the call and match the requested type.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    Some(data)
}

/// Alt+Tab also lists Microsoft Edge tabs ("Show tabs from apps"), or the
/// windows of every virtual desktop: then its list is not the open windows
/// of this desktop in order.
#[must_use]
pub fn alt_tab_lists_more_than_windows(edge_open: bool) -> bool {
    // Absent means the default: three Edge tabs, this desktop only.
    let tabs = dword(ADVANCED, "MultiTaskingAltTabFilter").unwrap_or(2) != 3;
    let all_desktops = dword(ADVANCED, "VirtualDesktopAltTabFilter").unwrap_or(1) == 0;
    (tabs && edge_open) || all_desktops
}

/// Read when the bar redraws: three registry values, no watch needed. A value
/// Windows has never written means the button is at its default, shown.
#[must_use]
pub fn taskbar_buttons() -> TaskbarButtons {
    TaskbarButtons {
        search: dword(SEARCH, "SearchboxTaskbarMode").unwrap_or(1) != 0,
        task_view: dword(ADVANCED, "ShowTaskViewButton").unwrap_or(1) != 0,
        // Windows 10 has no Widgets (its value is never written there, so it
        // would read as shown), and Win+W opens Windows Ink instead.
        widgets: crate::system::is_windows_11() && dword(ADVANCED, "TaskbarDa").unwrap_or(1) != 0,
    }
}

/* ---------------------------------------------------------------- Privacy */

/// Where Windows records which apps use the microphone and camera, and
/// when: the source of its own privacy icons in the tray.
pub const CONSENT_STORE: windows::core::PCWSTR = windows::core::w!(
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore"
);

/// Which sensitive devices some app is using right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Privacy {
    pub microphone: bool,
    pub camera: bool,
}

// Location is left out on purpose: Widgets keeps it "in use" all the time
// for the weather, and Windows shows no icon for that.

/// Reads [`CONSENT_STORE`]. Call it when [`watch_privacy`] reports a change,
/// not on a timer.
#[must_use]
pub fn privacy() -> Privacy {
    Privacy {
        microphone: capability_in_use("microphone"),
        camera: capability_in_use("webcam"),
    }
}

/// Tells a window, by posting it `message`, when an app starts or stops
/// using the microphone or camera.
#[must_use]
pub fn watch_privacy(hwnd: isize, message: u32) -> Option<crate::watch::RegistryWatch> {
    crate::watch::RegistryWatch::new(CONSENT_STORE, true, hwnd, message)
}

/// An app is using `capability` while its key has a start time and no stop
/// time. Store apps have a key each; desktop programs sit under `NonPackaged`.
fn capability_in_use(capability: &str) -> bool {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_QWORD, RegCloseKey, RegEnumKeyExW,
        RegGetValueW, RegOpenKeyExW,
    };
    use windows::core::{HSTRING, PCWSTR, PWSTR, w};

    let in_use = |parent: HKEY, path: &str| -> bool {
        let mut key = HKEY::default();
        // SAFETY: Read-only registry walk; every opened key is closed.
        unsafe {
            if RegOpenKeyExW(parent, &HSTRING::from(path), None, KEY_READ, &mut key).is_err() {
                return false;
            }
            let mut found = false;
            let mut index = 0;
            loop {
                let mut name = [0u16; 512];
                let mut len = name.len() as u32;
                if RegEnumKeyExW(
                    key,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
                .is_err()
                {
                    break;
                }
                index += 1;
                let read = |value: PCWSTR| {
                    let mut time = 0u64;
                    let mut size = size_of::<u64>() as u32;
                    RegGetValueW(
                        key,
                        PCWSTR(name.as_ptr()),
                        value,
                        RRF_RT_REG_QWORD,
                        None,
                        Some((&raw mut time).cast()),
                        Some(&mut size),
                    )
                    .is_ok()
                    .then_some(time)
                };
                if read(w!("LastUsedTimeStart")).is_some_and(|start| start != 0)
                    && read(w!("LastUsedTimeStop")) == Some(0)
                {
                    found = true;
                    break;
                }
            }
            let _ = RegCloseKey(key);
            found
        }
    };
    let base = format!(
        r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{capability}"
    );
    in_use(HKEY_CURRENT_USER, &base) || in_use(HKEY_CURRENT_USER, &format!(r"{base}\NonPackaged"))
}
