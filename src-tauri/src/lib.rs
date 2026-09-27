mod account_commands;
mod browser;
mod chat_commands;
mod chat_store;
mod cli_install;
mod commands;
pub mod exchange_rate;
mod glance;
mod insights_commands;
mod integrations;
mod ipc_guard;
mod limit_resets;
#[cfg(target_os = "macos")]
mod macos;
pub mod public_feeds;
mod service;
mod shortcut;
mod taskbar_strip;
mod updates;
mod usage_commands;

use tauri::menu::{ContextMenu, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};

use std::time::{Duration, Instant};

use service::{BackendService, safe_error};
use tauri_plugin_autostart::ManagerExt as _;

/// How often the app looks for a CLI that signed in, out, or into another account. Opening the
/// popup looks as well.
const CLI_LOGIN_CHECK: std::time::Duration = std::time::Duration::from_secs(30);

/// Keeps the log writer's worker alive; see [`flush_log`].
static LOG_GUARD: parking_lot::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>> =
    parking_lot::Mutex::new(None);

/// Write out every queued log line before the process ends without unwinding (the update
/// handover exits or restarts in place). Lines logged afterwards are dropped.
pub(crate) fn flush_log() {
    drop(LOG_GUARD.lock().take());
}

pub fn run() -> anyhow::Result<()> {
    #[cfg(not(windows))]
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "--cli") {
        let arguments = std::env::args_os()
            .skip(2)
            .map(|argument| argument.to_string_lossy().into_owned());
        std::process::exit(uc_api::cli::main(arguments));
    }
    if std::env::args().any(|arg| arg == "--diagnose") {
        return diagnose();
    }
    if std::env::args().any(|arg| arg == "--unregister-cli") {
        return cli_install::unregister().map_err(anyhow::Error::msg);
    }
    std::fs::create_dir_all(uc_core::paths::log_dir())?;
    let appender = tracing_appender::rolling::never(uc_core::paths::log_dir(), "UsageControl.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    *LOG_GUARD.lock() = Some(guard);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .try_init()
        .ok();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Err(error) = show_popup(app) {
                tracing::warn!("{error}");
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(shortcut::plugin())
        .plugin(tauri_plugin_autostart::Builder::new().app_name("Usage Control").build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(ipc_guard::trusted_handler(tauri::generate_handler![
            commands::app_info,
            usage_commands::usage_summary,
            usage_commands::usage_ledger_info,
            usage_commands::exchange_rate,
            usage_commands::context_windows,
            insights_commands::model_quality,
            insights_commands::rescan_model_quality,
            insights_commands::public_feed,
            insights_commands::refresh_public_feed,
            commands::catalog,
            commands::engine_state,
            commands::refresh,
            limit_resets::redeem_limit_reset,
            commands::set_enabled_providers,
            commands::load_document,
            commands::save_document,
            commands::resize_popup,
            commands::hide_popup,
            commands::open_url,
            commands::copy_text,
            commands::system_time_zone,
            commands::set_tray_icon,
            commands::quit_app,
            account_commands::list_accounts,
            account_commands::begin_account_login,
            account_commands::reopen_account_login,
            account_commands::cancel_account_login,
            account_commands::remove_account,
            account_commands::list_services,
            account_commands::add_api_key,
            account_commands::remove_api_key,
            chat_commands::list_chat_sessions,
            chat_commands::create_chat_session,
            chat_commands::open_chat_session,
            taskbar_strip::taskbar_info,
            taskbar_strip::set_taskbar_strip,
            glance::set_glance,
            shortcut::global_shortcut,
            shortcut::set_global_shortcut,
            shortcut::pause_global_shortcut,
            updates::update_status,
            updates::check_for_update,
            updates::install_update,
        ]))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            if !cfg!(debug_assertions)
                && app.autolaunch().is_enabled().unwrap_or(false)
                && let Err(error) = app.autolaunch().enable()
            {
                tracing::warn!("Could not refresh launch-at-login path: {error}");
            }
            app.manage(integrations::IntegrationStore::default_store());
            app.manage(usage_commands::UsageService::new()?);
            app.manage(insights_commands::InsightsService::new());
            let accounts = account_commands::Accounts::new(
                std::sync::Arc::new(uc_accounts::AccountStore::default_store()),
                uc_accounts::KeyStore::default_store(),
            );
            let runtimes = accounts.runtimes().map_err(anyhow::Error::msg)?;
            let service = BackendService::new(runtimes, &accounts.new_card_ids())?;
            app.manage(accounts);
            app.manage(service);
            app.manage(chat_store::ChatStore::default_store());
            app.manage(chat_commands::ChatWindows::default());
            app.manage(PopupAnchor::default());
            app.manage(PopupClicks::default());
            app.manage(commands::TrayImage::default());
            app.manage(limit_resets::Redemptions::default());
            app.manage(updates::Updates::new(app.handle()));
            app.manage(glance::Glance::default());
            let window =
                WebviewWindowBuilder::new(app, "popup", WebviewUrl::App("index.html".into()))
                    .title("Quota Control")
                    .inner_size(320.0, 400.0)
                    .resizable(false)
                    .decorations(false)
                    .skip_taskbar(true)
                    .always_on_top(true)
                    .visible(false)
                    .build()?;
            #[cfg(target_os = "macos")]
            macos::configure_popup(&window);
            let handle = app.handle().clone();
            window.on_window_event(move |event| match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = hide_popup(&handle);
                }
                WindowEvent::Focused(false) => {
                    if popup_visible(&handle) {
                        *handle.state::<PopupClicks>().blurred_at.lock() = Some(Instant::now());
                    }
                    let _ = hide_popup(&handle);
                }
                _ => {}
            });
            let menu = tray_menu(app.handle()).map_err(anyhow::Error::msg)?;
            TrayIconBuilder::with_id("main")
                .icon(tray_icon(app.handle())?)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("Quota Control")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        let _ = show_popup(app);
                    }
                    "refresh" => {
                        let engine = app.state::<BackendService>().engine();
                        tauri::async_runtime::spawn(async move {
                            engine.refresh_all(true).await;
                        });
                    }
                    "update" => {
                        let _ = show_popup(app);
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let updates = app.state::<updates::Updates>();
                            if updates.status().available.is_none() {
                                updates.check(&app, true).await;
                            }
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state,
                        ..
                    } = event
                    else {
                        return;
                    };
                    let app = tray.app_handle();
                    let pressed_at = &app.state::<PopupClicks>().tray_pressed_at;
                    if button_state == MouseButtonState::Down {
                        *pressed_at.lock() = Some(Instant::now());
                        return;
                    }
                    let pressed = pressed_at.lock().take();
                    if let Err(error) = click_popup(app, None, pressed) {
                        tracing::warn!("{error}");
                    }
                })
                .build(app)?;
            let strip_app = app.handle().clone();
            let cover_app = app.handle().clone();
            app.manage(taskbar_strip::TaskbarStrip::install(
                app.handle(),
                move |click| {
                    let result = match click.button {
                        taskbar_strip::StripButton::Primary => {
                            click_popup(&strip_app, Some(click.bounds), click.pressed_at)
                        }
                        taskbar_strip::StripButton::Secondary => show_tray_menu(&strip_app),
                    };
                    if let Err(error) = result {
                        tracing::warn!("{error}");
                    }
                },
                move |covered| {
                    cover_app
                        .state::<commands::TrayImage>()
                        .set_covered(&cover_app, covered);
                },
            ));
            #[cfg(target_os = "macos")]
            macos::start_island(app.handle());
            app.state::<BackendService>().start(app.handle());
            app.state::<updates::Updates>().start(app.handle());
            usage_commands::start(app.handle());
            insights_commands::start(app.handle());
            let label_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                account_commands::backfill_account_labels(&label_app).await;
            });
            let cli_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut ticks = tokio::time::interval(CLI_LOGIN_CHECK);
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                ticks.tick().await;
                loop {
                    ticks.tick().await;
                    account_commands::sync_logins(&cli_app).await;
                }
            });
            if let Err(error) = shortcut::restore(app.handle()) {
                tracing::warn!("The saved global shortcut is unavailable: {error}");
            }
            cli_install::sync_at_launch();
            let api_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match uc_api::server::bind().await {
                    Ok(listener) => {
                        tracing::info!(target: "local_api", "listening on 127.0.0.1:{}", uc_api::server::PORT);
                        uc_api::server::serve(listener, move || {
                            api_app.state::<BackendService>().api_state()
                        })
                        .await;
                    }
                    Err(error) => tracing::info!(target: "local_api", "disabled: {error}"),
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())?
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event
                && let Err(error) = show_popup(app)
            {
                tracing::warn!("{error}");
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
    Ok(())
}

