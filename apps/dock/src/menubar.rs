//! Minimal macOS-style menu bar across the top of the screen.
//!
//! Hiding the Windows taskbar also hides its clock and status area; this bar
//! brings them back: the name of the app in front, the battery, a control
//! center button (opens the Windows quick settings: network, volume,
//! brightness) and the date and time (opens the notification center).
//!
//! Cost while idle: one timer that fires once a minute, on the minute, to
//! redraw the clock. Everything else is event-driven.
//!
//! With auto-hide on, the bar reserves no space and stays off screen; a thin
//! strip along the top edge reveals it while the pointer is there.
//!
//! Status items, right to left as on a Mac: clock, control center, search,
//! network, volume, battery, then Task View and Widgets. Search, Task View and
//! Widgets appear only if the user has them on the Windows taskbar. Network
//! and volume changes arrive as `STATUS_CHANGED`; nothing polls.

use anyhow::Result;
use docky_shell::{
    appbar::{AppBar, Edge},
    launcher,
    status::{self, AudioWatch, Network, NetworkWatch, Privacy, TaskbarButtons, Volume},
    system::{self, Battery, Monitor},
    watch::RegistryWatch,
};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use windows::{
    UI::{
        Color,
        Composition::{
            CompositionBrush, Compositor, ContainerVisual, Desktop::DesktopWindowTarget,
            SpriteVisual, VisualCollection,
        },
    },
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM},
        Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute},
        System::{
            LibraryLoader::GetModuleHandleW, SystemInformation::GetLocalTime,
            WinRT::Composition::ICompositorDesktopInterop,
        },
        UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
        UI::WindowsAndMessaging::{
            CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, HHOOK,
            HWND_TOPMOST, IDC_ARROW, IDC_HAND, KillTimer, LoadCursorW, MSLLHOOKSTRUCT,
            PostMessageW, RegisterClassW, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_SHOWWINDOW,
            SetCursor, SetTimer, SetWindowPos, SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL,
            WM_APP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_NCHITTEST,
            WM_RBUTTONDOWN, WM_SETCURSOR, WM_SETTINGCHANGE, WM_TIMECHANGE, WM_TIMER,
            WM_XBUTTONDOWN, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
    core::{BOOL, Interface, w},
};
use windows_numerics::{Vector2, Vector3};

use crate::{
    app,
    gfx::{Gfx, Ink, LabelBrush},
    strings,
    usage::{
        self, ActiveProviders, Monitor as UsageMonitor, Provider, ProviderUsage, Snapshot, State,
    },
};

// Logical pixels.
const HEIGHT: f32 = 28.0;
const SIDE_MARGIN: f32 = 14.0;
const ITEM_GAP: f32 = 16.0;
/// Extra clickable room around an item.
const HIT_SLOP: f32 = 7.0;

/// How long the pointer must rest on the top edge before a hidden bar shows.
/// Just passing through, on the way to a window's close button or a browser
/// tab, does not reveal it.
const REVEAL_DWELL_MS: u32 = 250;
const REVEAL_TIMER: usize = 2;
/// Posted by the edge hook when the pointer reaches or leaves the top edge.
const EDGE_ENTER: u32 = WM_APP + 26;
const EDGE_LEAVE: u32 = WM_APP + 27;

const CLOCK_TIMER: usize = 1;
const ISLAND_SINGLE_WIDTH: f32 = 145.0;
const ISLAND_EXPANDED_WIDTH: f32 = 440.0;
const ISLAND_ROW_HEIGHT: f32 = 118.0;
const ISLAND_CARD_HEIGHT: f32 = 108.0;
const WM_POWERBROADCAST: u32 = 0x0218;
const WM_MOUSELEAVE: u32 = 0x02A3;
/// AppBar notifications need a message id; the bar ignores them.
const APPBAR_CALLBACK: u32 = WM_APP + 20;
/// Posted by the network and audio watches.
const STATUS_CHANGED: u32 = WM_APP + 21;
/// Posted by the bar to itself to update from the message loop.
const TICK: u32 = WM_APP + 22;
const USAGE_CHANGED: u32 = WM_APP + 23;
const DISMISS_USAGE: u32 = WM_APP + 24;
/// Posted when an app starts or stops using the microphone or camera.
const PRIVACY_CHANGED: u32 = WM_APP + 25;
/// `wParam` of WM_SETTINGCHANGE when the work area changes, which every
/// AppBar reservation (the dock's, this bar's) causes.
const SPI_SETWORKAREA: usize = 0x002F;
/// A `TICK` is already posted.
static TICK_PENDING: AtomicBool = AtomicBool::new(false);
static ISLAND_LEFT: AtomicI32 = AtomicI32::new(0);
static ISLAND_RIGHT: AtomicI32 = AtomicI32::new(0);
static BAR_TOP: AtomicI32 = AtomicI32::new(0);
static BAR_BOTTOM: AtomicI32 = AtomicI32::new(0);
static HEADER_BOTTOM: AtomicI32 = AtomicI32::new(0);
static PANEL_BOTTOM: AtomicI32 = AtomicI32::new(0);
/// The hook exists only while the usage panel is open.
static USAGE_HOOK_WINDOW: AtomicIsize = AtomicIsize::new(0);
/// The hidden bar's window, for the edge hook; 0 while no hook is installed.
static EDGE_HOOK_WINDOW: AtomicIsize = AtomicIsize::new(0);
/// The row of the top edge and its span, in screen pixels.
static EDGE_TOP: AtomicI32 = AtomicI32::new(0);
static EDGE_LEFT: AtomicI32 = AtomicI32::new(0);
static EDGE_RIGHT: AtomicI32 = AtomicI32::new(0);
/// The pointer is on the edge and `EDGE_ENTER` was posted.
static EDGE_ARMED: AtomicBool = AtomicBool::new(false);

/// Segoe Fluent Icons code points.
mod glyph {
    pub const WIFI: [char; 4] = ['\u{E872}', '\u{E873}', '\u{E874}', '\u{E701}'];
    pub const WIFI_FULL: char = '\u{E701}';
    pub const ETHERNET: char = '\u{E839}';
    pub const OFFLINE: char = '\u{F384}';
    pub const VOLUME: [char; 4] = ['\u{E992}', '\u{E993}', '\u{E994}', '\u{E995}'];
    pub const MUTE: char = '\u{E74F}';
    pub const SEARCH: char = '\u{E721}';
    pub const TASK_VIEW: char = '\u{E7C4}';
    pub const WIDGETS: char = '\u{F246}';
    /// The taskbar's "show hidden icons" arrow.
    pub const HIDDEN_ICONS: char = '\u{E70E}';
    pub const MICROPHONE: char = '\u{E720}';
    pub const CAMERA: char = '\u{E722}';
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    QuickSettings,
    Notifications,
    Sound,
    Search,
    TaskView,
    Widgets,
    HiddenIcons,
    InputLanguage,
    Microphone,
    Camera,
}

/// Network, volume and taskbar buttons, read only when Windows reports a change.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Status {
    network: Network,
    volume: Option<Volume>,
    buttons: TaskbarButtons,
    privacy: Privacy,
}

