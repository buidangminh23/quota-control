//! The starred metrics for the macOS Dynamic Island and desktop widget. The popup renders them
//! (`src/model/glance.ts`), the same way it renders the menu bar strip, and sends the document
//! here. The island gets every change at once. The widget reads `glance.json` next to the settings:
//! the file follows every change, but WidgetKit rations reloads, so the widget is asked to reload
//! only when a reading changed (not merely its time) and at most every [`RELOAD_INTERVAL`]. Other
//! platforms accept the document and ignore it.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

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
/// The folder inside the widget folder where the sandboxed widgets leave what their buttons ask
/// for (`WidgetActions.swift`): the only place under the settings they may write.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const REQUESTS_FOLDER: &str = "requests";
/// A request older than this is dropped unread: a reset is spent right after the user confirmed
/// it, never minutes later because the app happened to start then.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const REQUEST_LIFETIME_SECONDS: i64 = 120;
/// A request's own clock may run a little ahead of the app's.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const REQUEST_CLOCK_SKEW_SECONDS: i64 = 5;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MAX_REQUEST_BYTES: u64 = 4096;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const REQUEST_POLL: std::time::Duration = std::time::Duration::from_secs(1);
/// What the island and the widgets may ask of the popup; `src/glance/glanceActions.ts` checks the
/// rest of each request before doing anything.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const ACTION_KINDS: &[&str] = &["redeemLimitReset", "markBankedReset", "openResets"];

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

/// A button's request from the island or a widget, as the popup expects it, or `None` for anything
/// else.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn action(value: Value) -> Option<Value> {
    let kind = value.get("kind")?.as_str()?;
    (value.is_object() && ACTION_KINDS.contains(&kind)).then_some(value)
}

/// Hand a request to the popup. Opening the Reset tab also shows the popup, as the popup's own rows
/// do when pressed.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn relay_action(app: &AppHandle, action: Value) {
    if action.get("kind").and_then(Value::as_str) == Some("openResets")
        && let Err(error) = crate::show_popup(app)
    {
        tracing::warn!("{error}");
    }
    if let Err(error) = app.emit_to("popup", "glance-action", action) {
        tracing::warn!("cannot pass a glance request to the popup: {error}");
    }
}

/// Take the widgets' requests out of `folder`: each file is read once and removed, and only a fresh
/// request of a known kind comes back. Files a widget is still writing (their names start with a
/// dot) are left for the next pass, unless the widget stopped before finishing one long ago.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn take_requests(folder: &Path, now: DateTime<Utc>) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut requests = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".json") {
            continue;
        }
        if name.starts_with('.') {
            let abandoned = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .map(|modified| {
                    now.signed_duration_since(DateTime::<Utc>::from(modified))
                        .num_seconds()
                        > REQUEST_LIFETIME_SECONDS
                })
                .unwrap_or(false);
            if abandoned {
                let _ = std::fs::remove_file(entry.path());
            }
            continue;
        }
        let path = entry.path();
        let small = entry
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAX_REQUEST_BYTES);
        let bytes = small.then(|| std::fs::read(&path).ok()).flatten();
        let _ = std::fs::remove_file(&path);
        let Some(request) = bytes.and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        else {
            continue;
        };
        let fresh = request
            .get("requestedAt")
            .and_then(Value::as_str)
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .map(|at| now.signed_duration_since(at.with_timezone(&Utc)).num_seconds())
            .is_some_and(|age| (-REQUEST_CLOCK_SKEW_SECONDS..=REQUEST_LIFETIME_SECONDS).contains(&age));
        if !fresh {
            tracing::info!("dropped a widget request that was not fresh");
            continue;
        }
        if let Some(action) = request.get("action").cloned().and_then(action) {
            requests.push(action);
        }
    }
    requests
}

/// Make the requests folder and pass the widgets' requests to the popup for as long as the app
/// runs.
#[cfg(target_os = "macos")]
pub fn start_widget_requests(app: &AppHandle) {
    let folder = uc_core::paths::config_dir()
        .join(FOLDER_NAME)
        .join(REQUESTS_FOLDER);
    if let Err(error) = std::fs::create_dir_all(&folder) {
        tracing::warn!("cannot make the widget requests folder: {}", safe_error(error));
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            for action in take_requests(&folder, Utc::now()) {
                relay_action(&app, action);
            }
            tokio::time::sleep(REQUEST_POLL).await;
        }
    });
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
    fn only_the_known_requests_are_passed_on() {
        let redeem = json!({"kind": "redeemLimitReset", "providerId": "codex@1"});
        assert_eq!(action(redeem.clone()), Some(redeem));
        assert!(action(json!({"kind": "openResets", "provider": "claude"})).is_some());
        assert!(action(json!({"kind": "markBankedReset", "resetId": "7", "used": true})).is_some());
        assert!(action(json!({"kind": "wipe"})).is_none());
        assert!(action(json!({"providerId": "codex@1"})).is_none());
        assert!(action(json!("redeemLimitReset")).is_none());
    }

    #[test]
    fn widget_requests_are_read_once_and_only_while_fresh() {
        let folder = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-09-30T05:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let write = |name: &str, value: Value| {
            std::fs::write(folder.path().join(name), serde_json::to_vec(&value).unwrap()).unwrap();
        };
        let redeem = json!({"kind": "redeemLimitReset", "providerId": "codex@1"});
        write("a.json", json!({"requestedAt": "2026-09-30T04:59:30Z", "action": redeem}));
        write("b.json", json!({"requestedAt": "2026-09-30T04:57:00Z", "action": redeem}));
        write("c.json", json!({"requestedAt": "2026-09-30T05:00:30Z", "action": redeem}));
        write("d.json", json!({"requestedAt": "2026-09-30T04:59:59Z", "action": {"kind": "wipe"}}));
        write(".e.json", json!({"requestedAt": "2026-09-30T04:59:59Z", "action": redeem}));
        write(".old.json", json!({"requestedAt": "2026-09-30T04:40:00Z", "action": redeem}));
        std::fs::File::options()
            .write(true)
            .open(folder.path().join(".old.json"))
            .unwrap()
            .set_modified((now - chrono::Duration::minutes(20)).into())
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(folder.path().join(".e.json"))
            .unwrap()
            .set_modified((now - chrono::Duration::seconds(3)).into())
            .unwrap();
        write("f.txt", json!({"requestedAt": "2026-09-30T04:59:59Z", "action": redeem}));
        std::fs::write(folder.path().join("g.json"), b"not json").unwrap();
        std::fs::write(folder.path().join("h.json"), vec![b' '; 5000]).unwrap();

        assert_eq!(take_requests(folder.path(), now), vec![redeem]);
        let mut left: Vec<String> = std::fs::read_dir(folder.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec![".e.json", "f.txt"]);
        assert!(take_requests(folder.path(), now).is_empty());
        assert!(take_requests(&folder.path().join("missing"), now).is_empty());
    }

    #[test]
    fn the_widget_copy_leaves_out_the_alert() {
        let bytes = encode(&json!({"version": 1, "providers": [], "alert": {"id": "a"}})).unwrap();
        let widget: Value = serde_json::from_slice(&without_alert(&bytes)).unwrap();
        assert_eq!(widget, json!({"version": 1, "providers": []}));
    }
}
