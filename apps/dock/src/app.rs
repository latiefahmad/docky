//! Dock windows and lifecycle.
//!
//! There are two windows:
//! - the **visual** window draws everything and passes through clicks; it is
//!   larger than the dock to fit the shadow, magnified icons, and labels;
//! - the invisible **input** window covers only the area where the dock receives
//!   mouse input. It expands while the cursor is over the dock (hysteresis).
//!
//! While the dock is running, the Windows taskbar is hidden and the dock is
//! registered as an AppBar, reserving a strip along the bottom of the screen.

use std::{
    cell::RefCell,
    collections::HashMap,
    env,
    sync::{
        OnceLock,
        atomic::{AtomicIsize, AtomicU8, Ordering},
    },
    thread,
    time::Instant,
};

use anyhow::{Context, Result};
use docky_config::Config;
use docky_core::{
    bounce,
    layout::{ItemKind, Layout, Metrics, layout},
    magnify::{Magnifier, magnify},
    roster::Roster,
    spring::Spring,
};
use docky_shell::{
    appbar::{ABN_FULLSCREENAPP, AppBar, Edge},
    files, launcher,
    pins::{app_exists, link_target, taskbar_pins, taskbar_pins_if_readable},
    power,
    recycle_bin::{self, BinWatch},
    status,
    system::{
        animations_enabled, downloads_folder, primary_monitor, set_window_transitions,
        transparency_enabled, uses_light_theme,
    },
    taskbar,
    tracker::{self, TrackedWindow, WindowTracker, normalize_id},
};
use windows::{
    UI::Composition::{CompositionSurfaceBrush, Desktop::DesktopWindowTarget},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmFlush, DwmSetWindowAttribute},
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            Console::{CTRL_C_EVENT, SetConsoleCtrlHandler},
            LibraryLoader::GetModuleHandleW,
            Ole::{
                DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
                IDropTarget, OleInitialize, OleUninitialize, RegisterDragDrop, RevokeDragDrop,
            },
            WinRT::Composition::ICompositorDesktopInterop,
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE,
                TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY, VK_CONTROL, VK_DOWN, VK_END,
                VK_HOME, VK_LEFT, VK_LWIN, VK_MENU, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_TAB, VK_UP,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, EVENT_OBJECT_CLOAKED,
                EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
                EVENT_SYSTEM_MINIMIZESTART, FindWindowW, GetCursorPos, GetMessageW,
                GetSystemMetrics, HWND_TOPMOST, IDC_ARROW, KillTimer, LoadCursorW, MSG,
                OBJID_WINDOW, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage,
                RegisterClassW, RegisterShellHookWindow, RegisterWindowMessageW, SM_CXDRAG,
                SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetTimer, SetWindowPos,
                WINDOW_EX_STYLE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP,
                WM_CAPTURECHANGED, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEMOVE, WM_QUIT, WM_RBUTTONUP,
                WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
                WS_POPUP,
            },
        },
    },
    core::{BOOL, Interface, PCWSTR, w},
};

use crate::{
    assets::{Visuals, fallback_name},
    drop::DropTarget,
    flight::{Flight, Snapshot},
    gfx::{Gfx, LabelBrush},
    menu::{self, MenuItem},
    menubar::MenuBar,
    scene::{Drag, Frame, Scene, SceneInput, SlotContent, Theme},
    shell_work::{self, Action as ShellAction},
    strings::text,
    thumbnails::Thumbnails,
};

/// These constants are not named in the `windows` crate.
const WM_MOUSELEAVE: u32 = 0x02A3;
const HSHELL_WINDOWACTIVATED: usize = 4;
const HSHELL_RUDEAPPACTIVATED: usize = 0x8004;
const HSHELL_FLASH: usize = 0x8006;
/// A window's title or icon changed.
const HSHELL_REDRAW: usize = 6;
/// The keyboard language changed.
const HSHELL_LANGUAGE: usize = 8;
/// `wParam` of WM_SETTINGCHANGE when the work area changes. The dock's own
/// reservation causes this, so handling it would create a loop.
const SPI_SETWORKAREA: usize = 0x002F;
const WM_POWERBROADCAST: u32 = 0x0218;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
/// Sent when the charger is plugged in or out, and when energy saver turns
/// on or off.
const PBT_APMPOWERSTATUSCHANGE: usize = 0x000A;
const PBT_POWERSETTINGCHANGE: usize = 0x8013;

const WINDOW_CLASS: PCWSTR = docky_shell::dock_link::DOCK_WINDOW_CLASS;
const APPBAR_CALLBACK: u32 = WM_APP + 1;
const TASKBAR_TAKEN: u32 = WM_APP + 2;
const BIN_CHANGED: u32 = WM_APP + 3;
/// Result of checking the recycle bin in the background (`wParam` = 1 if non-empty).
const BIN_STATE: u32 = WM_APP + 4;
/// Posted (not sent) when mouse capture is lost, so it is handled outside the
/// call that released it.
const DRAG_CANCELLED: u32 = WM_APP + 5;
/// Runs a file drop after OLE's `Drop` returns, so the drag source is not
/// blocked while files are copied or an app starts.
const DROP_RUN: u32 = WM_APP + 6;
/// Posted when apps are pinned to or unpinned from the Windows taskbar.
const PINS_CHANGED: u32 = WM_APP + 7;
const ASSETS_READY: u32 = WM_APP + 8;

/// Magnification range, in icons on either side of the cursor.
const RANGE_IN_ICONS: f32 = 3.0;
/// Speed of magnification entry and exit (about 200 ms).
const HOVER_OMEGA: f32 = 28.0;
/// Bounce height as a fraction of the icon size.
const BOUNCE_HEIGHT: f32 = 0.5;
const LAUNCH_BOUNCE_LIMIT: f32 = 10.0;
const ATTENTION_HOPS: f32 = 3.0;

// Dimensions are in logical pixels and are multiplied by the display scale.
/// Gap between the dock and the bottom screen edge, and between the dock and
/// maximized windows.
const FLOAT_MARGIN: f32 = 8.0;
const SHADOW_ROOM: f32 = 44.0;
const LABEL_ROOM: f32 = 60.0;
/// Hysteresis distances: the effect starts at the first distance and ends at the second.
const ENTER_HALO: f32 = 5.0;
const LEAVE_HALO: f32 = 15.0;

/// Windows sends settings changes in bursts; coalesce them into one recomposition.
const REFRESH_TIMER: usize = 1;
const REFRESH_DELAY_MS: u32 = 120;
/// Window notifications also arrive in bursts when an app opens.
const SYNC_TIMER: usize = 2;
const SYNC_DELAY_MS: u32 = 40;
/// Minimized windows whose snapshot is kept to fly them back out of the dock.
/// Each costs a quarter of the window's pixels; older ones restore without
/// the genie.
const MAX_SNAPSHOTS: usize = 4;
/// After Explorer restarts or the system resumes, the new taskbar needs time
/// to apply its initial state before it can be taken over again.
const TASKBAR_TIMER: usize = 3;
const TASKBAR_DELAY_MS: u32 = 600;
const TRIM_TIMER: usize = 4;
const TRIM_DELAY_MS: u32 = 3000;
const BIN_TIMER: usize = 5;
const BIN_DELAY_MS: u32 = 300;
const HIDE_TIMER: usize = 6;
const HIDE_DELAY_MS: u32 = 300;
/// Explorer writes a pin change in a few steps; read it once they settle.
const PINS_TIMER: usize = 7;
const PINS_DELAY_MS: u32 = 400;
/// Holding a drag over an open app brings its window to the front after this
/// long, so the drop can land in it: the Windows taskbar's drag-hover, and
/// macOS's spring-loaded Dock icons. Passing over icons does not.
const SPRING_TIMER: usize = 8;
const SPRING_DELAY_MS: u32 = 600;
/// While the Alt+Tab switcher is open, checks how long it stays: Windows
/// reports when it comes to the front, but not reliably when it closes.
/// A duration as timer milliseconds; Windows caps timers at about 24 days.
fn millis(duration: std::time::Duration) -> u32 {
    duration.as_millis().min(0x7FFF_FFFF) as u32
}

/// The background update runs once a day: a single timer, aimed at the
/// moment the next look is due (see `schedule_update`).
const UPDATE_TIMER: usize = 12;
const UPDATE_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// After the dock starts, the look waits this long, so it never slows signing in.
const UPDATE_AFTER_START: std::time::Duration = std::time::Duration::from_secs(2 * 60);
/// While a full-screen app (a game, a presentation) is in front, try later.
const UPDATE_RETRY: std::time::Duration = std::time::Duration::from_secs(60 * 60);
const SWITCHER_TIMER: usize = 9;
/// Short enough to see every Tab press (a key stays down 50-100 ms).
const SWITCHER_POLL_MS: u32 = 16;
/// How long the switcher may take to show after coming to the front.
const SWITCHER_APPEAR: std::time::Duration = std::time::Duration::from_millis(400);
/// How soon after closing the switcher's window coming back means nothing.
const SWITCHER_ECHO: std::time::Duration = std::time::Duration::from_millis(150);
/// Recheck full-screen state while hidden in case Windows drops the exit event.
const FULLSCREEN_TIMER: usize = 10;
const FULLSCREEN_RECHECK_MS: u32 = 3_000;
/// Shell results arriving together cause one scene rebuild.
const ASSETS_TIMER: usize = 11;
const ASSETS_DELAY_MS: u32 = 16;

/// Auto-hide slide speed (about 250 ms).
const SLIDE_OMEGA: f32 = 24.0;
/// Screen-edge strip that reveals a hidden dock, in physical pixels.
const REVEAL_STRIP: i32 = 2;
/// An icon released this far above the magnified dock, in icon heights, leaves it.
const REMOVE_DISTANCE: f32 = 0.8;

const START_ID: &str = "docky:start";
const WINDOW_ID_PREFIX: &str = "docky:window:";
const WINDOW_TITLE_CHARS: usize = 40;
/// Fraction of a thumbnail slot covered by the app badge (see `scene`).
const BADGE_CORNER: f32 = crate::scene::BADGE_SIZE * crate::scene::BADGE_INSIDE;
const CMD_CLOSE_WINDOW: u32 = 8;
const CMD_SETTINGS: u32 = 9;

/// Context menu commands. Window commands start at `CMD_WINDOW`.
const CMD_OPEN: u32 = 1;
const CMD_TOGGLE_PIN: u32 = 2;
const CMD_REVEAL: u32 = 3;
const CMD_QUIT_APP: u32 = 4;
const CMD_EMPTY_BIN: u32 = 5;
const CMD_QUIT_DOCK: u32 = 6;
const CMD_TOGGLE_HIDING: u32 = 7;
const CMD_IMPORT_PINS: u32 = 10;
const CMD_RUN_AS_ADMIN: u32 = 11;
const CMD_SHOW_DESKTOP: u32 = 12;
const CMD_WINDOW: u32 = 100;
const MENU_WINDOWS: usize = 12;
const MENU_TITLE_CHARS: usize = 60;

static SHELL_HOOK: OnceLock<u32> = OnceLock::new();
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
/// Broadcast by the settings app after it saves the configuration.
static CONFIG_CHANGED: OnceLock<u32> = OnceLock::new();
/// Dock visual window, used to close it from the Ctrl+C handler running on
/// another thread.
static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);
static DROP_WINDOW: AtomicIsize = AtomicIsize::new(0);
static MINIMIZE_HOOK: AtomicIsize = AtomicIsize::new(0);
static CLOAK_HOOK: AtomicIsize = AtomicIsize::new(0);
static FOREGROUND_HOOK: AtomicIsize = AtomicIsize::new(0);
/// The window in front before the Alt+Tab switcher opened.
static LAST_FRONT: AtomicIsize = AtomicIsize::new(0);

/// One drive scan at a time; changes during a scan request one more pass.
struct BinQueryGate(AtomicU8);

impl BinQueryGate {
    const RUNNING: u8 = 1;
    const DIRTY: u8 = 2;

    const fn new() -> Self {
        Self(AtomicU8::new(0))
    }

