//! macOS: the popup's window style, the Dynamic Island, the desktop widget's reload and upkeep,
//! the menu bar strip drawn by the system, notifications through the system's notification center
//! and launch at login as a login item. They are written in Swift (`macos/Host`, built by
//! `build.rs`) and reached through this C interface. The Swift side hops to the main thread itself,
//! so every function here may run on any thread, except [`show_strip`].

use std::ffi::{CString, c_char, c_void};
use std::sync::OnceLock;
use std::time::Duration;

use tauri::{AppHandle, PhysicalPosition, PhysicalRect, PhysicalSize, WebviewWindow};

type IslandHandler = extern "C" fn(i32, f64, f64, f64, f64, f64);
type AppearanceHandler = extern "C" fn(bool);
type NotificationAccessHandler = extern "C" fn(*mut c_void, i32);
type NotificationOpenHandler = extern "C" fn();

unsafe extern "C" {
    fn qc_popup_configure(window: *mut c_void, radius: f64);
    fn qc_popup_refresh_shadow(window: *mut c_void);
    fn qc_island_start(handler: Option<IslandHandler>);
    fn qc_island_update(bytes: *const u8, length: usize);
    fn qc_island_popup_visible(visible: bool);
    fn qc_widgets_reload();
    fn qc_widgets_adopt_current();
    fn qc_menu_bar_appearance_start(handler: Option<AppearanceHandler>);
    fn qc_strip_show(item: *mut c_void, bytes: *const u8, length: usize) -> bool;
    fn qc_strip_clear();
    fn qc_notifications_start(handler: Option<NotificationOpenHandler>) -> bool;
    fn qc_notifications_access(context: *mut c_void, handler: Option<NotificationAccessHandler>);
    fn qc_notifications_request(context: *mut c_void, handler: Option<NotificationAccessHandler>);
    fn qc_notifications_send(
        title: *const c_char,
        body: *const c_char,
        identifier: *const c_char,
        thread: *const c_char,
        english: bool,
    );
    fn qc_login_item_state() -> i32;
    fn qc_login_item_set(enabled: bool, by_user: bool) -> i32;
}

/// Corner radius of the popup, in points.
const POPUP_RADIUS: f64 = 12.0;
/// The island event for a click (see `IslandEvent` in `Bridge.swift`).
const ISLAND_OPEN: i32 = 1;

static APP: OnceLock<AppHandle> = OnceLock::new();
static APPEARANCE: OnceLock<Box<dyn Fn(bool) + Send + Sync>> = OnceLock::new();

/// Float the popup above the menu bar on every Space, full-screen apps included, with rounded
/// corners and the system shadow.
pub fn configure_popup(window: &WebviewWindow) {
    match window.ns_window() {
        Ok(handle) => unsafe { qc_popup_configure(handle, POPUP_RADIUS) },
        Err(error) => tracing::warn!("cannot style the popup window: {error}"),
    }
}

/// Recompute the popup's shadow after a resize.
pub fn refresh_popup_shadow(window: &WebviewWindow) {
    if let Ok(handle) = window.ns_window() {
        unsafe { qc_popup_refresh_shadow(handle) }
    }
}

/// Start the Dynamic Island; it stays hidden until the popup sends readings with the island on.
pub fn start_island(app: &AppHandle) {
    let _ = APP.set(app.clone());
    unsafe { qc_island_start(Some(island_event)) }
}

pub fn update_island(document: &[u8]) {
    unsafe { qc_island_update(document.as_ptr(), document.len()) }
}

/// Tell the island whether the popup is open: it closes and stays closed meanwhile, so neither a
/// hover nor an alert covers the popup.
pub fn set_popup_visible(visible: bool) {
    unsafe { qc_island_popup_visible(visible) }
}

/// Hear whether the menu bar reads dark, once now and on every change, on the main thread. Only
/// the first listener is kept.
pub fn watch_menu_bar_appearance(on_change: impl Fn(bool) + Send + Sync + 'static) {
    if APPEARANCE.set(Box::new(on_change)).is_ok() {
        unsafe { qc_menu_bar_appearance_start(Some(menu_bar_appearance)) }
    }
}