/// What the bar was last drawn for.
#[derive(Clone, PartialEq)]
struct Look {
    screen: RECT,
    scale: f32,
    light: bool,
    /// See `system::transparency_enabled`.
    glass: bool,
    app_name: String,
    clock: String,
    /// The input language indicator, when more than one is installed.
    input: Option<String>,
    battery: Option<Battery>,
    status: Status,
    usage: Snapshot,
    usage_enabled: bool,
    active_providers: ActiveProviders,
    expanded: bool,
    usage_refreshing: bool,
}

pub struct MenuBar {
    hwnd: HWND,
    target: DesktopWindowTarget,
    appbar: Option<AppBar>,
    app_name: String,
    look: Option<Look>,
    /// Clickable spans, in window pixels.
    zones: Vec<(f32, f32, Action)>,
    visible: bool,
    auto_hide: bool,
    /// With auto-hide on: the pointer is at the top edge, so the bar shows.
    revealed: bool,
    /// Current visual tree and where the full bar goes on screen.
    root: Option<ContainerVisual>,
    bounds: RECT,
    status: Status,
    usage: Option<UsageMonitor>,
    expanded: bool,
    visual_height: i32,
    island_left: f32,
    island_right: f32,
    refresh_button: Option<(f32, f32, f32, f32)>,
    outside_click_hook: Option<HHOOK>,
    /// Watches the top edge while the bar auto-hides.
    edge_watch: Option<EdgeWatch>,
    /// Held for their callbacks; dropping them unsubscribes.
    _network: Option<NetworkWatch>,
    /// Hears when apps start or stop using the microphone or camera.
    privacy_watch: Option<RegistryWatch>,
    /// Energy saver on or off arrives as `WM_POWERBROADCAST`; glass follows.
    _saver_watch: docky_shell::power::SaverWatch,
    audio: Option<AudioWatch>,
}

unsafe extern "system" fn bar_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => launcher::note_menu_bar_press(),
        WM_LBUTTONUP => {
            let x = (l.0 & 0xFFFF) as i16;
            let y = (l.0 >> 16) as i16;
            app::menu_bar_clicked(f32::from(x), f32::from(y));
        }
        WM_MOUSEMOVE => {
            let x = (l.0 & 0xFFFF) as i16;
            app::menu_bar_pointer_moved(f32::from(x));
        }
        WM_MOUSELEAVE => app::menu_bar_hover(false),
        DISMISS_USAGE => app::menu_bar_dismiss_usage(),
        PRIVACY_CHANGED => app::menu_bar_privacy_changed(),
        WM_SETCURSOR => {
            let mut point = POINT::default();
            // SAFETY: The point is a valid output for the current pointer location.
            if unsafe { GetCursorPos(&mut point) }.is_ok()
                && point.x >= ISLAND_LEFT.load(Ordering::Relaxed)
                && point.x < ISLAND_RIGHT.load(Ordering::Relaxed)
                && point.y >= BAR_TOP.load(Ordering::Relaxed)
                && point.y < HEADER_BOTTOM.load(Ordering::Relaxed)
            {
                // SAFETY: The stock hand cursor is owned by Windows.
                if let Ok(hand) = unsafe { LoadCursorW(None, IDC_HAND) } {
                    unsafe { SetCursor(Some(hand)) };
                    return LRESULT(1);
                }
            }
            return unsafe { DefWindowProcW(hwnd, msg, w, l) };
        }
        WM_NCHITTEST => {
            let x = (l.0 & 0xFFFF) as i16 as i32;
            let y = (l.0 >> 16) as i16 as i32;
            if y >= BAR_BOTTOM.load(Ordering::Relaxed)
                && y < PANEL_BOTTOM.load(Ordering::Relaxed)
                && (x < ISLAND_LEFT.load(Ordering::Relaxed)
                    || x >= ISLAND_RIGHT.load(Ordering::Relaxed))
            {
                return LRESULT(-1);
            }
            // SAFETY: Default hit testing for the bar and the island.
            return unsafe { DefWindowProcW(hwnd, msg, w, l) };
        }
        // The minute changed, the charger was plugged or unplugged, the
        // network or volume changed, or a Windows setting (taskbar buttons).
        // Nothing the bar shows depends on the work area.
        WM_SETTINGCHANGE if w.0 == SPI_SETWORKAREA => {}
        EDGE_ENTER => {
            // SAFETY: A timer on our own window; setting it again restarts it.
            unsafe {
                SetTimer(Some(hwnd), REVEAL_TIMER, REVEAL_DWELL_MS, None);
            }
        }
        EDGE_LEAVE => {
            // SAFETY: Our own timer.
            unsafe {
                let _ = KillTimer(Some(hwnd), REVEAL_TIMER);
            }
        }
        WM_TIMER if w.0 == REVEAL_TIMER => app::menu_bar_edge_rested(),
        // Some of these are sent while the dock is busy reserving space, and
        // updating then was dropped. From the message loop the dock is never
        // busy, and a burst of them makes a single update.
        // WM_TIMECHANGE: the clock or time zone was changed.
        WM_TIMER | WM_POWERBROADCAST | WM_TIMECHANGE | STATUS_CHANGED | USAGE_CHANGED
        | WM_SETTINGCHANGE => {
            if !TICK_PENDING.swap(true, Ordering::Relaxed) {
                // SAFETY: Posting to our own live window.
                unsafe {
                    let _ = PostMessageW(Some(hwnd), TICK, WPARAM(0), LPARAM(0));
                }
            }
        }
        TICK => {
            TICK_PENDING.store(false, Ordering::Relaxed);
            app::menu_bar_tick();
        }
        // SAFETY: Standard forwarding of unhandled messages.
        _ => return unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
    LRESULT(0)
}

/// The thread that runs `edge_proc`. A low-level mouse hook runs on the
/// thread that installed it, and every pointer movement in the system waits
/// for it: on the dock's thread, a busy moment (loading icons, a Genie
/// flight) would make the whole mouse stutter, and Windows silently removes
/// hooks that answer late. This thread does nothing else.
struct EdgeWatch {
    thread: u32,
    join: Option<std::thread::JoinHandle<()>>,
}

