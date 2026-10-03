use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;
use uc_engine::{EngineState, ProviderEntry};

use crate::service::{AppInfo, BackendService, safe_error, validate_url};
use crate::taskbar_strip::{SLOT_ICON_HEIGHT, TraySlot};

#[tauri::command]
pub fn app_info(service: State<'_, BackendService>) -> AppInfo {
    service.info()
}

#[tauri::command]
pub fn catalog(service: State<'_, BackendService>) -> Vec<ProviderEntry> {
    service.engine().catalog()
}

#[tauri::command]
pub fn engine_state(service: State<'_, BackendService>) -> EngineState {
    service.engine().state()
}

#[tauri::command]
pub async fn refresh(
    service: State<'_, BackendService>,
    provider_id: Option<String>,
) -> Result<(), String> {
    let engine = service.engine();
    if let Some(id) = provider_id {
        service.validate_provider_ids(std::slice::from_ref(&id))?;
        engine.refresh(&id, true).await;
    } else {
        engine.refresh_all(true).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_enabled_providers(
    service: State<'_, BackendService>,
    provider_ids: Vec<String>,
) -> Result<(), String> {
    service.set_enabled(&provider_ids)
}

#[tauri::command]
pub async fn load_document(
    service: State<'_, BackendService>,
    name: String,
) -> Result<Option<Value>, String> {
    service.load(&name)
}

#[tauri::command]
pub async fn save_document(
    service: State<'_, BackendService>,
    name: String,
    value: Value,
) -> Result<(), String> {
    service.save(&name, &value)
}

#[tauri::command]
pub fn resize_popup(app: AppHandle, height: f64) -> Result<(), String> {
    if !height.is_finite() || height <= 0.0 {
        return Err("Popup height must be a positive finite number".into());
    }
    let window = app
        .get_webview_window("popup")
        .ok_or("Popup is unavailable")?;
    #[cfg(not(target_os = "macos"))]
    crate::position_popup(&app)?;
    let scale = window.scale_factor().map_err(safe_error)?;
    let maximum = window
        .current_monitor()
        .map_err(safe_error)?
        .map(|monitor| f64::from(monitor.work_area().size.height) / scale - 16.0)
        .unwrap_or(800.0)
        .max(80.0);
    window
        .set_size(tauri::LogicalSize::new(320.0, height.clamp(80.0, maximum)))
        .map_err(safe_error)?;
    crate::position_popup(&app)?;
    #[cfg(target_os = "macos")]
    crate::macos::refresh_popup_shadow(&window);
    Ok(())
}

#[tauri::command]
pub fn hide_popup(app: AppHandle) -> Result<(), String> {
    crate::hide_popup(&app)
}

#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    validate_url(&url)?;
    app.opener().open_url(url, None::<&str>).map_err(safe_error)
}

#[tauri::command]
pub async fn copy_text(app: AppHandle, text: String) -> Result<(), String> {
    if text.len() > 1_048_576 {
        return Err("Clipboard text exceeds the 1 MiB limit".into());
    }
    app.clipboard().write_text(text).map_err(safe_error)
}

/// The operating system's current IANA time zone (e.g. `Asia/Saigon`), read afresh on every call
/// so the popup follows a zone change made while the app runs; the webview can keep the zone its
/// process started with. `None` when the system does not name one.
#[tauri::command]
pub fn system_time_zone() -> Option<String> {
    iana_time_zone::get_timezone()
        .ok()
        .filter(|zone| !zone.is_empty())
}

#[cfg(test)]
mod time_zone_tests {
    #[test]
    fn names_the_system_time_zone_on_desktop_systems() {
        let zone = super::system_time_zone();
        if cfg!(any(windows, target_os = "macos")) {
            let zone = zone.expect("Windows and macOS always name a time zone");
            assert!(
                zone.contains('/') || zone == "UTC",
                "unexpected time zone {zone}"
            );
        }
    }
}

/// The tray icon the popup last asked for (`None`: the app icon), and what the taskbar strip wants
/// the icon's notification-area button to show. While the strip sits in the button, or asks it to
/// grow, the button shows a clear icon as wide as the strip needs, so nothing of the icon peeks out
/// from under the strip; the requested one comes back after.
#[derive(Default)]
pub struct TrayImage {
    requested: parking_lot::Mutex<Option<tauri::image::Image<'static>>>,
    slot: parking_lot::Mutex<TraySlot>,
    applied: std::sync::Arc<parking_lot::Mutex<TrayImageCache>>,
}

#[derive(Default)]
struct TrayImageCache {
    image: Option<tauri::image::Image<'static>>,
}

impl TrayImageCache {
    fn apply(
        &mut self,
        image: tauri::image::Image<'static>,
        update: impl FnOnce(tauri::image::Image<'static>) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.image.as_ref().is_some_and(|current| {
            current.width() == image.width()
                && current.height() == image.height()
                && current.rgba() == image.rgba()
        }) {
            return Ok(());
        }
        update(image.clone())?;
        self.image = Some(image);
        Ok(())
    }
}