extern "C" fn menu_bar_appearance(dark: bool) {
    if let Some(listener) = APPEARANCE.get() {
        listener(dark);
    }
}

/// Draw the strip `document` describes (`src/strip/native.ts`) in the menu bar item. `item` is the
/// tray's `NSStatusItem`, and this runs on the main thread, where the tray hands the item out.
/// `false` leaves the item as it was.
pub fn show_strip(item: *mut c_void, document: &[u8]) -> bool {
    unsafe { qc_strip_show(item, document.as_ptr(), document.len()) }
}

/// The menu bar item shows an ordinary image again.
pub fn clear_strip() {
    unsafe { qc_strip_clear() }
}

/// Whether the user lets the app show notifications, as the system's notification center tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationAccess {
    /// The notification center is out of reach: a development run outside an app bundle.
    Unavailable,
    /// The user was never asked.
    Undetermined,
    Denied,
    Granted,
}

impl NotificationAccess {
    fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Undetermined,
            1 => Self::Denied,
            2 => Self::Granted,
            _ => Self::Unavailable,
        }
    }
}

/// How long an answer about notification access may take. Reading the setting answers at once;
/// asking waits for the user, who may leave the question unanswered.
const ACCESS_READ: Duration = Duration::from_secs(5);
const ACCESS_ASKED: Duration = Duration::from_secs(120);

/// Send notifications through the system's notification center from now on; a click on one opens
/// the popup. `false` when the center is out of reach.
pub fn start_notifications(app: &AppHandle) -> bool {
    let _ = APP.set(app.clone());
    unsafe { qc_notifications_start(Some(notification_opened)) }
}

extern "C" fn notification_opened() {
    let Some(app) = APP.get().cloned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::show_popup(&app) {
            tracing::warn!("{error}");
        }
    });
}

pub async fn notification_access() -> NotificationAccess {
    answer(ACCESS_READ, |context| unsafe {
        qc_notifications_access(context, Some(access_answered))
    })
    .await
}

/// Ask the user for notifications when they were never asked; after a refusal the system's
/// settings open instead, the one place the answer changes then.
pub async fn request_notification_access() -> NotificationAccess {
    answer(ACCESS_ASKED, |context| unsafe {
        qc_notifications_request(context, Some(access_answered))
    })
    .await
}

/// An answer that does not come in time counts as none: the caller then treats the user as not
/// asked yet.
async fn answer(wait: Duration, ask: impl FnOnce(*mut c_void)) -> NotificationAccess {
    let (sender, receiver) = tokio::sync::oneshot::channel::<i32>();
    ask(Box::into_raw(Box::new(sender)).cast());
    match tokio::time::timeout(wait, receiver).await {
        Ok(Ok(code)) => NotificationAccess::from_code(code),
        _ => NotificationAccess::Undetermined,
    }
}

extern "C" fn access_answered(context: *mut c_void, code: i32) {
    if context.is_null() {
        return;
    }
    let sender = unsafe { Box::from_raw(context.cast::<tokio::sync::oneshot::Sender<i32>>()) };
    let _ = sender.send(code);
}

/// Show a notification. One with the same `identifier` replaces the one shown before it, and
/// Notification Center keeps those with the same `thread` together.
pub fn notify(title: &str, body: &str, identifier: &str, thread: &str, english: bool) {
    let text = |value: &str| CString::new(value.replace('\0', "")).unwrap_or_default();
    let (title, body, identifier, thread) =
        (text(title), text(body), text(identifier), text(thread));
    unsafe {
        qc_notifications_send(
            title.as_ptr(),
            body.as_ptr(),
            identifier.as_ptr(),
            thread.as_ptr(),
            english,
        );
    }
}

/// Where the app's login item stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginItem {
    /// No login item can be registered: a development run outside an app bundle, or the system
    /// refused.
    Unavailable,
    Off,
    On,
    /// Registered, but switched off in System Settings, where only the user switches it back.
    NeedsApproval,
}