    fn request(&self) -> bool {
        if self.0.fetch_or(Self::DIRTY, Ordering::AcqRel) & Self::RUNNING != 0 {
            return false;
        }
        self.0
            .compare_exchange(
                Self::DIRTY,
                Self::RUNNING | Self::DIRTY,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn begin_pass(&self) {
        self.0.fetch_and(!Self::DIRTY, Ordering::AcqRel);
    }

    fn needs_another_pass(&self) -> bool {
        self.0
            .compare_exchange(Self::RUNNING, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    }
}

static BIN_QUERY: BinQueryGate = BinQueryGate::new();

#[derive(Clone, PartialEq)]
struct Entry {
    kind: ItemKind,
    id: Option<String>,
    role: Role,
    running: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    App,
    Start,
    /// A minimized window, shown as a live thumbnail.
    Window,
    Folder,
    Trash,
}

fn window_id(hwnd: isize) -> String {
    format!("{WINDOW_ID_PREFIX}{hwnd}")
}

fn window_of(id: &str) -> Option<isize> {
    id.strip_prefix(WINDOW_ID_PREFIX)?.parse().ok()
}

/// Work deferred until the app state is released. These calls may open a modal
/// loop (a menu, shell error dialog, or recycle bin confirmation) that dispatches
/// messages back to the dock.
enum Outside {
    Open(String),
    OpenStart,
    Activate(Vec<isize>),
    Minimize(Vec<isize>),
    Reveal(String),
    EmptyBin,
    OpenWith(String, Vec<String>),
    Recycle(Vec<String>),
    Transfer {
        folder: String,
        files: Vec<String>,
        copy: bool,
    },
    Menu(MenuRequest),
    QuitDock,
    OpenSettings,
    OpenAsAdmin(String),
    /// Unpin from the Windows taskbar too (see `pins::unpin_from_taskbar`).
    UnpinFromTaskbar(String),
    ShowDesktop,
}

struct MenuRequest {
    gfx: Gfx,
    target: MenuTarget,
    items: Vec<MenuItem>,
    anchor: POINT,
    dark: bool,
}

enum MenuTarget {
    Entry {
        id: String,
        role: Role,
        windows: Vec<isize>,
    },
    Dock,
}

struct Asset {
    icon: Option<CompositionSurfaceBrush>,
    label: Option<LabelBrush>,
}

struct Press {
    id: String,
    at: POINT,
}

struct Dragging {
    id: String,
    point: POINT,
    removing: bool,
}

/// What dropping the dragged files at a given place would do.
enum DropAction {
    Nothing,
    OpenWith(String),
    Recycle,
    Transfer { folder: String, copy: bool },
    Pin { id: String, before: Option<String> },
}

impl DropAction {
    fn effect(&self) -> DROPEFFECT {
        match self {
            Self::Nothing => DROPEFFECT_NONE,
            Self::OpenWith(_) => DROPEFFECT_COPY,
            Self::Recycle => DROPEFFECT_MOVE,
            Self::Transfer { copy, .. } => {
                if *copy {
                    DROPEFFECT_COPY
                } else {
                    DROPEFFECT_MOVE
                }
            }
            Self::Pin { .. } => DROPEFFECT_LINK,
        }
    }
}

struct Bounce {
    start: Instant,
    /// Seconds after `start` when bouncing ends. `None` means until the app opens a window.
    until: Option<f32>,
}

/// State that determines the scene; recomposition is unnecessary if it is unchanged.
#[derive(PartialEq)]
struct StageKey {
    screen: RECT,
    scale: f32,
    light: bool,
    glass: bool,
    bin_full: bool,
    auto_hide: bool,
    icon_size: u32,
    max_scale: f32,
    indicators: bool,
    entries: Vec<Entry>,
}

/// Where the switcher's selection lands once the windows marked `false` in
/// `alive` close: it stays at the same place in the list (the next window
/// takes the closed one's place), or on the new last one. `None` if none is
/// left.
fn selection_after_closing(index: usize, alive: &[bool]) -> Option<usize> {
    let left = alive.iter().filter(|&&on| on).count();
    if left == 0 {
        return None;
    }
    let closed_before = alive[..index.min(alive.len())]
        .iter()
        .filter(|&&on| !on)
        .count();
    Some((index - closed_before).min(left - 1))
}

/// Where the Alt+Tab selection is believed to be.
struct SwitcherFollow {
    /// The windows the switcher lists, in its order.
    order: Vec<isize>,
    index: usize,
    tab_down: bool,
}

struct Stage {
    key: StageKey,
    window_zone: RECT,
    scene: Scene,
    layout: Layout,
    magnifier: Magnifier,
    icon: f32,
    gap: f32,
    scale: f32,
    dock_origin: POINT,
    /// Position of the resting dock inside the visual window.
    origin: (f32, f32),
    rest_zone: RECT,
    hover_zone: RECT,
    /// Thin strip at the screen edge that reveals the dock when auto-hide is on.
    edge_zone: RECT,
    /// How far the dock slides down to hide completely.
    slide_distance: f32,
}

struct App {
    visual: HWND,
    input: HWND,
    gfx: Gfx,
    target: DesktopWindowTarget,
    appbar: Option<AppBar>,
    fullscreen: bool,
    roster: Roster,
    tracker: WindowTracker,
    windows: Vec<TrackedWindow>,
    /// Where each window was last seen on screen while not maximized, for
    /// the genie of snapped windows (see `tracker::rect_before_minimize`).
    last_rects: HashMap<isize, RECT>,
    entries: Vec<Entry>,
    icons: Visuals,
    downloads: Option<String>,
    bin_full: bool,
    _bin_watch: Option<BinWatch>,
    /// Energy saver on or off: glass follows (see `system::transparency_enabled`).
    _saver_watch: power::SaverWatch,
    /// Hears pins and unpins on the Windows taskbar.
    pin_watch: Option<docky_shell::watch::RegistryWatch>,
    assets: HashMap<String, Asset>,
    /// Scale, theme and icon pixel size the cached assets were drawn for.
    assets_for: (f32, bool, u32),
    placeholder: Option<CompositionSurfaceBrush>,
    remove_label: Option<LabelBrush>,
    config: Config,
    /// 0 = shown, 1 = hidden (auto-hide).
    slide: Spring,
    press: Option<Press>,
    drag: Option<Dragging>,
    /// Files being dragged over the dock from another app.
    incoming: Vec<String>,
    /// Something (files, text, an image) is being dragged over the dock.
    drag_over: bool,
    /// The entry a drag is resting on, which springs open on `SPRING_TIMER`.
    spring: Option<String>,
    /// The Alt+Tab switcher is open; an auto-hidden dock shows meanwhile.
    switching: bool,
    switch_started: Instant,
    switch_ended: Option<Instant>,
    /// When the dock started (the update waits a little after it).
    launched: Instant,
    /// When this dock last started a background update.
    update_tried: Option<Instant>,
    /// The selection in the switcher, as far as it can be followed.
    follow: Option<SwitcherFollow>,
    /// The magnifier rests on the icon selected in Alt+Tab, not the pointer.
    pointed: bool,
    pending_drop: Option<Outside>,
    bounces: HashMap<String, Bounce>,
    /// Minimized windows, in the order they were minimized.
    minimized: Vec<isize>,
    thumbnails: Thumbnails,
    /// Genie animations in progress.
    flights: Vec<Flight>,
    /// What the windows minimized with the genie looked like, oldest first.
    snapshots: Vec<(isize, Snapshot)>,
    menubar: Option<MenuBar>,
    /// Display names already looked up, by app id.
    names: HashMap<String, String>,
    /// Windows whose system transitions are turned off (see `calm_windows`).
    calmed: Vec<isize>,
    /// Windows already minimized when the dock starts get no animation.
    started: bool,
    stage: Option<Stage>,
    hovering: bool,
    hovered: Option<usize>,
    /// Horizontal cursor position relative to the resting dock.
    cursor: f32,
    /// 0 = resting, 1 = full magnification.
    amount: Spring,
    last_frame: Instant,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn explorer_id() -> String {
    normalize_id(&format!(
        "{}\\explorer.exe",
        env::var("WINDIR").unwrap_or_default()
    ))
}

/// Saved settings or, on first launch, the defaults with the apps pinned to
/// the Windows taskbar.
fn load_config() -> Config {
    let path = docky_config::default_path();
    match docky_config::load(&path) {
        Ok(Some(config)) => config,
        Ok(None) => {
            let pins = taskbar_pins();
            let config = Config {
                pinned: pins.clone(),
                taskbar_pins: Some(pins),
                ..Config::default()
            };
            if let Err(error) = docky_config::save(&path, &config) {
                eprintln!("docky: {error}");
            }
            config
        }
        Err(error) => {
            // Do not overwrite it; the user may want to repair the file.
            eprintln!("docky: {error}; using the default values");
            Config::default()
        }
    }
}

/// The Explorer is always pinned, like Finder.
fn roster_from(config: &Config) -> Roster {
    let explorer = explorer_id();
    let mut pinned = config.pinned.clone();
    pinned.retain(|id| *id != explorer);
    pinned.insert(0, explorer);
    Roster::new(pinned, config.recent.clone())
}

fn save_settings(roster: &Roster, config: &Config) {
    let path = docky_config::default_path();
    if let Err(error) = docky_config::load(&path) {
        // Do not overwrite a corrupted file; report it and skip saving.
        eprintln!("docky: {error}; the dock changes are not saved");
        return;
    }
    let config = Config {
        pinned: roster.pinned().to_vec(),
        recent: roster.recent().to_vec(),
        ..config.clone()
    };
    if let Err(error) = docky_config::save(&path, &config) {
        eprintln!("docky: {error}");
    }
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let cut: String = text.chars().take(max - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

fn place(hwnd: HWND, zone: &RECT, visible: bool) -> Result<()> {
    let show = if visible {
        SWP_SHOWWINDOW
    } else {
        SWP_HIDEWINDOW
    };
    // SAFETY: `hwnd` is one of our live windows.
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            zone.left,
            zone.top,
            zone.right - zone.left,
            zone.bottom - zone.top,
            SWP_NOACTIVATE | show,
        )?;
    }
    Ok(())
}

pub fn request_quit() {
    // SAFETY: Searches for the window and posts a message with no associated data.
    unsafe {
        if let Ok(hwnd) = FindWindowW(WINDOW_CLASS, None) {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe extern "system" fn on_console_ctrl(event: u32) -> BOOL {
    let hwnd = MAIN_WINDOW.load(Ordering::Relaxed);
    if event == CTRL_C_EVENT && hwnd != 0 {
        // SAFETY: Only a message is sent; if the window no longer exists, the call has no effect.
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        return true.into();
    }
    false.into()
}

impl App {
    /// List of items: apps, recents, Downloads, and recycle bin, separated into
    /// sections as in macOS.
    fn current_entries(&self) -> Vec<Entry> {
        let separator = || Entry {
            kind: ItemKind::Separator,
            id: None,
            role: Role::Folder,
            running: false,
        };
        let icon = |id: &str, role: Role, running: bool| Entry {
            kind: ItemKind::Icon,
            id: Some(id.to_owned()),
            role,
            running,
        };

        let mut entries: Vec<Entry> = self
            .roster
            .apps()
            .iter()
            .map(|app| icon(&app.id, Role::App, app.running))
            .collect();
        entries.insert(0, icon(START_ID, Role::Start, false));
        if self.config.show_recents && !self.roster.recent().is_empty() {
            entries.push(separator());
            entries.extend(
                self.roster
                    .recent()
                    .iter()
                    .map(|id| icon(id, Role::App, false)),
            );
        }
        entries.push(separator());
        // Unless "show minimized windows" is on, minimized windows hide in
        // their app's icon instead of getting a thumbnail here.
        if self.config.show_minimized {
            entries.extend(
                self.minimized
                    .iter()
                    .map(|&hwnd| icon(&window_id(hwnd), Role::Window, false)),
            );
        }
        if let Some(downloads) = &self.downloads {
            entries.push(icon(downloads, Role::Folder, false));
        }
        entries.push(icon(recycle_bin::PARSING_NAME, Role::Trash, false));
        entries
    }

    /// `wanted` is the pixel side icons are drawn at: their magnified size.
    fn prepare_assets(&mut self, wanted: u32, scale: f32, light: bool) {
        if self.assets_for != (scale, light, wanted) {
            self.assets.clear();
            self.remove_label = None;
            self.placeholder = None;
            self.assets_for = (scale, light, wanted);
        }
        if self.remove_label.is_none() {
            self.remove_label = self.gfx.label_brush(text().remove, scale, light).ok();
        }
        if self.placeholder.is_none() {
            self.placeholder = self.gfx.placeholder_brush(wanted, light).ok();
        }
        for id in self.entries.iter().filter_map(|entry| entry.id.as_ref()) {
            let variant = if id == recycle_bin::PARSING_NAME {
                if self.bin_full { "full" } else { "empty" }
            } else {
                ""
            };
            let visual = if id == START_ID || window_of(id).is_some() {
                None
            } else {
                self.icons.get(id, variant)
            };
            if self.assets.contains_key(id) {
                continue;
            }
            if let Some(hwnd) = window_of(id) {
                let title = shorten(&tracker::title(hwnd), WINDOW_TITLE_CHARS);
                let label = self.gfx.label_brush(&title, scale, light).ok();
                self.assets.insert(id.clone(), Asset { icon: None, label });
                continue;
            }
            if id == START_ID {
                let icon = self.gfx.start_brush(wanted).ok();
                let label = self.gfx.label_brush(text().start, scale, light).ok();
                self.assets.insert(id.clone(), Asset { icon, label });
                continue;
            }
            // A missing icon should not prevent the rest of the dock from rendering.
            let icon = visual
                .and_then(|visual| {
                    let side = wanted.min(visual.icon.width);
                    self.gfx
                        .icon_brush(&visual.icon, side)
                        .inspect_err(|error| eprintln!("docky: icon of `{id}`: {error:#}"))
                        .ok()
                })
                .or_else(|| self.placeholder.clone());
            let name = visual.map_or_else(|| fallback_name(id), |visual| visual.name.clone());
            let label = self
                .gfx
                .label_brush(&name, scale, light)
                .inspect_err(|error| eprintln!("docky: label of `{id}`: {error:#}"))
                .ok();
            self.assets.insert(id.clone(), Asset { icon, label });
        }
        let entries = &self.entries;
        self.assets
            .retain(|id, _| entries.iter().any(|entry| entry.id.as_ref() == Some(id)));
        let windows = &self.windows;
        let explorer = explorer_id();
        self.icons.retain(|id| {
            id == explorer
                || entries.iter().any(|entry| entry.id.as_deref() == Some(id))
                || windows.iter().any(|window| window.app_id == id)
        });
    }

    fn assets_ready(&mut self) -> Result<()> {
        let loaded = self.icons.collect();
        if loaded.is_empty() {
            return Ok(());
        }
        for id in &loaded {
            self.assets.remove(id);
            self.names.remove(id);
        }
        self.stage = None;
        self.refresh()?;
        self.update_menu_bar_app()
    }

    /// Recalculates sizes, positions, and scene for the current state. Does
    /// nothing if the scene inputs are unchanged, since many Windows notifications
    /// do not affect the dock.
    fn refresh(&mut self) -> Result<()> {
        if let Some(bar) = &mut self.menubar {
            bar.refresh(&self.gfx)?;
        }
        let monitor = primary_monitor().context("no se pudo consultar el monitor principal")?;
        let key = StageKey {
            screen: monitor.screen,
            scale: monitor.scale,
            light: uses_light_theme(),
            // A backdrop brush made while Windows had transparency off stays
            // black after it comes back, so this rebuilds the dock both ways.
            glass: transparency_enabled(),
            bin_full: self.bin_full,
            auto_hide: self.config.auto_hide,
            icon_size: self.config.icon_size,
            max_scale: self.config.max_scale(),
            indicators: self.config.show_indicators,
            entries: self.current_entries(),
        };
        if self.stage.as_ref().is_some_and(|stage| stage.key == key) {
            return Ok(());
        }

        let scale = monitor.scale;
        let max_scale = key.max_scale;
        let metrics = Metrics {
            icon: key.icon_size as f32,
            ..Metrics::DEFAULT
        }
        .scaled(scale);
        let theme = Theme::for_system(key.light, key.glass);
        let px = |logical: f32| (logical * scale).round();

        self.entries = key.entries.clone();
        self.prepare_assets(
            (metrics.icon * max_scale).round() as u32,
            scale,
            theme.light,
        );

        let kinds: Vec<ItemKind> = self.entries.iter().map(|entry| entry.kind).collect();
        let mut dock = layout(&kinds, &metrics);
        dock.width = dock.width.round();
        dock.height = dock.height.round();
        let magnifier = Magnifier {
            range: RANGE_IN_ICONS * (metrics.icon + metrics.gap),
            max_scale,
        };

        let side_growth = (magnifier.max_growth() / 2.0).ceil();
        let rise = (metrics.icon * (max_scale - 1.0)).ceil();

        // Position relative to the screen, not the work area; the work area
        // already excludes the strip reserved by the dock.
        let screen = monitor.screen;
        let screen_width = (screen.right - screen.left) as f32;
        let dock_x = screen.left as f32 + ((screen_width - dock.width) / 2.0).round();
        let dock_y = screen.bottom as f32 - px(FLOAT_MARGIN) - dock.height;
        // A hidden dock leaves the whole screen to maximized windows.
        let strip = if self.config.auto_hide {
            0
        } else {
            (dock.height + 2.0 * px(FLOAT_MARGIN)) as i32
        };
        let (dock_right, dock_bottom) = (dock_x + dock.width, dock_y + dock.height);
        let zone = move |left: f32, top: f32, right: f32, bottom: f32| RECT {
            left: (dock_x - left) as i32,
            top: (dock_y - top) as i32,
            right: (dock_right + right) as i32,
            bottom: (dock_bottom + bottom) as i32,
        };

        let room = px(SHADOW_ROOM);
        let (room_x, room_top) = (room + side_growth, rise + px(LABEL_ROOM));
        let contents: Vec<SlotContent<'_>> = self
            .entries
            .iter()
            .map(|entry| {
                let asset = entry.id.as_ref().and_then(|id| self.assets.get(id));
                let badge = entry
                    .id
                    .as_deref()
                    .and_then(window_of)
                    .and_then(|hwnd| self.windows.iter().find(|window| window.hwnd == hwnd))
                    .and_then(|window| self.assets.get(&window.app_id))
                    .and_then(|asset| asset.icon.as_ref());
                SlotContent {
                    icon: asset.and_then(|asset| asset.icon.as_ref()),
                    label: asset.and_then(|asset| asset.label.as_ref()),
                    running: entry.running && self.config.show_indicators,
                    badge,
                }
            })
            .collect();
        let scene = Scene::build(
            &self.gfx,
            &SceneInput {
                layout: &dock,
                contents: &contents,
                theme,
                scale,
                origin: (room_x, room_top),
                remove_label: self.remove_label.as_ref(),
            },
        )?;
        self.target.SetRoot(&scene.root)?;

        let (enter, leave) = (px(ENTER_HALO), px(LEAVE_HALO));
        let stage = Stage {
            key,
            window_zone: zone(room_x, room_top, room_x, room),
            scene,
            magnifier,
            icon: metrics.icon,
            gap: metrics.gap,
            scale,
            origin: (room_x, room_top),
            dock_origin: POINT {
                x: dock_x as i32,
                y: dock_y as i32,
            },
            rest_zone: zone(enter, enter, enter, enter),
            hover_zone: zone(
                side_growth + leave,
                rise + leave,
                side_growth + leave,
                leave,
            ),
            edge_zone: RECT {
                left: (dock_x - side_growth) as i32,
                top: screen.bottom - REVEAL_STRIP,
                right: (dock_right + side_growth) as i32,
                bottom: screen.bottom,
            },
            slide_distance: dock.height + px(FLOAT_MARGIN) + room,
            layout: dock,
        };
        self.stage = Some(stage);
        self.place_windows()?;
        self.schedule_trim();
        self.render()?;
        // Last: reserving space is a synchronous call to Explorer (40-60 ms
        // measured), and the dock is already on screen meanwhile.
        if let Some(appbar) = &mut self.appbar {
            appbar.reserve(Edge::Bottom, screen, strip);
        }
        Ok(())
    }

    fn place_windows(&self) -> Result<()> {
        let Some(stage) = &self.stage else {
            return Ok(());
        };
        let visible = !self.fullscreen;
        place(self.visual, &stage.window_zone, visible)?;
        // Preserve the expanded input area if the cursor is already over the
        // dock (for example, after launching an app with a dock click).
        let input_zone = if self.hovering || self.drag.is_some() {
            &stage.hover_zone
        } else if self.config.auto_hide {
            &stage.edge_zone
        } else {
            &stage.rest_zone
        };
        place(self.input, input_zone, visible)
    }

    fn schedule_trim(&self) {
        // SAFETY: `self.input` is this live window, and this call runs on its UI thread.
        unsafe { SetTimer(Some(self.input), TRIM_TIMER, TRIM_DELAY_MS, None) };
    }

    fn trim_if_idle(&self) {
        if !self.hovering && !self.is_animating() {
            power::trim_memory();
        }
    }

    fn render(&mut self) -> Result<()> {
        let Some(stage) = &self.stage else {
            return Ok(());
        };
        let state = magnify(
            &stage.layout,
            self.cursor,
            self.amount.value,
            &stage.magnifier,
        );
        self.hovered = (self.hovering || self.pointed)
            .then(|| state.icon_at(&stage.layout, self.cursor, stage.gap))
            .flatten();

        let now = Instant::now();
        let lifts: Vec<f32> = self
            .entries
            .iter()
            .map(|entry| {
                entry
                    .id
                    .as_ref()
                    .and_then(|id| self.bounces.get(id))
                    .map_or(0.0, |bounce| {
                        let elapsed = now.duration_since(bounce.start).as_secs_f32();
                        bounce::lift(elapsed, stage.icon * BOUNCE_HEIGHT)
                    })
            })
            .collect();

        let drag = self.drag.as_ref().and_then(|drag| {
            Some(Drag {
                index: self
                    .entries
                    .iter()
                    .position(|entry| entry.id.as_deref() == Some(drag.id.as_str()))?,
                center: (
                    (drag.point.x - stage.dock_origin.x) as f32,
                    (drag.point.y - stage.dock_origin.y) as f32,
                ),
                removing: drag.removing,
            })
        });
        if drag.is_some() {
            self.hovered = None;
        }
        let at_rest = self.amount.value == 0.0
            && self.bounces.is_empty()
            && self.slide.is_settled()
            && drag.is_none();
        let slide = self.slide.value * stage.slide_distance;
        stage.scene.apply(&Frame {
            state: &state,
            hovered: self.hovered,
            lifts: &lifts,
            drag,
            slide,
            snap: at_rest,
        })?;

        let visible = !self.fullscreen;
        for (entry, rect) in self.entries.iter().zip(&state.rects) {
            if let Some(hwnd) = entry.id.as_deref().and_then(window_of) {
                let slot = (
                    stage.origin.0 + rect.x,
                    stage.origin.1 + rect.y + slide,
                    rect.width,
                    rect.height,
                );
                let has_badge = self
                    .windows
                    .iter()
                    .find(|window| window.hwnd == hwnd)
                    .and_then(|window| self.assets.get(&window.app_id))
                    .is_some_and(|asset| asset.icon.is_some());
                let corner = if has_badge { BADGE_CORNER } else { 0.0 };
                let opacity = match self.flights.iter().find(|flight| flight.hwnd == hwnd) {
                    _ if !visible => 0,
                    Some(flight) => flight.landing_opacity(now),
                    None => 255,
                };
                self.thumbnails.place(hwnd, slot, corner, opacity);
            }
        }
        Ok(())
    }

    /// Screen rectangle, at rest and with the window's aspect ratio, where a
    /// minimized window lives in the dock: its app's icon, or its own
    /// thumbnail slot with "show minimized windows". The genie flies into it
    /// and back out of it.
    fn minimized_slot(&self, hwnd: isize, aspect: f32) -> Option<RECT> {
        let stage = self.stage.as_ref()?;
        let id = if !self.config.show_minimized {
            self.windows
                .iter()
                .find(|window| window.hwnd == hwnd)?
                .app_id
                .clone()
        } else {
            window_id(hwnd)
        };
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id.as_deref() == Some(id.as_str()))?;
        let rect = stage.layout.slots.get(index)?.rect;
        let (width, height) = if aspect >= 1.0 {
            (rect.width, rect.width / aspect)
        } else {
            (rect.height * aspect, rect.height)
        };
        let left = stage.dock_origin.x as f32 + rect.x + (rect.width - width) / 2.0;
        let top = stage.dock_origin.y as f32 + rect.y + (rect.height - height) / 2.0;
        Some(RECT {
            left: left as i32,
            top: top as i32,
            right: (left + width) as i32,
            bottom: (top + height) as i32,
        })
    }

    /// A window starts minimizing: put it in the dock right away and fly it there.
    fn on_minimize_start(&mut self, hwnd: isize) -> Result<()> {
        if self.fullscreen
            || self.minimized.contains(&hwnd)
            || !self.windows.iter().any(|window| window.hwnd == hwnd)
        {
            return Ok(());
        }
        // The window has already vanished (its system transition is off):
        // the stand-in goes up before anything else, or the desktop shows.
        self.show_stand_ins(&[hwnd]);
        self.minimized.push(hwnd);
        self.sync_thumbnails();
        self.refresh()?;
        self.aim_flights();
        Ok(())
    }

    /// Live thumbnails exist only for minimized windows shown in the dock:
    /// none while they hide in their app's icon, so DWM draws nothing extra.
    fn sync_thumbnails(&mut self) {
        let shown: &[isize] = if self.config.show_minimized {
            &self.minimized
        } else {
            &[]
        };
        self.thumbnails.sync(shown);
    }

    /// Turns off the system's transitions for every tracked window. Windows
    /// starts its own minimize animation before the dock hears about the
    /// minimize, and it cannot be cancelled afterwards, so it has to be off in
    /// advance for the genie to be the only animation. The cost: those windows
    /// also lose the system's maximize and close animations while the dock runs.
    fn calm_windows(&mut self) {
        if !self.config.genie || !animations_enabled() {
            return;
        }
        let windows = &self.windows;
        self.calmed
            .retain(|hwnd| windows.iter().any(|window| window.hwnd == *hwnd));
        for window in windows {
            if !self.calmed.contains(&window.hwnd) {
                set_window_transitions(window.hwnd, false);
                self.calmed.push(window.hwnd);
            }
        }
    }

    fn restore_transitions(&mut self) {
        for hwnd in self.calmed.drain(..) {
            set_window_transitions(hwnd, true);
        }
    }

    /// Brings windows to the front. A minimized one flies out of the dock if
    /// it flew in; otherwise it gets its system transitions back first, so it
    /// returns with the system's restore animation (the next `sync` turns
    /// them off again).
    fn activating(&mut self, windows: Vec<isize>) -> Option<Outside> {
        if self.start_restore_flight(&windows) {
            // `tick` restores the window when the genie reaches full size.
            return None;
        }
        for hwnd in &windows {
            if self.minimized.contains(hwnd) {
                set_window_transitions(*hwnd, true);
                self.calmed.retain(|calmed| calmed != hwnd);
            }
        }
        Some(Outside::Activate(windows))
    }

    /// Flies out of the dock the window that activating `windows` would
    /// restore. `false` if the genie does not apply and the caller must
    /// restore it directly.
    fn start_restore_flight(&mut self, windows: &[isize]) -> bool {
        if !self.config.genie || !animations_enabled() || !self.flights.is_empty() {
            return false;
        }
        // Activation restores a window only when all of them are minimized,
        // and then only the first.
        let Some(&hwnd) = windows.first() else {
            return false;
        };
        if !windows.iter().all(|window| self.minimized.contains(window)) {
            return false;
        }
        let Some(position) = self.snapshots.iter().position(|(owner, _)| *owner == hwnd) else {
            return false;
        };
        let Some(screen) = primary_monitor().map(|monitor| monitor.screen) else {
            return false;
        };
        let flight = self.snapshots.get(position).and_then(|(_, snapshot)| {
            let slot = self.minimized_slot(hwnd, snapshot.aspect())?;
            Flight::restore(&self.gfx, hwnd, snapshot, slot, screen)
        });
        let Some(flight) = flight else {
            return false;
        };
        if !self.config.show_minimized {
            flight.tuck_under(self.visual);
        }
        self.snapshots.remove(position);
        self.flights.push(flight);
        self.last_frame = Instant::now();
        true
    }

    fn start_flights(&mut self, windows: &[isize]) {
        self.show_stand_ins(windows);
        self.aim_flights();
    }

    /// Gives each new minimize flight its slot, now that the dock shows it.
    fn aim_flights(&mut self) {
        let slots: Vec<(usize, Option<RECT>)> = self
            .flights
            .iter()
            .enumerate()
            .filter(|(_, flight)| flight.needs_aim())
            .map(|(index, flight)| (index, self.minimized_slot(flight.hwnd, flight.aspect())))
            .collect();
        for (index, slot) in slots {
            if let (Some(slot), Some(flight)) = (slot, self.flights.get_mut(index)) {
                flight.aim(&self.gfx, slot);
                if !self.config.show_minimized {
                    flight.tuck_under(self.visual);
                }
            }
        }
    }

    fn show_stand_ins(&mut self, windows: &[isize]) {
        // Respect "Animation effects" being turned off in Windows settings.
        // One genie at a time: minimizing everything at once (Win+D) would
        // otherwise take a screen snapshot per window.
        if !self.config.genie || !animations_enabled() || !self.flights.is_empty() {
            return;
        }
        let Some(screen) = primary_monitor().map(|monitor| monitor.screen) else {
            return;
        };
        for &hwnd in windows {
            let last_seen = self.last_rects.get(&hwnd).copied();
            let Some((from, frame)) = tracker::rect_before_minimize(hwnd, last_seen) else {
                continue;
            };
            if let Some(flight) = Flight::start(&self.gfx, hwnd, from, frame, screen) {
                self.flights.push(flight);
                self.last_frame = Instant::now();
                break;
            }
        }
    }

    fn on_mouse_move(&mut self) -> Result<()> {
        let Some(stage) = &self.stage else {
            return Ok(());
        };
        let mut point = POINT::default();
        // SAFETY: `point` remains alive for the duration of the call.
        unsafe { GetCursorPos(&mut point)? };

        let origin_x = stage.dock_origin.x;
        if self.update_drag(point) {
            self.cursor = (point.x - origin_x) as f32;
            return self.drag_to(point);
        }
        let Some(stage) = &self.stage else {
            return Ok(());
        };

        // Keep the active-area boundary aligned with the icons as they grow.
        let rise = stage.icon * (stage.magnifier.max_scale - 1.0) * self.amount.value;
        let ceiling = stage.dock_origin.y as f32 - rise - LEAVE_HALO * stage.scale;
        if self.hovering && (point.y as f32) < ceiling {
            return self.on_mouse_leave();
        }

        self.cursor = (point.x - stage.dock_origin.x) as f32;
        if !self.hovering {
            self.hovering = true;
            self.amount.target = 1.0;
            self.slide.target = 0.0;
            self.last_frame = Instant::now();
            // SAFETY: Timer on our own window; killing a timer that is not set is harmless.
            unsafe {
                let _ = KillTimer(Some(self.input), HIDE_TIMER);
            }
            self.place_windows()?;
        }
        // Every move, not only on entering: a menu or a drag that captures
        // the mouse cancels the request, and without it the dock would
        // never hear the pointer leave and stay magnified.
        self.track_leave()?;
        self.render()
    }

    fn track_leave(&self) -> Result<()> {
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.input,
            dwHoverTime: 0,
        };
        // SAFETY: `track` is a valid structure with its size initialized.
        unsafe { TrackMouseEvent(&mut track)? };
        Ok(())
    }

    /// A menu closed: it held the mouse, so the dock heard nothing of where
    /// the pointer went meanwhile. Settle the magnifier on where it is now.
    fn after_menu(&mut self) -> Result<()> {
        let mut point = POINT::default();
        // SAFETY: Read-only queries.
        let over = unsafe {
            GetCursorPos(&mut point).is_ok()
                && windows::Win32::UI::WindowsAndMessaging::WindowFromPoint(point) == self.input
        };
        if over {
            self.track_leave()
        } else {
            self.on_mouse_leave()
        }
    }

    fn on_mouse_leave(&mut self) -> Result<()> {
        if !self.hovering || self.drag.is_some() {
            return Ok(());
        }
        self.hovering = false;
        self.amount.target = 0.0;
        self.last_frame = Instant::now();
        if self.config.auto_hide {
            // SAFETY: Timer on our own window.
            unsafe { SetTimer(Some(self.input), HIDE_TIMER, HIDE_DELAY_MS, None) };
        }
        self.place_windows()?;
        self.render()
    }

    /// Alt+Tab (or Task View) opened: an auto-hidden dock comes up to show
    /// the open apps until the switcher closes. It cannot follow the
    /// selection: Windows 11's switcher reports it to no one (no accessible
    /// focus, selection or name on its items; checked with MSAA and UIA).
    fn task_switcher_opened(&mut self) -> Result<()> {
        // Only Alt+Tab (and Ctrl+Alt+Tab). Task View shares the switcher's
        // window, and on Windows 11 its virtual desktops sit along the
        // bottom edge, where the dock would cover them.
        // SAFETY: Read-only keyboard state query.
        let alt = unsafe { GetAsyncKeyState(i32::from(VK_MENU.0)) < 0 };
        // After it closes, Windows sometimes hands the focus back to the
        // switcher's hidden window for a moment: not a new Alt+Tab.
        let echo = self
            .switch_ended
            .is_some_and(|ended| ended.elapsed() < SWITCHER_ECHO)
            && tracker::task_switcher_state() != Some(true);
        if self.switching || !alt || echo {
            return Ok(());
        }
        self.switching = true;
        self.switch_started = Instant::now();
        // SAFETY: Timers on our own window.
        unsafe {
            SetTimer(Some(self.input), SWITCHER_TIMER, SWITCHER_POLL_MS, None);
            let _ = KillTimer(Some(self.input), HIDE_TIMER);
        }
        if self.config.auto_hide && !self.fullscreen {
            self.slide.target = 0.0;
            self.last_frame = Instant::now();
            self.place_windows()?;
            self.render()?;
        }
        if let Err(error) = self.start_following() {
            eprintln!("docky: Alt+Tab: {error:#}");
        }
        Ok(())
    }

    fn check_task_switcher(&mut self) -> Result<()> {
        let open = match tracker::task_switcher_state() {
            Some(shown) => shown || self.switch_started.elapsed() < SWITCHER_APPEAR,
            None => false,
        };
        if open {
            return self.follow_selection();
        }
        self.switching = false;
        self.switch_ended = Some(Instant::now());
        // SAFETY: Our own timer.
        unsafe {
            let _ = KillTimer(Some(self.input), SWITCHER_TIMER);
        }
        self.stop_following()?;
        self.hide_if_unused()
    }

    /// Starts following the switcher's selection, when it can be known:
    /// opened with Alt+Tab, and listing exactly this desktop's windows.
    ///
    /// Windows does not say which item is selected, so it is worked out the
    /// way the switcher moves: it lists the windows from the most recently
    /// used, starts on the second (the last with Alt+Shift+Tab), and each Tab
    /// moves one ahead, Shift+Tab one back.
    fn start_following(&mut self) -> Result<()> {
        // SAFETY: Read-only keyboard state queries.
        let (alt, win, shift) = unsafe {
            (
                GetAsyncKeyState(i32::from(VK_MENU.0)) < 0,
                GetAsyncKeyState(i32::from(VK_LWIN.0)) < 0
                    || GetAsyncKeyState(i32::from(VK_RWIN.0)) < 0,
                GetAsyncKeyState(i32::from(VK_SHIFT.0)) < 0,
            )
        };
        if !alt || win {
            return Ok(());
        }
        // Topmost first, which is the order of last use.
        let order: Vec<isize> = self
            .tracker
            .list()
            .iter()
            .map(|window| window.hwnd)
            .collect();
        let edge_open = self
            .windows
            .iter()
            .any(|window| window.app_id.ends_with("\\msedge.exe"));
        if order.len() < 2 || status::alt_tab_lists_more_than_windows(edge_open) {
            return Ok(());
        }
        // Opened from the desktop or another unlisted window, the switcher
        // starts on the first window rather than the second.
        let from_listed = order.first() == Some(&LAST_FRONT.load(Ordering::Relaxed));
        let index = match (shift, from_listed) {
            (true, _) => order.len() - 1,
            (false, true) => 1,
            (false, false) => 0,
        };
        self.follow = Some(SwitcherFollow {
            order,
            index,
            // The Tab that opened the switcher is still down: not a new press.
            tab_down: true,
        });
        self.point_at_selection()
    }

    /// Moves the selection with each Tab press. Arrow keys can change it in
    /// ways that cannot be followed: the highlight then goes away rather
    /// than point at the wrong app.
    fn follow_selection(&mut self) -> Result<()> {
        let Some(follow) = &mut self.follow else {
            return Ok(());
        };
        let down = |key: VIRTUAL_KEY| {
            // SAFETY: Read-only keyboard state query.
            unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 }
        };
        // Only these move the selection in ways that cannot be followed. A
        // click on an item switches to it and closes the switcher; a click
        // anywhere else, or moving the pointer, leaves the selection alone.
        let lost = [VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_HOME, VK_END]
            .into_iter()
            .any(down);
        if lost {
            return self.stop_following();
        }
        // A window closed from the switcher (its "x", or Delete) leaves the
        // list, and the switcher keeps the selection on the same place.
        let before = follow.order.len();
        let index = follow.index;
        let alive: Vec<bool> = follow
            .order
            .iter()
            .map(|&hwnd| {
                let window = HWND(hwnd as *mut _);
                // SAFETY: Read-only queries; a closed window simply fails them.
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(window)).as_bool()
                        && windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(window)
                            .as_bool()
                }
            })
            .collect();
        let mut position = 0;
        follow.order.retain(|_| {
            position += 1;
            alive[position - 1]
        });
        let closed = follow.order.len() != before;
        let Some(selected) = selection_after_closing(index, &alive) else {
            return self.stop_following();
        };
        follow.index = selected;
        let tab = down(VK_TAB);
        let pressed = tab && !follow.tab_down;
        follow.tab_down = tab;
        if !pressed {
            return if closed {
                self.point_at_selection()
            } else {
                Ok(())
            };
        }
        let count = follow.order.len();
        follow.index = if down(VK_SHIFT) {
            (follow.index + count - 1) % count
        } else {
            (follow.index + 1) % count
        };
        self.point_at_selection()
    }

    /// Magnifies the dock icon of the selected window, as if the pointer
    /// rested on it, and shows its name.
    fn point_at_selection(&mut self) -> Result<()> {
        let Some(follow) = &self.follow else {
            return Ok(());
        };
        let selected = follow.order[follow.index];
        let window_entry = window_id(selected);
        let app = self
            .windows
            .iter()
            .find(|window| window.hwnd == selected)
            .map(|window| window.app_id.clone());
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id.as_deref() == Some(window_entry.as_str()))
            .or_else(|| {
                let app = app?;
                self.entries
                    .iter()
                    .position(|entry| entry.role == Role::App && entry.id.as_ref() == Some(&app))
            });
        let (Some(index), Some(stage)) = (index, &self.stage) else {
            return self.stop_following();
        };
        let Some(slot) = stage.layout.slots.get(index) else {
            return self.stop_following();
        };
        if self.hovering {
            return Ok(());
        }
        self.cursor = slot.rect.x + slot.rect.width / 2.0;
        self.pointed = true;
        self.amount.target = 1.0;
        self.last_frame = Instant::now();
        self.render()
    }

    fn stop_following(&mut self) -> Result<()> {
        self.follow = None;
        if !self.pointed {
            return Ok(());
        }
        self.pointed = false;
        if !self.hovering {
            self.amount.target = 0.0;
            self.last_frame = Instant::now();
        }
        self.render()
    }

    /// Aims the update timer at the next look: a day after the last one,
    /// which survives restarts, and never sooner than two minutes after the
    /// dock starts. So the dock wakes for it once a day, not on a schedule.
    fn schedule_update(&self) {
        // SAFETY: Timers on our own window; setting one again replaces it.
        unsafe {
            if !self.config.check_updates || !self.config.auto_update {
                let _ = KillTimer(Some(self.input), UPDATE_TIMER);
                return;
            }
            // Also counted from the last try in this session: if the date
            // could not be saved, the registry would call it due again at
            // once, and the dock would start one update after another.
            let since_try = self
                .update_tried
                .map_or(std::time::Duration::ZERO, |tried| {
                    UPDATE_EVERY.saturating_sub(tried.elapsed())
                });
            let wait = docky_shell::updates::due_in(UPDATE_EVERY)
                .max(UPDATE_AFTER_START.saturating_sub(self.launched.elapsed()))
                .max(since_try);
            SetTimer(Some(self.input), UPDATE_TIMER, millis(wait), None);
        }
    }

    /// Starts the background update: Settings without a window, which does
    /// the network part and exits when there is nothing to install. The
    /// dock itself never touches the network.
    fn update_now(&mut self) {
        // SAFETY: Our own timer.
        unsafe {
            let _ = KillTimer(Some(self.input), UPDATE_TIMER);
        }
        if !self.config.check_updates || !self.config.auto_update {
            return;
        }
        // Installing restarts the dock: not in the middle of a game.
        if self.fullscreen {
            // SAFETY: Our own timer.
            unsafe { SetTimer(Some(self.input), UPDATE_TIMER, millis(UPDATE_RETRY), None) };
            return;
        }
        // Recorded first: whatever happens, the next look is tomorrow.
        docky_shell::updates::mark_checked();
        self.update_tried = Some(Instant::now());
        self.schedule_update();
        let settings = env::current_exe()
            .ok()
            .map(|exe| exe.with_file_name("docky-settings.exe"))
            .filter(|path| path.exists());
        if let Some(settings) = settings
            && let Err(error) = std::process::Command::new(settings).arg("--update").spawn()
        {
            eprintln!("docky: update: {error}");
        }
    }

    fn hide_if_unused(&mut self) -> Result<()> {
        if !self.config.auto_hide || self.hovering || self.drag.is_some() || self.switching {
            return Ok(());
        }
        self.slide.target = 1.0;
        self.last_frame = Instant::now();
        self.place_windows()
    }

    /// Only running or pinned apps can be dragged; the Explorer, Start,
    /// recent apps, Downloads, and the recycle bin stay in place.
    fn is_draggable(&self, entry: &Entry) -> bool {
        entry.role == Role::App
            && entry.id.as_ref().is_some_and(|id| {
                *id != explorer_id() && (self.roster.is_pinned(id) || self.roster.is_running(id))
            })
    }

    fn on_press(&mut self) -> Result<()> {
        let Some(entry) = self.hovered.and_then(|index| self.entries.get(index)) else {
            return Ok(());
        };
        if !self.is_draggable(entry) {
            return Ok(());
        }
        let mut at = POINT::default();
        // SAFETY: `at` lives for the call; capture keeps mouse input while dragging
        // outside the input window.
        unsafe {
            GetCursorPos(&mut at)?;
            SetCapture(self.input);
        }
        self.press = entry.id.clone().map(|id| Press { id, at });
        Ok(())
    }

    /// Starts a drag once the cursor moves past the system drag threshold.
    /// Returns `true` while a drag is in progress.
    fn update_drag(&mut self, point: POINT) -> bool {
        if self.drag.is_none()
            && let Some(press) = &self.press
        {
            // SAFETY: Read-only query.
            let threshold = unsafe { GetSystemMetrics(SM_CXDRAG) }.max(1);
            if (point.x - press.at.x).abs() > threshold || (point.y - press.at.y).abs() > threshold
            {
                self.drag = Some(Dragging {
                    id: press.id.clone(),
                    point,
                    removing: false,
                });
            }
        }
        self.drag.is_some()
    }

    /// Moves the dragged icon and reorders the pinned apps live, as in macOS.
    fn drag_to(&mut self, point: POINT) -> Result<()> {
        let (Some(stage), Some(drag)) = (&self.stage, &mut self.drag) else {
            return Ok(());
        };
        drag.point = point;
        let id = drag.id.clone();
        let pinned = self.roster.is_pinned(&id);
        let top = stage.dock_origin.y as f32
            - stage.icon * (stage.magnifier.max_scale - 1.0 + REMOVE_DISTANCE);
        drag.removing = pinned && (point.y as f32) < top;
        if drag.removing {
            return self.render();
        }

        let state = magnify(
            &stage.layout,
            self.cursor,
            self.amount.value,
            &stage.magnifier,
        );
        let explorer = explorer_id();
        let mut before: Option<String> = None;
        let mut pinned_end = f32::MIN;
        for (entry, rect) in self.entries.iter().zip(&state.rects) {
            let Some(other) = entry.id.as_ref() else {
                continue;
            };
            if entry.role != Role::App
                || *other == id
                || *other == explorer
                || !self.roster.is_pinned(other)
            {
                continue;
            }
            pinned_end = rect.x + rect.width;
            if before.is_none() && rect.x + rect.width / 2.0 > self.cursor {
                before = Some(other.clone());
            }
        }
        // An unpinned app is pinned only when dropped among the pinned ones.
        if !pinned && before.is_none() && self.cursor > pinned_end + stage.gap {
            return self.render();
        }

        let list = self.roster.pinned();
        let current = list
            .iter()
            .position(|pinned| *pinned == id)
            .map(|index| list.get(index + 1).cloned());
        if current == Some(before.clone()) {
            return self.render();
        }
        self.roster.pin_before(&id, before.as_deref());
        self.refresh()
    }

    /// Hover driven by a drag from another app: OLE takes the mouse, so the
    /// dock gets no WM_MOUSEMOVE meanwhile.
    fn hover_at(&mut self, x: i32) -> Result<()> {
        let Some(stage) = &self.stage else {
            return Ok(());
        };
        self.cursor = (x - stage.dock_origin.x) as f32;
        if !self.hovering {
            self.hovering = true;
            self.amount.target = 1.0;
            self.slide.target = 0.0;
            self.last_frame = Instant::now();
            self.place_windows()?;
        }
        self.render()
    }

    fn drop_action(&self) -> DropAction {
        let Some(stage) = &self.stage else {
            return DropAction::Nothing;
        };
        let state = magnify(
            &stage.layout,
            self.cursor,
            self.amount.value,
            &stage.magnifier,
        );
        let target = state
            .icon_at(&stage.layout, self.cursor, stage.gap)
            .and_then(|index| self.entries.get(index));
        let Some(entry) = target else {
            return self.pin_action(&state);
        };
        let Some(id) = entry.id.clone() else {
            return DropAction::Nothing;
        };
        match entry.role {
            Role::App if id.ends_with(".exe") => DropAction::OpenWith(id),
            Role::Trash => DropAction::Recycle,
            Role::Folder => DropAction::Transfer {
                copy: !self
                    .incoming
                    .iter()
                    .all(|file| files::same_volume(file, &id)),
                folder: id,
            },
            Role::App | Role::Start | Role::Window => DropAction::Nothing,
        }
    }

    /// A single app or shortcut dropped between icons is pinned there.
    fn pin_action(&self, state: &docky_core::magnify::Magnified) -> DropAction {
        let [file] = self.incoming.as_slice() else {
            return DropAction::Nothing;
        };
        let lower = file.to_lowercase();
        let target = if lower.ends_with(".lnk") {
            link_target(std::path::Path::new(file))
        } else {
            Some(file.clone())
        };
        let Some(id) = target
            .map(|target| normalize_id(&target))
            .filter(|id| id.ends_with(".exe"))
        else {
            return DropAction::Nothing;
        };
        let explorer = explorer_id();
        let before = self
            .entries
            .iter()
            .zip(&state.rects)
            .find(|(entry, rect)| {
                entry.role == Role::App
                    && entry.id.as_ref().is_some_and(|other| {
                        *other != explorer && *other != id && self.roster.is_pinned(other)
                    })
                    && rect.x + rect.width / 2.0 > self.cursor
            })
            .and_then(|(entry, _)| entry.id.clone());
        DropAction::Pin { id, before }
    }

    fn on_drag_enter(&mut self, incoming: Vec<String>, x: i32) -> Result<DROPEFFECT> {
        self.incoming = incoming;
        self.drag_over = true;
        self.on_drag_over(x)
    }

    fn on_drag_over(&mut self, x: i32) -> Result<DROPEFFECT> {
        self.hover_at(x)?;
        self.aim_spring();
        Ok(self.drop_action().effect())
    }

    fn on_drag_leave(&mut self) -> Result<()> {
        self.incoming.clear();
        self.end_drag_over();
        self.on_mouse_leave()
    }

    fn end_drag_over(&mut self) {
        self.drag_over = false;
        if self.spring.take().is_some() {
            // SAFETY: Our own timer on our own window.
            unsafe {
                let _ = KillTimer(Some(self.input), SPRING_TIMER);
            }
        }
    }

    /// The entry under a drag that can spring open: an app with windows, or
    /// a minimized window's thumbnail.
    fn spring_target(&self) -> Option<String> {
        let entry = self.hovered.and_then(|index| self.entries.get(index))?;
        let id = entry.id.clone()?;
        match entry.role {
            Role::App if !self.windows_of(&id).is_empty() => Some(id),
            Role::Window => Some(id),
            _ => None,
        }
    }

    /// Starts the spring delay when the drag moves onto another entry; the
    /// delay restarts on each new entry, so sweeping across the dock opens
    /// nothing.
    fn aim_spring(&mut self) {
        let target = self.spring_target();
        if target == self.spring {
            return;
        }
        // SAFETY: Our own timer on our own window; setting it again restarts it.
        unsafe {
            if target.is_some() {
                SetTimer(Some(self.input), SPRING_TIMER, SPRING_DELAY_MS, None);
            } else {
                let _ = KillTimer(Some(self.input), SPRING_TIMER);
            }
        }
        self.spring = target;
    }

    /// The drag rested on an app: bring its windows to the front, under the
    /// pointer, so the drop lands in the app.
    fn spring_open(&mut self) -> Result<Option<Outside>> {
        // SAFETY: Our own timer on our own window.
        unsafe {
            let _ = KillTimer(Some(self.input), SPRING_TIMER);
        }
        if !self.drag_over || self.spring_target() != self.spring {
            return Ok(None);
        }
        let Some(id) = self.spring.clone() else {
            return Ok(None);
        };
        let windows = match self.hovered.and_then(|index| self.entries.get(index)) {
            Some(entry) if entry.role == Role::Window => window_of(&id).into_iter().collect(),
            _ => self.windows_of(&id),
        };
        if windows.is_empty() || windows.contains(&tracker::foreground_window()) {
            return Ok(None);
        }
        Ok(self.activating(windows))
    }

    fn on_drop(&mut self, x: i32) -> Result<DROPEFFECT> {
        self.end_drag_over();
        self.hover_at(x)?;
        let action = self.drop_action();
        let effect = action.effect();
        let files = std::mem::take(&mut self.incoming);
        self.pending_drop = match action {
            DropAction::Nothing => None,
            DropAction::OpenWith(app) => {
                self.start_bounce(app.clone(), None);
                Some(Outside::OpenWith(app, files))
            }
            DropAction::Recycle => Some(Outside::Recycle(files)),
            DropAction::Transfer { folder, copy } => Some(Outside::Transfer {
                folder,
                files,
                copy,
            }),
            DropAction::Pin { id, before } => {
                self.roster.pin_before(&id, before.as_deref());
                save_settings(&self.roster, &self.config);
                self.refresh()?;
                None
            }
        };
        if self.pending_drop.is_some() {
            // SAFETY: Posting a message to our own window.
            unsafe {
                let _ = PostMessageW(Some(self.input), DROP_RUN, WPARAM(0), LPARAM(0));
            }
        }
        // The mouse is still over the dock; the next WM_MOUSEMOVE starts a
        // tracked hover again.
        self.on_mouse_leave()?;
        Ok(effect)
    }

    fn on_release(&mut self) -> Result<Option<Outside>> {
        self.press = None;
        let Some(drag) = self.drag.take() else {
            // SAFETY: Releasing capture we may not hold is harmless.
            unsafe {
                let _ = ReleaseCapture();
            }
            return self.on_click();
        };
        // SAFETY: Same as above.
        unsafe {
            let _ = ReleaseCapture();
        }
        if drag.removing {
            self.roster.unpin(&drag.id);
        }
        save_settings(&self.roster, &self.config);
        self.refresh()?;
        self.leave_if_outside(drag.point)?;
        Ok(None)
    }

    /// Capture can be lost mid-drag (Alt+Tab, a system dialog): keep the order
    /// reached so far.
    fn cancel_drag(&mut self) -> Result<()> {
        self.press = None;
        let Some(drag) = self.drag.take() else {
            return Ok(());
        };
        save_settings(&self.roster, &self.config);
        self.leave_if_outside(drag.point)?;
        self.render()
    }

    fn leave_if_outside(&mut self, point: POINT) -> Result<()> {
        let Some(stage) = &self.stage else {
            return Ok(());
        };
        let zone = stage.hover_zone;
        let inside = point.x >= zone.left
            && point.x < zone.right
            && point.y >= zone.top
            && point.y < zone.bottom;
        if inside {
            Ok(())
        } else {
            self.on_mouse_leave()
        }
    }

    fn windows_of(&self, id: &str) -> Vec<isize> {
        self.windows
            .iter()
            .filter(|window| window.app_id == id)
            .map(|window| window.hwnd)
            .collect()
    }

    /// On icon click, launches a closed app or brings its windows to the
    /// foreground. Clicking the app that is already in front minimizes it, as
    /// the Windows taskbar does; the next click brings it back.
    fn on_click(&mut self) -> Result<Option<Outside>> {
        let Some(entry) = self.hovered.and_then(|index| self.entries.get(index)) else {
            return Ok(None);
        };
        let (Some(id), role) = (entry.id.clone(), entry.role) else {
            return Ok(None);
        };
        let windows = self.windows_of(&id);

        // As on the Windows taskbar: Shift+click opens another window of the
        // app, Ctrl+Shift+click opens it as administrator.
        if role == Role::App {
            // SAFETY: Read-only keyboard state queries.
            let (shift, ctrl) = unsafe {
                (
                    GetKeyState(i32::from(VK_SHIFT.0)) < 0,
                    GetKeyState(i32::from(VK_CONTROL.0)) < 0,
                )
            };
            if shift {
                self.start_bounce(id.clone(), None);
                return Ok(Some(if ctrl {
                    Outside::OpenAsAdmin(id)
                } else {
                    Outside::Open(id)
                }));
            }
        }

        Ok(match role {
            Role::App if windows.is_empty() => {
                self.start_bounce(id.clone(), None);
                Some(Outside::Open(id))
            }
            Role::App if windows.contains(&tracker::foreground_window()) => {
                Some(Outside::Minimize(windows))
            }
            Role::App => self.activating(windows),
            Role::Start => Some(Outside::OpenStart),
            Role::Window => window_of(&id).and_then(|hwnd| self.activating(vec![hwnd])),
            Role::Folder | Role::Trash => Some(Outside::Open(id)),
        })
    }

    /// Middle click on an app opens another window of it, as on the Windows
    /// taskbar.
    fn on_middle_click(&mut self) -> Result<Option<Outside>> {
        let Some(entry) = self.hovered.and_then(|index| self.entries.get(index)) else {
            return Ok(None);
        };
        let (Some(id), Role::App) = (entry.id.clone(), entry.role) else {
            return Ok(None);
        };
        self.start_bounce(id.clone(), None);
        Ok(Some(Outside::Open(id)))
    }

    /// Right-click prepares the menu for the item under the cursor, or the dock
    /// menu when over empty space or a divider.
    fn on_right_click(&mut self) -> Result<Option<Outside>> {
        let Some(stage) = &self.stage else {
            return Ok(None);
        };
        let state = magnify(
            &stage.layout,
            self.cursor,
            self.amount.value,
            &stage.magnifier,
        );
        let target_entry = self.hovered.and_then(|index| {
            let entry = self.entries.get(index)?;
            Some((entry.id.clone()?, entry.role, *state.rects.get(index)?))
        });
        let gap = (stage.gap / 2.0).round() as i32;

        let (target, items, anchor) = match target_entry {
            Some((id, role, rect)) => {
                let windows = self.windows_of(&id);
                let anchor = POINT {
                    x: stage.dock_origin.x + (rect.x + rect.width / 2.0).round() as i32,
                    y: stage.dock_origin.y + rect.y.round() as i32 - gap,
                };
                let items = self.entry_menu(&id, role, &windows);
                (MenuTarget::Entry { id, role, windows }, items, anchor)
            }
            None => {
                let anchor = POINT {
                    x: stage.dock_origin.x + self.cursor.round() as i32,
                    y: stage.dock_origin.y - gap,
                };
                let hiding = if self.config.auto_hide {
                    text().turn_hiding_off
                } else {
                    text().turn_hiding_on
                };
                let items = vec![
                    MenuItem::action(CMD_TOGGLE_HIDING, hiding),
                    MenuItem::action(CMD_SHOW_DESKTOP, text().show_desktop),
                    MenuItem::action(CMD_IMPORT_PINS, text().import_pins),
                    MenuItem::action(CMD_SETTINGS, text().settings),
                    MenuItem::Separator,
                    MenuItem::action(CMD_QUIT_DOCK, text().quit_dock),
                ];
                (MenuTarget::Dock, items, anchor)
            }
        };

        // Hide the label while the menu is open, as in macOS.
        self.hovered = None;
        self.hovering = false;
        self.render()?;
        Ok(Some(Outside::Menu(MenuRequest {
            gfx: self.gfx.clone(),
            target,
            items,
            anchor,
            dark: !uses_light_theme(),
        })))
    }

    fn entry_menu(&self, id: &str, role: Role, windows: &[isize]) -> Vec<MenuItem> {
        let is_file = !id.starts_with("shell:") && !id.starts_with("::");
        let t = text();
        let mut items = Vec::new();
        match role {
            Role::App => {
                for (index, &hwnd) in windows.iter().take(MENU_WINDOWS).enumerate() {
                    let title = tracker::title(hwnd);
                    if !title.is_empty() {
                        items.push(MenuItem::action(
                            CMD_WINDOW + index as u32,
                            shorten(&title, MENU_TITLE_CHARS).replace('&', "&&"),
                        ));
                    }
                }
                if !items.is_empty() {
                    items.push(MenuItem::Separator);
                }
                let open = if windows.is_empty() {
                    t.open
                } else {
                    t.new_window
                };
                items.push(MenuItem::action(CMD_OPEN, open));
                if id != explorer_id() {
                    items.push(MenuItem::Action {
                        command: CMD_TOGGLE_PIN,
                        label: t.keep_in_dock.into(),
                        checked: self.roster.is_pinned(id),
                        enabled: true,
                    });
                }
                if is_file {
                    items.push(MenuItem::action(CMD_RUN_AS_ADMIN, t.run_as_admin));
                    items.push(MenuItem::action(CMD_REVEAL, t.show_in_explorer));
                }
                if !windows.is_empty() {
                    items.push(MenuItem::Separator);
                    items.push(MenuItem::action(CMD_QUIT_APP, t.quit_app));
                }
            }
            Role::Start => items.push(MenuItem::action(CMD_OPEN, t.open)),
            Role::Window => {
                items.push(MenuItem::action(CMD_OPEN, t.open));
                items.push(MenuItem::Separator);
                items.push(MenuItem::action(CMD_CLOSE_WINDOW, t.close_window));
            }
            Role::Folder => {
                items.push(MenuItem::action(CMD_OPEN, t.open));
                items.push(MenuItem::action(CMD_REVEAL, t.show_in_explorer));
            }
            Role::Trash => {
                items.push(MenuItem::action(CMD_OPEN, t.open));
                items.push(MenuItem::Separator);
                items.push(MenuItem::Action {
                    command: CMD_EMPTY_BIN,
                    label: t.empty_bin.into(),
                    checked: false,
                    enabled: self.bin_full,
                });
            }
        }
        items
    }

    fn on_command(&mut self, target: MenuTarget, command: u32) -> Result<Option<Outside>> {
        let MenuTarget::Entry { id, role, windows } = target else {
            return match command {
                CMD_QUIT_DOCK => Ok(Some(Outside::QuitDock)),
                CMD_TOGGLE_HIDING => self.toggle_hiding().map(|()| None),
                CMD_IMPORT_PINS => self.import_taskbar_pins().map(|()| None),
                CMD_SHOW_DESKTOP => Ok(Some(Outside::ShowDesktop)),
                CMD_SETTINGS => Ok(Some(Outside::OpenSettings)),
                _ => Ok(None),
            };
        };
        Ok(match command {
            CMD_OPEN if role == Role::Start => Some(Outside::OpenStart),
            CMD_OPEN if role == Role::Window => {
                window_of(&id).and_then(|hwnd| self.activating(vec![hwnd]))
            }
            CMD_CLOSE_WINDOW => {
                launcher::close(&window_of(&id).into_iter().collect::<Vec<_>>());
                None
            }
            CMD_OPEN => {
                if role == Role::App {
                    self.start_bounce(id.clone(), None);
                }
                Some(Outside::Open(id))
            }
            CMD_TOGGLE_PIN => {
                // Unpinning here also unpins on the Windows taskbar, as there is
                // only one taskbar for the user. Pinning cannot go the other way.
                let on_taskbar = self
                    .config
                    .taskbar_pins
                    .as_ref()
                    .is_some_and(|pins| pins.contains(&id));
                let unpinning = self.roster.is_pinned(&id);
                if unpinning {
                    self.roster.unpin(&id);
                } else {
                    self.roster.pin(&id);
                }
                save_settings(&self.roster, &self.config);
                self.refresh()?;
                (unpinning && on_taskbar).then_some(Outside::UnpinFromTaskbar(id))
            }
            CMD_RUN_AS_ADMIN => Some(Outside::OpenAsAdmin(id)),
            CMD_REVEAL => Some(Outside::Reveal(id)),
            CMD_QUIT_APP => {
                launcher::close(&windows);
                None
            }
            CMD_EMPTY_BIN => Some(Outside::EmptyBin),
            window => {
                let index = window.checked_sub(CMD_WINDOW).map(|index| index as usize);
                index
                    .and_then(|index| windows.get(index))
                    .and_then(|&hwnd| self.activating(vec![hwnd]))
            }
        })
    }

    /// The settings app saved a change: read it and apply what differs.
    fn reload_config(&mut self) -> Result<()> {
        let Ok(Some(saved)) = docky_config::load(&docky_config::default_path()) else {
            return Ok(());
        };
        // Pinned and recent apps belong to the dock; the settings app only
        // changes the rest.
        let config = Config {
            pinned: self.config.pinned.clone(),
            recent: self.config.recent.clone(),
            taskbar_pins: self.config.taskbar_pins.clone(),
            ..saved
        };
        if config == self.config {
            return Ok(());
        }
        if !config.genie {
            self.restore_transitions();
            self.snapshots.clear();
        }
        if config.auto_hide != self.config.auto_hide {
            self.slide.target = if config.auto_hide { 1.0 } else { 0.0 };
            self.last_frame = Instant::now();
        }
        self.config = config;
        // Updates may have been turned on or off.
        self.schedule_update();
        self.sync_thumbnails();
        self.apply_menu_bar_setting();
        self.update_menu_bar_app()?;
        self.calm_windows();
        self.refresh()
    }

    /// Adds the apps pinned to the Windows taskbar that the dock does not
    /// have yet, in the taskbar's order, after the dock's own pins. Nothing is
    /// removed or reordered: the dock imports them by itself only the first
    /// time it runs, and pins the user removed since then stay removed until
    /// they ask for this.
    /// Brings pins and unpins made on the Windows taskbar to the dock, as if
    /// it were the taskbar: what was pinned there is pinned here, what was
    /// unpinned there is unpinned here. Apps pinned only in the dock stay.
    /// Windows offers no way to do the reverse (pin to the taskbar).
    fn sync_taskbar_pins(&mut self) -> Result<()> {
        let pruned = self.prune_missing_pins();
        let Some(current) = taskbar_pins_if_readable() else {
            if pruned {
                save_settings(&self.roster, &self.config);
                return self.refresh();
            }
            return Ok(());
        };
        let Some(previous) = self.config.taskbar_pins.replace(current.clone()) else {
            // First look since this feature: keep the dock as the user left it,
            // but add the Store apps pinned in Windows. Versions before 0.1.7
            // could not import them, so their absence was never a choice; on
            // Windows 10 most default pins (Store, Mail) are Store apps.
            for id in current
                .iter()
                .filter(|id| id.starts_with("shell:AppsFolder\\"))
            {
                self.roster.pin(id);
            }
            save_settings(&self.roster, &self.config);
            return self.refresh();
        };
        if previous == current {
            if pruned {
                save_settings(&self.roster, &self.config);
                return self.refresh();
            }
            return Ok(());
        }
        for id in current.iter().filter(|id| !previous.contains(id)) {
            self.roster.pin(id);
        }
        for id in previous.iter().filter(|id| !current.contains(id)) {
            self.roster.unpin(id);
        }
        save_settings(&self.roster, &self.config);
        self.refresh()
    }

    /// Unpins apps that were uninstalled, so no broken icon stays behind, as
    /// the Windows taskbar drops them. Returns whether anything changed.
    fn prune_missing_pins(&mut self) -> bool {
        let missing: Vec<String> = self
            .roster
            .pinned()
            .iter()
            .filter(|id| !app_exists(id))
            .cloned()
            .collect();
        for id in &missing {
            self.roster.unpin(id);
        }
        !missing.is_empty()
    }

    fn import_taskbar_pins(&mut self) -> Result<()> {
        for id in taskbar_pins() {
            self.roster.pin(&id);
        }
        save_settings(&self.roster, &self.config);
        self.refresh()
    }

    fn toggle_hiding(&mut self) -> Result<()> {
        self.config.auto_hide = !self.config.auto_hide;
        save_settings(&self.roster, &self.config);
        self.slide.target = if self.config.auto_hide { 1.0 } else { 0.0 };
        self.last_frame = Instant::now();
        self.refresh()
    }

    fn set_bin_full(&mut self, full: bool) -> Result<()> {
        if full == self.bin_full {
            return Ok(());
        }
        self.bin_full = full;
        self.assets.remove(recycle_bin::PARSING_NAME);
        self.refresh()
    }

    /// `until: None` is the bounce of an app being opened, which the user can
    /// turn off; a request for attention always bounces.
    fn start_bounce(&mut self, id: String, until: Option<f32>) {
        if until.is_none() && !self.config.bounce_on_open {
            return;
        }
        self.bounces.entry(id).or_insert(Bounce {
            start: Instant::now(),
            until,
        });
        self.last_frame = Instant::now();
    }

    fn on_attention(&mut self, hwnd: isize) {
        if hwnd == tracker::foreground_window() {
            return;
        }
        if let Some(window) = self.windows.iter().find(|window| window.hwnd == hwnd) {
            self.start_bounce(
                window.app_id.clone(),
                Some(ATTENTION_HOPS * bounce::HOP_SECONDS),
            );
        }
    }

    /// Refreshes open windows and recomposes the dock if anything changed.
    fn sync(&mut self) -> Result<()> {
        self.windows = self.tracker.list();
        let windows = &self.windows;
        self.last_rects
            .retain(|hwnd, _| windows.iter().any(|window| window.hwnd == *hwnd));
        let tracked: Vec<isize> = self.windows.iter().map(|window| window.hwnd).collect();
        for hwnd in tracked {
            self.remember_rect(hwnd);
        }
        self.calm_windows();
        let recent_before = self.roster.recent().to_vec();
        let changed = self
            .roster
            .sync(self.windows.iter().map(|window| window.app_id.as_str()));
        if self.roster.recent() != recent_before.as_slice() {
            save_settings(&self.roster, &self.config);
        }

        // The launching app has opened a window, so finish its current bounce.
        let now = Instant::now();
        for (id, bounce) in &mut self.bounces {
            if bounce.until.is_none() && self.roster.is_running(id) {
                let elapsed = now.duration_since(bounce.start).as_secs_f32();
                bounce.until = Some(bounce::end_of_hop(elapsed));
            }
        }

        let minimized_before = self.minimized.clone();
        let now_minimized: Vec<isize> = self
            .windows
            .iter()
            .filter(|window| window.minimized)
            .map(|window| window.hwnd)
            .collect();
        // A window still flying in was added at minimize start and may not
        // report as minimized yet.
        let flights = &self.flights;
        self.minimized.retain(|hwnd| {
            now_minimized.contains(hwnd) || flights.iter().any(|flight| flight.hwnd == *hwnd)
        });
        for hwnd in now_minimized {
            if !self.minimized.contains(&hwnd) {
                self.minimized.push(hwnd);
            }
        }
        self.sync_thumbnails();
        let minimized = &self.minimized;
        self.snapshots.retain(|(hwnd, _)| minimized.contains(hwnd));

        if changed || self.minimized != minimized_before {
            self.refresh()?;
        }
        let newly_minimized: Vec<isize> = self
            .minimized
            .iter()
            .copied()
            .filter(|hwnd| !minimized_before.contains(hwnd))
            .collect();
        if self.started && !self.fullscreen {
            self.start_flights(&newly_minimized);
        }
        self.started = true;
        // A foreground-window change may mean an app entered or exited full-screen mode.
        self.update_fullscreen()
    }

    fn tick(&mut self) -> Result<Option<Outside>> {
        let now = Instant::now();
        let flying = self.flights.len();
        let gfx = &self.gfx;
        let snapshots = &mut self.snapshots;
        let mut restore = None;
        self.flights.retain_mut(|flight| {
            let going = flight.step(gfx, now);
            if flight.take_restore(!going) {
                restore = Some(flight.hwnd);
            }
            if !going && let Some(snapshot) = flight.take_snapshot_for_restore() {
                if snapshots.len() >= MAX_SNAPSHOTS {
                    snapshots.remove(0);
                }
                snapshots.push((flight.hwnd, snapshot));
            }
            going
        });
        if self.flights.len() != flying {
            // Confirm the windows really are minimized, or restored.
            // SAFETY: Timer on our own window.
            unsafe { SetTimer(Some(self.input), SYNC_TIMER, SYNC_DELAY_MS, None) };
        }
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.amount.step(dt);
        self.slide.step(dt);
        self.last_frame = now;

        self.bounces.retain(|_, bounce| {
            let elapsed = now.duration_since(bounce.start).as_secs_f32();
            let until = bounce.until.unwrap_or(LAUNCH_BOUNCE_LIMIT);
            elapsed < until
        });
        self.render()?;
        Ok(restore.map(|hwnd| Outside::Activate(vec![hwnd])))
    }

    /// Hides the taskbar and reserves its space again. After Explorer restarts,
    /// both the taskbar and AppBar registrations are new.
    fn retake_taskbar(&mut self) -> Result<()> {
        taskbar::take_over();
        self.appbar = None;
        self.appbar = AppBar::register(self.visual, APPBAR_CALLBACK);
        // The menu bar's reservation is gone as well: rebuild it.
        self.menubar = None;
        self.apply_menu_bar_setting();
        self.stage = None;
        self.refresh()
    }

    /// Hides the dock while the foreground window covers the entire screen and
    /// shows it again when it no longer does.
    /// Creates or removes the menu bar to match the settings.
    fn apply_menu_bar_setting(&mut self) {
        if !self.config.menu_bar {
            self.menubar = None;
        } else if self.menubar.is_none() {
            self.menubar = MenuBar::create(&self.gfx, self.config.ai_usage)
                .inspect_err(|error| eprintln!("docky: menu bar: {error:#}"))
                .ok();
        }
        if let Some(bar) = &mut self.menubar {
            let applied = bar
                .set_auto_hide(self.config.menu_bar_auto_hide)
                .and_then(|()| bar.set_usage_enabled(&self.gfx, self.config.ai_usage))
                .and_then(|()| bar.set_visible(!self.fullscreen));
            if let Err(error) = applied {
                eprintln!("docky: menu bar: {error:#}");
            }
        }
    }

    /// Shows the name of the app in front, as the macOS menu bar does.
    fn update_menu_bar_app(&mut self) -> Result<()> {
        if self.menubar.is_none() {
            return Ok(());
        }
        let front = tracker::foreground_window();
        let id = self
            .windows
            .iter()
            .find(|window| window.hwnd == front)
            .map_or_else(explorer_id, |window| window.app_id.clone());
        if !self.names.contains_key(&id) {
            let name = self
                .icons
                .get(&id, "")
                .map_or_else(|| fallback_name(&id), |visual| visual.name.clone());
            self.names.insert(id.clone(), name);
        }
        let name = self.names.get(&id).map_or("", String::as_str);
        match &mut self.menubar {
            Some(bar) => bar.set_app_name(&self.gfx, name),
            None => Ok(()),
        }
    }

    /// Notes where `hwnd` is on screen. A maximized window forgets it: its
    /// rectangle then comes from the work area, and a stale one from before
    /// could be taken after it is un-maximized.
    fn remember_rect(&mut self, hwnd: isize) {
        match tracker::screen_rect(hwnd) {
            Some((rect, false)) => {
                self.last_rects.insert(hwnd, rect);
            }
            Some((_, true)) => {
                self.last_rects.remove(&hwnd);
            }
            None => {}
        }
    }

    fn update_fullscreen(&mut self) -> Result<()> {
        // The window in front is where the user works: note where it is now.
        let front = tracker::foreground_window();
        if self.windows.iter().any(|window| window.hwnd == front) {
            self.remember_rect(front);
        }
        let Some(monitor) = primary_monitor() else {
            return Ok(());
        };
        let fullscreen = tracker::foreground_is_fullscreen(monitor.screen);
        // Windows sometimes omits the shell event when a full-screen app
        // closes or switches desktops. Poll only while our UI is hidden.
        // SAFETY: `self.input` is a live window on this UI thread.
        unsafe {
            if fullscreen {
                SetTimer(
                    Some(self.input),
                    FULLSCREEN_TIMER,
                    FULLSCREEN_RECHECK_MS,
                    None,
                );
            }
        }
        if fullscreen != self.fullscreen {
            if let Some(bar) = &mut self.menubar {
                bar.set_visible(!fullscreen)?;
            }
            let previous = self.fullscreen;
            self.fullscreen = fullscreen;
            if fullscreen {
                self.hovering = false;
                self.amount = Spring::at(0.0, HOVER_OMEGA);
                self.press = None;
                self.drag = None;
            }
            if let Err(error) = self.place_windows() {
                self.fullscreen = previous;
                return Err(error);
            }
        }
        if !fullscreen {
            // SAFETY: This UI thread owns the timer.
            unsafe {
                let _ = KillTimer(Some(self.input), FULLSCREEN_TIMER);
            }
        }
        // Updating an app name can fail transiently; it must never prevent
        // the dock or bar from returning after full-screen mode.
        if let Err(error) = self.update_menu_bar_app() {
            eprintln!("docky: menu bar name: {error:#}");
        }
        Ok(())
    }

    fn on_appbar_notice(&mut self, notice: usize) -> Result<()> {
        match notice {
            // Windows notifications may be missing or unreliable, so use them
            // only as a signal to recheck the state.
            ABN_FULLSCREENAPP => self.update_fullscreen(),
            // The dock's own reservation can trigger this. The strip changes
            // only with the screen or dock size, already handled by `refresh`.
            _ => Ok(()),
        }
    }

    fn is_animating(&self) -> bool {
        !self.amount.is_settled()
            || !self.slide.is_settled()
            || !self.bounces.is_empty()
            || !self.flights.is_empty()
    }
}

/// Runs `action` on the app and logs failures; a transient failure must not
/// terminate the dock.
fn with_app(what: &str, action: impl FnOnce(&mut App) -> Result<()>) {
    with_app_then(what, |app| action(app).map(|()| None));
}

/// Like `with_app`, but `action` can request work to perform after releasing
/// the app state (see [`Outside`]).
fn with_app_then(what: &str, action: impl FnOnce(&mut App) -> Result<Option<Outside>>) {
    let outside = APP.with(|cell| {
        // Modal calls run outside the borrow, so this should not happen. If it
        // does, discard the message rather than terminating the dock.
        let Ok(mut app) = cell.try_borrow_mut() else {
            eprintln!("docky: {what}: el dock estaba ocupado; se ignora");
            return None;
        };
        match app.as_mut().map(action) {
            Some(Ok(outside)) => outside,
            Some(Err(error)) => {
                eprintln!("docky: {what}: {error:#}");
                None
            }
            None => None,
        }
    });
    if let Some(outside) = outside {
        run_outside(outside);
    }
}

unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if object != OBJID_WINDOW.0 {
        return;
    }
    if event == EVENT_SYSTEM_MINIMIZESTART {
        with_app("minimizar", |app| app.on_minimize_start(hwnd.0 as isize));
    } else if event == EVENT_SYSTEM_MINIMIZEEND {
        // Restored from outside the dock (Alt+Tab, the app itself): no shell
        // hook reports it, and a window the dock still thought minimized kept
        // its thumbnail and skipped the genie on its next minimize.
        with_app("restaurar", App::sync);
    } else if event == EVENT_SYSTEM_FOREGROUND {
        if tracker::is_task_switcher(hwnd) {
            with_app("Alt+Tab", App::task_switcher_opened);
        } else if !tracker::is_switch_transient(hwnd) {
            LAST_FRONT.store(hwnd.0 as isize, Ordering::Relaxed);
        }
    } else if event == EVENT_OBJECT_CLOAKED || event == EVENT_OBJECT_UNCLOAKED {
        // Shows the windows of the virtual desktop in front, as the Windows
        // taskbar does. A desktop switch cloaks many windows at once; the
        // timer turns the burst into one re-read.
        // SAFETY: A timer on our own window.
        unsafe {
            SetTimer(Some(main_window()), SYNC_TIMER, SYNC_DELAY_MS, None);
        }
    }
}

