mod account_commands;
mod chat_commands;
mod chat_store;
mod commands;
mod ipc_guard;
mod service;

use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

use service::{BackendService, safe_error};

pub fn run() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--diagnose") {
        return diagnose();
    }
    std::fs::create_dir_all(uc_core::paths::log_dir())?;
    let appender = tracing_appender::rolling::never(uc_core::paths::log_dir(), "UsageControl.log");
    let (writer, _guard) = tracing_appender::non_blocking(appender);
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
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .invoke_handler(ipc_guard::trusted_handler(tauri::generate_handler![
            commands::app_info,
            commands::catalog,
            commands::engine_state,
            commands::refresh,
            commands::set_enabled_providers,
            commands::load_document,
            commands::save_document,
            commands::resize_popup,
            commands::hide_popup,
            commands::open_url,
            commands::copy_image_png,
            commands::copy_text,
            commands::set_tray_icon,
            commands::quit_app,
            account_commands::list_accounts,
            account_commands::import_current_account,
            account_commands::begin_account_login,
            account_commands::complete_account_login,
            account_commands::cancel_account_login,
            account_commands::remove_account,
            chat_commands::list_chat_sessions,
            chat_commands::create_chat_session,
            chat_commands::open_chat_session,
        ]))
        .setup(|app| {
            let accounts = account_commands::Accounts::new(std::sync::Arc::new(
                uc_accounts::AccountStore::default_store(),
            ));
            let service = BackendService::new(accounts.runtimes().map_err(anyhow::Error::msg)?)?;
            app.manage(accounts);
            app.manage(service);
            app.manage(chat_store::ChatStore::default_store());
            app.manage(chat_commands::ChatWindows::default());
            let window =
                WebviewWindowBuilder::new(app, "popup", WebviewUrl::App("index.html".into()))
                    .title("Usage Control")
                    .inner_size(320.0, 400.0)
                    .resizable(false)
                    .decorations(false)
                    .skip_taskbar(true)
                    .always_on_top(true)
                    .visible(false)
                    .build()?;
            let handle = app.handle().clone();
            window.on_window_event(move |event| match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = hide_popup(&handle);
                }
                WindowEvent::Focused(false) => {
                    let _ = hide_popup(&handle);
                }
                _ => {}
            });
            let menu = tray_menu(app.handle()).map_err(anyhow::Error::msg)?;
            TrayIconBuilder::with_id("main")
                .icon(
                    app.default_window_icon()
                        .cloned()
                        .ok_or("Missing app icon")?,
                )
                .tooltip("Usage Control")
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
                    "quit" => app.exit(0),
                    id if id.starts_with("chat:") || id.starts_with("chat-new:") => {
                        let app = app.clone();
                        let id = id.to_owned();
                        tauri::async_runtime::spawn(async move {
                            let store = app.state::<chat_store::ChatStore>();
                            let result = if let Some(provider) = id.strip_prefix("chat-new:") {
                                chat_commands::create_session(&app, &store, provider, None).await.map(|_| ())
                            } else {
                                chat_commands::open_session(&app, &store, &id[5..]).await
                            };
                            if result.is_err() {
                                use tauri_plugin_notification::NotificationExt;
                                let _ = app.notification().builder()
                                    .title("Usage Control")
                                    .body("Không mở được phiên chat. Phiên đã lưu vẫn được giữ nguyên.")
                                    .show();
                            }
                        });
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        let visible = app
                            .get_webview_window("popup")
                            .and_then(|w| w.is_visible().ok())
                            .unwrap_or(false);
                        let result = if visible {
                            hide_popup(app)
                        } else {
                            show_popup(app)
                        };
                        if let Err(error) = result {
                            tracing::warn!("{error}");
                        }
                    }
                })
                .build(app)?;
            app.state::<BackendService>().start(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())?;
    Ok(())
}

