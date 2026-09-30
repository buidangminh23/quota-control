//! The live usage strip on the taskbar (upstream's menu-bar text item). The popup renders each frame
//! as a PNG sized for the taskbar band; this module hosts it.
//!
//! Windows: a layered, never-activating child window inside `Shell_TrayWnd`, immediately left of the
//! notification area. When a taskbar styler draws the notification area as its own island away from
//! the taskbar's edge (Windhawk's centered taskbar), the strip becomes a matching island right after
//! it instead. When a styler rule widens the app's own notification-area button to fit the strip
//! (Windhawk's Taskbar Styler, by the button's name [`SLOT_NAME`]), the strip covers that button and
//! sits inside the notification area itself, on its real background. The button then shows a clear
//! icon as wide as the strip needs, so a rule that sizes the button by its image makes the
//! notification area grow and shrink with the strip; under a rule that fixes the button's width the
//! strip shrinks a little when it grows wider than the button. It only leaves the button for the
//! island placement, never for a spot over the notification area's own buttons. It is owned by one
//! dedicated
//! thread with its own message loop, re-anchors on a one-second timer, rebuilds itself after Explorer
//! restarts (`TaskbarCreated`) and reports taskbar size, scale and theme changes to the popup as
//! `taskbar-info`.
//! Linux: the frame's text becomes the tray title.
//! macOS: the frame becomes the menu bar item's image itself, drawn in color like the Windows taskbar
//! and the Linux panel, its text in the menu bar's own light or dark color. The frame carries a
//! description of the strip beside its picture (`src/strip/native.ts`), and the system draws the
//! strip from that (`macos/Host/MenuBarStrip.swift`): sharp on every display, in the menu bar's look
//! the moment it changes, its readings open to VoiceOver. The picture shows when the description
//! cannot be drawn. The Bars glyph and the plain icon stay templates the system tints. The three
//! share that one image, so the strip wins over the glyph and the glyph over the icon.
//! Other platforms report the strip unsupported.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, PhysicalRect, Runtime, State};

/// The tray icon's tooltip while the strip shows, which Windows also gives its notification-area
/// button as a name. Two taskbar styler rules find the button by this name and let it grow with the
/// strip: `SystemTray.NotifyIconView#NotifyItemIcon[AutomationProperties.Name=Quota Control]` with
/// `Width=Auto` and `MinWidth=88`, and the same target followed by ` > * > Image` with `Width=Auto`,
/// `Height=16` and `Stretch=Uniform`, so the button is as wide as the clear icon the app gives it
/// (see [`TraySlot::Clear`]). A fixed `Width=` on the button also works, without the growing.
#[cfg_attr(not(windows), allow(dead_code))]
pub const SLOT_NAME: &str = "Quota Control";

/// What the app's notification-area button shows while the strip may sit inside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TraySlot {
    /// The app icon, or the Bars glyph: the strip is elsewhere or gone.
    #[default]
    Icon,
    /// A clear icon `width` points wide and [`SLOT_ICON_HEIGHT`] tall. Under the styler rules in
    /// [`SLOT_NAME`] the button's image keeps that aspect ratio at 16 points tall, so the button
    /// becomes as wide as the strip needs; a rule that fixes the button's width ignores it. Only the
    /// Windows strip asks for it.
    #[cfg_attr(not(windows), allow(dead_code))]
    Clear { width: u32 },
}

/// Height, in points, of the clear icon that sizes the app's button.
pub const SLOT_ICON_HEIGHT: u32 = 16;

/// Points the button's width moves by, so a reading gaining a digit rarely resizes the
/// notification area.
#[cfg_attr(not(windows), allow(dead_code))]
const SLOT_STEP: u32 = 8;

/// A narrower strip gives points back only once it frees this many, so the button doesn't flap
/// between two widths while a reading moves across a digit.
#[cfg_attr(not(windows), allow(dead_code))]
const SLOT_SHRINK_SLACK: u32 = 16;

/// Largest frame the popup may send, in device pixels.
const MAX_FRAME_WIDTH: u32 = 4096;
const MAX_FRAME_HEIGHT: u32 = 512;
const MAX_PNG_BYTES: usize = 4 * 1_048_576;
const MAX_TEXT_CHARS: usize = 512;
const MAX_TOOLTIP_CHARS: usize = 1024;
/// Largest description of the strip the popup may send: the marks' paths and color logos make up
/// nearly all of it.
const MAX_NATIVE_BYTES: usize = 1_048_576;
const NATIVE_VERSION: u64 = 1;
/// Popup event carrying a changed [`TaskbarInfo`].
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
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
    /// The same strip as a description the system can draw itself (macOS).
    #[serde(default)]
    pub native: Option<serde_json::Value>,
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
    /// When the button went down on the strip, if it did.
    pub pressed_at: Option<std::time::Instant>,
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

/// Validate the strip's description and serialize it for the platform that draws it.
pub fn encode_native(document: &serde_json::Value) -> Result<Vec<u8>, String> {
    use serde_json::Value;
    if document.get("version").and_then(Value::as_u64) != Some(NATIVE_VERSION) {
        return Err("Unsupported strip description".into());
    }
    if !document
        .get("groups")
        .and_then(Value::as_array)
        .is_some_and(|groups| !groups.is_empty())
    {
        return Err("A strip description lists what it shows".into());
    }
    let bytes = serde_json::to_vec(document)
        .map_err(|_| "The strip description cannot be written".to_string())?;
    if bytes.len() > MAX_NATIVE_BYTES {
        return Err("The strip description is too large".into());
    }
    Ok(bytes)
}

impl Bitmap {
    /// Back to straight RGBA, for platforms that take the frame as an ordinary image.
    #[cfg_attr(windows, allow(dead_code))]
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

/// Whether a taskbar styler has moved the notification area away from `TrayNotifyWnd`, where a
/// stock taskbar draws it: its buttons then start left of that window (`tray_left` and `notify_left`
/// in the same coordinates). The left end decides, because a styler that also widens a button can
/// push the island's right end past the window's left edge.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn tray_moved(tray_left: i32, notify_left: i32, gap: i32) -> bool {
    tray_left + gap < notify_left
}

/// How far the notification area's island reaches above and below its buttons (`tray` top and
/// bottom), read from the taskbar frame's top and bottom while the frame stays centered on the
/// buttons as the island is. A frame that has grown lopsided, as while a dock animation magnifies
/// the app buttons, says nothing about the island.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn island_reach(frame: (i32, i32), tray: (i32, i32)) -> Option<(i32, i32)> {
    let ((frame_top, frame_bottom), (tray_top, tray_bottom)) = (frame, tray);
    let centered = ((frame_top + frame_bottom) - (tray_top + tray_bottom)).abs() <= 1;
    (centered && frame_top <= tray_top && frame_bottom >= tray_bottom)
        .then_some((tray_top - frame_top, frame_bottom - tray_bottom))
}

/// The island fill the strip paints, read off the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Fill {
    pub color: [u8; 3],
    /// Whether `color` came from a reading nothing could have tinted.
    pub settled: bool,
    /// A differing reading waiting for the next one to confirm it.
    pending: Option<[u8; 3]>,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Fill {
    /// Largest per-channel difference between two readings of the same fill.
    const TOLERANCE: u8 = 3;

    pub fn new(color: [u8; 3], settled: bool) -> Self {
        Self {
            color,
            settled,
            pending: None,
        }
    }

    /// Take a clean reading. An unsettled fill adopts it at once; a settled one adopts a differing
    /// reading only when the next reading agrees with it, so a single tinted reading never shows.
    pub fn read(&mut self, reading: [u8; 3]) {
        let similar = |one: [u8; 3], two: [u8; 3]| {
            one.iter()
                .zip(two)
                .all(|(a, b)| a.abs_diff(b) <= Self::TOLERANCE)
        };
        if !self.settled
            || self
                .pending
                .is_some_and(|pending| similar(pending, reading))
        {
            *self = Self::new(reading, true);
        } else if similar(reading, self.color) {
            self.pending = None;
        } else {
            self.pending = Some(reading);
        }
    }

    /// Let the next clean reading replace the fill, as after a theme or display change.
    pub fn unsettle(&mut self) {
        self.settled = false;
    }
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

/// Least the strip may shrink to fit the app's widened notification-area button. A button too
/// narrow for that leaves the strip beside the notification area, at full size.
#[cfg_attr(not(windows), allow(dead_code))]
pub const MIN_SLOT_SCALE: f64 = 0.75;

/// The part of `content` that shows anything, as (`left`, `top`, `right`, `bottom`) with `right` and
/// `bottom` exclusive: pixels above the alpha of 1 that keeps clear pixels clickable. Frames are as
/// tall as the taskbar band, so the rows above and below their text are clear.
fn ink(content: &Bitmap) -> Option<(u32, u32, u32, u32)> {
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for y in 0..content.height {
        for x in 0..content.width {
            if content.bgra[((y * content.width + x) * 4 + 3) as usize] > 1 {
                bounds = Some(
                    bounds.map_or((x, y, x + 1, y + 1), |(left, top, right, bottom)| {
                        (left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1))
                    }),
                );
            }
        }
    }
    bounds
}

