//! The global shortcut that toggles the popup from anywhere (upstream `AppShortcuts`). There is no
//! default combo: the user records one in Settings, and clearing it turns the shortcut off. The
//! combo is registered natively, so it works while the popup's page is hidden.

use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, State, Wry};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};

use crate::integrations::IntegrationStore;
use crate::service::safe_error;

pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _, event| {
            if event.state == ShortcutState::Pressed
                && let Err(error) = crate::toggle_popup(app, None)
            {
                tracing::warn!("{error}");
            }
        })
        .build()
}

/// Register the saved combo. At launch, a combo another app now holds is logged and stays off.
pub fn restore(app: &AppHandle) -> Result<(), String> {
    let manager = app.global_shortcut();
    manager.unregister_all().map_err(safe_error)?;
    match app.state::<IntegrationStore>().get().global_shortcut {
        Some(saved) => manager.register(saved.as_str()).map_err(safe_error),
        None => Ok(()),
    }
}

/// A combo is usable when it parses and, unless it is a function key, holds a modifier, so a plain
/// letter can never be taken from every other app.
fn validate(shortcut: &str) -> Result<String, String> {
    let shortcut = shortcut.trim();
    let parsed: Shortcut = shortcut
        .parse()
        .map_err(|_| "This key combination is not supported.".to_string())?;
    let function_key = matches!(
        parsed.key,
        Code::F1
            | Code::F2
            | Code::F3
            | Code::F4
            | Code::F5
            | Code::F6
            | Code::F7
            | Code::F8
            | Code::F9
            | Code::F10
            | Code::F11
            | Code::F12
            | Code::F13
            | Code::F14
            | Code::F15
            | Code::F16
            | Code::F17
            | Code::F18
            | Code::F19
            | Code::F20
            | Code::F21
            | Code::F22
            | Code::F23
            | Code::F24
    );
    if parsed.mods.is_empty() && !function_key {
        return Err("Add Ctrl, Alt, Shift or the Windows key to the shortcut.".into());
    }
    Ok(shortcut.to_string())
}

#[tauri::command]
pub fn global_shortcut(store: State<'_, IntegrationStore>) -> Option<String> {
    store.get().global_shortcut
}

/// Replace the combo; `None` clears it. When the new combo cannot be registered (another app holds
/// it), the previous one stays in effect and is kept.
#[tauri::command]
pub async fn set_global_shortcut(
    app: AppHandle,
    shortcut: Option<String>,
) -> Result<Option<String>, String> {
    let next = shortcut.as_deref().map(validate).transpose()?;
    let manager = app.global_shortcut();
    manager.unregister_all().map_err(safe_error)?;
    if let Some(next) = &next
        && manager.register(next.as_str()).is_err()
    {
        let _ = restore(&app);
        return Err("This shortcut is unavailable. Another app may already use it.".into());
    }
    let saved = app
        .state::<IntegrationStore>()
        .update(|integrations| integrations.global_shortcut = next.clone());
    if let Err(error) = saved {
        let _ = restore(&app);
        return Err(error);
    }
    Ok(next)
}

/// While Settings records a combo, the current one is released so pressing it records it instead
/// of toggling the popup.
#[tauri::command]
pub async fn pause_global_shortcut(app: AppHandle, paused: bool) -> Result<(), String> {
    if paused {
        app.global_shortcut().unregister_all().map_err(safe_error)
    } else {
        restore(&app)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_need_a_modifier_unless_they_use_a_function_key() {
        assert_eq!(validate(" Ctrl+Alt+KeyU ").unwrap(), "Ctrl+Alt+KeyU");
        assert!(validate("Super+Shift+Digit5").is_ok());
        assert!(validate("F9").is_ok());
        assert!(validate("KeyU").is_err());
        assert!(validate("Ctrl+").is_err());
        assert!(validate("Ctrl+NotAKey").is_err());
    }
}