/// The tray's resting icon: the app icon, or on macOS a monochrome gauge that the menu bar tints.
fn tray_icon(app: &AppHandle) -> Result<tauri::image::Image<'static>, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        menu_bar_icon().map_err(safe_error)
    }
    #[cfg(not(target_os = "macos"))]
    {
        app.default_window_icon()
            .map(|icon| icon.clone().to_owned())
            .ok_or_else(|| "Missing app icon".to_owned())
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn menu_bar_icon() -> tauri::Result<tauri::image::Image<'static>> {
    tauri::image::Image::from_bytes(include_bytes!("../icons/menubar-template.png"))
}

/// Whether native text (menus, notifications) should be English; the popup's language setting.
pub(crate) fn english(app: &AppHandle) -> bool {
    app.state::<BackendService>()
        .load("settings")
        .ok()
        .flatten()
        .and_then(|value| {
            value
                .get("language")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .is_some_and(|language| language == "en")
}

fn tray_menu(app: &AppHandle) -> Result<Menu<tauri::Wry>, String> {
    let english = english(app);
    let text = |vi, en| if english { en } else { vi };
    let show = MenuItem::with_id(
        app,
        "show",
        text("Mở Quota Control", "Show Quota Control"),
        true,
        None::<&str>,
    )
    .map_err(safe_error)?;
    let refresh = MenuItem::with_id(
        app,
        "refresh",
        text("Cập nhật hạn mức", "Refresh usage"),
        true,
        None::<&str>,
    )
    .map_err(safe_error)?;
    let update = app
        .state::<updates::Updates>()
        .menu_label(english)
        .map(|label| MenuItem::with_id(app, "update", label, true, None::<&str>))
        .transpose()
        .map_err(safe_error)?;
    let quit = MenuItem::with_id(app, "quit", text("Thoát", "Quit"), true, None::<&str>)
        .map_err(safe_error)?;
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![&show, &refresh];
    if let Some(update) = &update {
        items.push(update);
    }
    items.push(&quit);
    Menu::with_items(app, &items).map_err(safe_error)
}

pub(crate) fn update_tray_menu(app: &AppHandle) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(tray_menu(app)?)).map_err(safe_error)?;
    }
    Ok(())
}

