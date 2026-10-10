//! The popup hides and shows on request even after another process changed its native window's
//! visibility. The window library acts only when its own record of that visibility changes, so a
//! popup a UI probe had shown with a raw `ShowWindow` ignored every hide the app asked for and
//! could not be closed (10/10/2026).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsWindowVisible, SHOW_WINDOW_CMD, SW_HIDE, SW_SHOWNOACTIVATE, ShowWindow,
};

use super::{hide_popup, set_popup_window_visible};

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
const SETTLE: Duration = Duration::from_millis(150);

/// Whether the popup's native window was visible after each step.
#[derive(Debug)]
struct Observed {
    built: bool,
    shown_from_outside: bool,
    after_app_hide: bool,
    after_app_show: bool,
    hidden_from_outside: bool,
    after_app_show_again: bool,
    after_app_hide_again: bool,
}

#[derive(Default)]
struct ProbeState {
    completed: AtomicBool,
    outcome: Mutex<Option<Result<Observed, String>>>,
}

fn on_main<T: Send + 'static>(
    app: &AppHandle,
    action: impl FnOnce(AppHandle) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = mpsc::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = catch_unwind(AssertUnwindSafe(|| action(handle)))
            .unwrap_or_else(|_| Err("The native popup action panicked".into()));
        let _ = sender.send(result);
    })
    .map_err(|error| error.to_string())?;
    receiver
        .recv_timeout(CALL_TIMEOUT)
        .map_err(|error| format!("The native popup action did not finish: {error}"))?
}

fn finish(app: &AppHandle, state: &ProbeState, result: Result<Observed, String>) {
    if state.completed.swap(true, Ordering::SeqCst) {
        return;
    }
    let code = i32::from(result.is_err());
    *state.outcome.lock() = Some(result);
    for window in app.webview_windows().into_values() {
        let _ = window.destroy();
    }
    app.exit(code);
}

/// What another process does: change the window's visibility without the app or its window
/// library taking part. Called from the probe thread, not the window's own thread.
fn from_outside(hwnd: isize, command: SHOW_WINDOW_CMD) {
    unsafe { ShowWindow(hwnd as HWND, command) };
}

fn visible(hwnd: isize) -> bool {
    std::thread::sleep(SETTLE);
    unsafe { IsWindowVisible(hwnd as HWND) != 0 }
}

fn app_sets_visible(app: &AppHandle, shown: bool) -> Result<(), String> {
    on_main(app, move |app| {
        let window = app
            .get_webview_window("popup")
            .ok_or("Missing test popup")?;
        set_popup_window_visible(&window, shown)
    })
}

fn probe(app: &AppHandle) -> Result<Observed, String> {
    let hwnd = on_main(app, |app| {
        let window = app
            .get_webview_window("popup")
            .ok_or("Missing test popup")?;
        Ok(window.hwnd().map_err(|error| error.to_string())?.0 as isize)
    })?;
    let built = visible(hwnd);
    from_outside(hwnd, SW_SHOWNOACTIVATE);
    let shown_from_outside = visible(hwnd);
    on_main(app, |app| hide_popup(&app))?;
    let after_app_hide = visible(hwnd);
    app_sets_visible(app, true)?;
    let after_app_show = visible(hwnd);
    from_outside(hwnd, SW_HIDE);
    let hidden_from_outside = visible(hwnd);
    app_sets_visible(app, true)?;
    let after_app_show_again = visible(hwnd);
    on_main(app, |app| hide_popup(&app))?;
    let after_app_hide_again = visible(hwnd);
    Ok(Observed {
        built,
        shown_from_outside,
        after_app_hide,
        after_app_show,
        hidden_from_outside,
        after_app_show_again,
        after_app_hide_again,
    })
}

#[test]
#[ignore = "Requires Windows WebView2 and a desktop session; shows an isolated popup off screen"]
fn popup_follows_app_requests_after_outside_visibility_changes() {
    let profile = tempfile::Builder::new()
        .prefix("quota-control-popup-visibility-")
        .tempdir()
        .expect("Cannot create an isolated WebView2 profile");
    let data_directory = profile.path().join("webview");
    let state = Arc::new(ProbeState::default());
    let setup_state = state.clone();
    let mut context = tauri::test::mock_context::<tauri::Wry, _>(tauri::test::noop_assets());
    context.config_mut().identifier = format!(
        "com.buidangminh.quota-control.popup-visibility-test-{}",
        std::process::id()
    );
    let app = tauri::Builder::<tauri::Wry>::default()
        .any_thread()
        .setup(move |app| {
            let handle = app.handle().clone();
            let watchdog_handle = handle.clone();
            let watchdog_state = setup_state.clone();
            std::thread::spawn(move || {
                std::thread::sleep(PROBE_TIMEOUT);
                finish(
                    &watchdog_handle,
                    &watchdog_state,
                    Err("The isolated popup visibility probe timed out".into()),
                );
            });
            let window = WebviewWindowBuilder::new(
                app,
                "popup",
                WebviewUrl::External("about:blank".parse().expect("Valid blank-page URL")),
            )
            .title("Quota Control popup visibility test")
            .inner_size(320.0, 400.0)
            .position(-20_000.0, -20_000.0)
            .resizable(false)
            .decorations(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .focused(false)
            .visible(false)
            .data_directory(data_directory.clone())
            .build();
            if let Err(error) = window {
                finish(&handle, &setup_state, Err(error.to_string()));
                return Ok(());
            }
            let worker_state = setup_state.clone();
            std::thread::spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| probe(&handle)))
                    .unwrap_or_else(|_| Err("The popup visibility probe panicked".into()));
                finish(&handle, &worker_state, result);
            });
            Ok(())
        })
        .build(context)
        .expect("Cannot create the isolated native app");
    let code = app.run_return(|_, _| {});
    let observed = state
        .outcome
        .lock()
        .take()
        .expect("The native probe did not report")
        .expect("The popup visibility probe failed");
    println!("{observed:?}");
    assert_eq!(code, 0, "The isolated native app did not exit cleanly");
    assert!(!observed.built, "The test popup was built visible");
    assert!(
        observed.shown_from_outside,
        "The outside ShowWindow did not show the test popup"
    );
    assert!(
        observed.after_app_show,
        "The app could not show the popup it had hidden"
    );
    assert!(
        !observed.hidden_from_outside,
        "The outside ShowWindow did not hide the test popup"
    );
    assert!(
        !observed.after_app_hide_again,
        "The app could not hide the popup it had shown"
    );
    let failures: Vec<&str> = [
        (
            observed.after_app_hide,
            "a popup shown from outside stayed on screen when the app hid it",
        ),
        (
            !observed.after_app_show_again,
            "a popup hidden from outside stayed hidden when the app showed it",
        ),
    ]
    .into_iter()
    .filter_map(|(failed, message)| failed.then_some(message))
    .collect();
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