/// Like `with_app`, for callers that need a value back (OLE callbacks).
fn with_app_value<T: Default>(what: &str, action: impl FnOnce(&mut App) -> Result<T>) -> T {
    let mut value = T::default();
    with_app(what, |app| {
        value = action(app)?;
        Ok(())
    });
    value
}

pub fn menu_bar_clicked(x: f32, y: f32) {
    with_app("menu bar", |app| {
        if let Some(bar) = &mut app.menubar {
            bar.click(&app.gfx, x, y)?;
        }
        Ok(())
    });
}

/// The pointer rested on the top edge with the menu bar hidden.
pub fn menu_bar_edge_rested() {
    with_app("menu bar", |app| match &mut app.menubar {
        Some(bar) => bar.edge_rested(&app.gfx),
        None => Ok(()),
    });
}

pub fn menu_bar_hover(inside: bool) {
    with_app("menu bar", |app| match &mut app.menubar {
        Some(bar) => bar.hover(&app.gfx, inside),
        None => Ok(()),
    });
}

pub fn menu_bar_pointer_moved(x: f32) {
    with_app("menu bar", |app| match &mut app.menubar {
        Some(bar) => bar.pointer_moved(&app.gfx, x),
        None => Ok(()),
    });
}

pub fn menu_bar_dismiss_usage() {
    with_app("menu bar", |app| match &mut app.menubar {
        Some(bar) => bar.dismiss_usage(&app.gfx),
        None => Ok(()),
    });
}