/// Space kept clear inside the button at each end when the strip has to shrink, so it never
/// touches the button's rounded edges.
fn slot_margin(height: u32) -> u32 {
    (height / 10).max(2)
}

/// How much `content` has to shrink to fit a `width` x `height` button: 1 while what it shows fits,
/// less when that is wider or taller than the button (keeping [`slot_margin`] clear at each end).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn slot_scale(content: &Bitmap, width: u32, height: u32) -> f64 {
    let (left, top, right, bottom) = ink(content).unwrap_or((0, 0, content.width, content.height));
    let (shown_width, shown_height) = ((right - left).max(1), (bottom - top).max(1));
    if shown_width <= width && shown_height <= height {
        return 1.0;
    }
    let room = width.saturating_sub(slot_margin(height) * 2);
    (f64::from(room) / f64::from(shown_width))
        .min(f64::from(height) / f64::from(shown_height))
        .min(1.0)
}

/// Whether a taskbar styler widened the app's notification-area button, `width` x `height`, on
/// purpose: icon buttons are about square, so it must be at least twice as wide as tall.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn slot_capable(width: i32, height: i32) -> bool {
    height > 0 && width >= height * 2
}

/// Whether the app's notification-area button, `width` x `height`, holds the strip: it must be
/// widened for it ([`slot_capable`]), and the strip must keep at least [`MIN_SLOT_SCALE`] of its size
/// inside it.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn slot_holds(content: &Bitmap, width: i32, height: i32) -> bool {
    slot_capable(width, height)
        && slot_scale(content, width as u32, height as u32) >= MIN_SLOT_SCALE
}

/// Points the app's button must be wide to hold `content` at full size, with the margin [`slotted`]
/// keeps at each end of a `height`-pixel button, rounded up to [`SLOT_STEP`]; `scale` is device
/// pixels per point.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn slot_width_for(content: &Bitmap, height: u32, scale: f64) -> u32 {
    let (left, _, right, _) = ink(content).unwrap_or((0, 0, content.width, content.height));
    let pixels = (right - left) + slot_margin(height) * 2;
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let points = (f64::from(pixels) / scale).ceil() as u32;
    points.div_ceil(SLOT_STEP) * SLOT_STEP
}

/// The width to ask of the button when the strip wants `wanted` points and `current` was asked for
/// last: grow at once, shrink only past [`SLOT_SHRINK_SLACK`].
#[cfg_attr(not(windows), allow(dead_code))]
pub fn next_slot_width(current: Option<u32>, wanted: u32) -> u32 {
    match current {
        Some(current) if wanted <= current && current - wanted < SLOT_SHRINK_SLACK => current,
        _ => wanted,
    }
}

