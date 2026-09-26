use std::path::PathBuf;

use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use url::Url;

use crate::chat_store::{ChatSession, ChatStore};
use crate::service::safe_error;

#[derive(Default)]
pub struct ChatWindows {
    operations: tokio::sync::Mutex<()>,
}

fn official_url(provider: &str) -> Result<Url, String> {
    match provider {
        "claude" => Ok(Url::parse("https://claude.ai/new").unwrap()),
        "codex" => Ok(Url::parse("https://chatgpt.com/").unwrap()),
        _ => Err("Unsupported chat provider".into()),
    }
}

fn allowed_navigation(url: &Url) -> bool {
    if url.as_str() == "about:blank" {
        return true;
    }
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.host(), Some(url::Host::Domain(host))
            if host != "localhost" && !host.ends_with(".localhost") && host.contains('.'))
}

fn build_chat_window(
    app: &AppHandle,
    session: &ChatSession,
    profile: PathBuf,
    label: &str,
    initial_url: Url,
    features: Option<NewWindowFeatures>,
) -> Result<WebviewWindow, String> {
    let provider_name = if session.provider == "codex" {
        "ChatGPT"
    } else {
        "Claude"
    };
    let title = format!("{provider_name} · {} · Quota Control", session.label);
    let app_handle = app.clone();
    let child_session = session.clone();
    let child_profile = profile.clone();
    let page_title = title.clone();
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::External(initial_url))
        .title(&title)
        .inner_size(1120.0, 800.0)
        .min_inner_size(640.0, 480.0)
        .data_directory(profile)
        .on_navigation(allowed_navigation)
        .on_page_load(move |window, payload| {
            if let Some(host) = payload.url().host_str() {
                let _ = window.set_title(&format!("{page_title} · {host}"));
            }
        })
        .on_new_window(move |url, features| {
            if !allowed_navigation(&url) {
                return NewWindowResponse::Deny;
            }
            let label = format!("chat-child-{}", uuid::Uuid::new_v4());
            match build_chat_window(
                &app_handle,
                &child_session,
                child_profile.clone(),
                &label,
                Url::parse("about:blank").unwrap(),
                Some(features),
            ) {
                Ok(window) => NewWindowResponse::Create { window },
                Err(_) => {
                    tracing::warn!("Could not open a chat authentication window");
                    NewWindowResponse::Deny
                }
            }
        });
    if let Some(features) = features {
        builder = builder.window_features(features);
    }
    builder.build().map_err(safe_error)
}

pub async fn open_session(app: &AppHandle, store: &ChatStore, id: &str) -> Result<(), String> {
    let windows = app.state::<ChatWindows>();
    let _operation = windows.operations.lock().await;
    open_session_inner(app, store, id)
}

fn open_session_inner(app: &AppHandle, store: &ChatStore, id: &str) -> Result<(), String> {
    let session = store.get(id)?;
    let label = format!("chat-{}", session.id);
    if let Some(window) = app.get_webview_window(&label) {
        window.unminimize().map_err(safe_error)?;
        window.show().map_err(safe_error)?;
        return window.set_focus().map_err(safe_error);
    }
    build_chat_window(
        app,
        &session,
        store.profile_directory(id)?,
        &label,
        official_url(&session.provider)?,
        None,
    )?;
    Ok(())
}

pub async fn create_session(
    app: &AppHandle,
    store: &ChatStore,
    provider: &str,
    label: Option<String>,
) -> Result<ChatSession, String> {
    let windows = app.state::<ChatWindows>();
    let _operation = windows.operations.lock().await;
    let session = store.create(provider, label)?;
    if crate::update_tray_menu(app).is_err() {
        tracing::warn!("Could not update the saved chat session menu");
    }
    if app
        .emit_to("popup", "chat-sessions-changed", store.list()?)
        .is_err()
    {
        tracing::warn!("Could not publish saved chat sessions");
    }
    if let Err(error) = open_session_inner(app, store, &session.id) {
        return Err(format!(
            "Chat session was saved, but its window could not open: {error}"
        ));
    }
    Ok(session)
}

#[tauri::command]
pub async fn list_chat_sessions(store: State<'_, ChatStore>) -> Result<Vec<ChatSession>, String> {
    store.list()
}

#[tauri::command]
pub async fn create_chat_session(
    app: AppHandle,
    store: State<'_, ChatStore>,
    provider: String,
    label: Option<String>,
) -> Result<ChatSession, String> {
    create_session(&app, &store, &provider, label).await
}

#[tauri::command]
pub async fn open_chat_session(
    app: AppHandle,
    store: State<'_, ChatStore>,
    session_id: String,
) -> Result<(), String> {
    open_session(&app, &store, &session_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_blocks_native_origins_and_unsafe_schemes() {
        for url in [
            "tauri://localhost/index.html",
            "http://localhost:1420",
            "https://tauri.localhost/",
            "https://ipc.localhost/",
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://user:pass@claude.ai/",
        ] {
            assert!(!allowed_navigation(&Url::parse(url).unwrap()), "{url}");
        }
        for url in [
            "https://claude.ai/new",
            "https://chatgpt.com/",
            "https://accounts.google.com/",
            "about:blank",
        ] {
            assert!(allowed_navigation(&Url::parse(url).unwrap()), "{url}");
        }
    }

    #[test]
    fn start_urls_are_fixed_official_sites() {
        assert_eq!(
            official_url("codex").unwrap().host_str(),
            Some("chatgpt.com")
        );
        assert_eq!(
            official_url("claude").unwrap().host_str(),
            Some("claude.ai")
        );
        assert!(official_url("https://evil.example").is_err());
    }
}