fn popup_visible(app: &AppHandle) -> bool {
    app.get_webview_window("popup")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

fn toggle_popup(app: &AppHandle, anchor: Option<PhysicalRect<i32, u32>>) -> Result<(), String> {
    if popup_visible(app) {
        hide_popup(app)
    } else {
        show_popup_at(app, anchor)
    }
}

/// How long before a click's press the popup may have hidden and still count as closed by that
/// click: its focus loss and the press reach the app on different threads, a little apart.
const CLICK_CLOSE_SLACK: Duration = Duration::from_millis(150);

/// Without a recorded press, how recent a hide must be to count as closed by the click.
const CLICK_CLOSE_WINDOW: Duration = Duration::from_millis(500);

/// When the popup last hid because it lost focus, and when the tray icon's button last went down.
#[derive(Default)]
struct PopupClicks {
    blurred_at: parking_lot::Mutex<Option<Instant>>,
    tray_pressed_at: parking_lot::Mutex<Option<Instant>>,
}

/// Whether a click that ended at `now` closed the popup rather than asking for it. Pressing the
/// tray icon or the taskbar strip takes focus from an open popup, which hides it before the
/// button comes back up, so the popup is already gone when the click arrives.
fn closed_by_click(pressed_at: Option<Instant>, blurred_at: Option<Instant>, now: Instant) -> bool {
    let Some(blurred) = blurred_at.filter(|blurred| *blurred <= now) else {
        return false;
    };
    match pressed_at {
        Some(pressed) => blurred + CLICK_CLOSE_SLACK >= pressed,
        None => now.duration_since(blurred) <= CLICK_CLOSE_WINDOW,
    }
}

/// A primary click on the tray icon or the taskbar strip: opens the popup, and closes it when it is
/// open, including when the click's own press already hid it.
fn click_popup(
    app: &AppHandle,
    anchor: Option<PhysicalRect<i32, u32>>,
    pressed_at: Option<Instant>,
) -> Result<(), String> {
    let blurred_at = app.state::<PopupClicks>().blurred_at.lock().take();
    if !popup_visible(app) && closed_by_click(pressed_at, blurred_at, Instant::now()) {
        return Ok(());
    }
    toggle_popup(app, anchor)
}

/// The tray icon's menu at the pointer, for a right click on the taskbar strip.
fn show_tray_menu(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("popup")
        .ok_or("Popup is unavailable")?;
    tray_menu(app)?
        .popup(window.as_ref().window())
        .map_err(safe_error)
}

pub(crate) fn show_popup(app: &AppHandle) -> Result<(), String> {
    show_popup_at(app, None)
}

/// Show the popup under `anchor`: the Dynamic Island's rectangle, in global points on macOS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn show_popup_anchored(
    app: &AppHandle,
    anchor: PhysicalRect<i32, u32>,
) -> Result<(), String> {
    show_popup_at(app, Some(anchor))
}