/// Redraws the menu bar from what it reads on each draw (input language,
/// clock), without asking the network or audio again.
pub fn menu_bar_refresh() {
    with_app("menu bar", |app| match &mut app.menubar {
        Some(bar) => bar.refresh(&app.gfx),
        None => Ok(()),
    });
}

pub fn menu_bar_privacy_changed() {
    with_app("menu bar", |app| match &mut app.menubar {
        Some(bar) => bar.privacy_changed(&app.gfx),
        None => Ok(()),
    });
}

pub fn menu_bar_tick() {
    with_app("menu bar", |app| match &mut app.menubar {
        Some(bar) => bar.tick(&app.gfx),
        None => Ok(()),
    });
}

pub fn drag_enter(files: Vec<String>, x: i32, _y: i32) -> DROPEFFECT {
    with_app_value("file drag", |app| app.on_drag_enter(files, x))
}

pub fn drag_over(x: i32, _y: i32) -> DROPEFFECT {
    with_app_value("file drag", |app| app.on_drag_over(x))
}

pub fn drag_leave() {
    with_app("file drag", App::on_drag_leave);
}

pub fn drop_files(x: i32, _y: i32) -> DROPEFFECT {
    with_app_value("file drop", |app| app.on_drop(x))
}

/// Checks the recycle bin on a background thread by scanning each drive
/// (measured at 50–75 ms), so the UI is not blocked. The result arrives via
/// `BIN_STATE`.
fn query_bin_in_background() {
    if !BIN_QUERY.request() {
        return;
    }
    let window = MAIN_WINDOW.load(Ordering::Relaxed);
    let spawned = thread::Builder::new()
        .name("docky-recycle-query".into())
        .spawn(move || {
            power::set_thread_efficiency(true);
            // SAFETY: COM is initialized and released on this thread; the message
            // is harmless if the window no longer exists.
            unsafe {
                let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                loop {
                    BIN_QUERY.begin_pass();
                    let full = recycle_bin::has_items();
                    let _ = PostMessageW(
                        Some(HWND(window as *mut _)),
                        BIN_STATE,
                        WPARAM(usize::from(full)),
                        LPARAM(0),
                    );
                    if !BIN_QUERY.needs_another_pass() {
                        break;
                    }
                }
                if com.is_ok() {
                    CoUninitialize();
                }
            }
        });
    if let Err(error) = spawned {
        BIN_QUERY.0.store(0, Ordering::Release);
        eprintln!("docky: could not query the Recycle Bin: {error}");
    }
}

