//! The live usage strip on the taskbar (upstream's menu-bar text item). The popup renders each frame
//! as a PNG sized for the taskbar band; this module hosts it.
//!
//! Windows: a layered, never-activating child window inside `Shell_TrayWnd`, immediately left of the
//! notification area. When a taskbar styler draws the notification area as its own island away from
//! the taskbar's edge (Windhawk's centered taskbar), the strip becomes a matching island right after
//! it instead. It is owned by one dedicated thread with its own message loop, re-anchors on a
//! one-second timer, rebuilds itself after Explorer restarts (`TaskbarCreated`) and reports taskbar
//! size, scale and theme changes to the popup as `taskbar-info`.
//! Linux: the frame's text becomes the tray title.
//! macOS: the frame becomes the menu bar item's image itself, drawn as a template so the system
//! tints it for the menu bar, exactly like upstream's status item. The Bars glyph and the plain icon
//! share that one image, so the strip wins over the glyph and the glyph over the icon.
//! Other platforms report the strip unsupported.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, PhysicalRect, Runtime, State};

/// Largest frame the popup may send, in device pixels.
const MAX_FRAME_WIDTH: u32 = 4096;
const MAX_FRAME_HEIGHT: u32 = 512;
const MAX_PNG_BYTES: usize = 4 * 1_048_576;
const MAX_TEXT_CHARS: usize = 512;
const MAX_TOOLTIP_CHARS: usize = 1024;
/// Popup event carrying a changed [`TaskbarInfo`].
#[cfg_attr(not(windows), allow(dead_code))]
pub const TASKBAR_INFO_EVENT: &str = "taskbar-info";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum TaskbarTheme {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum TaskbarEdge {
    Bottom,
    Top,
    Left,
    Right,
}

/// What the popup needs to render a frame that fits the taskbar band.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskbarInfo {
    pub supported: bool,
    /// Device-pixel height of the taskbar band.
    pub height: u32,
    /// Device pixels per logical pixel on the taskbar's monitor.
    pub scale: f64,
    /// The taskbar's own (system) theme, which can differ from the app theme.
    pub theme: TaskbarTheme,
    pub edge: TaskbarEdge,
}

impl TaskbarInfo {
    #[cfg_attr(any(target_os = "linux", target_os = "macos"), allow(dead_code))]
    pub const UNSUPPORTED: TaskbarInfo = TaskbarInfo {
        supported: false,
        height: 0,
        scale: 1.0,
        theme: TaskbarTheme::Dark,
        edge: TaskbarEdge::Bottom,
    };
}

/// One strip frame as the popup sends it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StripFrame {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Plain-text rendering of the same readings (tray title on Linux).
    pub text: String,
    pub tooltip: String,
}

/// Which mouse button released over the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum StripButton {
    Primary,
    Secondary,
}

/// A click on the strip, with the strip's screen bounds in physical pixels so the popup can open
/// right against it.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct StripClick {
    pub button: StripButton,
    pub bounds: PhysicalRect<i32, u32>,
}

/// A decoded frame: premultiplied BGRA rows, top-down, ready for a 32-bit DIB.
#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub text: String,
    pub tooltip: String,
}

/// Validate a frame and convert it for the platform window.
pub fn decode_frame(frame: &StripFrame) -> Result<Bitmap, String> {
    if frame.text.chars().count() > MAX_TEXT_CHARS {
        return Err("Strip text is too long".into());
    }
    if frame.tooltip.chars().count() > MAX_TOOLTIP_CHARS {
        return Err("Strip tooltip is too long".into());
    }
    if frame.width == 0
        || frame.height == 0
        || frame.width > MAX_FRAME_WIDTH
        || frame.height > MAX_FRAME_HEIGHT
    {
        return Err("Strip frame dimensions are out of range".into());
    }
    let bytes = &frame.png;
    if bytes.len() > MAX_PNG_BYTES || bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("Expected a PNG strip frame".into());
    }
    let image = tauri::image::Image::from_bytes(bytes)
        .map_err(|_| "The strip frame is not a readable PNG".to_string())?;
    if image.width() != frame.width || image.height() != frame.height {
        return Err("Strip frame size does not match its PNG".into());
    }
    Ok(Bitmap {
        width: frame.width,
        height: frame.height,
        bgra: premultiplied_bgra(image.rgba()),
        text: frame.text.clone(),
        tooltip: frame.tooltip.clone(),
    })
}

impl Bitmap {
    /// Back to straight RGBA, for platforms that take the frame as an ordinary image.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn straight_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bgra.len());
        for pixel in self.bgra.as_chunks::<4>().0 {
            let alpha = pixel[3];
            let unscale = |channel: u8| {
                if alpha == 0 {
                    0
                } else {
                    ((u16::from(channel) * 255 + u16::from(alpha) / 2) / u16::from(alpha)).min(255)
                        as u8
                }
            };
            let alpha = if alpha <= 1 && pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0 {
                0
            } else {
                alpha
            };
            out.extend_from_slice(&[
                unscale(pixel[2]),
                unscale(pixel[1]),
                unscale(pixel[0]),
                alpha,
            ]);
        }
        out
    }
}

/// Straight RGBA to premultiplied BGRA. Fully transparent pixels keep an alpha of 1 so the whole
/// strip stays clickable: a layered window lets clicks fall through where alpha is 0.
pub fn premultiplied_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for pixel in rgba.as_chunks::<4>().0 {
        let alpha = u16::from(pixel[3]);
        let scale = |channel: u8| ((u16::from(channel) * alpha + 127) / 255) as u8;
        out.extend_from_slice(&[
            scale(pixel[2]),
            scale(pixel[1]),
            scale(pixel[0]),
            pixel[3].max(1),
        ]);
    }
    out
}

/// Where the strip sits inside the taskbar's client area: just left of the notification area,
/// vertically centered, never past the taskbar's left edge.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn strip_origin(
    taskbar_height: i32,
    notify_left: i32,
    width: i32,
    height: i32,
    gap: i32,
) -> (i32, i32) {
    let x = (notify_left - gap - width).max(0);
    let y = ((taskbar_height - height) / 2).max(0);
    (x, y)
}