impl EdgeWatch {
    fn start() -> Result<Self> {
        use windows::Win32::{
            System::Threading::{
                GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
            },
            UI::WindowsAndMessaging::{GetMessageW, MSG, PM_NOREMOVE, PeekMessageW},
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let join = std::thread::Builder::new()
            .name("docky-edge".into())
            .spawn(move || {
                // SAFETY: The hook procedure lives for the whole process; the
                // hook is removed before the thread ends.
                unsafe {
                    let mut msg = MSG::default();
                    // Creates the message queue before anyone posts WM_QUIT.
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                    // Input waits for this thread: never let busy apps delay it.
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
                    let hook = GetModuleHandleW(None).and_then(|module| {
                        SetWindowsHookExW(WH_MOUSE_LL, Some(edge_proc), Some(module.into()), 0)
                    });
                    let started = hook
                        .as_ref()
                        .map(|_| GetCurrentThreadId())
                        .map_err(Clone::clone);
                    let _ = sender.send(started);
                    let Ok(hook) = hook else {
                        return;
                    };
                    // Hooks are called from inside GetMessage; nothing else arrives.
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
                    let _ = UnhookWindowsHookEx(hook);
                }
            })?;
        match receiver.recv() {
            Ok(Ok(thread)) => Ok(Self {
                thread,
                join: Some(join),
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error.into())
            }
            Err(_) => Err(anyhow::anyhow!("edge thread ended")),
        }
    }
}

impl Drop for EdgeWatch {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        // SAFETY: Posting to a thread this struct started and still owns.
        unsafe {
            let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Low-level mouse hook on the `EdgeWatch` thread: while the bar is hidden,
/// posts when the pointer reaches the top edge and when it leaves it, once
/// each. It never consumes input.
unsafe extern "system" fn edge_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && w.0 as u32 == WM_MOUSEMOVE {
        // SAFETY: Windows provides an MSLLHOOKSTRUCT for mouse hook actions.
        let point = unsafe { (*(l.0 as *const MSLLHOOKSTRUCT)).pt };
        // The exact row: with a monitor above, the pointer only passes through.
        let on_edge = point.y == EDGE_TOP.load(Ordering::Relaxed)
            && point.x >= EDGE_LEFT.load(Ordering::Relaxed)
            && point.x < EDGE_RIGHT.load(Ordering::Relaxed);
        let message = match (on_edge, EDGE_ARMED.load(Ordering::Relaxed)) {
            (true, false) => Some(EDGE_ENTER),
            (false, true) => Some(EDGE_LEAVE),
            _ => None,
        };
        let window = EDGE_HOOK_WINDOW.load(Ordering::Relaxed);
        if let Some(message) = message
            && window != 0
        {
            EDGE_ARMED.store(on_edge, Ordering::Relaxed);
            // SAFETY: The window remains live until the hook is removed.
            unsafe {
                let _ = PostMessageW(Some(HWND(window as *mut _)), message, WPARAM(0), LPARAM(0));
            }
        }
    }
    // SAFETY: Never consume input; the window under the pointer must get it.
    unsafe { CallNextHookEx(None, code, w, l) }
}

unsafe extern "system" fn outside_click_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0
        && matches!(
            w.0 as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    {
        // SAFETY: Windows provides an MSLLHOOKSTRUCT for mouse hook actions.
        let point = unsafe { (*(l.0 as *const MSLLHOOKSTRUCT)).pt };
        if point.x < ISLAND_LEFT.load(Ordering::Relaxed)
            || point.x >= ISLAND_RIGHT.load(Ordering::Relaxed)
            || point.y < BAR_TOP.load(Ordering::Relaxed)
            || point.y >= PANEL_BOTTOM.load(Ordering::Relaxed)
        {
            let window = USAGE_HOOK_WINDOW.load(Ordering::Relaxed);
            if window != 0 {
                // SAFETY: The window remains live until the hook is removed in Drop.
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(window as *mut _)),
                        DISMISS_USAGE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        }
    }
    // SAFETY: Never consume the click; the target application must receive it.
    unsafe { CallNextHookEx(None, code, w, l) }
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color {
        A: a,
        R: r,
        G: g,
        B: b,
    }
}

fn add_visual(
    compositor: &Compositor,
    layers: &VisualCollection,
    brush: &CompositionBrush,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Result<SpriteVisual> {
    let visual = compositor.CreateSpriteVisual()?;
    visual.SetSize(Vector2 {
        X: width,
        Y: height,
    })?;
    visual.SetOffset(Vector3 {
        X: x.round(),
        Y: y.round(),
        Z: 0.0,
    })?;
    visual.SetBrush(brush)?;
    layers.InsertAtTop(&visual)?;
    Ok(visual)
}

#[expect(
    clippy::too_many_arguments,
    reason = "A rounded shape needs its rectangle, radius and two colors"
)]
fn add_rounded_box(
    compositor: &Compositor,
    layers: &VisualCollection,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    fill: Color,
    border: Option<Color>,
) -> Result<()> {
    let geometry = compositor.CreateRoundedRectangleGeometry()?;
    let inset = if border.is_some() { 0.5 } else { 0.0 };
    geometry.SetOffset(Vector2 { X: inset, Y: inset })?;
    geometry.SetSize(Vector2 {
        X: (width - inset * 2.0).max(0.0),
        Y: (height - inset * 2.0).max(0.0),
    })?;
    geometry.SetCornerRadius(Vector2 {
        X: radius.min(width / 2.0),
        Y: radius.min(height / 2.0),
    })?;
    let shape = compositor.CreateSpriteShapeWithGeometry(&geometry)?;
    shape.SetFillBrush(&compositor.CreateColorBrushWithColor(fill)?)?;
    if let Some(border) = border {
        shape.SetStrokeBrush(&compositor.CreateColorBrushWithColor(border)?)?;
        shape.SetStrokeThickness(1.0)?;
    }
    let visual = compositor.CreateShapeVisual()?;
    visual.SetSize(Vector2 {
        X: width,
        Y: height,
    })?;
    visual.SetOffset(Vector3 {
        X: x.round(),
        Y: y.round(),
        Z: 0.0,
    })?;
    visual.Shapes()?.Append(&shape)?;
    layers.InsertAtTop(&visual)?;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Typography and placement are chosen together for panel labels"
)]
fn add_panel_text(
    gfx: &Gfx,
    layers: &VisualCollection,
    value: &str,
    x: f32,
    y: f32,
    scale: f32,
    font_size: f32,
    bold: bool,
    ink: Ink,
    opacity: f32,
) -> Result<LabelBrush> {
    let label = gfx.text_brush_sized(value, scale, font_size, bold, ink)?;
    add_visual(
        &gfx.compositor,
        layers,
        &label.brush.cast()?,
        x,
        y,
        label.width,
        label.height,
    )?
    .SetOpacity(opacity)?;
    Ok(label)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Right-aligned panel labels choose typography and placement together"
)]
fn add_panel_text_right(
    gfx: &Gfx,
    layers: &VisualCollection,
    value: &str,
    right: f32,
    y: f32,
    scale: f32,
    font_size: f32,
    bold: bool,
    ink: Ink,
    opacity: f32,
) -> Result<LabelBrush> {
    let label = gfx.text_brush_sized(value, scale, font_size, bold, ink)?;
    add_visual(
        &gfx.compositor,
        layers,
        &label.brush.cast()?,
        right - label.width,
        y,
        label.width,
        label.height,
    )?
    .SetOpacity(opacity)?;
    Ok(label)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Each provider card draws its own colors, data and measured rectangle"
)]
fn draw_usage_card(
    gfx: &Gfx,
    layers: &VisualCollection,
    provider: Provider,
    usage: &ProviderUsage,
    accent: Color,
    scale: f32,
    light: bool,
    island_width: f32,
    y: f32,
) -> Result<()> {
    let c = &gfx.compositor;
    let px = |logical: f32| (logical * scale).round();
    let ink = if light { Ink::Dark } else { Ink::Light };
    let text = strings::text();
    let left = px(23.0);
    let width = island_width - px(46.0);
    let right = left + width;
    let card_fill = if light {
        rgba(32, 52, 77, 13)
    } else {
        rgba(237, 244, 255, 16)
    };
    let card_border = if light {
        rgba(41, 59, 82, 26)
    } else {
        rgba(222, 234, 251, 31)
    };
    add_rounded_box(
        c,
        layers,
        left,
        y,
        width,
        px(ISLAND_CARD_HEIGHT),
        px(13.0),
        card_fill,
        Some(card_border),
    )?;
    add_rounded_box(
        c,
        layers,
        left + px(12.0),
        y + px(11.0),
        px(32.0),
        px(32.0),
        px(10.0),
        rgba(accent.R, accent.G, accent.B, if light { 31 } else { 41 }),
        None,
    )?;
    if let Some(image) = usage::logo(provider) {
        let icon_size = px(20.0) as u32;
        let image_brush = gfx.icon_brush(image, icon_size)?;
        let brush: CompositionBrush =
            if !matches!(provider, Provider::Claude | Provider::Gemini) && light {
                let tinted = c.CreateMaskBrush()?;
                tinted.SetMask(&image_brush)?;
                tinted.SetSource(&c.CreateColorBrushWithColor(rgba(27, 34, 45, 255))?)?;
                tinted.cast()?
            } else {
                image_brush.cast()?
            };
        add_visual(
            c,
            layers,
            &brush,
            left + px(18.0),
            y + px(17.0),
            icon_size as f32,
            icon_size as f32,
        )?;
    }
    add_panel_text(
        gfx,
        layers,
        provider.name(),
        left + px(53.0),
        y + px(10.0),
        scale,
        14.0,
        true,
        ink,
        1.0,
    )?;
    if let Some(main) = usage.main_window() {
        add_panel_text(
            gfx,
            layers,
            &main.title(),
            left + px(53.0),
            y + px(30.0),
            scale,
            11.5,
            false,
            ink,
            0.72,
        )?;
        add_panel_text_right(
            gfx,
            layers,
            &format!("{}%", main.used_percent),
            right - px(14.0),
            y + px(6.0),
            scale,
            20.0,
            true,
            ink,
            1.0,
        )?;
        add_panel_text_right(
            gfx,
            layers,
            text.usage_used,
            right - px(15.0),
            y + px(31.0),
            scale,
            11.0,
            false,
            ink,
            0.7,
        )?;
        let track_left = left + px(14.0);
        let track_width = width - px(28.0);
        let track_y = y + px(53.0);
        add_rounded_box(
            c,
            layers,
            track_left,
            track_y,
            track_width,
            px(6.0),
            px(3.0),
            if light {
                rgba(54, 73, 95, 43)
            } else {
                rgba(224, 235, 248, 48)
            },
            None,
        )?;
        let fill = track_width * f32::from(main.used_percent) / 100.0;
        if fill > 0.0 {
            add_rounded_box(
                c,
                layers,
                track_left,
                track_y,
                fill,
                px(6.0),
                px(3.0),
                accent,
                None,
            )?;
        }
        if let Some(reset) = main.reset_text() {
            add_panel_text_right(
                gfx,
                layers,
                &format!("{} {reset}", text.usage_resets),
                right - px(14.0),
                y + px(64.0),
                scale,
                11.0,
                false,
                ink,
                0.78,
            )?;
        }
        if usage.state == State::Unavailable
            || usage.age_minutes().is_some_and(|minutes| minutes > 5)
        {
            add_panel_text(
                gfx,
                layers,
                text.usage_stale,
                left + px(14.0),
                y + px(64.0),
                scale,
                11.0,
                false,
                ink,
                0.7,
            )?;
        }
        if let Some(secondary) = usage.windows.get(1) {
            add_visual(
                c,
                layers,
                &c.CreateColorBrushWithColor(if light {
                    rgba(50, 68, 88, 24)
                } else {
                    rgba(228, 236, 247, 29)
                })?
                .cast()?,
                left + px(14.0),
                y + px(83.0),
                width - px(28.0),
                px(1.0),
            )?;
            add_panel_text(
                gfx,
                layers,
                &format!("{} · {}%", secondary.title(), secondary.used_percent),
                left + px(14.0),
                y + px(87.0),
                scale,
                11.5,
                true,
                ink,
                0.94,
            )?;
            if let Some(reset) = secondary.reset_text() {
                add_panel_text_right(
                    gfx,
                    layers,
                    &format!("{} {reset}", text.usage_resets),
                    right - px(14.0),
                    y + px(87.0),
                    scale,
                    11.5,
                    false,
                    ink,
                    0.76,
                )?;
            }
        }
    } else {
        let message = match usage.state {
            State::Loading => text.usage_loading,
            State::SignIn => text.usage_sign_in,
            State::Ready | State::Unavailable => text.usage_unavailable,
        };
        add_panel_text(
            gfx,
            layers,
            message,
            left + px(53.0),
            y + px(43.0),
            scale,
            12.0,
            false,
            ink,
            0.72,
        )?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Text placement needs its coordinates and typography at the call site"
)]
fn add_text(
    gfx: &Gfx,
    layers: &VisualCollection,
    value: &str,
    x: f32,
    y: f32,
    scale: f32,
    bold: bool,
    ink: Ink,
) -> Result<LabelBrush> {
    let label = gfx.text_brush(value, scale, bold, ink)?;
    add_visual(
        &gfx.compositor,
        layers,
        &label.brush.cast()?,
        x,
        y,
        label.width,
        label.height,
    )?;
    Ok(label)
}