fn main_window() -> HWND {
    HWND(MAIN_WINDOW.load(Ordering::Relaxed) as *mut _)
}

fn run_outside(outside: Outside) {
    let submit = |action| {
        let result = shell_work::dispatch(action);
        if let Err(error) = result {
            eprintln!("docky: operation did not start: {error:#}");
            shell_work::notify_not_started();
        }
    };
    match outside {
        Outside::Open(id) => submit(ShellAction::Open { id }),
        Outside::OpenStart => launcher::open_start_menu(),
        Outside::Activate(windows) => launcher::activate(&windows),
        Outside::Minimize(windows) => launcher::minimize(&windows),
        Outside::Reveal(path) => submit(ShellAction::Reveal { path }),
        Outside::OpenWith(app, files) => submit(ShellAction::OpenWith { app, files }),
        Outside::Recycle(files) => submit(ShellAction::Recycle { files }),
        Outside::Transfer {
            folder,
            files: paths,
            copy,
        } => submit(ShellAction::Transfer {
            files: paths,
            folder,
            copy,
        }),
        Outside::EmptyBin => submit(ShellAction::EmptyBin),
        Outside::QuitDock => request_quit(),
        Outside::OpenAsAdmin(id) => submit(ShellAction::OpenAsAdmin { id }),
        Outside::ShowDesktop => launcher::show_desktop(),
        Outside::UnpinFromTaskbar(id) => submit(ShellAction::Unpin { id }),
        Outside::OpenSettings => {
            // The settings app is installed next to the dock.
            let settings = env::current_exe()
                .ok()
                .map(|exe| exe.with_file_name("docky-settings.exe"));
            match settings {
                Some(path) if path.exists() => submit(ShellAction::Open {
                    id: path.to_string_lossy().into_owned(),
                }),
                _ => eprintln!("docky: no docky-settings.exe found next to the dock"),
            }
        }
        Outside::Menu(request) => {
            let owner = HWND(MAIN_WINDOW.load(Ordering::Relaxed) as *mut _);
            let chosen = menu::show(
                &request.gfx,
                owner,
                &request.items,
                request.anchor,
                request.dark,
            );
            with_app("menu closed", App::after_menu);
            if let Some(command) = chosen {
                with_app_then("menu command", |app| {
                    app.on_command(request.target, command)
                });
            }
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        ASSETS_READY => {
            // SAFETY: Coalesce background results on this UI window.
            unsafe { SetTimer(Some(hwnd), ASSETS_TIMER, ASSETS_DELAY_MS, None) };
        }
        WM_TIMER if w.0 == ASSETS_TIMER => {
            // SAFETY: This UI thread owns the one-shot timer.
            unsafe {
                let _ = KillTimer(Some(hwnd), ASSETS_TIMER);
            }
            with_app("shell icons", App::assets_ready);
        }
        WM_MOUSEMOVE => with_app("mouse move", App::on_mouse_move),
        WM_MOUSELEAVE => with_app("mouse leave", App::on_mouse_leave),
        WM_LBUTTONDOWN => with_app("press", App::on_press),
        WM_LBUTTONUP => with_app_then("clic", App::on_release),
        WM_MBUTTONUP => with_app_then("clic central", App::on_middle_click),
        WM_CAPTURECHANGED => {
            // SAFETY: Posting a message to our own window.
            unsafe {
                let _ = PostMessageW(Some(hwnd), DRAG_CANCELLED, WPARAM(0), LPARAM(0));
            }
        }
        DRAG_CANCELLED => with_app("drag", App::cancel_drag),
        DROP_RUN => with_app_then("file drop", |app| Ok(app.pending_drop.take())),
        WM_RBUTTONUP => with_app_then("menu", App::on_right_click),
        BIN_STATE => with_app("recycle bin", |app| app.set_bin_full(w.0 != 0)),
        BIN_CHANGED => {
            // SAFETY: `hwnd` is this live window on its owning UI thread.
            unsafe { SetTimer(Some(hwnd), BIN_TIMER, BIN_DELAY_MS, None) };
        }
        WM_SETTINGCHANGE if w.0 == SPI_SETWORKAREA => {}
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => {
            // SAFETY: `hwnd` is our window; scheduling the same timer resets it.
            unsafe { SetTimer(Some(hwnd), REFRESH_TIMER, REFRESH_DELAY_MS, None) };
        }
        // On battery or charging, or energy saver turned on or off: glass may
        // have to become solid, or the other way round.
        WM_POWERBROADCAST if w.0 == PBT_APMPOWERSTATUSCHANGE || w.0 == PBT_POWERSETTINGCHANGE => {
            // SAFETY: `hwnd` is our window; scheduling the same timer resets it.
            unsafe { SetTimer(Some(hwnd), REFRESH_TIMER, REFRESH_DELAY_MS, None) };
        }
        WM_TIMER if w.0 == BIN_TIMER => {
            // SAFETY: `hwnd` is this live window on its owning UI thread.
            unsafe {
                let _ = KillTimer(Some(hwnd), w.0);
            }
            query_bin_in_background();
        }
        PINS_CHANGED => {
            with_app("apps fijadas", |app| {
                if let Some(watch) = &mut app.pin_watch {
                    watch.arm();
                }
                Ok(())
            });
            // SAFETY: `hwnd` is our window; scheduling the same timer resets it.
            unsafe { SetTimer(Some(hwnd), PINS_TIMER, PINS_DELAY_MS, None) };
        }
        WM_TIMER if w.0 == PINS_TIMER => {
            // SAFETY: `hwnd` is this live window on its owning UI thread.
            unsafe {
                let _ = KillTimer(Some(hwnd), w.0);
            }
            with_app("apps fijadas", App::sync_taskbar_pins);
        }
        WM_TIMER if w.0 == UPDATE_TIMER => {
            with_app("update", |app| {
                app.update_now();
                Ok(())
            });
        }
        WM_TIMER if w.0 == SWITCHER_TIMER => {
            with_app("Alt+Tab", App::check_task_switcher);
        }
        WM_TIMER if w.0 == FULLSCREEN_TIMER => {
            with_app("full screen", App::update_fullscreen);
        }
        WM_TIMER if w.0 == SPRING_TIMER => {
            with_app_then("drag over an app", App::spring_open);
        }
        WM_TIMER if w.0 == HIDE_TIMER => {
            // SAFETY: Same as above.
            unsafe {
                let _ = KillTimer(Some(hwnd), w.0);
            }
            with_app("auto-hide", App::hide_if_unused);
        }
        WM_TIMER if w.0 == TRIM_TIMER => {
            // SAFETY: `hwnd` is this live window on its owning UI thread.
            unsafe {
                let _ = KillTimer(Some(hwnd), w.0);
            }
            with_app("recorte de memoria", |app| {
                app.trim_if_idle();
                Ok(())
            });
        }
        WM_TIMER if [REFRESH_TIMER, SYNC_TIMER, TASKBAR_TIMER].contains(&w.0) => {
            // SAFETY: `hwnd` is this live window on its owning UI thread.
            unsafe {
                let _ = KillTimer(Some(hwnd), w.0);
            }
            match w.0 {
                REFRESH_TIMER => with_app("dock recomposition", App::refresh),
                SYNC_TIMER => with_app("window read", App::sync),
                _ => with_app("hide the Windows taskbar", App::retake_taskbar),
            }
        }
        APPBAR_CALLBACK => with_app("AppBar notice", |app| app.on_appbar_notice(w.0)),
        // Once the taskbar is set to auto-hide, the dock strip moves to the
        // screen edge, so reserve and reposition it again.
        TASKBAR_TAKEN => with_app("reserva de espacio", |app| {
            app.stage = None;
            app.refresh()
        }),
        WM_POWERBROADCAST if w.0 == PBT_APMRESUMEAUTOMATIC => {
            // SAFETY: `hwnd` is this live window on its owning UI thread.
            unsafe { SetTimer(Some(hwnd), TASKBAR_TIMER, TASKBAR_DELAY_MS, None) };
            // Timers do not count the time asleep: aim it again by the clock.
            with_app("update", |app| {
                app.schedule_update();
                Ok(())
            });
        }
        _ if CONFIG_CHANGED.get() == Some(&msg) => with_app("settings", App::reload_config),
        _ if TASKBAR_CREATED.get() == Some(&msg) => {
            // SAFETY: `hwnd` is this live window on its owning UI thread.
            unsafe { SetTimer(Some(hwnd), TASKBAR_TIMER, TASKBAR_DELAY_MS, None) };
        }
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ if SHELL_HOOK.get() == Some(&msg) => match w.0 {
            // Changing the foreground window does not change the app list; it
            // can only enter or exit full-screen mode.
            HSHELL_WINDOWACTIVATED | HSHELL_RUDEAPPACTIVATED => {
                with_app("active window", App::update_fullscreen);
            }
            // Terminals and browsers retitle their windows many times a minute.
            // The dock shows no titles, so only a window it does not list yet
            // (an app that declares its identity late) needs a new look.
            HSHELL_REDRAW if is_tracked(l.0) => {}
            // The input language indicator of the menu bar.
            HSHELL_LANGUAGE => menu_bar_refresh(),
            code => {
                if code == HSHELL_FLASH {
                    with_app("attention notice", |app| {
                        app.on_attention(l.0);
                        Ok(())
                    });
                }
                // SAFETY: `hwnd` is this live window on its owning UI thread.
                unsafe { SetTimer(Some(hwnd), SYNC_TIMER, SYNC_DELAY_MS, None) };
            }
        },
        _ => return unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
    LRESULT(0)
}

/// `true` if the dock already lists the window `hwnd`.
fn is_tracked(hwnd: isize) -> bool {
    APP.with(|cell| {
        cell.try_borrow().is_ok_and(|app| {
            app.as_ref()
                .is_some_and(|app| app.windows.iter().any(|window| window.hwnd == hwnd))
        })
    })
}

fn is_animating() -> bool {
    APP.with_borrow(|app| app.as_ref().is_some_and(App::is_animating))
}

/// Main loop. Sleeps while idle (0% CPU) and advances one frame per display
/// refresh while an animation is running.
///
/// Outside animations, the thread uses EcoQoS because remaining work (window
/// tracking and recomposition) is not time-sensitive.
fn message_loop() {
    let mut msg = MSG::default();
    let mut was_animating = None;
    unsafe {
        loop {
            let animating = is_animating();
            if was_animating != Some(animating) {
                power::set_thread_efficiency(!animating);
                if was_animating == Some(true) {
                    with_app("recorte de memoria", |app| {
                        app.schedule_trim();
                        Ok(())
                    });
                }
                was_animating = Some(animating);
            }
            if animating {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        return;
                    }
                    DispatchMessageW(&msg);
                }
                with_app_then("animation", App::tick);
                // Wait for the next refresh to pace animation without timers.
                let _ = DwmFlush();
            } else {
                let status = GetMessageW(&mut msg, None, 0, 0);
                if status.0 <= 0 {
                    if status.0 < 0 {
                        eprintln!("docky: error reading the Windows message queue");
                    }
                    return;
                }
                DispatchMessageW(&msg);
            }
        }
    }
}

