//! macOS: the popup's window style, the Dynamic Island and the desktop widget reload. They are
//! written in Swift (`macos/Host`, built by `build.rs`) and reached through this C interface. The
//! Swift side hops to the main thread itself, so every function here may run on any thread.

use std::ffi::c_void;
use std::sync::OnceLock;

use tauri::{AppHandle, PhysicalPosition, PhysicalRect, PhysicalSize, WebviewWindow};

type IslandHandler = extern "C" fn(i32, f64, f64, f64, f64, f64);

unsafe extern "C" {
    fn qc_popup_configure(window: *mut c_void, radius: f64);
    fn qc_popup_refresh_shadow(window: *mut c_void);
    fn qc_island_start(handler: Option<IslandHandler>);
    fn qc_island_update(bytes: *const u8, length: usize);
    fn qc_island_popup_visible(visible: bool);
    fn qc_widgets_reload();
}

/// Corner radius of the popup, in points.
const POPUP_RADIUS: f64 = 12.0;
/// The island event for a click (see `IslandEvent` in `Bridge.swift`).
const ISLAND_OPEN: i32 = 1;

static APP: OnceLock<AppHandle> = OnceLock::new();

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

/// Ask WidgetKit to reload the desktop widget's timeline.
pub fn reload_widgets() {
    unsafe { qc_widgets_reload() }
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
