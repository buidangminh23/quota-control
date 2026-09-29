//! Notifications and launch at login where the operating system has a way of its own that serves
//! the user better than the Tauri plugins' way. macOS has: notifications through the system's
//! notification center (a click opens the popup, a newer alert replaces the older one about the
//! same limit, Notification Center groups them by account, and the user's refusal is known), and
//! launch at login as a login item of the app, listed by name in System Settings. Every command
//! here answers "not here" on Windows and Linux, and on macOS whenever the system's way is out of
//! reach, and the popup then uses the plugins as before.

use serde::Serialize;
use tauri::AppHandle;

/// Whether the user lets the app show notifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum NotificationAccess {
    Granted,
    Denied,
    /// The user was never asked.
    Undetermined,
}

/// The most the system is handed of a notification; what is longer is cut, never refused.
const MAX_TITLE_CHARS: usize = 256;
const MAX_BODY_CHARS: usize = 2048;
const MAX_NAME_CHARS: usize = 256;

fn clipped(text: &str, limit: usize) -> &str {
    text.char_indices()
        .nth(limit)
        .map_or(text, |(end, _)| &text[..end])
}

/// The system's own answer about notification access, or `None` where the plugin answers.
#[tauri::command]
pub async fn system_notification_access() -> Option<NotificationAccess> {
    platform::notification_access().await
}

/// Ask the user for notifications through the system, or `None` where the plugin asks.
#[tauri::command]
pub async fn request_system_notification_access() -> Option<NotificationAccess> {
    platform::request_notification_access().await
}

/// Show a notification through the system. `id` names what it is about, so the next one about
/// the same thing replaces it; `group` keeps an account's notifications together. `false` means
/// it was not shown here and the plugin should show it.
#[tauri::command]
pub async fn send_system_notification(
    app: AppHandle,
    title: String,
    body: String,
    id: Option<String>,
    group: Option<String>,
) -> bool {
    platform::notify(
        &app,
        clipped(&title, MAX_TITLE_CHARS),
        clipped(&body, MAX_BODY_CHARS),
        clipped(id.as_deref().unwrap_or_default(), MAX_NAME_CHARS),
        clipped(group.as_deref().unwrap_or_default(), MAX_NAME_CHARS),
    )
    .await
}

/// A notification from the core itself, in the system's way where there is one and through the
/// plugin otherwise.
pub fn announce(app: &AppHandle, title: String, body: String, id: &'static str) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if platform::notify(&app, &title, &body, id, id).await {
            return;
        }
        use tauri_plugin_notification::NotificationExt;
        if let Err(error) = app.notification().builder().title(title).body(body).show() {
            tracing::warn!(
                "could not show a notification: {}",
                crate::service::safe_error(error)
            );
        }
    });
}

/// Whether the app launches at login by the system's own login item, or `None` where the plugin
/// keeps that.
#[tauri::command]
pub fn system_launch_at_login(app: AppHandle) -> Option<bool> {
    platform::launch_at_login(&app)
}

/// Switch launch at login through the system's own login item and tell where it stands after, or
/// `None` where the plugin should switch it.
#[tauri::command]
pub fn set_system_launch_at_login(app: AppHandle, enabled: bool) -> Option<bool> {
    platform::set_launch_at_login(&app, enabled)
}

/// At launch: take over the notifications where the system has its own way, and say in the log
/// which way notifications and launch at login go.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn start(app: &AppHandle) {
    platform::start(app);
}