fn tray_menu(app: &AppHandle) -> Result<Menu<tauri::Wry>, String> {
    let english = app
        .state::<BackendService>()
        .load("settings")
        .ok()
        .flatten()
        .and_then(|value| {
            value
                .get("language")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .is_some_and(|language| language == "en");
    let text = |vi, en| if english { en } else { vi };
    let show = MenuItem::with_id(
        app,
        "show",
        text("Mở Usage Control", "Show Usage Control"),
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
    let sessions = Submenu::with_id(
        app,
        "chat-sessions",
        text("Các phiên chat đã lưu", "Saved chat sessions"),
        true,
    )
    .map_err(safe_error)?;
    match app.state::<chat_store::ChatStore>().list() {
        Ok(records) if !records.is_empty() => {
            for record in records {
                let provider = if record.provider == "codex" {
                    "ChatGPT"
                } else {
                    "Claude"
                };
                let label = format!("{provider} · {} · {}", record.label, &record.id[..8]);
                sessions
                    .append(
                        &MenuItem::with_id(
                            app,
                            format!("chat:{}", record.id),
                            label,
                            true,
                            None::<&str>,
                        )
                        .map_err(safe_error)?,
                    )
                    .map_err(safe_error)?;
            }
        }
        result => {
            let label = if result.is_err() {
                text(
                    "Không đọc được danh sách phiên",
                    "Cannot read saved sessions",
                )
            } else {
                text("Chưa có phiên chat", "No saved chat sessions")
            };
            sessions
                .append(
                    &MenuItem::with_id(app, "chat-empty", label, false, None::<&str>)
                        .map_err(safe_error)?,
                )
                .map_err(safe_error)?;
        }
    }
    let claude = MenuItem::with_id(
        app,
        "chat-new:claude",
        text("Đăng nhập Claude mới…", "New Claude sign-in…"),
        true,
        None::<&str>,
    )
    .map_err(safe_error)?;
    let codex = MenuItem::with_id(
        app,
        "chat-new:codex",
        text("Đăng nhập ChatGPT mới…", "New ChatGPT sign-in…"),
        true,
        None::<&str>,
    )
    .map_err(safe_error)?;
    let quit = MenuItem::with_id(app, "quit", text("Thoát", "Quit"), true, None::<&str>)
        .map_err(safe_error)?;
    Menu::with_items(app, &[&show, &refresh, &sessions, &claude, &codex, &quit]).map_err(safe_error)
}

pub(crate) fn update_tray_menu(app: &AppHandle) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(tray_menu(app)?)).map_err(safe_error)?;
    }
    Ok(())
}

fn show_popup(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("popup")
        .ok_or("Popup is unavailable")?;
    position_popup(app)?;
    window.show().map_err(safe_error)?;
    window.set_focus().map_err(safe_error)?;
    window.emit("popup-visibility", true).map_err(safe_error)?;
    app.state::<BackendService>().engine().wake();
    Ok(())
}

fn hide_popup(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("popup")
        .ok_or("Popup is unavailable")?;
    window.hide().map_err(safe_error)?;
    window.emit("popup-visibility", false).map_err(safe_error)
}

fn position_popup(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("popup")
        .ok_or("Popup is unavailable")?;
    let tray_rect = app
        .tray_by_id("main")
        .and_then(|tray| tray.rect().ok().flatten());
    let scale = window.scale_factor().map_err(safe_error)?;
    let anchor = tray_rect.map(|rect| rect.position.to_physical::<i32>(scale));
    let monitor = if let Some(point) = anchor {
        window
            .monitor_from_point(f64::from(point.x), f64::from(point.y))
            .map_err(safe_error)?
    } else {
        window.current_monitor().map_err(safe_error)?
    }
    .or(window.primary_monitor().map_err(safe_error)?);
    if let Some(monitor) = monitor {
        let area = monitor.work_area();
        let size = window.outer_size().map_err(safe_error)?;
        let min_x = area.position.x;
        let min_y = area.position.y;
        let max_x = (min_x + area.size.width as i32 - size.width as i32).max(min_x);
        let max_y = (min_y + area.size.height as i32 - size.height as i32).max(min_y);
        let point = anchor.unwrap_or(PhysicalPosition::new(max_x, max_y + size.height as i32));
        let x = (point.x - size.width as i32 / 2).clamp(min_x, max_x);
        let y = if point.y < min_y + area.size.height as i32 / 2 {
            point.y + 24
        } else {
            point.y - size.height as i32 - 8
        }
        .clamp(min_y, max_y);
        window
            .set_position(PhysicalPosition::new(x, y))
            .map_err(safe_error)?;
    }
    Ok(())
}

fn diagnose() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let mut runtimes = uc_providers::default_runtimes();
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