impl TrayImage {
    pub fn set_slot(&self, app: &AppHandle, slot: TraySlot) {
        *self.slot.lock() = slot;
        if let Err(error) = self.apply(app) {
            tracing::warn!("could not update the tray icon: {error}");
        }
    }

    #[cfg_attr(any(target_os = "macos", target_os = "linux"), allow(dead_code))]
    fn request(
        &self,
        app: &AppHandle,
        image: Option<tauri::image::Image<'static>>,
    ) -> Result<(), String> {
        *self.requested.lock() = image;
        self.apply(app)
    }

    /// An icon nobody can see, `width` by [`SLOT_ICON_HEIGHT`]: only its aspect ratio matters, to
    /// a styler rule that sizes the button by it. Its pixels keep an alpha of 1, because Windows
    /// draws an icon whose alpha is zero throughout as if it had no alpha at all: a black square.
    fn clear_icon(width: u32) -> tauri::image::Image<'static> {
        let width = width.max(1);
        tauri::image::Image::new_owned(
            [0, 0, 0, 1].repeat((width * SLOT_ICON_HEIGHT) as usize),
            width,
            SLOT_ICON_HEIGHT,
        )
    }

    fn apply(&self, app: &AppHandle) -> Result<(), String> {
        let tray = app.tray_by_id("main").ok_or("Tray is unavailable")?;
        let slot = *self.slot.lock();
        let image = if let TraySlot::Clear { width } = slot {
            Self::clear_icon(width)
        } else if let Some(image) = self.requested.lock().clone() {
            image
        } else {
            let icon = app
                .default_window_icon()
                .ok_or("Default icon is unavailable")?;
            tauri::image::Image::new_owned(icon.rgba().to_vec(), icon.width(), icon.height())
        };
        let applied = self.applied.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let result = applied.lock().apply(image, |image| {
                tray.set_icon(Some(image)).map_err(safe_error)
            });
            let _ = sender.send(result);
        })
        .map_err(safe_error)?;
        receiver.recv().map_err(safe_error)?
    }
}

#[tauri::command]
pub fn set_tray_icon(
    app: AppHandle,
    images: State<'_, TrayImage>,
    png: Option<Vec<u8>>,
    tooltip: String,
) -> Result<(), String> {
    if tooltip.len() > 512 {
        return Err("Tooltip is too long".into());
    }
    let glyph = png.as_deref().map(decode_image).transpose()?;
    set_tray_glyph(&app, &images, glyph, tooltip)
}