/// Show the popup against `anchor` (a screen rectangle in physical pixels, such as the taskbar
/// strip), or against the tray icon when there is none.
fn show_popup_at(app: &AppHandle, anchor: Option<PhysicalRect<i32, u32>>) -> Result<(), String> {
    let window = app
        .get_webview_window("popup")
        .ok_or("Popup is unavailable")?;
    *app.state::<PopupAnchor>().0.lock() = anchor;
    position_popup(app)?;
    #[cfg(target_os = "macos")]
    macos::set_popup_visible(true);
    if let Some(strip) = app.try_state::<taskbar_strip::TaskbarStrip>() {
        strip.set_popup_visible(true);
    }
    window.show().map_err(safe_error)?;
    window.set_focus().map_err(safe_error)?;
    window.emit("popup-visibility", true).map_err(safe_error)?;
    app.state::<BackendService>().engine().wake();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move { account_commands::sync_logins(&handle).await });
    Ok(())
}

fn hide_popup(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("popup")
        .ok_or("Popup is unavailable")?;
    window.hide().map_err(safe_error)?;
    #[cfg(target_os = "macos")]
    macos::set_popup_visible(false);
    if let Some(strip) = app.try_state::<taskbar_strip::TaskbarStrip>() {
        strip.set_popup_visible(false);
    }
    window.emit("popup-visibility", false).map_err(safe_error)
}

/// Gap, in physical pixels, between the popup and the rectangle it opens against.
const POPUP_GAP: i32 = 8;

/// What the popup was last opened against (`None`: the tray icon), so resizes keep it there. On
/// macOS the rectangle is in global points, elsewhere in physical pixels.
#[derive(Default)]
struct PopupAnchor(parking_lot::Mutex<Option<PhysicalRect<i32, u32>>>);

fn rect_center(rect: PhysicalRect<i32, u32>) -> PhysicalPosition<i32> {
    PhysicalPosition::new(
        rect.position.x + rect.size.width as i32 / 2,
        rect.position.y + rect.size.height as i32 / 2,
    )
}

/// Center the popup on its anchor (falling back to the tray icon), opening away from the screen
/// edge the anchor sits on and staying inside that monitor's work area.
#[cfg(not(target_os = "macos"))]
fn position_popup(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("popup")
        .ok_or("Popup is unavailable")?;
    let scale = window.scale_factor().map_err(safe_error)?;
    let stored = *app.state::<PopupAnchor>().0.lock();
    let anchor = stored.or_else(|| {
        app.tray_by_id("main")
            .and_then(|tray| tray.rect().ok().flatten())
            .map(|rect| PhysicalRect {
                position: rect.position.to_physical::<i32>(scale),
                size: rect.size.to_physical::<u32>(scale),
            })
    });
    let monitor = if let Some(rect) = anchor {
        let center = rect_center(rect);
        window
            .monitor_from_point(f64::from(center.x), f64::from(center.y))
            .map_err(safe_error)?
    } else {
        window.current_monitor().map_err(safe_error)?
    }
    .or(window.primary_monitor().map_err(safe_error)?);
    if let Some(monitor) = monitor {
        let size = window.outer_size().map_err(safe_error)?;
        window
            .set_position(popup_origin(anchor, *monitor.work_area(), size))
            .map_err(safe_error)?;
    }
    Ok(())
}