/// `region` of `content` (`left`, `top`, `right`, `bottom`, the last two exclusive) resampled to
/// `width` x `height` by averaging the premultiplied pixels each target pixel covers, so shrunk
/// text stays smooth.
fn resized(content: &Bitmap, region: (u32, u32, u32, u32), width: u32, height: u32) -> Bitmap {
    let (left, top, right, bottom) = region;
    let step_x = f64::from(right - left) / f64::from(width);
    let step_y = f64::from(bottom - top) / f64::from(height);
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for target_y in 0..height {
        let from_y = f64::from(top) + f64::from(target_y) * step_y;
        let to_y = from_y + step_y;
        for target_x in 0..width {
            let from_x = f64::from(left) + f64::from(target_x) * step_x;
            let to_x = from_x + step_x;
            let mut sums = [0.0_f64; 4];
            let mut area = 0.0;
            let mut y = from_y.floor() as u32;
            while f64::from(y) < to_y && y < bottom {
                let cover_y = to_y.min(f64::from(y + 1)) - from_y.max(f64::from(y));
                let mut x = from_x.floor() as u32;
                while f64::from(x) < to_x && x < right {
                    let cover = (to_x.min(f64::from(x + 1)) - from_x.max(f64::from(x))) * cover_y;
                    let index = ((y * content.width + x) * 4) as usize;
                    for (sum, value) in sums.iter_mut().zip(&content.bgra[index..index + 4]) {
                        *sum += f64::from(*value) * cover;
                    }
                    area += cover;
                    x += 1;
                }
                y += 1;
            }
            bgra.extend(sums.map(|sum| (sum / area).round().clamp(0.0, 255.0) as u8));
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

/// The strip inside the app's widened notification-area button: `content` centered in a `width` x
/// `height` box that is otherwise clear, so the notification area's own background shows through.
/// Clear pixels keep an alpha of 1 so the whole button stays the strip's to click. Clear rows and
/// columns outside the box are clipped; when what the strip shows is wider or taller than the box,
/// that part shrinks to fit (see [`slot_scale`]) so the strip never has to leave the button.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn slotted(content: &Bitmap, width: u32, height: u32) -> Bitmap {
    let scale = slot_scale(content, width, height);
    let shrunk = (scale < 1.0).then(|| {
        let region = ink(content).unwrap_or((0, 0, content.width, content.height));
        let (left, top, right, bottom) = region;
        resized(
            content,
            region,
            ((f64::from(right - left) * scale).floor() as u32).max(1),
            ((f64::from(bottom - top) * scale).floor() as u32).max(1),
        )
    });
    let content = shrunk.as_ref().unwrap_or(content);
    let mut bgra = [0, 0, 0, 1].repeat((width * height) as usize);
    let shift_x = (i64::from(width) - i64::from(content.width)) / 2;
    let shift_y = (i64::from(height) - i64::from(content.height)) / 2;
    for row in 0..content.height {
        let Ok(y) = u32::try_from(i64::from(row) + shift_y) else {
            continue;
        };
        if y >= height {
            continue;
        }
        for column in 0..content.width {
            let Ok(x) = u32::try_from(i64::from(column) + shift_x) else {
                continue;
            };
            if x >= width {
                continue;
            }
            let source = ((row * content.width + column) * 4) as usize;
            let target = ((y * width + x) * 4) as usize;
            bgra[target..target + 4].copy_from_slice(&content.bgra[source..source + 4]);
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
    /// Start the strip. `on_click` runs on the main thread, and so does `on_cover`, which hears what
    /// the app's notification-area button should show: a clear icon sized for the strip while the
    /// strip sits inside it or asks it to grow, so nothing of the icon peeks out from under the
    /// strip, or the app icon.
    pub fn install<R: Runtime>(
        app: &AppHandle<R>,
        on_click: impl Fn(StripClick) + Send + Sync + 'static,
        on_cover: impl Fn(TraySlot) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: platform::Strip::start(
                app.clone(),
                std::sync::Arc::new(on_click),
                std::sync::Arc::new(on_cover),
            ),
        }
    }

    pub fn info(&self) -> TaskbarInfo {
        self.inner.info()
    }

    pub fn set(&self, bitmap: Option<Bitmap>) {
        self.inner.set(bitmap);
    }

    /// The description of the strip the next frame comes with, which macOS draws itself in place
    /// of the frame's picture. Other systems show the picture.
    pub fn set_native(&self, document: Option<Vec<u8>>) {
        #[cfg(target_os = "macos")]
        self.inner.set_native(document);
        #[cfg(not(target_os = "macos"))]
        let _ = document;
    }

    /// The tray icon glyph (the Bars style), or `None` for the app icon, with its tooltip. The
    /// macOS menu bar and the Linux panel show the strip as the tray image itself, so the strip
    /// decides which of them shows; on Windows the caller sets the tray icon directly.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn set_glyph(&self, glyph: Option<tauri::image::Image<'static>>, tooltip: String) {
        self.inner.set_glyph(glyph, tooltip);
    }

    /// Whether the popup is open. Its shadow reaches onto the taskbar, so the strip takes no color
    /// readings from the taskbar meanwhile.
    pub fn set_popup_visible(&self, visible: bool) {
        self.inner.set_popup_visible(visible);
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
    let native = frame
        .as_ref()
        .and_then(|frame| frame.native.as_ref())
        .and_then(|document| match encode_native(document) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                tracing::warn!("the strip's picture shows without its description: {error}");
                None
            }
        });
    strip.set_native(native);
    strip.set(bitmap);
    Ok(())
}

#[cfg(windows)]
mod platform {
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use tauri::{AppHandle, Emitter, PhysicalPosition, PhysicalRect, PhysicalSize, Runtime};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
        CLR_INVALID, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
        GetDC, GetMonitorInfoW, GetPixel, MONITOR_DEFAULTTONEAREST, MONITORINFO, MapWindowPoints,
        MonitorFromWindow, ReleaseDC, SelectObject,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows_sys::Win32::UI::Controls::{
        ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, SetWindowTheme,
        TOOLTIPS_CLASSW, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH,
        TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
    };
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowExW,
        FindWindowW, GW_CHILD, GetMessageW, GetParent, GetWindow, GetWindowRect, HWND_TOP,
        IDC_ARROW, IsChild, IsWindow, LoadCursorW, MA_NOACTIVATE, MSG, PostMessageW,
        RegisterClassExW, RegisterWindowMessageW, SWP_ASYNCWINDOWPOS, SWP_HIDEWINDOW,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SendMessageW, SetTimer,
        SetWindowPos, TranslateMessage, ULW_ALPHA, UpdateLayeredWindow, WM_APP, WM_DISPLAYCHANGE,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_NCDESTROY, WM_RBUTTONUP,
        WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_NOPARENTNOTIFY, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        WS_VISIBLE, WindowFromPoint,
    };

    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationCacheRequest,
        IUIAutomationCondition, TreeScope_Descendants, UIA_AutomationIdPropertyId,
        UIA_BoundingRectanglePropertyId, UIA_ClassNamePropertyId, UIA_NamePropertyId,
    };
    use windows::core::Interface;

    use super::{
        Bitmap, Fill, Island, SLOT_NAME, StripButton, StripClick, TASKBAR_INFO_EVENT, TaskbarEdge,
        TaskbarInfo, TaskbarTheme, TraySlot, framed, island_reach, island_strip_origin,
        next_slot_width, slot_capable, slot_holds, slot_width_for, slotted, strip_origin,
        tray_moved,
    };

    const WM_APP_FRAME: u32 = WM_APP + 1;
    const WM_APP_TASKBAR_CHANGED: u32 = WM_APP + 2;
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
    /// UI Automation id of every notification-area icon button; [`SLOT_NAME`] tells the app's apart.
    const SLOT_AUTOMATION_ID: &str = "NotifyItemIcon";
    const AUTOMATION_TIMEOUT_MS: u32 = 1000;
    /// Maximum age of a cached notification-area layout from the current display generation.
    const LAYOUT_GRACE: Duration = Duration::from_secs(10);
    /// Passes in a row without the widened button before the strip leaves it, so a button being laid
    /// out again never makes the strip jump out and back.
    const SLOT_EXIT_PASSES: u8 = 2;
    /// Passes in a row that the strip, asking a widened button to grow, still doesn't fit it before
    /// the button gets the app icon back: a styler rule that fixes the button's width never lets it
    /// grow.
    const SLOT_STRETCH_PASSES: u8 = 3;
    const COLOR_REFRESH: Duration = Duration::from_secs(30);
    const TOOLTIP_MAX_WIDTH_POINTS: f64 = 360.0;
    /// The dark common-controls theme Explorer itself uses for tooltips over a dark taskbar.
    const DARK_TOOLTIP_THEME: &str = "DarkMode_Explorer";

    type ClickHandler = Arc<dyn Fn(StripClick) + Send + Sync>;
    type CoverHandler = Arc<dyn Fn(TraySlot) + Send + Sync>;
    /// Hands a click job to the main thread (window procedures must never block on Tauri).
    type Dispatch = Box<dyn Fn(Box<dyn FnOnce() + Send>) + Send>;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct Strip {
        host: Arc<AtomicIsize>,
        pending: Arc<Mutex<Option<Option<Bitmap>>>>,
        info: Arc<Mutex<TaskbarInfo>>,
        popup: Arc<AtomicBool>,
    }

    impl Strip {
        pub fn start<R: Runtime>(
            app: AppHandle<R>,
            on_click: ClickHandler,
            on_cover: CoverHandler,
        ) -> Self {
            let host = Arc::new(AtomicIsize::new(0));
            let pending = Arc::new(Mutex::new(None));
            let info = Arc::new(Mutex::new(
                read_taskbar()
                    .map(|taskbar| taskbar.info)
                    .unwrap_or(TaskbarInfo::UNSUPPORTED),
            ));
            let popup = Arc::new(AtomicBool::new(false));
            let thread = Shared {
                host: host.clone(),
                pending: pending.clone(),
                info: info.clone(),
                popup: popup.clone(),
            };
            let spawned = std::thread::Builder::new()
                .name("taskbar-strip".into())
                .spawn(move || run(app, on_click, on_cover, thread));
            if let Err(error) = spawned {
                tracing::warn!("taskbar strip thread failed to start: {error}");
            }
            Self {
                host,
                pending,
                info,
                popup,
            }
        }

        pub fn set_popup_visible(&self, visible: bool) {
            self.popup.store(visible, Ordering::Release);
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
        /// Whether the popup is open, which puts its shadow on the taskbar.
        popup: Arc<AtomicBool>,
    }

    /// The taskbar as read now: its window, the notification area's left edge in taskbar client
    /// coordinates, and the info the popup renders against.
    struct Taskbar {
        hwnd: HWND,
        origin: (i32, i32),
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

    fn read_edge(hwnd: HWND, rect: &RECT) -> TaskbarEdge {
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if monitor.is_null() || unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return TaskbarEdge::Bottom;
        }
        if rect.right - rect.left >= rect.bottom - rect.top {
            if (rect.top - info.rcMonitor.top).abs() < (rect.bottom - info.rcMonitor.bottom).abs() {
                TaskbarEdge::Top
            } else {
                TaskbarEdge::Bottom
            }
        } else if (rect.left - info.rcMonitor.left).abs()
            < (rect.right - info.rcMonitor.right).abs()
        {
            TaskbarEdge::Left
        } else {
            TaskbarEdge::Right
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
        let edge = read_edge(hwnd, &rect);
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
            origin: (rect.left, rect.top),
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
            cache.AddProperty(UIA_AutomationIdPropertyId).ok()?;
            cache.AddProperty(UIA_NamePropertyId).ok()?;
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

    /// Whether the screen pixel at (`x`, `y`) shows the taskbar rather than a window over it, such
    /// as a screenshot tool's dimmed overlay or a full-screen app.
    fn taskbar_shows(taskbar: HWND, x: i32, y: i32) -> bool {
        let window = unsafe { WindowFromPoint(POINT { x, y }) };
        window == taskbar || (!window.is_null() && unsafe { IsChild(taskbar, window) } != 0)
    }

    struct ColorSample {
        at: (i32, i32),
        fill: Fill,
        taken: Instant,
    }

    /// The notification area as UI Automation reports it, in screen coordinates.
    #[derive(Clone, Copy)]
    struct Layout {
        /// The notification-area buttons' combined box.
        tray: RECT,
        /// The taskbar frame's box, as tall as the notification area's island on styled taskbars.
        frame: Option<RECT>,
        /// The app's own notification-area button.
        slot: Option<RECT>,
        apps: Option<RECT>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct LayoutKey {
        taskbar: isize,
        origin: (i32, i32),
        geometry: (i32, i32, i32),
        scale: u64,
        generation: u64,
    }

    impl LayoutKey {
        fn new(taskbar: &Taskbar, generation: u64) -> Self {
            Self {
                taskbar: taskbar.hwnd as isize,
                origin: taskbar.origin,
                geometry: (taskbar.notify_left, taskbar.width, taskbar.height),
                scale: taskbar.info.scale.to_bits(),
                generation,
            }
        }
    }

    struct LayoutResult {
        key: LayoutKey,
        layout: Option<Layout>,
        read: Instant,
    }

    #[derive(Default)]
    struct LayoutMailbox {
        request: Option<LayoutKey>,
        result: Option<LayoutResult>,
    }

    struct LayoutReader {
        mailbox: Arc<Mutex<LayoutMailbox>>,
        wake: mpsc::SyncSender<()>,
        stopped: Arc<AtomicBool>,
    }

    impl LayoutReader {
        fn start() -> Self {
            let mailbox = Arc::new(Mutex::new(LayoutMailbox::default()));
            let stopped = Arc::new(AtomicBool::new(false));
            let (wake, events) = mpsc::sync_channel(1);
            let worker_mailbox = mailbox.clone();
            let worker_stopped = stopped.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("taskbar-layout".into())
                .spawn(move || {
                    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
                        tracing::warn!("taskbar layout COM initialization failed");
                        return;
                    }
                    {
                        let automation = create_automation();
                        if automation.is_none() {
                            tracing::warn!("taskbar layout UI Automation is unavailable");
                        }
                        while events.recv().is_ok() && !worker_stopped.load(Ordering::Acquire) {
                            let key = worker_mailbox
                                .lock()
                                .ok()
                                .and_then(|mut mailbox| mailbox.request.take());
                            let Some(key) = key else { continue };
                            let layout = automation.as_ref().and_then(|automation| {
                                read_layout(automation, key.taskbar as HWND)
                            });
                            if let Ok(mut mailbox) = worker_mailbox.lock() {
                                mailbox.result = Some(LayoutResult {
                                    key,
                                    layout,
                                    read: Instant::now(),
                                });
                            }
                        }
                    }
                    unsafe { CoUninitialize() };
                })
            {
                tracing::warn!("taskbar layout reader failed to start: {error}");
            }
            Self {
                mailbox,
                wake,
                stopped,
            }
        }

        fn poll(&self, key: LayoutKey) -> Option<LayoutResult> {
            let result = self.mailbox.try_lock().ok().and_then(|mut mailbox| {
                mailbox.request = Some(key);
                mailbox.result.take().filter(|result| result.key == key)
            });
            let _ = self.wake.try_send(());
            result
        }
    }

    impl Drop for LayoutReader {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Release);
            let _ = self.wake.try_send(());
        }
    }

    /// Reads the notification area's island. The Windows 11 tray is XAML without windows of its
    /// own, so its visible place comes from UI Automation; `TrayNotifyWnd` only marks where a stock
    /// taskbar draws it.
    struct Islands {
        reader: LayoutReader,
        generation: u64,
        key: Option<LayoutKey>,
        sample: Option<ColorSample>,
        /// The last layout read, when, and the taskbar/display generation it was read against.
        last: Option<(Layout, Instant, LayoutKey)>,
        /// How far the island reaches above and below the tray buttons, as last read from a frame
        /// centered on them.
        reach: Option<(i32, i32)>,
    }

    impl Islands {
        fn new() -> Self {
            Self {
                reader: LayoutReader::start(),
                generation: 0,
                key: None,
                sample: None,
                last: None,
                reach: None,
            }
        }

        /// Take the next clean color reading as the island's fill, after a theme or display change.
        fn distrust_fill(&mut self) {
            if let Some(sample) = self.sample.as_mut() {
                sample.fill.unsettle();
            }
        }

        /// Forget everything read from a taskbar that Explorer has since replaced.
        fn forget(&mut self) {
            self.generation = self.generation.wrapping_add(1);
            self.key = None;
            self.last = None;
            self.reach = None;
            self.distrust_fill();
        }

        fn layout(&mut self, taskbar: &Taskbar) -> Option<Layout> {
            let key = LayoutKey::new(taskbar, self.generation);
            if self.key != Some(key) {
                self.key = Some(key);
                self.last = None;
                self.reach = None;
            }
            if let Some(result) = self.reader.poll(key)
                && let Some(layout) = result.layout
            {
                self.last = Some((layout, result.read, result.key));
            }
            self.last
                .filter(|(_, read, seen)| *seen == key && read.elapsed() < LAYOUT_GRACE)
                .map(|(layout, _, _)| layout)
        }
    }

    fn read_layout(automation: &Automation, taskbar: HWND) -> Option<Layout> {
        let site = xaml_site(taskbar)?;
        let mut tray: Option<RECT> = None;
        let mut frame = None;
        let mut slot = None;
        let mut apps = None;
        unsafe {
            let root = automation
                .client
                .ElementFromHandle(windows::Win32::Foundation::HWND(site))
                .ok()?;
            let children = root
                .FindAllBuildCache(TreeScope_Descendants, &automation.all, &automation.cache)
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
                let id = child.CachedAutomationId().ok().map(|id| id.to_string());
                if class.starts_with("Taskbar.TaskListButton")
                    || id.as_deref().is_some_and(|id| {
                        id.starts_with("Appid:") || matches!(id, "StartButton" | "SearchButton")
                    })
                {
                    apps = Some(apps.map_or(rect, |apps| union(apps, rect)));
                }
                if class.starts_with(TRAY_CLASS_PREFIX) {
                    tray = Some(tray.map_or(rect, |tray| union(tray, rect)));
                    let is_slot = child
                        .CachedAutomationId()
                        .is_ok_and(|id| id == SLOT_AUTOMATION_ID)
                        && child.CachedName().is_ok_and(|name| name == SLOT_NAME);
                    if is_slot && slot.is_none() {
                        slot = Some(rect);
                    }
                } else if class == TASKBAR_FRAME_CLASS {
                    frame = Some(rect);
                }
            }
        }
        Some(Layout {
            tray: tray?,
            frame,
            slot,
            apps,
        })
    }

    fn placement_covers_apps(placement: (i32, i32, i32, i32), apps: RECT) -> bool {
        let (x, y, width, height) = placement;
        x < apps.right && x + width > apps.left && y < apps.bottom && y + height > apps.top
    }

    impl Islands {
        /// The island's fill at `at`. Readings are taken only where the taskbar itself shows and
        /// never while the popup's shadow reaches the taskbar (`popup_open`); a reading that differs
        /// from the fill in use replaces it once the next reading agrees (see [`Fill::read`]), so a
        /// passing overlay never tints the strip.
        fn color(&mut self, at: (i32, i32), taskbar: HWND, popup_open: bool) -> Option<[u8; 3]> {
            let reading = || {
                (!popup_open && taskbar_shows(taskbar, at.0, at.1))
                    .then(|| screen_pixel(at.0, at.1))
                    .flatten()
            };
            match self.sample.as_mut() {
                Some(sample) if sample.at == at => {
                    let due = !sample.fill.settled || sample.taken.elapsed() >= COLOR_REFRESH;
                    if due && let Some(color) = reading() {
                        sample.fill.read(color);
                        sample.taken = Instant::now();
                    }
                    Some(sample.fill.color)
                }
                previous => {
                    let clean = reading();
                    let color = clean
                        .or(previous.map(|sample| sample.fill.color))
                        .or_else(|| screen_pixel(at.0, at.1))?;
                    self.sample = Some(ColorSample {
                        at,
                        fill: Fill::new(color, clean.is_some()),
                        taken: Instant::now(),
                    });
                    Some(color)
                }
            }
        }

        /// The tray's island in taskbar client coordinates, or `None` on a stock taskbar, where the
        /// tray still sits at `TrayNotifyWnd` (see [`tray_moved`]). The fill is read in the island's
        /// padding on the screen-edge side, away from the windows and flyouts above the taskbar, or
        /// in the Show Desktop sliver at its right end when the island has no padding.
        fn island(
            &mut self,
            taskbar: &Taskbar,
            layout: &Layout,
            gap: i32,
            popup_open: bool,
        ) -> Option<Island> {
            let tray = to_client(taskbar.hwnd, layout.tray);
            if !tray_moved(tray.left, taskbar.notify_left, gap) {
                return None;
            }
            if let Some(frame) = layout.frame.map(|frame| to_client(taskbar.hwnd, frame))
                && let Some(reach) =
                    island_reach((frame.top, frame.bottom), (tray.top, tray.bottom))
            {
                self.reach = Some(reach);
            }
            let (above, below) = self.reach.unwrap_or((0, 0));
            let scale = taskbar.info.scale;
            let inside = layout.tray.right - (FRAME_RADIUS_POINTS * scale * 2.0).round() as i32;
            let upper = (inside, layout.tray.top - (above + 1) / 2);
            let lower = (inside, layout.tray.bottom + below / 2);
            let probe = match taskbar.info.edge {
                TaskbarEdge::Top if above >= 2 => upper,
                _ if below >= 2 => lower,
                _ if above >= 2 => upper,
                _ => (
                    layout.tray.right - 3,
                    (layout.tray.top + layout.tray.bottom) / 2,
                ),
            };
            Some(Island {
                left: tray.left - (ISLAND_INSET_POINTS * scale).round() as i32,
                right: tray.right,
                top: tray.top - above,
                bottom: tray.bottom + below,
                color: self.color(probe, taskbar.hwnd, popup_open)?,
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

    /// How the current frame is composed for its place, so it is recomposed only when this changes.
    #[derive(Clone, Copy, PartialEq)]
    enum Composition {
        /// An island of its own beside the notification area's.
        Framed {
            height: u32,
            padding: u32,
            radius: f64,
            color: [u8; 3],
        },
        /// Inside the app's widened notification-area button.
        Slotted { width: u32, height: u32 },
    }

    struct State<R: Runtime> {
        app: AppHandle<R>,
        shared: Shared,
        on_cover: CoverHandler,
        window: Option<Window>,
        bitmap: Option<Bitmap>,
        /// The current frame composed for its place; `None` while it sits as sent, beside a stock
        /// notification area.
        composed: Option<(Composition, Bitmap)>,
        islands: Islands,
        painted: bool,
        placed: Option<(i32, i32, i32, i32)>,
        /// The widened notification-area button the strip covers, in taskbar client coordinates.
        slot: Option<RECT>,
        /// Passes in a row that found that button gone or too narrow for the frame.
        slot_misses: u8,
        /// What the app last heard its button should show.
        tray: TraySlot,
        /// Points last asked of the button, and the width the strip wanted then.
        slot_request: Option<u32>,
        slot_wanted: Option<u32>,
        /// Passes since the strip wanted that width in which it did not fit the button.
        slot_unheld: u8,
    }

    thread_local! {
        static STATE: RefCell<Option<Box<dyn StripThread>>> = const { RefCell::new(None) };
        static CLICK: RefCell<Option<(ClickHandler, Dispatch)>> = const { RefCell::new(None) };
        static PRESSED: Cell<Option<Instant>> = const { Cell::new(None) };
        static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
        static TASKBAR_CHANGE: RefCell<PendingTaskbarChange> = const { RefCell::new(PendingTaskbarChange(None)) };
    }

    struct PendingTaskbarChange(Option<bool>);

    impl PendingTaskbarChange {
        fn push(&mut self, restarted: bool) -> bool {
            let post = self.0.is_none();
            self.0 = Some(self.0.unwrap_or(false) || restarted);
            post
        }

        fn take(&mut self) -> Option<bool> {
            self.0.take()
        }
    }

    fn queue_taskbar_change(hwnd: HWND, restarted: bool) {
        let post = TASKBAR_CHANGE.with(|change| change.borrow_mut().push(restarted));
        if post && unsafe { PostMessageW(hwnd, WM_APP_TASKBAR_CHANGED, 0, 0) } == 0 {
            TASKBAR_CHANGE.with(|change| change.borrow_mut().take());
        }
    }

    /// Object-safe view of the thread state, so the window procedures need no runtime generic.
    trait StripThread {
        fn apply_pending(&mut self);
        fn sync(&mut self);
        /// Re-read the taskbar after a theme or display change, or (`restarted`) after Explorer
        /// replaced the taskbar.
        fn taskbar_changed(&mut self, restarted: bool);
        fn window_destroyed(&mut self, hwnd: HWND) -> Option<HWND>;
    }

    impl<R: Runtime> StripThread for State<R> {
        fn apply_pending(&mut self) {
            let update = self
                .shared
                .pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.take());
            if let Some(bitmap) = update {
                self.bitmap = bitmap;
                self.composed = None;
                self.painted = false;
                self.sync();
            }
        }

        fn taskbar_changed(&mut self, restarted: bool) {
            self.islands.forget();
            self.slot = None;
            self.slot_misses = 0;
            if restarted {
                self.slot_wanted = None;
            }
            self.sync();
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
                self.discard_window();
                self.window = create_window(taskbar.hwnd);
            }
            let (Some(window), Some(content)) = (self.window.as_mut(), self.bitmap.as_ref()) else {
                return;
            };
            let scale = taskbar.info.scale;
            let gap = (GAP_POINTS * scale).round() as i32;
            let padding = (FRAME_PADDING_POINTS * scale).round() as u32;
            let popup_open = self.shared.popup.load(Ordering::Acquire);
            let previous_key = self.islands.key;
            let layout = self.islands.layout(taskbar);
            if self.islands.key != previous_key {
                self.slot = None;
                self.slot_misses = 0;
            }
            if layout.is_none() {
                unsafe {
                    SetWindowPos(
                        window.strip,
                        HWND_TOP,
                        0,
                        0,
                        0,
                        0,
                        SWP_ASYNCWINDOWPOS
                            | SWP_NOMOVE
                            | SWP_NOSIZE
                            | SWP_NOACTIVATE
                            | SWP_HIDEWINDOW,
                    );
                }
                self.placed = None;
                return;
            }
            let button = layout
                .and_then(|layout| layout.slot)
                .map(|slot| to_client(taskbar.hwnd, slot))
                .filter(|slot| slot_capable(slot.right - slot.left, slot.bottom - slot.top));
            let wanted =
                button.map(|slot| slot_width_for(content, (slot.bottom - slot.top) as u32, scale));
            let fitting = button
                .filter(|slot| slot_holds(content, slot.right - slot.left, slot.bottom - slot.top));
            match fitting {
                Some(slot) => {
                    self.slot = Some(slot);
                    self.slot_misses = 0;
                }
                None if self.slot.is_some() && self.slot_misses + 1 < SLOT_EXIT_PASSES => {
                    self.slot_misses += 1;
                }
                None => {
                    self.slot = None;
                    self.slot_misses = 0;
                }
            }
            let (x, y, composition) = if let Some(slot) = self.slot {
                let composition = Composition::Slotted {
                    width: (slot.right - slot.left) as u32,
                    height: (slot.bottom - slot.top) as u32,
                };
                (slot.left, slot.top, Some(composition))
            } else if let Some((island, (x, y))) = layout
                .and_then(|layout| self.islands.island(taskbar, &layout, gap, popup_open))
                .and_then(|island| {
                    let width = (content.width + padding * 2) as i32;
                    island_strip_origin(taskbar.width, &island, width, gap)
                        .map(|origin| (island, origin))
                })
            {
                let composition = Composition::Framed {
                    height: (island.bottom - island.top).max(1) as u32,
                    padding,
                    radius: FRAME_RADIUS_POINTS * scale,
                    color: island.color,
                };
                (x, y, Some(composition))
            } else {
                let (x, y) = strip_origin(
                    taskbar.height,
                    taskbar.notify_left,
                    content.width as i32,
                    content.height as i32,
                    gap,
                );
                (x, y, None)
            };
            match composition {
                Some(composition)
                    if self.composed.as_ref().map(|(current, _)| *current) != Some(composition) =>
                {
                    let bitmap = match composition {
                        Composition::Framed {
                            height,
                            padding,
                            radius,
                            color,
                        } => framed(content, height, padding, radius, color),
                        Composition::Slotted { width, height } => slotted(content, width, height),
                    };
                    self.composed = Some((composition, bitmap));
                    self.painted = false;
                }
                None if self.composed.take().is_some() => self.painted = false,
                _ => {}
            }
            let bitmap = self.composed.as_ref().map_or(content, |(_, bitmap)| bitmap);
            let placement = (x, y, bitmap.width as i32, bitmap.height as i32);
            let covered = layout
                .and_then(|layout| layout.apps)
                .map(|apps| to_client(taskbar.hwnd, apps))
                .is_some_and(|apps| placement_covers_apps(placement, apps));
            if covered {
                unsafe {
                    SetWindowPos(
                        window.strip,
                        HWND_TOP,
                        0,
                        0,
                        0,
                        0,
                        SWP_ASYNCWINDOWPOS
                            | SWP_NOMOVE
                            | SWP_NOSIZE
                            | SWP_NOACTIVATE
                            | SWP_HIDEWINDOW,
                    );
                }
                self.placed = None;
            } else if self.placed != Some(placement) || !self.painted {
                unsafe {
                    SetWindowPos(
                        window.strip,
                        HWND_TOP,
                        placement.0,
                        placement.1,
                        placement.2,
                        placement.3,
                        SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    )
                };
                self.placed = Some(placement);
            } else if unsafe { GetWindow(taskbar.hwnd, GW_CHILD) } != window.strip {
                unsafe {
                    SetWindowPos(
                        window.strip,
                        HWND_TOP,
                        0,
                        0,
                        0,
                        0,
                        SWP_ASYNCWINDOWPOS | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    )
                };
            }
            style_tooltip(window, &taskbar.info);
            if !self.painted {
                self.painted = paint(window.strip, bitmap);
                update_tooltip(window, &bitmap.tooltip);
            }
            let tray = self.tray_for(wanted);
            self.cover(tray);
        }

        /// What the app's button should show while it is widened for the strip, which wants
        /// `wanted` points of it: a clear icon that wide (see [`TraySlot::Clear`]), until the strip
        /// has not fitted the button for [`SLOT_STRETCH_PASSES`] passes, which means a styler rule
        /// keeps the button's width fixed and the strip sits elsewhere.
        fn tray_for(&mut self, wanted: Option<u32>) -> TraySlot {
            let Some(wanted) = wanted.filter(|_| self.painted) else {
                self.slot_request = None;
                self.slot_wanted = None;
                self.slot_unheld = 0;
                return TraySlot::Icon;
            };
            if self.slot_wanted != Some(wanted) {
                self.slot_wanted = Some(wanted);
                self.slot_unheld = 0;
            }
            let request = next_slot_width(self.slot_request, wanted);
            self.slot_request = Some(request);
            self.slot_unheld = if self.slot.is_some() {
                0
            } else {
                self.slot_unheld.saturating_add(1)
            };
            if self.slot_unheld > SLOT_STRETCH_PASSES {
                TraySlot::Icon
            } else {
                TraySlot::Clear { width: request }
            }
        }

        /// Tell the app, when it changes, what its notification-area button should show.
        fn cover(&mut self, tray: TraySlot) {
            if self.tray == tray {
                return;
            }
            self.tray = tray;
            let on_cover = self.on_cover.clone();
            if self.app.run_on_main_thread(move || on_cover(tray)).is_err() {
                tracing::warn!("could not report where the taskbar strip sits");
            }
        }

        fn discard_window(&mut self) {
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

        fn close(&mut self) {
            self.discard_window();
            self.slot = None;
            self.slot_misses = 0;
            self.slot_request = None;
            self.slot_wanted = None;
            self.slot_unheld = 0;
            self.cover(TraySlot::Icon);
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
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_NOPARENTNOTIFY,
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

    fn click(hwnd: HWND, button: StripButton, pressed_at: Option<Instant>) {
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
            pressed_at,
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
        let taskbar_created = TASKBAR_CREATED.with(Cell::get);
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
                queue_taskbar_change(hwnd, false);
                0
            }
            _ if taskbar_created != 0 && message == taskbar_created => {
                queue_taskbar_change(hwnd, true);
                0
            }
            WM_APP_TASKBAR_CHANGED => {
                if let Some(restarted) = TASKBAR_CHANGE.with(|change| change.borrow_mut().take()) {
                    with_state(|state| state.taskbar_changed(restarted));
                }
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
            WM_SETTINGCHANGE | WM_DISPLAYCHANGE => 0,
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_LBUTTONDOWN => {
                PRESSED.with(|pressed| pressed.set(Some(Instant::now())));
                0
            }
            WM_LBUTTONUP => {
                click(hwnd, StripButton::Primary, PRESSED.with(Cell::take));
                0
            }
            WM_RBUTTONUP => {
                click(hwnd, StripButton::Secondary, None);
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

    fn run<R: Runtime>(
        app: AppHandle<R>,
        on_click: ClickHandler,
        on_cover: CoverHandler,
        shared: Shared,
    ) {
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
        TASKBAR_CREATED.with(|created| created.set(taskbar_created));
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
                on_cover,
                window: None,
                bitmap: None,
                composed: None,
                islands: Islands::new(),
                painted: false,
                placed: None,
                slot: None,
                slot_misses: 0,
                tray: TraySlot::Icon,
                slot_request: None,
                slot_wanted: None,
                slot_unheld: 0,
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
    #[cfg(test)]
    mod tests {
        use super::*;

        fn reader() -> (LayoutReader, mpsc::Receiver<()>) {
            let (wake, events) = mpsc::sync_channel(1);
            (
                LayoutReader {
                    mailbox: Arc::new(Mutex::new(LayoutMailbox::default())),
                    wake,
                    stopped: Arc::new(AtomicBool::new(false)),
                },
                events,
            )
        }

        fn taskbar() -> Taskbar {
            Taskbar {
                hwnd: 1_isize as HWND,
                origin: (0, 880),
                notify_left: 1000,
                width: 1470,
                height: 43,
                info: TaskbarInfo {
                    supported: true,
                    height: 43,
                    scale: 1.0,
                    theme: TaskbarTheme::Dark,
                    edge: TaskbarEdge::Bottom,
                },
            }
        }

        fn layout() -> Layout {
            Layout {
                tray: RECT {
                    left: 1000,
                    top: 880,
                    right: 1470,
                    bottom: 923,
                },
                frame: None,
                slot: None,
                apps: None,
            }
        }

        #[test]
        fn a_tray_resize_waits_for_its_layout_without_reusing_old_coordinates() {
            let (reader, _events) = reader();
            let original = taskbar();
            let key = LayoutKey::new(&original, 0);
            let mut islands = Islands {
                reader,
                generation: 0,
                key: Some(key),
                sample: None,
                last: Some((layout(), Instant::now(), key)),
                reach: None,
            };
            let mut resized = taskbar();
            resized.notify_left -= 400;
            assert!(islands.layout(&resized).is_none());
            let resized_key = LayoutKey::new(&resized, 0);
            islands.reader.mailbox.lock().unwrap().result = Some(LayoutResult {
                key,
                layout: Some(layout()),
                read: Instant::now(),
            });
            assert!(islands.layout(&resized).is_none());
            islands.reader.mailbox.lock().unwrap().result = Some(LayoutResult {
                key: resized_key,
                layout: Some(layout()),
                read: Instant::now(),
            });
            assert!(islands.layout(&resized).is_some());
        }

        #[test]
        fn strip_placement_cannot_cover_start_search_or_application_buttons() {
            let apps = RECT {
                left: 193,
                top: 4,
                right: 733,
                bottom: 52,
            };
            assert!(placement_covers_apps((590, 7, 440, 42), apps));
            assert!(placement_covers_apps((280, 4, 440, 48), apps));
            assert!(!placement_covers_apps((733, 7, 440, 42), apps));
            assert!(!placement_covers_apps((0, 7, 193, 42), apps));
            assert!(!placement_covers_apps((300, 52, 440, 42), apps));
        }

        #[test]
        fn layout_poll_never_waits_for_a_busy_mailbox() {
            let (reader, _events) = reader();
            let guard = reader.mailbox.lock().unwrap();
            let started = Instant::now();
            assert!(reader.poll(LayoutKey::new(&taskbar(), 0)).is_none());
            assert!(started.elapsed() < Duration::from_millis(250));
            drop(guard);
        }

        #[test]
        fn pending_layout_requests_keep_only_the_latest_display() {
            let (reader, events) = reader();
            let mut current = taskbar();
            for width in 1000..2000 {
                current.width = width;
                assert!(reader.poll(LayoutKey::new(&current, 0)).is_none());
            }
            assert_eq!(
                reader.mailbox.lock().unwrap().request,
                Some(LayoutKey::new(&current, 0))
            );
            assert!(events.try_recv().is_ok());
            assert!(events.try_recv().is_err());
        }

        #[test]
        fn layout_poll_remains_responsive_during_a_stalled_provider() {
            let (reader, events) = reader();
            let mailbox = reader.mailbox.clone();
            let (entered, ready) = mpsc::channel();
            let (release, blocked) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                events.recv().unwrap();
                let request = mailbox.lock().unwrap().request.take().unwrap();
                entered.send(()).unwrap();
                blocked.recv().unwrap();
                mailbox.lock().unwrap().result = Some(LayoutResult {
                    key: request,
                    layout: Some(layout()),
                    read: Instant::now(),
                });
            });
            let key = LayoutKey::new(&taskbar(), 0);
            reader.poll(key);
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            let started = Instant::now();
            for _ in 0..100 {
                assert!(reader.poll(key).is_none());
            }
            let elapsed = started.elapsed();
            release.send(()).unwrap();
            worker.join().unwrap();
            assert!(elapsed < Duration::from_millis(250));
            assert!(reader.poll(key).unwrap().layout.is_some());
        }

        #[test]
        fn layouts_from_replaced_handles_displays_or_generations_are_discarded() {
            let (reader, _events) = reader();
            let old = LayoutKey::new(&taskbar(), 0);
            let mut variants = vec![LayoutKey {
                generation: 1,
                ..old
            }];
            let mut changed = taskbar();
            changed.hwnd = 2_isize as HWND;
            variants.push(LayoutKey::new(&changed, 0));
            changed = taskbar();
            changed.origin.1 = 1037;
            variants.push(LayoutKey::new(&changed, 0));
            changed = taskbar();
            changed.width = 1920;
            variants.push(LayoutKey::new(&changed, 0));
            changed = taskbar();
            changed.info.scale = 1.5;
            variants.push(LayoutKey::new(&changed, 0));
            for key in variants {
                reader.mailbox.lock().unwrap().result = Some(LayoutResult {
                    key: old,
                    layout: Some(layout()),
                    read: Instant::now(),
                });
                assert!(reader.poll(key).is_none());
            }
        }

        #[test]
        fn cached_layout_is_invalidated_even_when_restart_reuses_the_handle() {
            let (reader, _events) = reader();
            let key = LayoutKey::new(&taskbar(), 0);
            let mut islands = Islands {
                reader,
                generation: 0,
                key: Some(key),
                sample: None,
                last: Some((layout(), Instant::now(), key)),
                reach: None,
            };
            assert!(islands.layout(&taskbar()).is_some());
            islands.forget();
            assert!(islands.layout(&taskbar()).is_none());
        }

        #[test]
        fn broadcast_bursts_coalesce_without_losing_an_explorer_restart() {
            let mut pending = PendingTaskbarChange(None);
            assert!(pending.push(false));
            for _ in 0..100 {
                assert!(!pending.push(false));
            }
            assert!(!pending.push(true));
            assert!(!pending.push(false));
            assert_eq!(pending.take(), Some(true));
            assert_eq!(pending.take(), None);
            assert!(pending.push(false));
            assert_eq!(pending.take(), Some(false));
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use tauri::image::Image;
    use tauri::{AppHandle, Runtime};

    use super::{Bitmap, StripClick, TaskbarEdge, TaskbarInfo, TaskbarTheme};

    const TRAY_ID: &str = "main";
    /// GNOME's AppIndicator extension shows an image at least 1.5 times as wide as it is tall at
    /// its own size, one image pixel per logical pixel (`_loadCustomImage`), not at the panel's
    /// icon size. The strip is therefore drawn at one pixel per point, 24 tall: two readings
    /// stacked like the Windows clock, inside Ubuntu's 32-pixel top bar.
    const PANEL_HEIGHT: u32 = 24;

    #[derive(Default)]
    struct Images {
        strip: Option<(Image<'static>, String)>,
        glyph: Option<Image<'static>>,
        tooltip: String,
    }

    type Show = Box<dyn Fn(Option<Image<'static>>, String) + Send + Sync>;

    /// Linux has no taskbar band to embed in, so the strip becomes the panel indicator's image,
    /// the same picture as the Windows strip: brand marks, window names and readings. The label
    /// stays empty; the old plain-text title could not show marks and truncated long names.
    pub struct Strip {
        images: Mutex<Images>,
        show: Show,
    }

    impl Strip {
        pub fn start<R: Runtime>(
            app: AppHandle<R>,
            _on_click: Arc<dyn Fn(StripClick) + Send + Sync>,
            _on_cover: Arc<dyn Fn(super::TraySlot) + Send + Sync>,
        ) -> Self {
            Self {
                images: Mutex::new(Images::default()),
                show: Box::new(move |image, tooltip| {
                    let Some(tray) = app.tray_by_id(TRAY_ID) else {
                        return;
                    };
                    let image = image.or_else(|| app.default_window_icon().cloned());
                    if tray.set_icon(image).is_err()
                        || tray.set_title(None::<&str>).is_err()
                        || tray
                            .set_tooltip(Some(tooltip).filter(|tip| !tip.is_empty()))
                            .is_err()
                    {
                        tracing::warn!("could not update the panel indicator");
                    }
                }),
            }
        }

        pub fn info(&self) -> TaskbarInfo {
            TaskbarInfo {
                supported: true,
                height: PANEL_HEIGHT,
                scale: 1.0,
                theme: TaskbarTheme::Dark,
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

        pub fn set_popup_visible(&self, _visible: bool) {}

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

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

    use parking_lot::Mutex;
    use tauri::image::Image;
    use tauri::{AppHandle, Emitter, Runtime};

    use super::{Bitmap, StripClick, TASKBAR_INFO_EVENT, TaskbarEdge, TaskbarInfo, TaskbarTheme};

    const TRAY_ID: &str = "main";
    /// Who drew the strip last, so the log tells each change once: the system from the strip's
    /// description, or the popup's picture when the system could not.
    const BY_SYSTEM: u8 = 1;
    const BY_PICTURE: u8 = 2;
    /// tray-icon draws every status item image this many points tall. The strip is drawn in color
    /// like the Windows taskbar and the Linux panel, its text in the menu bar's own light or dark
    /// color, so it goes up as a plain image; the Bars glyph and the resting icon stay templates
    /// the system tints. Every image goes through `set_icon_with_as_template`, because plain
    /// `set_icon` clears the flag.
    const ICON_POINTS: f64 = 18.0;

    #[derive(Default)]
    struct Images {
        strip: Option<(Image<'static>, String)>,
        /// The strip as a description for the system to draw; `strip` is its picture.
        native: Option<Vec<u8>>,
        glyph: Option<Image<'static>>,
        tooltip: String,
    }

    /// What the menu bar item shows.
    enum Shown {
        /// The strip the system draws from its description, or its picture when it cannot.
        Strip {
            document: Option<Vec<u8>>,
            picture: Image<'static>,
        },
        /// The Bars glyph, or the resting icon for `None`: templates the system tints.
        Template(Option<Image<'static>>),
    }

    type Show = Box<dyn Fn(Shown, String) + Send + Sync>;

    pub struct Strip {
        images: Mutex<Images>,
        scale: f64,
        dark: Arc<AtomicBool>,
        show: Show,
    }

    fn info_for(scale: f64, dark: bool) -> TaskbarInfo {
        TaskbarInfo {
            supported: true,
            height: (ICON_POINTS * scale).round() as u32,
            scale,
            theme: if dark {
                TaskbarTheme::Dark
            } else {
                TaskbarTheme::Light
            },
            edge: TaskbarEdge::Top,
        }
    }

    impl Strip {
        pub fn start<R: Runtime>(
            app: AppHandle<R>,
            _on_click: Arc<dyn Fn(StripClick) + Send + Sync>,
            _on_cover: Arc<dyn Fn(super::TraySlot) + Send + Sync>,
        ) -> Self {
            let scale = app
                .primary_monitor()
                .ok()
                .flatten()
                .map(|monitor| monitor.scale_factor())
                .filter(|scale| scale.is_finite() && *scale >= 1.0)
                .unwrap_or(2.0);
            let dark = Arc::new(AtomicBool::new(false));
            let watcher = app.clone();
            let seen = Arc::clone(&dark);
            crate::macos::watch_menu_bar_appearance(move |now| {
                seen.store(now, Ordering::Relaxed);
                if watcher
                    .emit_to("popup", TASKBAR_INFO_EVENT, info_for(scale, now))
                    .is_err()
                {
                    tracing::warn!("could not publish the menu bar appearance");
                }
            });
            let drawn_by = AtomicU8::new(0);
            Self {
                images: Mutex::new(Images::default()),
                scale,
                dark,
                show: Box::new(move |shown, tooltip| {
                    let Some(tray) = app.tray_by_id(TRAY_ID) else {
                        return;
                    };
                    let tooltip = Some(tooltip).filter(|tip| !tip.is_empty());
                    let (image, template) = match shown {
                        Shown::Strip { document, picture } => {
                            let described = document.is_some();
                            let drawn = document.is_some_and(|document| draw(&tray, document));
                            let drawer = if drawn { BY_SYSTEM } else { BY_PICTURE };
                            if described && drawer != drawn_by.swap(drawer, Ordering::Relaxed) {
                                if drawn {
                                    tracing::info!("the system draws the menu bar strip");
                                } else {
                                    tracing::warn!(
                                        "the system could not draw the menu bar strip; its picture shows"
                                    );
                                }
                            }
                            if drawn {
                                if tray.set_tooltip(tooltip).is_err() {
                                    tracing::warn!("could not update the menu bar item");
                                }
                                return;
                            }
                            (Some(picture), false)
                        }
                        Shown::Template(image) => {
                            (image.or_else(|| crate::menu_bar_icon().ok()), true)
                        }
                    };
                    if tray
                        .with_inner_tray_icon(|_| crate::macos::clear_strip())
                        .is_err()
                        || tray.set_icon_with_as_template(image, template).is_err()
                        || tray.set_tooltip(tooltip).is_err()
                    {
                        tracing::warn!("could not update the menu bar item");
                    }
                }),
            }
        }

        pub fn info(&self) -> TaskbarInfo {
            info_for(self.scale, self.dark.load(Ordering::Relaxed))
        }

        /// Kept for the frame that follows; `set` shows both.
        pub fn set_native(&self, document: Option<Vec<u8>>) {
            self.images.lock().native = document;
        }

        pub fn set(&self, bitmap: Option<Bitmap>) {
            let strip = bitmap.map(|bitmap| {
                let image = Image::new_owned(bitmap.straight_rgba(), bitmap.width, bitmap.height);
                (image, bitmap.tooltip)
            });
            let mut images = self.images.lock();
            if strip.is_none() {
                images.native = None;
            }
            images.strip = strip;
            drop(images);
            self.apply();
        }

        /// The menu bar item is the strip itself, so the popup's shadow changes nothing here.
        pub fn set_popup_visible(&self, _visible: bool) {}

        pub fn set_glyph(&self, glyph: Option<Image<'static>>, tooltip: String) {
            let mut images = self.images.lock();
            images.glyph = glyph;
            images.tooltip = tooltip;
            drop(images);
            self.apply();
        }

        fn apply(&self) {
            let (shown, tooltip) = {
                let images = self.images.lock();
                match &images.strip {
                    Some((picture, tooltip)) => (
                        Shown::Strip {
                            document: images.native.clone(),
                            picture: picture.clone(),
                        },
                        tooltip.clone(),
                    ),
                    None => (
                        Shown::Template(images.glyph.clone()),
                        images.tooltip.clone(),
                    ),
                }
            };
            (self.show)(shown, tooltip);
        }
    }

    /// Have the system draw the strip in the tray's menu bar item, on the main thread, where the
    /// tray hands its item out. `false` when it did not, and the item is as it was.
    fn draw<R: Runtime>(tray: &tauri::tray::TrayIcon<R>, document: Vec<u8>) -> bool {
        tray.with_inner_tray_icon(move |inner| {
            inner.ns_status_item().is_some_and(|item| {
                crate::macos::show_strip(std::ptr::from_ref(&*item).cast_mut().cast(), &document)
            })
        })
        .unwrap_or(false)
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
            _on_cover: Arc<dyn Fn(super::TraySlot) + Send + Sync>,
        ) -> Self {
            Self
        }

        pub fn info(&self) -> TaskbarInfo {
            TaskbarInfo::UNSUPPORTED
        }

        pub fn set(&self, _bitmap: Option<Bitmap>) {}

        pub fn set_popup_visible(&self, _visible: bool) {}
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
            native: None,
        }
    }

    #[test]
    fn takes_a_strip_description_with_something_to_show_and_no_other() {
        let group =
            serde_json::json!({ "brand": "claude", "rows": [{ "label": "5h", "value": "12%" }] });
        let document = serde_json::json!({ "version": 1, "groups": [group] });
        let bytes = encode_native(&document).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            document
        );
        assert!(encode_native(&serde_json::json!({ "version": 2, "groups": [group] })).is_err());
        assert!(encode_native(&serde_json::json!({ "version": 1, "groups": [] })).is_err());
        assert!(encode_native(&serde_json::json!({ "version": 1 })).is_err());
        let heavy = serde_json::json!({ "version": 1, "groups": [group], "text": "x".repeat(MAX_NATIVE_BYTES) });
        assert!(encode_native(&heavy).is_err());
    }

    #[test]
    fn frames_from_a_popup_without_a_description_still_read() {
        let frame: StripFrame = serde_json::from_value(serde_json::json!({
            "png": TWO_PIXELS.to_vec(),
            "width": 2,
            "height": 1,
            "text": "Claude 12%",
            "tooltip": "Usage",
        }))
        .unwrap();
        assert!(frame.native.is_none());
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

    /// A frame as the popup renders it: `width` x `height`, clear (alpha 1) except rows `ink`.
    fn frame_with_ink(
        width: u32,
        height: u32,
        ink: std::ops::Range<u32>,
        pixel: [u8; 4],
    ) -> Bitmap {
        let mut frame = bitmap(width, height, [0, 0, 0, 1]);
        for y in ink {
            for x in 0..width {
                let index = ((y * width + x) * 4) as usize;
                frame.bgra[index..index + 4].copy_from_slice(&pixel);
            }
        }
        frame
    }

    #[test]
    fn asks_the_button_for_the_strip_width_plus_its_margins_in_points() {
        let content = frame_with_ink(300, 42, 10..30, [255, 255, 255, 255]);
        assert_eq!(slot_width_for(&content, 42, 1.0), 312);
        let hidpi = frame_with_ink(450, 63, 15..45, [255, 255, 255, 255]);
        assert_eq!(slot_width_for(&hidpi, 63, 1.5), 312);
        assert!(slot_capable(88, 42));
        assert!(!slot_capable(40, 42));
    }

    #[test]
    fn grows_the_button_at_once_and_shrinks_it_only_past_the_slack() {
        assert_eq!(next_slot_width(None, 256), 256);
        assert_eq!(next_slot_width(Some(256), 264), 264);
        assert_eq!(next_slot_width(Some(256), 248), 256);
        assert_eq!(next_slot_width(Some(256), 240), 240);
    }

    #[test]
    fn slots_the_content_centered_into_a_clear_button_and_clips_clear_rows() {
        let pixel = |image: &Bitmap, x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            image.bgra[index..index + 4].to_vec()
        };
        let slot = slotted(&frame_with_ink(4, 8, 2..6, [255, 255, 255, 255]), 10, 6);
        assert_eq!((slot.width, slot.height), (10, 6));
        assert_eq!(slot.text, "Claude 12%");
        assert_eq!(pixel(&slot, 0, 0), vec![0, 0, 0, 1]);
        assert_eq!(pixel(&slot, 3, 0), vec![0, 0, 0, 1]);
        assert_eq!(pixel(&slot, 3, 1), vec![255, 255, 255, 255]);
        assert_eq!(pixel(&slot, 6, 4), vec![255, 255, 255, 255]);
        assert_eq!(pixel(&slot, 3, 5), vec![0, 0, 0, 1]);
        assert_eq!(pixel(&slot, 2, 3), vec![0, 0, 0, 1]);
        assert_eq!(pixel(&slot, 7, 3), vec![0, 0, 0, 1]);
    }

    #[test]
    fn shrinks_a_strip_wider_than_the_button_to_fit_inside_it() {
        let pixel = |image: &Bitmap, x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            image.bgra[index..index + 4].to_vec()
        };
        let wide = frame_with_ink(40, 8, 2..6, [200, 200, 200, 200]);
        assert!((slot_scale(&wide, 20, 10) - 0.4).abs() < 1e-9);
        let slot = slotted(&wide, 20, 10);
        assert_eq!((slot.width, slot.height), (20, 10));
        assert_eq!(pixel(&slot, 2, 4), vec![200, 200, 200, 200]);
        assert_eq!(pixel(&slot, 17, 4), vec![200, 200, 200, 200]);
        assert_eq!(pixel(&slot, 1, 4), vec![0, 0, 0, 1]);
        assert_eq!(pixel(&slot, 18, 4), vec![0, 0, 0, 1]);
        assert_eq!(pixel(&slot, 10, 3), vec![0, 0, 0, 1]);
        assert_eq!(pixel(&slot, 10, 5), vec![0, 0, 0, 1]);
        assert!(slot.bgra.chunks(4).all(|pixel| pixel[3] >= 1));

        let mut stripes = bitmap(4, 1, [100, 100, 100, 100]);
        stripes.bgra[4..8].copy_from_slice(&[200, 200, 200, 200]);
        stripes.bgra[12..16].copy_from_slice(&[200, 200, 200, 200]);
        let halved = resized(&stripes, (0, 0, 4, 1), 2, 1);
        assert_eq!(halved.bgra, vec![150, 150, 150, 150, 150, 150, 150, 150]);
        assert_eq!(
            slot_scale(&frame_with_ink(40, 8, 2..6, [9, 9, 9, 9]), 60, 10),
            1.0
        );
    }

    #[test]
    fn holds_the_strip_only_in_a_button_widened_for_it() {
        let strip = frame_with_ink(190, 56, 18..38, [255, 255, 255, 255]);
        assert!(slot_holds(&strip, 212, 42));
        assert!(slot_holds(&strip, 180, 42));
        assert!(!slot_holds(&strip, 120, 42));
        assert!(!slot_holds(&strip, 32, 42));
        assert!(!slot_holds(&strip, 212, 0));
        let short = frame_with_ink(30, 56, 18..38, [255, 255, 255, 255]);
        assert!(!slot_holds(&short, 32, 42));
        assert!(slot_holds(&short, 84, 42));
    }

    #[test]
    fn tells_a_moved_notification_area_from_the_stock_one() {
        assert!(!tray_moved(1570, 1566, 4));
        assert!(!tray_moved(1562, 1566, 4));
        assert!(tray_moved(962, 1566, 4));
        assert!(tray_moved(962, 1386, 4));
    }

    #[test]
    fn reads_the_island_reach_only_from_a_frame_centered_on_the_tray() {
        assert_eq!(island_reach((1028, 1076), (1031, 1073)), Some((3, 3)));
        assert_eq!(island_reach((1024, 1076), (1031, 1073)), None);
        assert_eq!(island_reach((1025, 1076), (1031, 1073)), None);
        assert_eq!(island_reach((1027, 1076), (1031, 1073)), Some((4, 3)));
        assert_eq!(island_reach((1031, 1073), (1031, 1073)), Some((0, 0)));
        assert_eq!(island_reach((1033, 1071), (1031, 1073)), None);
    }

    #[test]
    fn a_settled_fill_changes_only_when_two_readings_agree() {
        let mut fill = Fill::new([36, 43, 64], true);
        fill.read([14, 17, 26]);
        assert_eq!(fill.color, [36, 43, 64]);
        fill.read([37, 42, 65]);
        assert_eq!(fill.color, [36, 43, 64]);
        fill.read([14, 17, 26]);
        assert_eq!(fill.color, [36, 43, 64]);
        fill.read([15, 17, 27]);
        assert_eq!(fill, Fill::new([15, 17, 27], true));
    }

    #[test]
    fn an_unsettled_fill_takes_the_next_clean_reading() {
        let mut fill = Fill::new([48, 66, 119], false);
        fill.read([36, 43, 64]);
        assert_eq!(fill, Fill::new([36, 43, 64], true));
        fill.unsettle();
        fill.read([200, 200, 200]);
        assert_eq!(fill, Fill::new([200, 200, 200], true));
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