/// The macOS menu bar and the Linux panel draw the glyph, the strip and the app icon into the one
/// tray image, so the strip decides which of them shows.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn set_tray_glyph(
    app: &AppHandle,
    _images: &TrayImage,
    glyph: Option<tauri::image::Image<'static>>,
    tooltip: String,
) -> Result<(), String> {
    app.state::<crate::taskbar_strip::TaskbarStrip>()
        .set_glyph(glyph, tooltip);
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn set_tray_glyph(
    app: &AppHandle,
    images: &TrayImage,
    glyph: Option<tauri::image::Image<'static>>,
    tooltip: String,
) -> Result<(), String> {
    let tray = app.tray_by_id("main").ok_or("Tray is unavailable")?;
    images.request(app, glyph)?;
    tray.set_tooltip(Some(tooltip)).map_err(safe_error)
}

fn decode_image(bytes: &[u8]) -> Result<tauri::image::Image<'static>, String> {
    if bytes.len() > 8 * 1_048_576 || bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("Expected a PNG image no larger than 8 MiB".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_777_216 {
        return Err("PNG dimensions are invalid or too large".into());
    }
    tauri::image::Image::from_bytes(bytes).map_err(safe_error)
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use uc_core::{Provider, ProviderRuntime, ProviderSnapshot, RefreshContext, WidgetDescriptor};
    use uc_engine::{DocumentStore, Engine, EngineConfig, SnapshotCache};

    #[test]
    fn tray_image_updates_do_not_replace_an_unchanged_sizing_icon() {
        let mut cache = TrayImageCache::default();
        let mut updates = 0;
        for width in [600, 600, 600, 608, 608] {
            cache
                .apply(TrayImage::clear_icon(width), |_| {
                    updates += 1;
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(updates, 2);
    }

    #[test]
    fn tray_image_updates_preserve_new_pixels_and_dimensions() {
        let mut cache = TrayImageCache::default();
        let mut updates = 0;
        for (pixels, width, height) in [
            (vec![0; 8], 2, 1),
            (vec![0; 8], 2, 1),
            (vec![1; 8], 2, 1),
            (vec![1; 8], 1, 2),
        ] {
            cache
                .apply(
                    tauri::image::Image::new_owned(pixels, width, height),
                    |_| {
                        updates += 1;
                        Ok(())
                    },
                )
                .unwrap();
        }
        assert_eq!(updates, 3);
    }

    #[test]
    fn tray_image_updates_retry_failures_and_restore_icons() {
        let mut cache = TrayImageCache::default();
        let mut attempts = 0;
        let mut apply = |width, fail| {
            cache.apply(TrayImage::clear_icon(width), |_| {
                attempts += 1;
                if fail {
                    Err("unavailable".into())
                } else {
                    Ok(())
                }
            })
        };
        apply(600, false).unwrap();
        assert!(apply(608, true).is_err());
        apply(608, false).unwrap();
        apply(608, false).unwrap();
        apply(16, false).unwrap();
        apply(600, false).unwrap();
        assert_eq!(attempts, 5);
    }

    struct FixtureProvider(Provider);

    #[async_trait]
    impl ProviderRuntime for FixtureProvider {
        fn provider(&self) -> &Provider {
            &self.0
        }
        fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
            vec![]
        }
        async fn refresh(&self, _: RefreshContext) -> ProviderSnapshot {
            ProviderSnapshot::make(&self.0, None, vec![], chrono::Utc::now())
        }
        async fn has_local_credentials(&self) -> bool {
            false
        }
    }

    #[test]
    fn ipc_contract_persists_settings_and_rejects_unknown_providers() {
        let dir = tempfile::tempdir().unwrap();
        let config = EngineConfig::default();
        let cache = SnapshotCache::new(dir.path().join("cache.json"), config.refresh_interval);
        let engine = Arc::new(Engine::new(
            vec![Arc::new(FixtureProvider(Provider::new("codex", "Codex")))],
            cache.clone(),
            config,
        ));
        let service = BackendService::with_storage(
            engine.clone(),
            DocumentStore::new(dir.path().to_path_buf()),
            cache,
            &[],
        )
        .unwrap();
        let app = mock_builder()
            .manage(service)
            .invoke_handler(tauri::generate_handler![
                app_info,
                catalog,
                engine_state,
                refresh,
                load_document,
                save_document,
                set_enabled_providers
            ])
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "popup", Default::default())
            .build()
            .unwrap();
        let invoke = |name: &str, body: Value| {
            tauri::test::get_ipc_response(
                &window,
                tauri::webview::InvokeRequest {
                    cmd: name.into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: if cfg!(any(windows, target_os = "android")) {
                        "http://tauri.localhost"
                    } else {
                        "tauri://localhost"
                    }
                    .parse()
                    .unwrap(),
                    body: tauri::ipc::InvokeBody::Json(body),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.into(),
                },
            )
            .map(|response| response.deserialize::<Value>().unwrap())
        };
        assert_eq!(
            invoke("catalog", serde_json::json!({})).unwrap()[0]["provider"]["id"],
            "codex"
        );
        assert_eq!(
            invoke("engine_state", serde_json::json!({})).unwrap()["refreshIntervalMs"],
            300_000
        );
        assert_eq!(
            invoke("app_info", serde_json::json!({})).unwrap()["name"],
            "Quota Control"
        );
        invoke(
            "set_enabled_providers",
            serde_json::json!({"providerIds":[]}),
        )
        .unwrap();
        assert!(!engine.is_enabled("codex"));
        assert_eq!(
            invoke("load_document", serde_json::json!({"name":"settings"})).unwrap()["enabledProviders"],
            serde_json::json!([])
        );
        assert!(
            invoke(
                "set_enabled_providers",
                serde_json::json!({"providerIds":["unknown"]})
            )
            .is_err()
        );
        assert!(invoke("refresh", serde_json::json!({"providerId":"unknown"})).is_err());
        assert!(invoke("load_document", serde_json::json!({"name":"../../auth"})).is_err());
        invoke("save_document", serde_json::json!({"name":"settings","value":{"enabledProviders":["codex"],"theme":"dark"}})).unwrap();
        assert!(engine.is_enabled("codex"));
        assert_eq!(
            invoke("load_document", serde_json::json!({"name":"settings"})).unwrap()["theme"],
            "dark"
        );
    }

    #[test]
    fn png_validation_rejects_oversized_decoded_images() {
        assert!(decode_image(b"not a png").is_err());
        let mut header = vec![0; 24];
        header[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        header[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        header[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode_image(&header).is_err());
        assert!(decode_image(include_bytes!("../icons/32x32.png")).is_ok());
    }
}