/// macOS works in global points throughout: displays may mix backing scales, tao reads a monitor
/// lookup point as points, and a logical position is placed exactly. Each monitor's physical
/// frame is turned back into points with its own scale, and so is the tray icon's rectangle.
#[cfg(target_os = "macos")]
fn position_popup(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("popup")
        .ok_or("Popup is unavailable")?;
    let window_scale = window.scale_factor().map_err(safe_error)?;
    let outer = window.outer_size().map_err(safe_error)?;
    let size = PhysicalSize::new(
        (f64::from(outer.width) / window_scale).round() as u32,
        (f64::from(outer.height) / window_scale).round() as u32,
    );
    let monitors: Vec<(PhysicalRect<i32, u32>, PhysicalRect<i32, u32>, f64)> = window
        .available_monitors()
        .map_err(safe_error)?
        .iter()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let bounds = PhysicalRect {
                position: *monitor.position(),
                size: *monitor.size(),
            };
            (
                in_points(bounds, scale),
                in_points(*monitor.work_area(), scale),
                scale,
            )
        })
        .collect();
    let containing = |rect: PhysicalRect<i32, u32>| {
        let center = rect_center(rect);
        monitors
            .iter()
            .find(|(bounds, _, _)| contains(*bounds, center))
    };
    let stored = *app.state::<PopupAnchor>().0.lock();
    let anchor = stored.or_else(|| {
        let rect = app.tray_by_id("main")?.rect().ok().flatten()?;
        let physical = PhysicalRect {
            position: rect.position.to_physical::<i32>(1.0),
            size: rect.size.to_physical::<u32>(1.0),
        };
        monitors
            .iter()
            .map(|(bounds, _, scale)| (*bounds, in_points(physical, *scale)))
            .find(|(bounds, candidate)| contains(*bounds, rect_center(*candidate)))
            .map(|(_, candidate)| candidate)
    });
    let area = anchor
        .and_then(|rect| containing(rect).map(|(_, area, _)| *area))
        .or_else(|| {
            let primary = window.primary_monitor().ok().flatten()?;
            Some(in_points(*primary.work_area(), primary.scale_factor()))
        });
    if let Some(area) = area {
        let origin = popup_origin(anchor, area, size);
        window
            .set_position(tauri::LogicalPosition::new(
                f64::from(origin.x),
                f64::from(origin.y),
            ))
            .map_err(safe_error)?;
    }
    Ok(())
}

