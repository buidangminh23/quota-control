use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, PhysicalSize, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use super::{PopupAnchor, commands};

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
const REPEATED_REQUESTS: usize = 24;

#[derive(Debug)]
struct ProbeReport {
    settled: PhysicalSize<u32>,
    frame_height: u32,
    repeated_requests: usize,
    resize_events: Vec<PhysicalSize<u32>>,
}

#[derive(Default)]
struct ProbeState {
    completed: AtomicBool,
    outcome: Mutex<Option<Result<ProbeReport, String>>>,
    resize_events: Mutex<Vec<PhysicalSize<u32>>>,
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

fn finish(app: &AppHandle, state: &ProbeState, result: Result<ProbeReport, String>) {
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

fn probe(app: &AppHandle, state: &ProbeState) -> Result<ProbeReport, String> {
    let (height, expected, frame_height) = on_main(app, |app| {
        let window = app
            .get_webview_window("popup")
            .ok_or("Missing test popup")?;
        let scale = window.scale_factor().map_err(|error| error.to_string())?;
        let inner = window.inner_size().map_err(|error| error.to_string())?;
        let outer = window.outer_size().map_err(|error| error.to_string())?;
        let monitor = window
            .current_monitor()
            .map_err(|error| error.to_string())?
            .ok_or("The test popup has no monitor")?;
        let area = monitor.work_area().size;
        let frame = PhysicalSize::new(
            outer.width.saturating_sub(inner.width),
            outer.height.saturating_sub(inner.height),
        );
        if frame.height == 0 {
            return Err("The test popup has no native frame height to exercise".into());
        }
        let margin = (16.0 * scale).ceil() as u32;
        let desired = tauri::LogicalSize::new(320.0, f64::from(area.height) / scale + 1000.0);
        let requested = desired.to_physical::<u32>(scale);
        let expected = PhysicalSize::new(
            requested.width.min(
                area.width
                    .saturating_sub(margin)
                    .saturating_sub(frame.width)
                    .max(1),
            ),
            area.height
                .saturating_sub(margin)
                .saturating_sub(frame.height)
                .max(1),
        );
        commands::resize_popup(app, desired.height)?;
        Ok((desired.height, expected, frame.height))
    })?;
    let settle_deadline = Instant::now() + Duration::from_secs(2);
    let mut stable_samples = 0;
    while stable_samples < 4 {
        if Instant::now() >= settle_deadline {
            return Err(format!("The popup did not settle at {expected:?}"));
        }
        std::thread::sleep(Duration::from_millis(40));
        let size = on_main(app, |app| {
            app.get_webview_window("popup")
                .ok_or("Missing test popup")?
                .inner_size()
                .map_err(|error| error.to_string())
        })?;
        stable_samples = if size == expected {
            stable_samples + 1
        } else {
            0
        };
    }
    state.resize_events.lock().clear();
    for _ in 0..REPEATED_REQUESTS {
        on_main(app, move |app| commands::resize_popup(app, height))?;
        std::thread::sleep(Duration::from_millis(20));
    }
    let settled = on_main(app, |app| {
        app.get_webview_window("popup")
            .ok_or("Missing test popup")?
            .inner_size()
            .map_err(|error| error.to_string())
    })?;
    if settled != expected {
        return Err(format!(
            "Expected final size {expected:?}, observed {settled:?}"
        ));
    }
    Ok(ProbeReport {
        settled,
        frame_height,
        repeated_requests: REPEATED_REQUESTS,
        resize_events: state.resize_events.lock().clone(),
    })
}

#[test]
#[ignore = "Requires Windows WebView2 and a desktop session; creates an isolated hidden popup"]
fn repeated_tall_popup_requests_keep_native_size_stable() {
    let profile = tempfile::Builder::new()
        .prefix("quota-control-popup-resize-")
        .tempdir()
        .expect("Cannot create an isolated WebView2 profile");
    let data_directory = profile.path().join("webview");
    let state = Arc::new(ProbeState::default());
    let setup_state = state.clone();
    let mut context = tauri::test::mock_context::<tauri::Wry, _>(tauri::test::noop_assets());
    context.config_mut().identifier = format!(
        "com.buidangminh.quota-control.popup-resize-test-{}",
        std::process::id()
    );
    let app = tauri::Builder::<tauri::Wry>::default()
        .any_thread()
        .manage(PopupAnchor::default())
        .setup(move |app| {
            let handle = app.handle().clone();
            let watchdog_handle = handle.clone();
            let watchdog_state = setup_state.clone();
            std::thread::spawn(move || {
                std::thread::sleep(PROBE_TIMEOUT);
                finish(
                    &watchdog_handle,
                    &watchdog_state,
                    Err("The isolated native popup probe timed out".into()),
                );
            });
            let window = WebviewWindowBuilder::new(
                app,
                "popup",
                WebviewUrl::External("about:blank".parse().expect("Valid blank-page URL")),
            )
            .title("Quota Control popup resize test")
            .inner_size(320.0, 400.0)
            .resizable(false)
            .decorations(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .visible(false)
            .data_directory(data_directory.clone())
            .build();
            let window = match window {
                Ok(window) => window,
                Err(error) => {
                    finish(&handle, &setup_state, Err(error.to_string()));
                    return Ok(());
                }
            };
            let event_state = setup_state.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Resized(size) = event {
                    event_state.resize_events.lock().push(*size);
                }
            });
            let worker_state = setup_state.clone();
            std::thread::spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| probe(&handle, &worker_state)))
                    .unwrap_or_else(|_| Err("The native popup probe panicked".into()));
                finish(&handle, &worker_state, result);
            });
            Ok(())
        })
        .build(context)
        .expect("Cannot create the isolated native app");
    let code = app.run_return(|_, _| {});
    let result = state
        .outcome
        .lock()
        .take()
        .expect("The native probe did not report");
    let report = result.expect("The native popup probe failed");
    println!("{report:?}");
    assert_eq!(code, 0, "The isolated native app did not exit cleanly");
    assert!(report.frame_height > 0);
    assert_eq!(report.repeated_requests, REPEATED_REQUESTS);
    assert!(
        report.resize_events.is_empty(),
        "Repeated tall requests resized a settled popup at {:?}: {:?}",
        report.settled,
        report.resize_events
    );
}