/// The notification area when a taskbar styler draws it as an island apart from the rest of the
/// taskbar: its box in taskbar client coordinates (device pixels, `right` and `bottom` exclusive) and
/// its fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Island {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
    /// Fill as RGB.
    pub color: [u8; 3],
}

/// Where a framed strip `width` wide goes beside `island`: right after it while the taskbar has room,
/// otherwise just before it; `None` when neither side fits.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn island_strip_origin(
    taskbar_width: i32,
    island: &Island,
    width: i32,
    gap: i32,
) -> Option<(i32, i32)> {
    let after = island.right + gap;
    if after + width <= taskbar_width {
        return Some((after, island.top));
    }
    let before = island.left - gap - width;
    (before >= 0).then_some((before, island.top))
}

fn scale_channel(channel: u8, alpha: u8) -> u8 {
    ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8
}

/// How much of pixel (`x`, `y`) a `width` x `height` box with `radius` corners covers, 0...1.
fn box_coverage(x: u32, y: u32, width: u32, height: u32, radius: f64) -> f64 {
    let center = |position: u32, size: u32| {
        let middle = f64::from(position) + 0.5;
        if middle < radius {
            radius - middle
        } else if middle > f64::from(size) - radius {
            middle - (f64::from(size) - radius)
        } else {
            0.0
        }
    };
    let (dx, dy) = (center(x, width), center(y, height));
    if dx == 0.0 || dy == 0.0 {
        return 1.0;
    }
    (radius - dx.hypot(dy) + 0.5).clamp(0.0, 1.0)
}

/// The strip as an island of its own: `content` centered in a `height`-tall box with rounded
/// corners, filled with `color` and `padding` wider on each side. Content rows outside the box are
/// clipped.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn framed(content: &Bitmap, height: u32, padding: u32, radius: f64, color: [u8; 3]) -> Bitmap {
    let width = content.width + padding * 2;
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let alpha = (box_coverage(x, y, width, height, radius) * 255.0).round() as u8;
            bgra.extend_from_slice(&[
                scale_channel(color[2], alpha),
                scale_channel(color[1], alpha),
                scale_channel(color[0], alpha),
                alpha,
            ]);
        }
    }
    let shift = (i64::from(height) - i64::from(content.height)) / 2;
    for row in 0..content.height {
        let Ok(y) = u32::try_from(i64::from(row) + shift) else {
            continue;
        };
        if y >= height {
            continue;
        }
        for column in 0..content.width {
            let source = ((row * content.width + column) * 4) as usize;
            let target = ((y * width + column + padding) * 4) as usize;
            let cover = 255 - content.bgra[source + 3];
            for channel in 0..4 {
                bgra[target + channel] = content.bgra[source + channel]
                    .saturating_add(scale_channel(bgra[target + channel], cover));
            }
        }
    }
    Bitmap {
        width,
        height,
        bgra,
        text: content.text.clone(),
        tooltip: content.tooltip.clone(),
    }
}

/// Tauri state: the running strip for this platform.
pub struct TaskbarStrip {
    inner: platform::Strip,
}

impl TaskbarStrip {
    /// Start the strip. `on_click` runs on the main thread.
    pub fn install<R: Runtime>(
        app: &AppHandle<R>,
        on_click: impl Fn(StripClick) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: platform::Strip::start(app.clone(), std::sync::Arc::new(on_click)),
        }
    }

    pub fn info(&self) -> TaskbarInfo {
        self.inner.info()
    }

    pub fn set(&self, bitmap: Option<Bitmap>) {
        self.inner.set(bitmap);
    }

    /// The tray icon glyph (the Bars style), or `None` for the app icon, with its tooltip. Only
    /// macOS draws both into one menu bar image; elsewhere the caller sets the tray icon directly.
    #[cfg(target_os = "macos")]
    pub fn set_glyph(&self, glyph: Option<tauri::image::Image<'static>>, tooltip: String) {
        self.inner.set_glyph(glyph, tooltip);
    }
}

#[tauri::command]
pub fn taskbar_info(strip: State<'_, TaskbarStrip>) -> TaskbarInfo {
    strip.info()
}

#[tauri::command]
pub fn set_taskbar_strip(
    strip: State<'_, TaskbarStrip>,
    frame: Option<StripFrame>,
) -> Result<(), String> {
    let bitmap = frame.as_ref().map(decode_frame).transpose()?;
    strip.set(bitmap);
    Ok(())
}

