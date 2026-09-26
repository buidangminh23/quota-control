//! The starred metrics for the macOS Dynamic Island and desktop widget. The popup renders them
//! (`src/model/glance.ts`), the same way it renders the menu bar strip, and sends the document
//! here. The island gets every change at once. The widget reads `glance.json` next to the settings:
//! the file follows every change, but WidgetKit rations reloads, so the widget is asked to reload
//! only when a reading changed (not merely its time) and at most every [`RELOAD_INTERVAL`]. Other
//! platforms accept the document and ignore it.

use serde_json::Value;
use tauri::{AppHandle, State};

use crate::service::safe_error;

const MAX_BYTES: usize = 256 * 1024;
const VERSION: u64 = 1;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const FILE_NAME: &str = "glance.json";
/// The widget file's own folder inside the settings folder: the sandboxed widgets may read only
/// this folder, never the accounts and credentials beside it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const FOLDER_NAME: &str = "widget";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const RELOAD_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// Unchanged readings still reload the widget this often, so its "Updated" time and its stale
/// marker (shown after 20 minutes) follow a running app even when no reading changes.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const FRESHNESS_RELOAD: std::time::Duration = std::time::Duration::from_secs(15 * 60);

#[derive(Default)]
pub struct Glance {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    state: std::sync::Arc<parking_lot::Mutex<Published>>,
}

#[derive(Default)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct Published {
    /// What the widget file holds, so an unchanged document is not written again.
    file: Option<Vec<u8>>,
    /// The widget file's readings without their time, so a new time alone does not reload it.
    readings: Option<Value>,
    last_reload: Option<std::time::Instant>,
    reload_scheduled: bool,
}

/// Validate the document and serialize it compactly.
pub fn encode(document: &Value) -> Result<Vec<u8>, String> {
    if document.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err("Unsupported glance document".into());
    }
    if !document.get("providers").is_some_and(Value::is_array) {
        return Err("A glance document lists providers".into());
    }
    let bytes = serde_json::to_vec(document).map_err(safe_error)?;
    if bytes.len() > MAX_BYTES {
        return Err("The glance document is too large".into());
    }
    Ok(bytes)
}

#[tauri::command]
pub fn set_glance(
    app: AppHandle,
    glance: State<'_, Glance>,
    document: Value,
) -> Result<(), String> {
    let bytes = encode(&document)?;
    glance.publish(&app, bytes);
    Ok(())
}

impl Glance {
    #[cfg(not(target_os = "macos"))]
    fn publish(&self, _app: &AppHandle, _bytes: Vec<u8>) {}

    #[cfg(target_os = "macos")]
    fn publish(&self, _app: &AppHandle, bytes: Vec<u8>) {
        crate::macos::update_island(&bytes);
        let widget = without_alert(&bytes);
        let mut state = self.state.lock();
        if state.file.as_deref() == Some(widget.as_slice()) {
            return;
        }
        let path = widget_file();
        if let Err(error) = uc_core::paths::write_atomic(&path, &widget) {
            tracing::warn!("cannot write {}: {}", path.display(), safe_error(error));
            return;
        }
        let readings = readings(&widget);
        let changed = state.readings.as_ref() != Some(&readings)
            || state
                .last_reload
                .is_none_or(|at| at.elapsed() >= FRESHNESS_RELOAD);
        state.file = Some(widget);
        state.readings = Some(readings);
        drop(state);
        if changed {
            self.reload_widgets();
        }
    }

    /// Reload now, or once when the interval since the last reload has passed.
    #[cfg(target_os = "macos")]
    fn reload_widgets(&self) {
        let mut state = self.state.lock();
        let wait = state
            .last_reload
            .map(|last| RELOAD_INTERVAL.saturating_sub(last.elapsed()))
            .unwrap_or_default();
        if wait.is_zero() {
            state.last_reload = Some(std::time::Instant::now());
            drop(state);
            crate::macos::reload_widgets();
            return;
        }
        if state.reload_scheduled {
            return;
        }
        state.reload_scheduled = true;
        let shared = self.state.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(wait).await;
            {
                let mut state = shared.lock();
                state.reload_scheduled = false;
                state.last_reload = Some(std::time::Instant::now());
            }
            crate::macos::reload_widgets();
        });
    }
}

/// Where the widgets read the readings. A file an earlier version left beside the settings is
/// removed, since the widgets no longer look there and may not read that folder.
#[cfg(target_os = "macos")]
fn widget_file() -> std::path::PathBuf {
    let config = uc_core::paths::config_dir();
    let legacy = config.join(FILE_NAME);
    if legacy.is_file() {
        let _ = std::fs::remove_file(&legacy);
    }
    config.join(FOLDER_NAME).join(FILE_NAME)
}

/// The document without its time, to tell a changed reading from a mere refresh.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn readings(bytes: &[u8]) -> Value {
    let mut document = serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null);
    if let Value::Object(fields) = &mut document {
        fields.remove("generatedAt");
    }
    document
}

/// The widget has no use for the island's alert, and dropping it keeps an alert alone from
/// rewriting the file.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn without_alert(bytes: &[u8]) -> Vec<u8> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(mut document)) => {
            document.remove("alert");
            serde_json::to_vec(&Value::Object(document)).unwrap_or_else(|_| bytes.to_vec())
        }
        _ => bytes.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn documents_need_the_current_version_and_a_provider_list() {
        assert!(encode(&json!({"version": 1, "providers": []})).is_ok());
        assert!(encode(&json!({"version": 2, "providers": []})).is_err());
        assert!(encode(&json!({"version": 1})).is_err());
        let huge = "x".repeat(MAX_BYTES);
        assert!(encode(&json!({"version": 1, "providers": [huge]})).is_err());
    }

    #[test]
    fn a_new_time_alone_leaves_the_readings_unchanged() {
        let one = encode(&json!({"version": 1, "generatedAt": "a", "providers": [1]})).unwrap();
        let two = encode(&json!({"version": 1, "generatedAt": "b", "providers": [1]})).unwrap();
        let three = encode(&json!({"version": 1, "generatedAt": "b", "providers": [2]})).unwrap();
        assert_eq!(readings(&one), readings(&two));
        assert_ne!(readings(&two), readings(&three));
    }

    #[test]
    fn the_widget_copy_leaves_out_the_alert() {
        let bytes = encode(&json!({"version": 1, "providers": [], "alert": {"id": "a"}})).unwrap();
        let widget: Value = serde_json::from_slice(&without_alert(&bytes)).unwrap();
        assert_eq!(widget, json!({"version": 1, "providers": []}));
    }
}