/// At launch: keep launch at login pointing at this copy of the app. macOS moves an older launch
/// agent to the app's login item; elsewhere, and when that fails, the plugin's entry is written
/// again for the path the app runs from now.
pub fn keep_launch_at_login(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    use tauri_plugin_autostart::ManagerExt as _;
    if !app.autolaunch().is_enabled().unwrap_or(false) || platform::adopt_launch_at_login(app) {
        return;
    }
    if let Err(error) = app.autolaunch().enable() {
        tracing::warn!("Could not refresh launch-at-login path: {error}");
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use tauri::AppHandle;
    use tauri_plugin_autostart::ManagerExt as _;

    use std::sync::atomic::{AtomicBool, Ordering};

    use super::NotificationAccess;
    use crate::macos::{self, LoginItem};

    /// The system's question is up, asked by a notification: the next ones do not ask again.
    static ASKING: AtomicBool = AtomicBool::new(false);

    fn access(native: macos::NotificationAccess) -> Option<NotificationAccess> {
        match native {
            macos::NotificationAccess::Unavailable => None,
            macos::NotificationAccess::Undetermined => Some(NotificationAccess::Undetermined),
            macos::NotificationAccess::Denied => Some(NotificationAccess::Denied),
            macos::NotificationAccess::Granted => Some(NotificationAccess::Granted),
        }
    }

    pub async fn notification_access() -> Option<NotificationAccess> {
        access(macos::notification_access().await)
    }

    pub async fn request_notification_access() -> Option<NotificationAccess> {
        access(macos::request_notification_access(true).await)
    }

    /// Allowed: shown. Refused: the user's word stands, nothing is shown and the plugin is not
    /// asked to either. Never asked: the question goes up without holding this notification back,
    /// which the plugin shows meanwhile.
    pub async fn notify(app: &AppHandle, title: &str, body: &str, id: &str, group: &str) -> bool {
        match macos::notification_access().await {
            macos::NotificationAccess::Granted => {
                macos::notify(title, body, id, group, crate::english(app));
                true
            }
            macos::NotificationAccess::Denied => true,
            macos::NotificationAccess::Undetermined => {
                if !ASKING.swap(true, Ordering::Relaxed) {
                    tauri::async_runtime::spawn(async {
                        macos::request_notification_access(false).await;
                        ASKING.store(false, Ordering::Relaxed);
                    });
                }
                false
            }
            macos::NotificationAccess::Unavailable => false,
        }
    }

    pub fn start(app: &AppHandle) {
        if !macos::start_notifications(app) {
            tracing::info!(
                "notifications and launch at login go through the plugins: no app bundle"
            );
            return;
        }
        let login_item = macos::login_item();
        tauri::async_runtime::spawn(async move {
            let access = macos::notification_access().await;
            tracing::info!(
                "notifications go through the notification center ({access:?}), launch at login through the login item ({login_item:?})"
            );
        });
    }

    fn agent_enabled(app: &AppHandle) -> bool {
        app.autolaunch().is_enabled().unwrap_or(false)
    }

    /// The launch agent of earlier versions would start the app a second time beside the login
    /// item, so it goes once the login item is there.
    fn drop_agent(app: &AppHandle) {
        if agent_enabled(app)
            && let Err(error) = app.autolaunch().disable()
        {
            tracing::warn!("Could not remove the earlier launch agent: {error}");
        }
    }

    /// When the system refuses the login item, the launch agent of earlier versions does the
    /// work, as it did.
    fn enable_agent(app: &AppHandle) -> bool {
        match app.autolaunch().enable() {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!("Could not write the launch agent: {error}");
                false
            }
        }
    }

    pub fn launch_at_login(app: &AppHandle) -> Option<bool> {
        match macos::login_item() {
            LoginItem::Unavailable => None,
            LoginItem::On => Some(true),
            LoginItem::Off | LoginItem::NeedsApproval => Some(agent_enabled(app)),
        }
    }

    /// Switching on: the login item, or the launch agent when the system refuses the item; an
    /// item that waits for the user's approval in System Settings is left to the user. Switching
    /// off removes both, and tells when the login item stayed.
    pub fn set_launch_at_login(app: &AppHandle, enabled: bool) -> Option<bool> {
        let item = macos::set_login_item(enabled, true);
        if item == LoginItem::Unavailable {
            return None;
        }
        if !enabled {
            drop_agent(app);
            return Some(item == LoginItem::On || agent_enabled(app));
        }
        Some(match item {
            LoginItem::On => {
                drop_agent(app);
                true
            }
            LoginItem::NeedsApproval => agent_enabled(app),
            LoginItem::Off | LoginItem::Unavailable => enable_agent(app),
        })
    }

    pub fn adopt_launch_at_login(app: &AppHandle) -> bool {
        if macos::set_login_item(true, false) != LoginItem::On {
            return false;
        }
        drop_agent(app);
        true
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use tauri::AppHandle;

    use super::NotificationAccess;

    pub async fn notification_access() -> Option<NotificationAccess> {
        None
    }

    pub async fn request_notification_access() -> Option<NotificationAccess> {
        None
    }

    pub async fn notify(
        _app: &AppHandle,
        _title: &str,
        _body: &str,
        _id: &str,
        _group: &str,
    ) -> bool {
        false
    }

    pub fn launch_at_login(_app: &AppHandle) -> Option<bool> {
        None
    }

    pub fn set_launch_at_login(_app: &AppHandle, _enabled: bool) -> Option<bool> {
        None
    }

    pub fn adopt_launch_at_login(_app: &AppHandle) -> bool {
        false
    }

    #[allow(dead_code)]
    pub fn start(_app: &AppHandle) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_a_long_notification_at_a_character_and_leaves_a_short_one() {
        assert_eq!(clipped("Claude", MAX_TITLE_CHARS), "Claude");
        assert_eq!(clipped("", 4), "");
        assert_eq!(clipped("hạn mức", 3), "hạn");
        assert_eq!(clipped("hạn mức", 7), "hạn mức");
        let long = "ợ".repeat(MAX_BODY_CHARS + 9);
        assert_eq!(
            clipped(&long, MAX_BODY_CHARS).chars().count(),
            MAX_BODY_CHARS
        );
    }

    #[test]
    fn names_access_the_way_the_popup_reads_it() {
        let names = [
            NotificationAccess::Granted,
            NotificationAccess::Denied,
            NotificationAccess::Undetermined,
        ]
        .map(|access| serde_json::to_string(&access).unwrap());
        assert_eq!(names, ["\"granted\"", "\"denied\"", "\"undetermined\""]);
    }
}