/// `rect` in physical pixels of a display with backing `scale`, as whole points.
#[cfg(target_os = "macos")]
fn in_points(rect: PhysicalRect<i32, u32>, scale: f64) -> PhysicalRect<i32, u32> {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let points = |value: f64| (value / scale).round();
    PhysicalRect {
        position: PhysicalPosition::new(
            points(f64::from(rect.position.x)) as i32,
            points(f64::from(rect.position.y)) as i32,
        ),
        size: PhysicalSize::new(
            points(f64::from(rect.size.width)) as u32,
            points(f64::from(rect.size.height)) as u32,
        ),
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn contains(rect: PhysicalRect<i32, u32>, point: PhysicalPosition<i32>) -> bool {
    point.x >= rect.position.x
        && point.y >= rect.position.y
        && point.x < rect.position.x + rect.size.width as i32
        && point.y < rect.position.y + rect.size.height as i32
}

/// The popup's top-left: centered on `anchor` and opening away from the screen edge it sits on,
/// kept inside `area`; with no anchor, the area's bottom-right corner.
fn popup_origin(
    anchor: Option<PhysicalRect<i32, u32>>,
    area: PhysicalRect<i32, u32>,
    size: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    let min_x = area.position.x;
    let min_y = area.position.y;
    let max_x = (min_x + area.size.width as i32 - size.width as i32).max(min_x);
    let max_y = (min_y + area.size.height as i32 - size.height as i32).max(min_y);
    let Some(rect) = anchor else {
        return PhysicalPosition::new(max_x, max_y);
    };
    let top = rect.position.y;
    let x = rect_center(rect).x - size.width as i32 / 2;
    let y = if top < min_y + area.size.height as i32 / 2 {
        top + rect.size.height as i32 + POPUP_GAP
    } else {
        top - size.height as i32 - POPUP_GAP
    };
    PhysicalPosition::new(x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}

fn diagnose() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let mut runtimes = uc_providers::default_runtimes();
        let services = uc_api::ServiceCards::scan(
            uc_accounts::KeyStore::default_store(),
            uc_services::Roots::system(),
        );
        runtimes.extend(services.runtimes());
        runtimes.push(std::sync::Arc::new(uc_logscan::LocalHistoryRuntime::new(
            uc_logscan::LogSource::Claude,
        )));
        runtimes.push(std::sync::Arc::new(uc_logscan::LocalHistoryRuntime::new(
            uc_logscan::LogSource::Codex,
        )));
        let mut summary = Vec::new();
        for provider in runtimes {
            let snapshot = provider.refresh(uc_core::RefreshContext::manual()).await;
            summary.push(serde_json::json!({
                "provider": provider.provider().id,
                "status": if snapshot.error_category.is_some() { "error" } else { "ok" },
                "error": snapshot.error_text(),
                "metrics": snapshot.lines.iter().map(|line| line.label()).collect::<Vec<_>>(),
                "warning": snapshot.warning,
            }));
        }
        println!("{}", serde_json::to_string_pretty(&summary)?);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_whose_press_hid_the_popup_leaves_it_closed() {
        let pressed = Instant::now();
        let after = |ms| pressed + Duration::from_millis(ms);
        assert!(closed_by_click(Some(pressed), Some(after(40)), after(120)));
        assert!(closed_by_click(Some(after(20)), Some(pressed), after(120)));
    }

    #[test]
    fn a_popup_that_hid_before_the_press_opens_again() {
        let blurred = Instant::now();
        let after = |ms| blurred + Duration::from_millis(ms);
        assert!(!closed_by_click(
            Some(after(400)),
            Some(blurred),
            after(480)
        ));
        assert!(!closed_by_click(
            Some(after(5_000)),
            Some(blurred),
            after(5_080)
        ));
        assert!(!closed_by_click(Some(after(400)), None, after(480)));
        assert!(!closed_by_click(None, None, after(10)));
    }

    #[test]
    fn without_a_press_only_a_recent_hide_counts() {
        let blurred = Instant::now();
        let after = |ms| blurred + Duration::from_millis(ms);
        assert!(closed_by_click(None, Some(blurred), after(200)));
        assert!(!closed_by_click(None, Some(blurred), after(1_500)));
    }

    const POPUP: PhysicalSize<u32> = PhysicalSize {
        width: 336,
        height: 797,
    };

    fn rect(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect<i32, u32> {
        PhysicalRect {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    /// A 1080p monitor with a 56 px taskbar along the bottom.
    fn work_area() -> PhysicalRect<i32, u32> {
        rect(0, 0, 1920, 1024)
    }

    fn strip() -> PhysicalRect<i32, u32> {
        rect(1469, 1024, 125, 56)
    }

    #[test]
    fn a_rectangle_contains_its_near_edges_but_not_its_far_ones() {
        let area = rect(0, 0, 100, 50);
        assert!(contains(area, PhysicalPosition::new(0, 0)));
        assert!(contains(area, PhysicalPosition::new(99, 49)));
        assert!(!contains(area, PhysicalPosition::new(100, 10)));
        assert!(!contains(area, PhysicalPosition::new(10, -1)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn each_display_turns_back_into_points_with_its_own_scale() {
        let built_in = in_points(rect(0, 0, 2940, 1912), 2.0);
        assert_eq!((built_in.size.width, built_in.size.height), (1470, 956));
        let external = in_points(rect(1470, 0, 1920, 1080), 1.0);
        assert_eq!((external.position.x, external.size.width), (1470, 1920));
        let island = rect(582, 0, 307, 32);
        assert!(contains(built_in, rect_center(island)));
        assert!(!contains(external, rect_center(island)));
        assert_eq!(in_points(rect(10, 10, 20, 20), f64::NAN).size.width, 20);
    }

    #[test]
    fn opens_centered_above_a_strip_on_a_bottom_taskbar() {
        let origin = popup_origin(Some(strip()), work_area(), POPUP);
        assert_eq!((origin.x, origin.y), (1363, 219));
    }

    #[test]
    fn opens_below_an_anchor_on_a_top_taskbar() {
        let top_area = rect(0, 56, 1920, 1024);
        let origin = popup_origin(Some(rect(1469, 0, 125, 56)), top_area, POPUP);
        assert_eq!((origin.x, origin.y), (1363, 64));
    }

    #[test]
    fn stays_inside_the_work_area_near_its_edges() {
        let corner = rect(1880, 1024, 40, 56);
        let origin = popup_origin(Some(corner), work_area(), POPUP);
        assert_eq!((origin.x, origin.y), (1584, 219));
        let tall = PhysicalSize::new(336, 1100);
        assert_eq!(popup_origin(Some(strip()), work_area(), tall).y, 0);
    }

    #[test]
    fn falls_back_to_the_bottom_right_corner_without_an_anchor() {
        let origin = popup_origin(None, work_area(), POPUP);
        assert_eq!((origin.x, origin.y), (1584, 227));
    }
}