impl MenuBar {
    pub fn create(gfx: &Gfx, usage_enabled: bool) -> Result<Self> {
        let class = w!("DockyMenuBar");
        // SAFETY: Standard class registration and window creation on the UI
        // thread; the window is destroyed in `Drop`.
        let hwnd = unsafe {
            let instance = GetModuleHandleW(None)?;
            // Registering an existing class fails harmlessly.
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(bar_proc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: class,
                ..Default::default()
            });
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
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
            )?;
            // Without this, the system backdrop brush appears black.
            let enable = BOOL::from(true);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                (&raw const enable).cast(),
                size_of::<BOOL>() as u32,
            );
            hwnd
        };
        let target = unsafe {
            // SAFETY: `hwnd` is the live window created above.
            gfx.compositor
                .cast::<ICompositorDesktopInterop>()?
                .CreateDesktopWindowTarget(hwnd, true)?
        };
        let mut bar = Self {
            hwnd,
            target,
            appbar: AppBar::register(hwnd, APPBAR_CALLBACK),
            app_name: String::new(),
            look: None,
            zones: Vec::new(),
            visible: true,
            auto_hide: false,
            revealed: false,
            root: None,
            bounds: RECT::default(),
            status: Status {
                network: Network::Offline,
                volume: None,
                buttons: TaskbarButtons::default(),
                privacy: Privacy::default(),
            },
            usage: usage_enabled.then(|| UsageMonitor::new(hwnd.0 as isize, USAGE_CHANGED)),
            expanded: false,
            visual_height: 0,
            island_left: 0.0,
            island_right: 0.0,
            refresh_button: None,
            outside_click_hook: None,
            edge_watch: None,
            _network: NetworkWatch::new(hwnd.0 as isize, STATUS_CHANGED),
            privacy_watch: status::watch_privacy(hwnd.0 as isize, PRIVACY_CHANGED),
            _saver_watch: docky_shell::power::SaverWatch::new(hwnd.0 as isize),
            audio: AudioWatch::new(hwnd.0 as isize, STATUS_CHANGED),
        };
        bar.update_status();
        bar.refresh(gfx)?;
        bar.schedule_next_minute();
        Ok(bar)
    }

    pub fn set_app_name(&mut self, gfx: &Gfx, name: &str) -> Result<()> {
        if self.app_name == name {
            return Ok(());
        }
        name.clone_into(&mut self.app_name);
        self.refresh(gfx)
    }

    /// Hidden while an app is full screen.
    pub fn set_visible(&mut self, visible: bool) -> Result<()> {
        if self.visible == visible {
            return Ok(());
        }
        self.visible = visible;
        self.place()
    }

    pub fn set_auto_hide(&mut self, auto_hide: bool) -> Result<()> {
        if self.auto_hide == auto_hide {
            return Ok(());
        }
        self.auto_hide = auto_hide;
        self.revealed = false;
        self.place()
    }

    pub fn set_usage_enabled(&mut self, gfx: &Gfx, enabled: bool) -> Result<()> {
        if self.usage.is_some() == enabled {
            return Ok(());
        }
        self.usage = enabled.then(|| UsageMonitor::new(self.hwnd.0 as isize, USAGE_CHANGED));
        self.expanded = false;
        self.refresh(gfx)
    }

    /// The pointer reached the top edge or left the bar.
    pub fn hover(&mut self, gfx: &Gfx, inside: bool) -> Result<()> {
        if !inside && self.expanded {
            self.expanded = false;
            self.refresh(gfx)?;
        }
        if !self.auto_hide || self.revealed == inside {
            return Ok(());
        }
        self.revealed = inside;
        self.place()?;
        if inside {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            // SAFETY: `track` is a valid structure with its size initialized.
            unsafe { TrackMouseEvent(&mut track)? };
        }
        self.refresh(gfx)
    }

    /// Leaving the island sideways still stays inside the full-width bar window.
    pub fn pointer_moved(&mut self, gfx: &Gfx, x: f32) -> Result<()> {
        if self.expanded && (x < self.island_left || x >= self.island_right) {
            self.dismiss_usage(gfx)?;
        }
        self.hover(gfx, true)
    }

    pub fn dismiss_usage(&mut self, gfx: &Gfx) -> Result<()> {
        if self.expanded {
            self.expanded = false;
            self.refresh(gfx)?;
        }
        Ok(())
    }

    /// Positions the window and reserves (or frees) its strip of the screen.
    fn place(&mut self) -> Result<()> {
        let bounds = self.bounds;
        let bar_height = bounds.bottom - bounds.top;
        if bar_height <= 0 {
            return Ok(());
        }
        let hidden = self.auto_hide && !self.revealed;
        if let Some(appbar) = &mut self.appbar {
            // An auto-hiding bar leaves the whole screen to the windows.
            let strip = if self.auto_hide { 0 } else { bar_height };
            appbar.reserve(Edge::Top, bounds, strip);
        }
        if let Some(root) = &self.root {
            // Hidden: the content sits above the screen edge, and only the
            // reveal strip of the window remains to catch the pointer.
            let lift = if hidden {
                -(self.visual_height as f32)
            } else {
                0.0
            };
            root.SetOffset(Vector3 {
                X: 0.0,
                Y: lift,
                Z: 0.0,
            })?;
        }
        // Hidden, the bar keeps no window on screen, not even a thin strip:
        // a strip along the top edge took the clicks meant for the close
        // button of maximized windows and for browser tabs. The edge hook
        // watches the pointer instead.
        let show = if self.visible && !hidden {
            SWP_SHOWWINDOW
        } else {
            SWP_HIDEWINDOW
        };
        // SAFETY: Our own live window.
        unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                self.visual_height,
                SWP_NOACTIVATE | show,
            )?;
        }
        EDGE_TOP.store(bounds.top, Ordering::Relaxed);
        EDGE_LEFT.store(bounds.left, Ordering::Relaxed);
        EDGE_RIGHT.store(bounds.right, Ordering::Relaxed);
        self.sync_edge_hook()?;
        self.sync_outside_click_hook()
    }

    /// The edge hook runs only while the bar auto-hides and is not covered
    /// by a full-screen app, and reports only while the bar is hidden.
    fn sync_edge_hook(&mut self) -> Result<()> {
        let watching = self.auto_hide && self.visible;
        if !watching {
            self.edge_watch = None;
        } else if self.edge_watch.is_none() {
            self.edge_watch = Some(EdgeWatch::start()?);
        }
        let hidden = watching && !self.revealed;
        EDGE_ARMED.store(false, Ordering::Relaxed);
        EDGE_HOOK_WINDOW.store(
            if hidden { self.hwnd.0 as isize } else { 0 },
            Ordering::Relaxed,
        );
        if !hidden {
            // SAFETY: Our own timer.
            unsafe {
                let _ = KillTimer(Some(self.hwnd), REVEAL_TIMER);
            }
        }
        Ok(())
    }

    /// The pointer has rested on the top edge: reveal the bar, unless it is
    /// over a window's minimize, maximize or close button, which the bar
    /// would cover. Then keep checking while the pointer stays on the edge.
    pub fn edge_rested(&mut self, gfx: &Gfx) -> Result<()> {
        let mut point = POINT::default();
        // SAFETY: A valid output for the pointer position.
        let on_edge = unsafe { GetCursorPos(&mut point) }.is_ok()
            && point.y == EDGE_TOP.load(Ordering::Relaxed)
            && EDGE_ARMED.load(Ordering::Relaxed);
        if !on_edge {
            // SAFETY: Our own timer.
            unsafe {
                let _ = KillTimer(Some(self.hwnd), REVEAL_TIMER);
            }
            return Ok(());
        }
        let screen = RECT {
            left: EDGE_LEFT.load(Ordering::Relaxed),
            top: EDGE_TOP.load(Ordering::Relaxed),
            right: EDGE_RIGHT.load(Ordering::Relaxed),
            bottom: i32::MAX,
        };
        if docky_shell::tracker::caption_button_at(point, screen) {
            return Ok(());
        }
        // SAFETY: Our own timer.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), REVEAL_TIMER);
        }
        EDGE_ARMED.store(false, Ordering::Relaxed);
        self.hover(gfx, true)
    }

    fn sync_outside_click_hook(&mut self) -> Result<()> {
        let needed = self.expanded && self.visible && (!self.auto_hide || self.revealed);
        if needed && self.outside_click_hook.is_none() {
            // SAFETY: This process owns the hook procedure and installs the hook
            // only while the panel is visible.
            let module = unsafe { GetModuleHandleW(None)? };
            let hook = unsafe {
                SetWindowsHookExW(
                    WH_MOUSE_LL,
                    Some(outside_click_proc),
                    Some(module.into()),
                    0,
                )?
            };
            USAGE_HOOK_WINDOW.store(self.hwnd.0 as isize, Ordering::Relaxed);
            self.outside_click_hook = Some(hook);
        } else if !needed && let Some(hook) = self.outside_click_hook {
            // SAFETY: This hook was installed by this MenuBar.
            unsafe { UnhookWindowsHookEx(hook)? };
            self.outside_click_hook = None;
            USAGE_HOOK_WINDOW.store(0, Ordering::Relaxed);
        }
        Ok(())
    }

    /// The minute changed, or something the bar shows did: redraw what differs.
    /// An app started or stopped using the microphone or camera.
    pub fn privacy_changed(&mut self, gfx: &Gfx) -> Result<()> {
        if let Some(watch) = &mut self.privacy_watch {
            watch.arm();
        }
        self.tick(gfx)
    }

    pub fn tick(&mut self, gfx: &Gfx) -> Result<()> {
        self.update_status();
        self.schedule_next_minute();
        self.refresh(gfx)
    }

    fn update_status(&mut self) {
        self.status = Status {
            network: status::network(),
            volume: self.audio.as_mut().and_then(AudioWatch::volume),
            buttons: status::taskbar_buttons(),
            privacy: status::privacy(),
        };
    }

    /// One wake-up per minute, aligned to the minute, instead of polling.
    fn schedule_next_minute(&self) {
        // SAFETY: Read-only query, then a timer on our own window (setting an
        // existing timer id replaces it).
        unsafe {
            let now: SYSTEMTIME = GetLocalTime();
            let elapsed = u32::from(now.wSecond) * 1000 + u32::from(now.wMilliseconds);
            SetTimer(
                Some(self.hwnd),
                CLOCK_TIMER,
                60_000 - elapsed.min(59_900) + 50,
                None,
            );
        }
    }

    pub fn click(&mut self, gfx: &Gfx, x: f32, y: f32) -> Result<()> {
        if x >= self.island_left && x < self.island_right {
            let bar_height = self.bounds.bottom as f32 - self.bounds.top as f32;
            if self.expanded
                && self
                    .refresh_button
                    .is_some_and(|(left, top, right, bottom)| {
                        x >= left && x < right && y >= top && y < bottom
                    })
            {
                if let Some(usage) = &mut self.usage {
                    usage.refresh(true);
                }
                return self.refresh(gfx);
            }
            let header_bottom = if self.expanded {
                49.0 * self.look.as_ref().map_or(1.0, |look| look.scale)
            } else {
                bar_height
            };
            if y < header_bottom {
                self.expanded = !self.expanded;
                if self.expanded {
                    if let Some(usage) = &mut self.usage {
                        usage.refresh(false);
                    }
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: `track` describes our live window.
                    unsafe { TrackMouseEvent(&mut track)? };
                }
                return self.refresh(gfx);
            }
            return Ok(());
        }
        if self.expanded {
            self.expanded = false;
            self.refresh(gfx)?;
        }
        let hit = self
            .zones
            .iter()
            .find(|(left, right, _)| x >= *left && x < *right);
        // Where a flyout opened from this item should hang: under its centre.
        let anchor = hit.map_or((0, 0), |(left, right, _)| {
            (
                self.bounds.left + ((left + right) / 2.0) as i32,
                self.bounds.bottom,
            )
        });
        match hit.map(|(_, _, action)| *action) {
            Some(Action::QuickSettings) => launcher::open_quick_settings(),
            Some(Action::Notifications) => launcher::open_notifications(),
            Some(Action::Sound) => launcher::open_sound(),
            Some(Action::Search) => launcher::open_search(),
            Some(Action::TaskView) => launcher::open_task_view(),
            Some(Action::Widgets) => launcher::open_widgets(),
            Some(Action::HiddenIcons) => launcher::toggle_hidden_icons(anchor),
            Some(Action::InputLanguage) => launcher::open_input_switcher(),
            // Windows' own pages list which apps use each device.
            Some(Action::Microphone) => {
                let _ = launcher::open("ms-settings:privacy-microphone");
            }
            Some(Action::Camera) => {
                let _ = launcher::open("ms-settings:privacy-webcam");
            }
            None => {}
        }
        Ok(())
    }

    /// Redraws the bar if anything it shows has changed.
    pub fn refresh(&mut self, gfx: &Gfx) -> Result<()> {
        let Some(monitor) = system::primary_monitor() else {
            return Ok(());
        };
        let active_providers = if self.usage.is_some() {
            usage::active_providers()
        } else {
            ActiveProviders::default()
        };
        if !active_providers.any() {
            self.expanded = false;
            if let Some(usage) = &mut self.usage {
                usage.mark_inactive();
            }
        } else if self.visible
            && (!self.auto_hide || self.revealed)
            && let Some(usage) = &mut self.usage
        {
            usage.refresh(false);
        }
        let look = Look {
            screen: monitor.screen,
            scale: monitor.scale,
            light: system::uses_light_theme(),
            glass: system::transparency_enabled(),
            app_name: self.app_name.clone(),
            clock: system::clock_text(),
            input: system::input_language(),
            battery: system::battery(),
            status: self.status,
            usage: self
                .usage
                .as_ref()
                .map_or_else(Snapshot::default, UsageMonitor::snapshot),
            usage_enabled: self.usage.is_some() && active_providers.any(),
            active_providers,
            expanded: self.expanded,
            usage_refreshing: self.usage.as_ref().is_some_and(UsageMonitor::is_refreshing),
        };
        if self.look.as_ref() == Some(&look) {
            return Ok(());
        }
        self.build(gfx, &monitor, &look)?;
        self.look = Some(look);
        Ok(())
    }

    fn build(&mut self, gfx: &Gfx, monitor: &Monitor, look: &Look) -> Result<()> {
        let c = &gfx.compositor;
        let scale = look.scale;
        let px = |logical: f32| (logical * scale).round();
        let screen = monitor.screen;
        let (width, height) = ((screen.right - screen.left) as f32, px(HEIGHT));
        let visual_height = if look.expanded {
            px(74.0 + look.active_providers.count() as f32 * ISLAND_ROW_HEIGHT)
        } else {
            height
        };
        let ink = if look.light { Ink::Dark } else { Ink::Light };

        let root = c.CreateContainerVisual()?;
        root.SetSize(Vector2 {
            X: width,
            Y: visual_height,
        })?;
        let layers = root.Children()?;
        // Without transparency the bar is solid, as macOS draws it with
        // "Reduce transparency".
        let tint = match (look.light, look.glass) {
            (true, true) => rgba(255, 255, 255, 110),
            (false, true) => rgba(24, 24, 26, 120),
            (true, false) => rgba(236, 236, 238, 255),
            (false, false) => rgba(36, 36, 38, 255),
        };
        let backdrop = if look.glass {
            Some(c.CreateHostBackdropBrush()?.cast()?)
        } else {
            None
        };
        let fills: [windows::UI::Composition::CompositionBrush; 1] =
            [c.CreateColorBrushWithColor(tint)?.cast()?];
        for brush in backdrop.iter().chain(&fills) {
            let fill = c.CreateSpriteVisual()?;
            fill.SetSize(Vector2 {
                X: width,
                Y: height,
            })?;
            fill.SetBrush(brush)?;
            layers.InsertAtTop(&fill)?;
        }

        let place = |layers: &windows::UI::Composition::VisualCollection,
                     item: &LabelBrush,
                     left: f32|
         -> Result<SpriteVisual> {
            let visual = c.CreateSpriteVisual()?;
            visual.SetSize(Vector2 {
                X: item.width,
                Y: item.height,
            })?;
            visual.SetOffset(Vector3 {
                X: left.round(),
                Y: ((height - item.height) / 2.0).round(),
                Z: 0.0,
            })?;
            visual.SetBrush(&item.brush)?;
            layers.InsertAtTop(&visual)?;
            Ok(visual)
        };

        self.zones.clear();
        let mut left_end = px(SIDE_MARGIN);
        if !look.app_name.is_empty() {
            let name = gfx.text_brush(&look.app_name, scale, true, ink)?;
            place(&layers, &name, px(SIDE_MARGIN))?;
            left_end += name.width;
        }

        // Right side, from the edge inward.
        let slop = px(HIT_SLOP);
        let mut right = width - px(SIDE_MARGIN);
        let status = look.status;
        let symbol = |layers: &[(char, f32)]| gfx.glyph_brush(layers, scale, ink);
        let mut items: Vec<(LabelBrush, Action)> = vec![
            (
                gfx.text_brush(&look.clock, scale, false, ink)?,
                Action::Notifications,
            ),
            (gfx.control_center_brush(scale, ink)?, Action::QuickSettings),
        ];
        // Privacy indicators, as the Windows tray shows them: something is
        // using the microphone or camera right now.
        let privacy = status.privacy;
        for (on, glyph, action) in [
            (privacy.microphone, glyph::MICROPHONE, Action::Microphone),
            (privacy.camera, glyph::CAMERA, Action::Camera),
        ] {
            if on {
                items.push((symbol(&[(glyph, 1.0)])?, action));
            }
        }
        if status.buttons.search {
            items.push((symbol(&[(glyph::SEARCH, 1.0)])?, Action::Search));
        }
        let network = match status.network {
            // The full fan stays faint behind the bars received, as on a Mac.
            Network::Wifi { bars } => vec![
                (glyph::WIFI_FULL, 0.3),
                (glyph::WIFI[usize::from(bars.clamp(1, 4)) - 1], 1.0),
            ],
            Network::Ethernet => vec![(glyph::ETHERNET, 1.0)],
            Network::Offline => vec![(glyph::OFFLINE, 1.0)],
        };
        items.push((symbol(&network)?, Action::QuickSettings));
        if let Some(volume) = status.volume {
            let speaker = if volume.muted {
                glyph::MUTE
            } else {
                glyph::VOLUME[usize::from(volume.level.min(3))]
            };
            items.push((symbol(&[(speaker, 1.0)])?, Action::Sound));
        }
        if let Some(battery) = look.battery {
            items.push((
                gfx.battery_brush(battery.percent, battery.charging, scale, ink)?,
                Action::QuickSettings,
            ));
        }
        if let Some(language) = &look.input {
            items.push((
                gfx.text_brush(language, scale, false, ink)?,
                Action::InputLanguage,
            ));
        }
        // The notification area (antivirus, OneDrive, apps in the background)
        // lives only in the hidden taskbar; this arrow opens its flyout.
        items.push((symbol(&[(glyph::HIDDEN_ICONS, 1.0)])?, Action::HiddenIcons));
        if status.buttons.task_view {
            items.push((symbol(&[(glyph::TASK_VIEW, 1.0)])?, Action::TaskView));
        }
        if status.buttons.widgets {
            items.push((symbol(&[(glyph::WIDGETS, 1.0)])?, Action::Widgets));
        }
        for (item, action) in &items {
            let left = right - item.width;
            place(&layers, item, left)?;
            self.zones.push((left - slop, right + slop, *action));
            right = left - px(ITEM_GAP);
        }

        if look.usage_enabled {
            self.draw_island(gfx, look, &layers, width, height, left_end, right)?;
        } else {
            self.island_left = 0.0;
            self.island_right = 0.0;
            self.refresh_button = None;
        }

        self.target.SetRoot(&root)?;
        self.root = Some(root);
        self.visual_height = visual_height as i32;
        self.bounds = RECT {
            bottom: screen.top + height as i32,
            ..screen
        };
        ISLAND_LEFT.store(screen.left + self.island_left as i32, Ordering::Relaxed);
        ISLAND_RIGHT.store(screen.left + self.island_right as i32, Ordering::Relaxed);
        BAR_TOP.store(screen.top, Ordering::Relaxed);
        BAR_BOTTOM.store(screen.top + height as i32, Ordering::Relaxed);
        HEADER_BOTTOM.store(
            screen.top
                + if look.expanded {
                    px(49.0) as i32
                } else {
                    height as i32
                },
            Ordering::Relaxed,
        );
        PANEL_BOTTOM.store(screen.top + self.visual_height, Ordering::Relaxed);
        self.place()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "The island receives the measured bar bounds from the current layout"
    )]
    fn draw_island(
        &mut self,
        gfx: &Gfx,
        look: &Look,
        layers: &VisualCollection,
        width: f32,
        bar_height: f32,
        left_end: f32,
        right_start: f32,
    ) -> Result<()> {
        let c = &gfx.compositor;
        let scale = look.scale;
        let px = |logical: f32| (logical * scale).round();
        let requested = px(if look.expanded {
            ISLAND_EXPANDED_WIDTH
        } else if look.active_providers.count() == 1 {
            ISLAND_SINGLE_WIDTH
        } else {
            134.0 * look.active_providers.count() as f32
        });
        let left_limit = left_end + px(8.0);
        let usable = (right_start - left_limit - px(8.0)).max(px(145.0));
        let island_width = requested.min(usable);
        let island_left = ((width - island_width) / 2.0)
            .max(left_limit)
            .min((right_start - island_width - px(8.0)).max(left_limit));
        let island_height = if look.expanded {
            px(74.0 + look.active_providers.count() as f32 * ISLAND_ROW_HEIGHT)
        } else {
            bar_height
        };
        self.island_left = island_left;
        self.island_right = island_left + island_width;

        let island = c.CreateContainerVisual()?;
        island.SetSize(Vector2 {
            X: island_width,
            Y: island_height,
        })?;
        island.SetOffset(Vector3 {
            X: island_left.round(),
            Y: 0.0,
            Z: 0.0,
        })?;
        let content = island.Children()?;
        let silhouette = gfx.island_mask(island_width, island_height, scale)?;
        let shadow = c.CreateDropShadow()?;
        shadow.SetMask(&silhouette)?;
        shadow.SetBlurRadius(px(12.0))?;
        shadow.SetOffset(Vector3 {
            X: 0.0,
            Y: px(5.0),
            Z: 0.0,
        })?;
        shadow.SetColor(rgba(0, 0, 0, 92))?;
        let shadow_host = c.CreateSpriteVisual()?;
        shadow_host.SetSize(Vector2 {
            X: island_width,
            Y: island_height,
        })?;
        shadow_host.SetShadow(&shadow)?;
        content.InsertAtTop(&shadow_host)?;

        let tint = match (look.light, look.glass) {
            (true, true) => rgba(247, 249, 252, 243),
            (false, true) => rgba(7, 9, 13, 242),
            (true, false) => rgba(247, 249, 252, 255),
            (false, false) => rgba(10, 12, 17, 255),
        };
        if look.glass {
            // The backdrop cannot be used as a mask-brush source. Inset its
            // rounded clip so it stays entirely inside the curved silhouette.
            let inset = px(12.0);
            let glass_width = island_width - inset * 2.0;
            let glass = c.CreateContainerVisual()?;
            glass.SetSize(Vector2 {
                X: glass_width,
                Y: island_height,
            })?;
            glass.SetOffset(Vector3 {
                X: inset,
                Y: 0.0,
                Z: 0.0,
            })?;
            let outline = c.CreateRoundedRectangleGeometry()?;
            outline.SetSize(Vector2 {
                X: glass_width,
                Y: island_height + px(22.0),
            })?;
            outline.SetOffset(Vector2 {
                X: 0.0,
                Y: -px(22.0),
            })?;
            outline.SetCornerRadius(Vector2 {
                X: px(22.0),
                Y: px(22.0),
            })?;
            glass.SetClip(&c.CreateGeometricClipWithGeometry(&outline)?)?;
            add_visual(
                c,
                &glass.Children()?,
                &c.CreateHostBackdropBrush()?.cast()?,
                0.0,
                0.0,
                glass_width,
                island_height,
            )?;
            content.InsertAtTop(&glass)?;
        }
        let masked = c.CreateMaskBrush()?;
        masked.SetSource(&c.CreateColorBrushWithColor(tint)?)?;
        masked.SetMask(&silhouette)?;
        add_visual(
            c,
            &content,
            &masked.cast()?,
            0.0,
            0.0,
            island_width,
            island_height,
        )?;

        let ink = if look.light { Ink::Dark } else { Ink::Light };
        let providers = Provider::ALL
            .into_iter()
            .filter(|provider| look.active_providers.has(*provider))
            .map(|provider| {
                let accent = match provider {
                    Provider::Claude => rgba(217, 119, 87, 255),
                    Provider::Codex => rgba(135, 190, 245, 255),
                    Provider::Copilot => rgba(147, 164, 186, 255),
                    Provider::OpenCode => rgba(132, 196, 166, 255),
                    Provider::Gemini => rgba(111, 146, 255, 255),
                };
                (provider, provider.name(), look.usage.get(provider), accent)
            })
            .collect::<Vec<_>>();
        if look.expanded {
            let title = strings::text();
            add_panel_text(
                gfx,
                &content,
                title.usage_title,
                px(28.0),
                px(15.0),
                scale,
                13.0,
                true,
                ink,
                0.94,
            )?;
            let button_text = if look.usage_refreshing {
                title.usage_refreshing
            } else {
                title.usage_refresh
            };
            let button_label = gfx.text_brush_sized(button_text, scale, 11.5, true, ink)?;
            let button_width = button_label.width + px(20.0);
            let button_height = px(27.0);
            let button_left = island_width - px(23.0) - button_width;
            let button_top = px(8.0);
            add_rounded_box(
                c,
                &content,
                button_left,
                button_top,
                button_width,
                button_height,
                px(13.5),
                if look.light {
                    rgba(28, 51, 79, 18)
                } else {
                    rgba(235, 242, 252, 26)
                },
                Some(if look.light {
                    rgba(41, 62, 86, 36)
                } else {
                    rgba(226, 237, 251, 43)
                }),
            )?;
            add_visual(
                c,
                &content,
                &button_label.brush.cast()?,
                button_left + px(10.0),
                button_top + (button_height - button_label.height) / 2.0,
                button_label.width,
                button_label.height,
            )?;
            self.refresh_button = Some((
                island_left + button_left,
                button_top,
                island_left + button_left + button_width,
                button_top + button_height,
            ));
            for (index, (provider, _name, usage, accent)) in providers.iter().enumerate() {
                draw_usage_card(
                    gfx,
                    &content,
                    *provider,
                    usage,
                    *accent,
                    scale,
                    look.light,
                    island_width,
                    px(53.0 + index as f32 * ISLAND_ROW_HEIGHT),
                )?;
            }
            let newest_age = providers
                .iter()
                .filter_map(|provider| provider.2.age_minutes())
                .min();
            if let Some(minutes) = newest_age {
                let status = format!("{} {minutes} min", title.usage_updated);
                let label = gfx.text_brush_sized(&status, scale, 11.0, false, ink)?;
                add_visual(
                    c,
                    &content,
                    &label.brush.cast()?,
                    (island_width - label.width) / 2.0,
                    island_height - px(21.0),
                    label.width,
                    label.height,
                )?
                .SetOpacity(0.53)?;
            }
        } else {
            self.refresh_button = None;
            let half = island_width / providers.len() as f32;
            for (index, (provider, _name, usage, accent)) in providers.iter().enumerate() {
                let icon_size = px(18.0) as u32;
                let start = index as f32 * half
                    + if half < px(84.0) {
                        (half - icon_size as f32) / 2.0
                    } else {
                        px(21.0)
                    };
                if let Some(image) = usage::logo(*provider) {
                    let image_brush = gfx.icon_brush(image, icon_size)?;
                    let brush: CompositionBrush =
                        if !matches!(provider, Provider::Claude | Provider::Gemini) && look.light {
                            let tinted = c.CreateMaskBrush()?;
                            tinted.SetMask(&image_brush)?;
                            tinted
                                .SetSource(&c.CreateColorBrushWithColor(rgba(27, 34, 45, 255))?)?;
                            tinted.cast()?
                        } else {
                            image_brush.cast()?
                        };
                    add_visual(
                        c,
                        &content,
                        &brush,
                        start,
                        (bar_height - icon_size as f32) / 2.0,
                        icon_size as f32,
                        icon_size as f32,
                    )?;
                }
                let percent = usage.main_window().map_or("—".to_owned(), |window| {
                    format!("{}%", window.used_percent)
                });
                if half > px(106.0) {
                    add_text(
                        gfx,
                        &content,
                        &percent,
                        start + px(25.0),
                        px(6.0),
                        scale,
                        true,
                        ink,
                    )?;
                }
                if half >= px(84.0) {
                    let ring = gfx.usage_ring(
                        usage.main_window().map(|window| window.used_percent),
                        scale,
                        *accent,
                    )?;
                    add_visual(
                        c,
                        &content,
                        &ring.brush.cast()?,
                        (index + 1) as f32 * half - px(16.0) - ring.width,
                        (bar_height - ring.height) / 2.0,
                        ring.width,
                        ring.height,
                    )?;
                }
            }
        }
        layers.InsertAtTop(&island)?;
        Ok(())
    }
}

impl Drop for MenuBar {
    fn drop(&mut self) {
        USAGE_HOOK_WINDOW.store(0, Ordering::Relaxed);
        EDGE_HOOK_WINDOW.store(0, Ordering::Relaxed);
        self.edge_watch = None;
        if let Some(hook) = self.outside_click_hook.take() {
            // SAFETY: Do not leave a callback pointing at a destroyed window.
            unsafe {
                let _ = UnhookWindowsHookEx(hook);
            }
        }
        // Give the reserved strip back before the window goes away.
        self.appbar = None;
        // SAFETY: Our own window, destroyed once.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