#[cfg(windows)]
mod platform {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use tauri::{AppHandle, Emitter, PhysicalPosition, PhysicalRect, PhysicalSize, Runtime};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
        CLR_INVALID, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
        GetDC, GetPixel, MapWindowPoints, ReleaseDC, SelectObject,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows_sys::Win32::UI::Controls::{
        ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, SetWindowTheme,
        TOOLTIPS_CLASSW, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH,
        TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
    };
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::Shell::{
        ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowExW,
        FindWindowW, GetMessageW, GetParent, GetWindowRect, HWND_TOP, IDC_ARROW, IsWindow,
        LoadCursorW, MA_NOACTIVATE, MSG, PostMessageW, RegisterClassExW, RegisterWindowMessageW,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, SendMessageW, SetTimer, SetWindowPos, TranslateMessage,
        ULW_ALPHA, UpdateLayeredWindow, WM_APP, WM_DISPLAYCHANGE, WM_LBUTTONUP, WM_MOUSEACTIVATE,
        WM_NCDESTROY, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_CHILD,
        WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        WS_POPUP, WS_VISIBLE,
    };

    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationCacheRequest,
        IUIAutomationCondition, TreeScope_Children, UIA_BoundingRectanglePropertyId,
        UIA_ClassNamePropertyId,
    };
    use windows::core::Interface;

    use super::{
        Bitmap, Island, StripButton, StripClick, TASKBAR_INFO_EVENT, TaskbarEdge, TaskbarInfo,
        TaskbarTheme, framed, island_strip_origin, strip_origin,
    };

    const WM_APP_FRAME: u32 = WM_APP + 1;
    const SYNC_TIMER: usize = 1;
    const SYNC_INTERVAL_MS: u32 = 1000;
    const GAP_POINTS: f64 = 4.0;
    /// Space between the strip's content and the edges of its island.
    const FRAME_PADDING_POINTS: f64 = 6.0;
    /// Corner radius of the notification-area island Windhawk's taskbar styles draw.
    const FRAME_RADIUS_POINTS: f64 = 4.0;
    /// How far that island reaches left of the first notification-area button.
    const ISLAND_INSET_POINTS: f64 = 8.0;
    /// Windows that host the Windows 11 taskbar's XAML, whose UI Automation tree holds the tray.
    const XAML_BRIDGE_CLASS: &str = "Windows.UI.Composition.DesktopWindowContentBridge";
    const XAML_SITE_CLASS: &str = "Windows.UI.Input.InputSite.WindowClass";
    const TRAY_CLASS_PREFIX: &str = "SystemTray.";
    const TASKBAR_FRAME_CLASS: &str = "Taskbar.TaskbarFrameAutomationPeer";
    const AUTOMATION_TIMEOUT_MS: u32 = 1000;
    const COLOR_REFRESH: Duration = Duration::from_secs(30);
    const TOOLTIP_MAX_WIDTH_POINTS: f64 = 360.0;
    /// The dark common-controls theme Explorer itself uses for tooltips over a dark taskbar.
    const DARK_TOOLTIP_THEME: &str = "DarkMode_Explorer";

    type ClickHandler = Arc<dyn Fn(StripClick) + Send + Sync>;
    /// Hands a click job to the main thread (window procedures must never block on Tauri).
    type Dispatch = Box<dyn Fn(Box<dyn FnOnce() + Send>) + Send>;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct Strip {
        host: Arc<AtomicIsize>,
        pending: Arc<Mutex<Option<Option<Bitmap>>>>,
        info: Arc<Mutex<TaskbarInfo>>,
    }

    impl Strip {
        pub fn start<R: Runtime>(app: AppHandle<R>, on_click: ClickHandler) -> Self {
            let host = Arc::new(AtomicIsize::new(0));
            let pending = Arc::new(Mutex::new(None));
            let info = Arc::new(Mutex::new(
                read_taskbar()
                    .map(|taskbar| taskbar.info)
                    .unwrap_or(TaskbarInfo::UNSUPPORTED),
            ));
            let thread = Shared {
                host: host.clone(),
                pending: pending.clone(),
                info: info.clone(),
            };
            let spawned = std::thread::Builder::new()
                .name("taskbar-strip".into())
                .spawn(move || run(app, on_click, thread));
            if let Err(error) = spawned {
                tracing::warn!("taskbar strip thread failed to start: {error}");
            }
            Self {
                host,
                pending,
                info,
            }
        }

        pub fn info(&self) -> TaskbarInfo {
            self.info
                .lock()
                .map(|info| info.clone())
                .unwrap_or(TaskbarInfo::UNSUPPORTED)
        }

        pub fn set(&self, bitmap: Option<Bitmap>) {
            if let Ok(mut pending) = self.pending.lock() {
                *pending = Some(bitmap);
            }
            let host = self.host.load(Ordering::Acquire);
            if host != 0 {
                unsafe { PostMessageW(host as HWND, WM_APP_FRAME, 0, 0) };
            }
        }
    }

    struct Shared {
        host: Arc<AtomicIsize>,
        pending: Arc<Mutex<Option<Option<Bitmap>>>>,
        info: Arc<Mutex<TaskbarInfo>>,
    }

    /// The taskbar as read now: its window, the notification area's left edge in taskbar client
    /// coordinates, and the info the popup renders against.
    struct Taskbar {
        hwnd: HWND,
        notify_left: i32,
        width: i32,
        height: i32,
        info: TaskbarInfo,
    }

    fn read_theme() -> TaskbarTheme {
        let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let value = wide("SystemUsesLightTheme");
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut data as *mut u32).cast(),
                &mut size,
            )
        };
        if status == 0 && data == 1 {
            TaskbarTheme::Light
        } else {
            TaskbarTheme::Dark
        }
    }

    fn read_edge() -> TaskbarEdge {
        let mut data: APPBARDATA = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<APPBARDATA>() as u32;
        let found = unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) };
        if found == 0 {
            return TaskbarEdge::Bottom;
        }
        match data.uEdge {
            ABE_TOP => TaskbarEdge::Top,
            ABE_LEFT => TaskbarEdge::Left,
            ABE_RIGHT => TaskbarEdge::Right,
            ABE_BOTTOM => TaskbarEdge::Bottom,
            _ => TaskbarEdge::Bottom,
        }
    }

    fn read_taskbar() -> Option<Taskbar> {
        let class = wide("Shell_TrayWnd");
        let hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
        if hwnd.is_null() {
            return None;
        }
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return None;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 };
        let edge = read_edge();
        let notify_class = wide("TrayNotifyWnd");
        let notify = unsafe {
            FindWindowExW(
                hwnd,
                std::ptr::null_mut(),
                notify_class.as_ptr(),
                std::ptr::null(),
            )
        };
        let mut notify_left = width;
        if !notify.is_null() {
            let mut notify_rect: RECT = unsafe { std::mem::zeroed() };
            if unsafe { GetWindowRect(notify, &mut notify_rect) } != 0 {
                let mut corner = POINT {
                    x: notify_rect.left,
                    y: notify_rect.top,
                };
                unsafe { MapWindowPoints(std::ptr::null_mut(), hwnd, &mut corner, 1) };
                notify_left = corner.x;
            }
        }
        let horizontal = matches!(edge, TaskbarEdge::Bottom | TaskbarEdge::Top) && width > height;
        let info = TaskbarInfo {
            supported: horizontal && height > 0,
            height: height.max(0) as u32,
            scale,
            theme: read_theme(),
            edge,
        };
        Some(Taskbar {
            hwnd,
            notify_left,
            width,
            height,
            info,
        })
    }

    struct Automation {
        client: IUIAutomation,
        cache: IUIAutomationCacheRequest,
        all: IUIAutomationCondition,
    }

    fn create_automation() -> Option<Automation> {
        unsafe {
            let client: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            if let Ok(timeouts) = client.cast::<IUIAutomation2>() {
                let _ = timeouts.SetConnectionTimeout(AUTOMATION_TIMEOUT_MS);
                let _ = timeouts.SetTransactionTimeout(AUTOMATION_TIMEOUT_MS);
            }
            let cache = client.CreateCacheRequest().ok()?;
            cache.AddProperty(UIA_ClassNamePropertyId).ok()?;
            cache.AddProperty(UIA_BoundingRectanglePropertyId).ok()?;
            let all = client.CreateTrueCondition().ok()?;
            Some(Automation { client, cache, all })
        }
    }

    /// The window whose UI Automation children are the taskbar frame and the tray buttons.
    fn xaml_site(taskbar: HWND) -> Option<HWND> {
        let bridge_class = wide(XAML_BRIDGE_CLASS);
        let site_class = wide(XAML_SITE_CLASS);
        unsafe {
            let bridge = FindWindowExW(
                taskbar,
                std::ptr::null_mut(),
                bridge_class.as_ptr(),
                std::ptr::null(),
            );
            if bridge.is_null() {
                return None;
            }
            let site = FindWindowExW(
                bridge,
                std::ptr::null_mut(),
                site_class.as_ptr(),
                std::ptr::null(),
            );
            (!site.is_null()).then_some(site)
        }
    }

    fn union(one: RECT, two: RECT) -> RECT {
        RECT {
            left: one.left.min(two.left),
            top: one.top.min(two.top),
            right: one.right.max(two.right),
            bottom: one.bottom.max(two.bottom),
        }
    }

    /// `rect` in `window`'s client coordinates.
    fn to_client(window: HWND, rect: RECT) -> RECT {
        let mut corners = [
            POINT {
                x: rect.left,
                y: rect.top,
            },
            POINT {
                x: rect.right,
                y: rect.bottom,
            },
        ];
        unsafe { MapWindowPoints(std::ptr::null_mut(), window, corners.as_mut_ptr(), 2) };
        RECT {
            left: corners[0].x,
            top: corners[0].y,
            right: corners[1].x,
            bottom: corners[1].y,
        }
    }

    /// The composited screen pixel at (`x`, `y`) as RGB.
    fn screen_pixel(x: i32, y: i32) -> Option<[u8; 3]> {
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return None;
            }
            let value = GetPixel(screen, x, y);
            ReleaseDC(std::ptr::null_mut(), screen);
            (value != CLR_INVALID).then_some([
                (value & 0xff) as u8,
                ((value >> 8) & 0xff) as u8,
                ((value >> 16) & 0xff) as u8,
            ])
        }
    }

    struct ColorSample {
        at: (i32, i32),
        color: [u8; 3],
        taken: Instant,
    }

    /// Reads the notification area's island. The Windows 11 tray is XAML without windows of its
    /// own, so its visible place comes from UI Automation; `TrayNotifyWnd` only marks where a stock
    /// taskbar draws it.
    struct Islands {
        automation: Option<Automation>,
        unavailable: bool,
        sample: Option<ColorSample>,
    }

    impl Islands {
        const fn new() -> Self {
            Self {
                automation: None,
                unavailable: false,
                sample: None,
            }
        }

        fn automation(&mut self) -> Option<&Automation> {
            if self.automation.is_none() && !self.unavailable {
                self.automation = create_automation();
                if self.automation.is_none() {
                    self.unavailable = true;
                    tracing::warn!(
                        "UI Automation is unavailable; the taskbar strip stays beside the notification area"
                    );
                }
            }
            self.automation.as_ref()
        }

        /// The tray buttons' combined box and the taskbar frame's box, in screen coordinates.
        fn tray(&mut self, taskbar: HWND) -> Option<(RECT, Option<RECT>)> {
            let site = xaml_site(taskbar)?;
            let automation = self.automation()?;
            let mut tray: Option<RECT> = None;
            let mut frame = None;
            unsafe {
                let root = automation
                    .client
                    .ElementFromHandle(windows::Win32::Foundation::HWND(site))
                    .ok()?;
                let children = root
                    .FindAllBuildCache(TreeScope_Children, &automation.all, &automation.cache)
                    .ok()?;
                for index in 0..children.Length().ok()? {
                    let Ok(child) = children.GetElement(index) else {
                        continue;
                    };
                    let (Ok(class), Ok(bounds)) =
                        (child.CachedClassName(), child.CachedBoundingRectangle())
                    else {
                        continue;
                    };
                    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
                        continue;
                    }
                    let rect = RECT {
                        left: bounds.left,
                        top: bounds.top,
                        right: bounds.right,
                        bottom: bounds.bottom,
                    };
                    let class = class.to_string();
                    if class.starts_with(TRAY_CLASS_PREFIX) {
                        tray = Some(tray.map_or(rect, |tray| union(tray, rect)));
                    } else if class == TASKBAR_FRAME_CLASS {
                        frame = Some(rect);
                    }
                }
            }
            Some((tray?, frame))
        }

        /// The island's fill, sampled where no button draws: above the buttons when the island
        /// has room there, otherwise in the Show Desktop sliver at its right end.
        fn color(&mut self, at: (i32, i32)) -> Option<[u8; 3]> {
            let fresh = self
                .sample
                .as_ref()
                .filter(|sample| sample.at == at && sample.taken.elapsed() < COLOR_REFRESH);
            if let Some(sample) = fresh {
                return Some(sample.color);
            }
            let color = screen_pixel(at.0, at.1)?;
            self.sample = Some(ColorSample {
                at,
                color,
                taken: Instant::now(),
            });
            Some(color)
        }

        /// The tray's island in taskbar client coordinates, or `None` on a stock taskbar, where the
        /// tray still sits at `TrayNotifyWnd`.
        fn read(&mut self, taskbar: &Taskbar, gap: i32) -> Option<Island> {
            let (tray_screen, frame_screen) = self.tray(taskbar.hwnd)?;
            let tray = to_client(taskbar.hwnd, tray_screen);
            if tray.right + gap >= taskbar.notify_left {
                return None;
            }
            let (top, bottom) = frame_screen
                .map(|frame| to_client(taskbar.hwnd, frame))
                .filter(|frame| frame.top <= tray.top && frame.bottom >= tray.bottom)
                .map_or((tray.top, tray.bottom), |frame| (frame.top, frame.bottom));
            let scale = taskbar.info.scale;
            let probe = if top < tray.top {
                (
                    tray_screen.right - (FRAME_RADIUS_POINTS * scale * 2.0).round() as i32,
                    tray_screen.top - (tray.top - top + 1) / 2,
                )
            } else {
                (
                    tray_screen.right - 3,
                    (tray_screen.top + tray_screen.bottom) / 2,
                )
            };
            Some(Island {
                left: tray.left - (ISLAND_INSET_POINTS * scale).round() as i32,
                right: tray.right,
                top,
                bottom,
                color: self.color(probe)?,
            })
        }
    }

    struct Window {
        strip: HWND,
        tooltip: HWND,
        tip: Vec<u16>,
        /// Theme and maximum width (device pixels) last applied to the tooltip.
        tip_style: Option<(TaskbarTheme, isize)>,
    }

    /// What a framed frame was composed for, so it is recomposed only when one of these changes.
    #[derive(Clone, Copy, PartialEq)]
    struct FrameKey {
        height: u32,
        padding: u32,
        radius: f64,
        color: [u8; 3],
    }

    struct State<R: Runtime> {
        app: AppHandle<R>,
        shared: Shared,
        taskbar_created: u32,
        window: Option<Window>,
        bitmap: Option<Bitmap>,
        /// The current frame composed as an island, while the strip sits beside the tray island.
        framed: Option<(FrameKey, Bitmap)>,
        islands: Islands,
        painted: bool,
        placed: Option<(i32, i32, i32, i32)>,
    }

    thread_local! {
        static STATE: RefCell<Option<Box<dyn StripThread>>> = const { RefCell::new(None) };
        static CLICK: RefCell<Option<(ClickHandler, Dispatch)>> = const { RefCell::new(None) };
    }

    /// Object-safe view of the thread state, so the window procedures need no runtime generic.
    trait StripThread {
        fn taskbar_created(&self) -> u32;
        fn apply_pending(&mut self);
        fn sync(&mut self);
        fn window_destroyed(&mut self, hwnd: HWND) -> Option<HWND>;
    }

    impl<R: Runtime> StripThread for State<R> {
        fn taskbar_created(&self) -> u32 {
            self.taskbar_created
        }

        fn apply_pending(&mut self) {
            let update = self
                .shared
                .pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.take());
            if let Some(bitmap) = update {
                self.bitmap = bitmap;
                self.framed = None;
                self.painted = false;
                self.sync();
            }
        }

        fn sync(&mut self) {
            let taskbar = read_taskbar();
            let info = taskbar
                .as_ref()
                .map(|taskbar| taskbar.info.clone())
                .unwrap_or(TaskbarInfo::UNSUPPORTED);
            let changed = self.shared.info.lock().map(|mut current| {
                let changed = *current != info;
                *current = info.clone();
                changed
            });
            if changed.unwrap_or(false)
                && self
                    .app
                    .emit_to("popup", TASKBAR_INFO_EVENT, &info)
                    .is_err()
            {
                tracing::warn!("could not publish taskbar info");
            }
            match (taskbar, &self.bitmap) {
                (Some(taskbar), Some(_)) if taskbar.info.supported => self.show(&taskbar),
                _ => self.close(),
            }
        }

        fn window_destroyed(&mut self, hwnd: HWND) -> Option<HWND> {
            match &self.window {
                Some(window) if window.strip == hwnd => {
                    let tooltip = window.tooltip;
                    self.window = None;
                    self.painted = false;
                    self.placed = None;
                    Some(tooltip)
                }
                _ => None,
            }
        }
    }

    impl<R: Runtime> State<R> {
        fn show(&mut self, taskbar: &Taskbar) {
            let alive = self
                .window
                .as_ref()
                .is_some_and(|window| unsafe { IsWindow(window.strip) } != 0 && unsafe { GetParent(window.strip) } == taskbar.hwnd);
            if !alive {
                self.close();
                self.window = create_window(taskbar.hwnd);
            }
            let (Some(window), Some(content)) = (self.window.as_mut(), self.bitmap.as_ref()) else {
                return;
            };
            let scale = taskbar.info.scale;
            let gap = (GAP_POINTS * scale).round() as i32;
            let padding = (FRAME_PADDING_POINTS * scale).round() as u32;
            let beside = self.islands.read(taskbar, gap).and_then(|island| {
                let width = (content.width + padding * 2) as i32;
                island_strip_origin(taskbar.width, &island, width, gap)
                    .map(|origin| (island, origin))
            });
            let (x, y, bitmap) = match beside {
                Some((island, (x, y))) => {
                    let key = FrameKey {
                        height: (island.bottom - island.top).max(1) as u32,
                        padding,
                        radius: FRAME_RADIUS_POINTS * scale,
                        color: island.color,
                    };
                    if self.framed.as_ref().map(|(current, _)| *current) != Some(key) {
                        self.framed = Some((
                            key,
                            framed(content, key.height, key.padding, key.radius, key.color),
                        ));
                        self.painted = false;
                    }
                    let Some((_, bitmap)) = self.framed.as_ref() else {
                        return;
                    };
                    (x, y, bitmap)
                }
                None => {
                    if self.framed.take().is_some() {
                        self.painted = false;
                    }
                    let (x, y) = strip_origin(
                        taskbar.height,
                        taskbar.notify_left,
                        content.width as i32,
                        content.height as i32,
                        gap,
                    );
                    (x, y, content)
                }
            };
            let width = bitmap.width as i32;
            let height = bitmap.height as i32;
            if self.placed != Some((x, y, width, height)) || !self.painted {
                unsafe {
                    SetWindowPos(
                        window.strip,
                        HWND_TOP,
                        x,
                        y,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    )
                };
                self.placed = Some((x, y, width, height));
            }
            style_tooltip(window, &taskbar.info);
            if !self.painted {
                self.painted = paint(window.strip, bitmap);
                update_tooltip(window, &bitmap.tooltip);
            }
        }

        fn close(&mut self) {
            if let Some(window) = self.window.take() {
                unsafe {
                    if !window.tooltip.is_null() {
                        DestroyWindow(window.tooltip);
                    }
                    DestroyWindow(window.strip);
                }
            }
            self.painted = false;
            self.placed = None;
        }
    }

    fn with_state(action: impl FnOnce(&mut dyn StripThread)) {
        STATE.with(|cell| {
            if let Ok(mut state) = cell.try_borrow_mut()
                && let Some(state) = state.as_mut()
            {
                action(state.as_mut());
            }
        });
    }

    const HOST_CLASS: &str = "QuotaControlTaskbarHost";
    const STRIP_CLASS: &str = "QuotaControlTaskbarStrip";

    fn register_classes() -> bool {
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let host_class = wide(HOST_CLASS);
        let strip_class = wide(STRIP_CLASS);
        let cursor = unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) };
        let mut class: WNDCLASSEXW = unsafe { std::mem::zeroed() };
        class.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        class.hInstance = instance;
        class.hCursor = cursor;
        class.lpfnWndProc = Some(host_proc);
        class.lpszClassName = host_class.as_ptr();
        let host = unsafe { RegisterClassExW(&class) };
        class.lpfnWndProc = Some(strip_proc);
        class.lpszClassName = strip_class.as_ptr();
        let strip = unsafe { RegisterClassExW(&class) };
        host != 0 && strip != 0
    }

    fn create_window(taskbar: HWND) -> Option<Window> {
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = wide(STRIP_CLASS);
        let title = wide("Quota Control");
        let strip = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE,
                class.as_ptr(),
                title.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                taskbar,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if strip.is_null() {
            tracing::warn!(
                "taskbar strip window could not be created: {}",
                std::io::Error::last_os_error()
            );
            return None;
        }
        let tooltip_class = TOOLTIPS_CLASSW;
        let tooltip = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                tooltip_class,
                std::ptr::null(),
                WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                0,
                0,
                0,
                0,
                strip,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        let window = Window {
            strip,
            tooltip,
            tip: wide(""),
            tip_style: None,
        };
        if !tooltip.is_null() {
            let mut info = tool_info(&window);
            unsafe {
                SendMessageW(
                    tooltip,
                    TTM_ADDTOOLW,
                    0,
                    (&mut info as *mut TTTOOLINFOW) as LPARAM,
                )
            };
        }
        Some(window)
    }

    /// Match the tooltip to the taskbar it pops up from: dark over a dark taskbar, and a wrap width
    /// that scales with the taskbar's DPI.
    fn style_tooltip(window: &mut Window, info: &TaskbarInfo) {
        let max_width = (TOOLTIP_MAX_WIDTH_POINTS * info.scale).round() as isize;
        let style = (info.theme, max_width);
        if window.tooltip.is_null() || window.tip_style == Some(style) {
            return;
        }
        let dark = wide(DARK_TOOLTIP_THEME);
        let theme = match info.theme {
            TaskbarTheme::Dark => dark.as_ptr(),
            TaskbarTheme::Light => std::ptr::null(),
        };
        unsafe {
            SetWindowTheme(window.tooltip, theme, std::ptr::null());
            SendMessageW(window.tooltip, TTM_SETMAXTIPWIDTH, 0, max_width);
        }
        window.tip_style = Some(style);
    }

    fn tool_info(window: &Window) -> TTTOOLINFOW {
        let mut info: TTTOOLINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<TTTOOLINFOW>() as u32;
        info.uFlags = TTF_IDISHWND | TTF_SUBCLASS;
        info.hwnd = window.strip;
        info.uId = window.strip as usize;
        info.lpszText = window.tip.as_ptr().cast_mut();
        info
    }

    fn update_tooltip(window: &mut Window, text: &str) {
        if window.tooltip.is_null() {
            return;
        }
        window.tip = wide(text);
        let mut info = tool_info(window);
        unsafe {
            SendMessageW(
                window.tooltip,
                TTM_UPDATETIPTEXTW,
                0,
                (&mut info as *mut TTTOOLINFOW) as LPARAM,
            )
        };
    }

    /// Blit the premultiplied frame into the layered window; `true` when the window took it.
    fn paint(hwnd: HWND, bitmap: &Bitmap) -> bool {
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let memory = CreateCompatibleDC(screen);
            let mut header: BITMAPINFO = std::mem::zeroed();
            header.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bitmap.width as i32,
                biHeight: -(bitmap.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(
                memory,
                &header,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            let mut painted = false;
            if !dib.is_null() && !bits.is_null() {
                std::ptr::copy_nonoverlapping(
                    bitmap.bgra.as_ptr(),
                    bits.cast::<u8>(),
                    bitmap.bgra.len(),
                );
                let previous = SelectObject(memory, dib);
                let size = SIZE {
                    cx: bitmap.width as i32,
                    cy: bitmap.height as i32,
                };
                let source = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                painted = UpdateLayeredWindow(
                    hwnd,
                    screen,
                    std::ptr::null(),
                    &size,
                    memory,
                    &source,
                    0,
                    &blend,
                    ULW_ALPHA,
                ) != 0;
                if !painted {
                    tracing::warn!(
                        "taskbar strip paint failed: {}",
                        std::io::Error::last_os_error()
                    );
                }
                SelectObject(memory, previous);
                DeleteObject(dib);
            }
            DeleteDC(memory);
            ReleaseDC(std::ptr::null_mut(), screen);
            painted
        }
    }

    fn click(hwnd: HWND, button: StripButton) {
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return;
        }
        let click = StripClick {
            button,
            bounds: PhysicalRect {
                position: PhysicalPosition::new(rect.left, rect.top),
                size: PhysicalSize::new(
                    (rect.right - rect.left).max(0) as u32,
                    (rect.bottom - rect.top).max(0) as u32,
                ),
            },
        };
        CLICK.with(|cell| {
            if let Ok(handler) = cell.try_borrow()
                && let Some((on_click, dispatch)) = handler.as_ref()
            {
                let on_click = on_click.clone();
                dispatch(Box::new(move || on_click(click)));
            }
        });
    }

    unsafe extern "system" fn host_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let mut taskbar_created = 0;
        STATE.with(|cell| {
            if let Ok(state) = cell.try_borrow() {
                taskbar_created = state.as_ref().map_or(0, |state| state.taskbar_created());
            }
        });
        match message {
            WM_APP_FRAME => {
                with_state(|state| state.apply_pending());
                0
            }
            WM_TIMER if wparam == SYNC_TIMER => {
                with_state(|state| state.sync());
                0
            }
            WM_SETTINGCHANGE | WM_DISPLAYCHANGE => {
                with_state(|state| state.sync());
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
            _ if taskbar_created != 0 && message == taskbar_created => {
                with_state(|state| state.sync());
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    unsafe extern "system" fn strip_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_LBUTTONUP => {
                click(hwnd, StripButton::Primary);
                0
            }
            WM_RBUTTONUP => {
                click(hwnd, StripButton::Secondary);
                0
            }
            WM_NCDESTROY => {
                let mut orphan_tooltip = None;
                with_state(|state| orphan_tooltip = state.window_destroyed(hwnd));
                if let Some(tooltip) = orphan_tooltip.filter(|tooltip| !tooltip.is_null()) {
                    unsafe { DestroyWindow(tooltip) };
                }
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    fn run<R: Runtime>(app: AppHandle<R>, on_click: ClickHandler, shared: Shared) {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        unsafe {
            let controls = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_WIN95_CLASSES,
            };
            InitCommonControlsEx(&controls);
        }
        if !register_classes() {
            tracing::warn!(
                "taskbar strip classes could not be registered: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = wide(HOST_CLASS);
        let host = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if host.is_null() {
            tracing::warn!(
                "taskbar strip host could not be created: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        let created = wide("TaskbarCreated");
        let taskbar_created = unsafe { RegisterWindowMessageW(created.as_ptr()) };
        let dispatcher = app.clone();
        CLICK.with(|cell| {
            *cell.borrow_mut() = Some((
                on_click,
                Box::new(move |job: Box<dyn FnOnce() + Send>| {
                    if dispatcher.run_on_main_thread(job).is_err() {
                        tracing::warn!("could not dispatch a taskbar strip click");
                    }
                }),
            ));
        });
        shared.host.store(host as isize, Ordering::Release);
        STATE.with(|cell| {
            *cell.borrow_mut() = Some(Box::new(State {
                app,
                shared,
                taskbar_created,
                window: None,
                bitmap: None,
                framed: None,
                islands: Islands::new(),
                painted: false,
                placed: None,
            }));
        });
        unsafe { SetTimer(host, SYNC_TIMER, SYNC_INTERVAL_MS, None) };
        with_state(|state| {
            state.apply_pending();
            state.sync();
        });
        let mut message: MSG = unsafe { std::mem::zeroed() };
        while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::sync::Arc;

    use tauri::{AppHandle, Runtime};

    use super::{Bitmap, StripClick, TaskbarEdge, TaskbarInfo, TaskbarTheme};

    const TRAY_ID: &str = "main";

    /// Linux has no taskbar band to embed in; the strip's text becomes the tray title instead.
    pub struct Strip {
        set_title: Box<dyn Fn(Option<String>) + Send + Sync>,
    }

    impl Strip {
        pub fn start<R: Runtime>(
            app: AppHandle<R>,
            _on_click: Arc<dyn Fn(StripClick) + Send + Sync>,
        ) -> Self {
            Self {
                set_title: Box::new(move |title| {
                    if let Some(tray) = app.tray_by_id(TRAY_ID)
                        && tray.set_title(title).is_err()
                    {
                        tracing::warn!("could not set the tray title");
                    }
                }),
            }
        }

        pub fn info(&self) -> TaskbarInfo {
            TaskbarInfo {
                supported: true,
                height: 24,
                scale: 1.0,
                theme: TaskbarTheme::Dark,
                edge: TaskbarEdge::Top,
            }
        }

        pub fn set(&self, bitmap: Option<Bitmap>) {
            (self.set_title)(
                bitmap
                    .map(|bitmap| bitmap.text)
                    .filter(|text| !text.is_empty()),
            );
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use tauri::image::Image;
    use tauri::{AppHandle, Runtime};

    use super::{Bitmap, StripClick, TaskbarEdge, TaskbarInfo, TaskbarTheme};

    const TRAY_ID: &str = "main";
    /// tray-icon draws every status item image this many points tall. Its plain `set_icon` also
    /// clears the template flag, which leaves the black drawing black on a dark menu bar, so every
    /// image goes through `set_icon_with_as_template`.
    const ICON_POINTS: f64 = 18.0;

    #[derive(Default)]
    struct Images {
        strip: Option<(Image<'static>, String)>,
        glyph: Option<Image<'static>>,
        tooltip: String,
    }

    type Show = Box<dyn Fn(Option<Image<'static>>, String) + Send + Sync>;

    pub struct Strip {
        images: Mutex<Images>,
        scale: f64,
        show: Show,
    }

    impl Strip {
        pub fn start<R: Runtime>(
            app: AppHandle<R>,
            _on_click: Arc<dyn Fn(StripClick) + Send + Sync>,
        ) -> Self {
            let scale = app
                .primary_monitor()
                .ok()
                .flatten()
                .map(|monitor| monitor.scale_factor())
                .filter(|scale| scale.is_finite() && *scale >= 1.0)
                .unwrap_or(2.0);
            Self {
                images: Mutex::new(Images::default()),
                scale,
                show: Box::new(move |image, tooltip| {
                    let Some(tray) = app.tray_by_id(TRAY_ID) else {
                        return;
                    };
                    let image = image.or_else(|| crate::menu_bar_icon().ok());
                    if tray.set_icon_with_as_template(image, true).is_err()
                        || tray
                            .set_tooltip(Some(tooltip).filter(|tip| !tip.is_empty()))
                            .is_err()
                    {
                        tracing::warn!("could not update the menu bar item");
                    }
                }),
            }
        }

        pub fn info(&self) -> TaskbarInfo {
            TaskbarInfo {
                supported: true,
                height: (ICON_POINTS * self.scale).round() as u32,
                scale: self.scale,
                theme: TaskbarTheme::Light,
                edge: TaskbarEdge::Top,
            }
        }

        pub fn set(&self, bitmap: Option<Bitmap>) {
            let strip = bitmap.map(|bitmap| {
                let image = Image::new_owned(bitmap.straight_rgba(), bitmap.width, bitmap.height);
                (image, bitmap.tooltip)
            });
            self.images.lock().strip = strip;
            self.apply();
        }

        pub fn set_glyph(&self, glyph: Option<Image<'static>>, tooltip: String) {
            let mut images = self.images.lock();
            images.glyph = glyph;
            images.tooltip = tooltip;
            drop(images);
            self.apply();
        }

        fn apply(&self) {
            let (image, tooltip) = {
                let images = self.images.lock();
                match &images.strip {
                    Some((image, tooltip)) => (Some(image.clone()), tooltip.clone()),
                    None => (images.glyph.clone(), images.tooltip.clone()),
                }
            };
            (self.show)(image, tooltip);
        }
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    use std::sync::Arc;

    use tauri::{AppHandle, Runtime};

    use super::{Bitmap, StripClick, TaskbarInfo};

    pub struct Strip;

    impl Strip {
        pub fn start<R: Runtime>(
            _app: AppHandle<R>,
            _on_click: Arc<dyn Fn(StripClick) + Send + Sync>,
        ) -> Self {
            Self
        }

        pub fn info(&self) -> TaskbarInfo {
            TaskbarInfo::UNSUPPORTED
        }

        pub fn set(&self, _bitmap: Option<Bitmap>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x1 RGBA PNG: one opaque white pixel, one fully transparent pixel.
    const TWO_PIXELS: [u8; 72] = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 244, 34, 127, 138, 0, 0, 0, 15, 73, 68, 65, 84, 120, 218, 99, 248, 15, 4, 12, 12,
        12, 12, 0, 25, 239, 3, 253, 140, 168, 185, 31, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
        130,
    ];

    fn frame() -> StripFrame {
        StripFrame {
            png: TWO_PIXELS.to_vec(),
            width: 2,
            height: 1,
            text: "Claude 12%".into(),
            tooltip: "Usage".into(),
        }
    }

    #[test]
    fn premultiplies_into_bgra_and_keeps_transparent_pixels_clickable() {
        let bgra = premultiplied_bgra(&[255, 128, 0, 128, 10, 20, 30, 0, 255, 255, 255, 255]);
        assert_eq!(&bgra[0..4], &[0, 64, 128, 128]);
        assert_eq!(&bgra[4..8], &[0, 0, 0, 1]);
        assert_eq!(&bgra[8..12], &[255, 255, 255, 255]);
    }

    #[test]
    fn decodes_a_valid_frame() {
        let bitmap = decode_frame(&frame()).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (2, 1));
        assert_eq!(bitmap.bgra, vec![255, 255, 255, 255, 0, 0, 0, 1]);
        assert_eq!(bitmap.text, "Claude 12%");
    }

    #[test]
    fn rejects_frames_that_lie_about_their_size_or_are_not_png() {
        let mut lying = frame();
        lying.width = 3;
        assert!(decode_frame(&lying).is_err());
        let mut garbage = frame();
        garbage.png = vec![0; 64];
        assert!(decode_frame(&garbage).is_err());
        let mut huge = frame();
        huge.height = MAX_FRAME_HEIGHT + 1;
        assert!(decode_frame(&huge).is_err());
        let mut chatty = frame();
        chatty.tooltip = "x".repeat(MAX_TOOLTIP_CHARS + 1);
        assert!(decode_frame(&chatty).is_err());
    }

    #[test]
    fn anchors_left_of_the_notification_area_and_centers_vertically() {
        assert_eq!(strip_origin(56, 1598, 180, 56, 4), (1414, 0));
        assert_eq!(strip_origin(56, 1598, 180, 48, 4), (1414, 4));
        assert_eq!(strip_origin(48, 100, 180, 48, 4), (0, 0));
    }

    const ISLAND: Island = Island {
        left: 22,
        right: 376,
        top: 4,
        bottom: 52,
        color: [35, 41, 61],
    };

    #[test]
    fn sits_right_after_the_tray_island_and_falls_back_before_it() {
        assert_eq!(island_strip_origin(1920, &ISLAND, 203, 4), Some((380, 4)));
        assert_eq!(island_strip_origin(560, &ISLAND, 203, 4), None);
        let right_edge = Island {
            left: 1566,
            right: 1920,
            ..ISLAND
        };
        assert_eq!(
            island_strip_origin(1920, &right_edge, 203, 4),
            Some((1359, 4))
        );
    }

    fn bitmap(width: u32, height: u32, pixel: [u8; 4]) -> Bitmap {
        Bitmap {
            width,
            height,
            bgra: pixel.repeat((width * height) as usize),
            text: "Claude 12%".into(),
            tooltip: "Usage".into(),
        }
    }

    #[test]
    fn frames_the_content_as_a_rounded_island_and_clips_its_margins() {
        let pixel = |image: &Bitmap, x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            image.bgra[index..index + 4].to_vec()
        };
        let transparent = framed(&bitmap(10, 12, [0, 0, 0, 1]), 8, 3, 4.0, [35, 41, 61]);
        assert_eq!((transparent.width, transparent.height), (16, 8));
        assert_eq!(transparent.text, "Claude 12%");
        assert_eq!(pixel(&transparent, 8, 4), vec![61, 41, 35, 255]);
        assert_eq!(pixel(&transparent, 1, 4), vec![61, 41, 35, 255]);
        assert_eq!(pixel(&transparent, 0, 0)[3], 0);
        assert_eq!(pixel(&transparent, 15, 7)[3], 0);
        assert!(pixel(&transparent, 1, 1)[3] > 0 && pixel(&transparent, 1, 1)[3] < 255);
        let white = framed(
            &bitmap(10, 12, [255, 255, 255, 255]),
            8,
            3,
            4.0,
            [35, 41, 61],
        );
        assert_eq!(pixel(&white, 3, 0), vec![255, 255, 255, 255]);
        assert_eq!(pixel(&white, 12, 7), vec![255, 255, 255, 255]);
        assert_eq!(pixel(&white, 2, 4), vec![61, 41, 35, 255]);
    }

    #[test]
    fn serializes_info_for_the_popup() {
        let info = TaskbarInfo {
            supported: true,
            height: 56,
            scale: 1.25,
            theme: TaskbarTheme::Dark,
            edge: TaskbarEdge::Bottom,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "supported": true, "height": 56, "scale": 1.25, "theme": "dark", "edge": "bottom" })
        );
    }
}