pub fn run() -> Result<()> {
    let gfx = Gfx::new()?;
    // SAFETY: The thread is already a single-threaded COM apartment (see
    // `main`); OLE adds drag and drop on top of it and is released at the end.
    unsafe { OleInitialize(None)? };

    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WINDOW_CLASS;
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: class,
            ..Default::default()
        });

        let base = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        let create = |extra: WINDOW_EX_STYLE| {
            CreateWindowExW(
                base | extra,
                class,
                w!("Docky"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
        };
        let visual = create(WS_EX_LAYERED | WS_EX_TRANSPARENT)?;
        let input = create(WINDOW_EX_STYLE::default())?;

        // Without this, the system backdrop brush appears black. Windows 10
        // does not know the attribute; the dock then draws solid glass
        // (`system::transparency_enabled`), so the error is not fatal.
        let enable = BOOL::from(true);
        let _ = DwmSetWindowAttribute(
            visual,
            DWMWA_USE_HOSTBACKDROPBRUSH,
            (&raw const enable).cast(),
            size_of::<BOOL>() as u32,
        );

        let target = gfx
            .compositor
            .cast::<ICompositorDesktopInterop>()?
            .CreateDesktopWindowTarget(visual, true)?;

        docky_config::migrate_legacy_folder();
        let config = load_config();
        let roster = roster_from(&config);
        let auto_hide = config.auto_hide;
        let mut app = App {
            visual,
            input,
            gfx,
            target,
            appbar: AppBar::register(visual, APPBAR_CALLBACK),
            fullscreen: false,
            roster,
            tracker: WindowTracker::default(),
            windows: Vec::new(),
            last_rects: HashMap::new(),
            entries: Vec::new(),
            icons: Visuals::new(visual, ASSETS_READY)?,
            downloads: downloads_folder(),
            bin_full: false,
            _bin_watch: BinWatch::register(input, BIN_CHANGED),
            _saver_watch: power::SaverWatch::new(input.0 as isize),
            pin_watch: docky_shell::pins::watch(visual.0 as isize, PINS_CHANGED),
            assets: HashMap::new(),
            assets_for: (0.0, false, 0),
            placeholder: None,
            remove_label: None,
            config,
            slide: Spring::at(if auto_hide { 1.0 } else { 0.0 }, SLIDE_OMEGA),
            press: None,
            drag: None,
            incoming: Vec::new(),
            drag_over: false,
            spring: None,
            switching: false,
            switch_started: Instant::now(),
            switch_ended: None,
            launched: Instant::now(),
            update_tried: None,
            follow: None,
            pointed: false,
            pending_drop: None,
            bounces: HashMap::new(),
            minimized: Vec::new(),
            thumbnails: Thumbnails::new(visual),
            flights: Vec::new(),
            snapshots: Vec::new(),
            menubar: None,
            names: HashMap::new(),
            calmed: Vec::new(),
            started: false,
            stage: None,
            hovering: false,
            hovered: None,
            cursor: 0.0,
            amount: Spring::at(0.0, HOVER_OMEGA),
            last_frame: Instant::now(),
        };
        let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));
        let _ = CONFIG_CHANGED.set(RegisterWindowMessageW(
            docky_shell::dock_link::CONFIG_CHANGED,
        ));
        MAIN_WINDOW.store(visual.0 as isize, Ordering::Relaxed);
        // The dock inherits the "ignore Ctrl+C" attribute from its parent
        // (some tools enable it), so clear it to ensure Ctrl+C is handled.
        let _ = SetConsoleCtrlHandler(None, false);
        let _ = SetConsoleCtrlHandler(Some(on_console_ctrl), true);

        // Apps uninstalled while Docky was closed leave the dock.
        if app.prune_missing_pins() {
            save_settings(&app.roster, &app.config);
        }
        // Pins and unpins made on the Windows taskbar while Docky was closed.
        if let Err(error) = app.sync_taskbar_pins() {
            eprintln!("docky: apps pinned to the Windows taskbar: {error:#}");
        }
        app.sync()?;
        // `sync` recomposes only when the list changes; run it unconditionally at startup.
        if app.stage.is_none() {
            app.refresh()?;
        }
        // After the dock: creating the bar and reserving its strip take
        // 60-130 ms, which the dock no longer waits for.
        app.apply_menu_bar_setting();
        // Otherwise the bar shows no app name until the user switches windows.
        if let Err(error) = app.update_menu_bar_app() {
            eprintln!("docky: menu bar: {error:#}");
        }

        // The shell notifies this window when a window is created, closed,
        // activated, or requests attention.
        let _ = SHELL_HOOK.set(RegisterWindowMessageW(w!("SHELLHOOK")));
        RegisterShellHookWindow(input).ok()?;
        // Delivered through this thread's message loop; no code runs in other processes.
        let minimize_hook = SetWinEventHook(
            EVENT_SYSTEM_MINIMIZESTART,
            EVENT_SYSTEM_MINIMIZEEND,
            None,
            Some(on_win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        MINIMIZE_HOOK.store(minimize_hook.0 as isize, Ordering::Relaxed);
        // Switching virtual desktops hides (cloaks) the windows of the one
        // left and shows those of the one entered, and no shell hook says so.
        let cloak_hook = SetWinEventHook(
            EVENT_OBJECT_CLOAKED,
            EVENT_OBJECT_UNCLOAKED,
            None,
            Some(on_win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        CLOAK_HOOK.store(cloak_hook.0 as isize, Ordering::Relaxed);
        // Alt+Tab: the switcher coming to the front. One event per window
        // switch, so it costs nothing while the user works in one app.
        let foreground_hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        FOREGROUND_HOOK.store(foreground_hook.0 as isize, Ordering::Relaxed);
        LAST_FRONT.store(tracker::foreground_window(), Ordering::Relaxed);
        let target: IDropTarget = DropTarget.into();
        RegisterDragDrop(input, &target)?;
        DROP_WINDOW.store(input.0 as isize, Ordering::Relaxed);

        app.schedule_update();
        APP.set(Some(app));
    }

    // Hiding the taskbar is a synchronous Explorer call that takes 50–500 ms
    // depending on load. Run it off the UI thread after the dock is visible.
    let visual_window = MAIN_WINDOW.load(Ordering::Relaxed);
    query_bin_in_background();
    let taking_over = thread::spawn(move || {
        taskbar::take_over();
        // SAFETY: Only a message is sent; if the window no longer exists, the call has no effect.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(visual_window as *mut _)),
                TASKBAR_TAKEN,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });

    message_loop();
    // SAFETY: The drop target was registered on this window in this thread.
    unsafe {
        let _ = RevokeDragDrop(HWND(DROP_WINDOW.load(Ordering::Relaxed) as *mut _));
        for hook in [&MINIMIZE_HOOK, &CLOAK_HOOK, &FOREGROUND_HOOK] {
            let _ = UnhookWinEvent(HWINEVENTHOOK(hook.load(Ordering::Relaxed) as *mut _));
        }
    }
    APP.with_borrow_mut(|app| {
        if let Some(app) = app {
            app.restore_transitions();
        }
    });
    APP.set(None);
    // SAFETY: Balances `OleInitialize` above, on the same thread.
    unsafe { OleUninitialize() };
    // Wait for taskbar takeover to finish before restoring it.
    let _ = taking_over.join();
    Ok(())
}

#[cfg(test)]
mod performance_tests {
    use super::BinQueryGate;

    #[test]
    fn recycle_notifications_share_one_scan_and_preserve_the_last_change() {
        let gate = BinQueryGate::new();
        assert!(gate.request());
        gate.begin_pass();
        for _ in 0..10_000 {
            assert!(!gate.request());
        }
        assert!(gate.needs_another_pass());
        gate.begin_pass();
        assert!(!gate.needs_another_pass());
        assert!(gate.request());
    }

    #[test]
    fn quiet_recycle_scan_allows_the_next_independent_scan() {
        let gate = BinQueryGate::new();
        assert!(gate.request());
        gate.begin_pass();
        assert!(!gate.needs_another_pass());
        assert!(gate.request());
    }
}

#[cfg(test)]
mod switcher_tests {
    use super::selection_after_closing;

    #[test]
    fn selection_after_closing_should_keep_its_place_in_the_list() {
        // Nothing closed.
        assert_eq!(selection_after_closing(1, &[true, true, true]), Some(1));
        // A window before the selection closes: the selection moves up one.
        assert_eq!(selection_after_closing(2, &[true, false, true]), Some(1));
        // A window after it closes: nothing moves.
        assert_eq!(selection_after_closing(0, &[true, true, false]), Some(0));
        // The selected window closes: the next one takes its place.
        assert_eq!(selection_after_closing(1, &[true, false, true]), Some(1));
        // The selected window was the last: the new last one.
        assert_eq!(selection_after_closing(2, &[true, true, false]), Some(1));
        // Everything closed.
        assert_eq!(selection_after_closing(0, &[false, false]), None);
    }
}