impl LoginItem {
    fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Off,
            1 => Self::On,
            2 => Self::NeedsApproval,
            _ => Self::Unavailable,
        }
    }
}

pub fn login_item() -> LoginItem {
    LoginItem::from_code(unsafe { qc_login_item_state() })
}

/// Register or remove the app's login item and tell where it stands after. When the user asked
/// (`by_user`) and the item waits for approval, System Settings opens where it is given.
pub fn set_login_item(enabled: bool, by_user: bool) -> LoginItem {
    LoginItem::from_code(unsafe { qc_login_item_set(enabled, by_user) })
}

/// Ask WidgetKit to reload the desktop widget's timeline.
pub fn reload_widgets() {
    unsafe { qc_widgets_reload() }
}

/// Stop desktop widget extension processes left over from an earlier version of the app (an
/// in-app update deletes the old bundle while its extension keeps running), register this bundle's
/// extension again and reload the widgets, so the widget always runs this version (Rules.md §0.61).
/// Returns at once; the work runs on a background queue.
pub fn adopt_current_widget() {
    unsafe { qc_widgets_adopt_current() }
}

/// The island was clicked: open the popup right under it. The rectangle arrives in global points
/// with a top-left origin and stays in points: `position_popup` works in points on macOS, where
/// displays may mix backing scales.
extern "C" fn island_event(kind: i32, x: f64, y: f64, width: f64, height: f64, _scale: f64) {
    if kind != ISLAND_OPEN {
        return;
    }
    let Some(app) = APP.get().cloned() else {
        return;
    };
    let rect = island_rect(x, y, width, height);
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::show_popup_anchored(&app, rect) {
            tracing::warn!("{error}");
        }
    });
}

/// The island's rectangle in whole points, in the `PhysicalRect` the popup anchor is kept in.
fn island_rect(x: f64, y: f64, width: f64, height: f64) -> PhysicalRect<i32, u32> {
    let whole = |value: f64| {
        if value.is_finite() {
            value.round()
        } else {
            0.0
        }
    };
    PhysicalRect {
        position: PhysicalPosition::new(whole(x) as i32, whole(y) as i32),
        size: PhysicalSize::new(whole(width).max(1.0) as u32, whole(height).max(1.0) as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test runs outside an app bundle, where neither is within reach: the answers still come
    /// back through the Swift side.
    #[tokio::test]
    async fn outside_an_app_bundle_the_system_ways_are_out_of_reach() {
        assert_eq!(notification_access().await, NotificationAccess::Unavailable);
        assert_eq!(
            request_notification_access().await,
            NotificationAccess::Unavailable
        );
        assert_eq!(login_item(), LoginItem::Unavailable);
        assert_eq!(set_login_item(true, false), LoginItem::Unavailable);
        assert!(!show_strip(std::ptr::null_mut(), b"{}"));
        clear_strip();
        notify("Claude", "Almost out", "usage.claude", "usage.claude", true);
    }

    #[test]
    fn answers_from_swift_keep_their_meaning() {
        assert_eq!(
            [-1, 0, 1, 2, 9].map(NotificationAccess::from_code),
            [
                NotificationAccess::Unavailable,
                NotificationAccess::Undetermined,
                NotificationAccess::Denied,
                NotificationAccess::Granted,
                NotificationAccess::Unavailable,
            ]
        );
        assert_eq!(
            [-1, 0, 1, 2, 9].map(LoginItem::from_code),
            [
                LoginItem::Unavailable,
                LoginItem::Off,
                LoginItem::On,
                LoginItem::NeedsApproval,
                LoginItem::Unavailable,
            ]
        );
    }

    #[test]
    fn island_rectangles_stay_in_whole_points() {
        let rect = island_rect(595.0, 0.0, 330.5, 37.0);
        assert_eq!((rect.position.x, rect.position.y), (595, 0));
        assert_eq!((rect.size.width, rect.size.height), (331, 37));
        let fallback = island_rect(10.0, f64::NAN, 0.0, 20.0);
        assert_eq!(
            (
                fallback.position.x,
                fallback.position.y,
                fallback.size.width
            ),
            (10, 0, 1)
        );
    }
}
