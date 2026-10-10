use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use url::Url;

use crate::chat_store::{ChatSession, ChatStore};
use crate::service::safe_error;

#[derive(Default)]
pub struct ChatWindows {
    operations: tokio::sync::Mutex<()>,
    loaded: parking_lot::Mutex<HashSet<String>>,
    billing_requested: tokio::sync::Notify,
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
    visible: bool,
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
    let page_app = app.clone();
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::External(initial_url))
        .title(&title)
        .inner_size(1120.0, 800.0)
        .min_inner_size(640.0, 480.0)
        .visible(visible)
        .focused(visible)
        .data_directory(profile)
        .on_navigation(allowed_navigation)
        .on_page_load(move |window, payload| {
            let windows = page_app.state::<ChatWindows>();
            match payload.event() {
                tauri::webview::PageLoadEvent::Started => {
                    windows.loaded.lock().remove(window.label());
                }
                tauri::webview::PageLoadEvent::Finished => {
                    windows.loaded.lock().insert(window.label().to_string());
                    if window.label().starts_with("chat-")
                        && ["claude", "codex"]
                            .iter()
                            .any(|provider| billing_origin(provider, payload.url()))
                    {
                        windows.billing_requested.notify_one();
                    }
                }
            }
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
                true,
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
    #[cfg(target_os = "macos")]
    {
        builder = builder.data_store_identifier(data_store_identifier(&session.id));
    }
    builder.build().map_err(safe_error)
}

/// WKWebView ignores data directories; on macOS each session keeps its cookies and storage in a
/// data store named after the session instead.
#[cfg(target_os = "macos")]
fn data_store_identifier(session_id: &str) -> [u8; 16] {
    uuid::Uuid::parse_str(session_id)
        .map(|id| *id.as_bytes())
        .unwrap_or_else(|_| {
            let mut bytes = [0_u8; 16];
            for (index, byte) in session_id.bytes().enumerate() {
                bytes[index % 16] = bytes[index % 16].rotate_left(3) ^ byte;
            }
            bytes
        })
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
        true,
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

#[tauri::command]
pub async fn open_account_billing(
    app: AppHandle,
    store: State<'_, ChatStore>,
    accounts: State<'_, crate::account_commands::Accounts>,
    account_id: String,
) -> Result<bool, String> {
    let label = accounts.billing_label(&account_id)?;
    let provider = account_id
        .split_once('@')
        .map(|(provider, _)| provider)
        .filter(|provider| matches!(*provider, "claude" | "codex"))
        .ok_or("Billing requires a supported connected account")?;
    let periods = uc_providers::BillingPeriods::default_store();
    let chrome = read_chrome_billing(provider).await;
    if let Err(error) = &chrome {
        tracing::debug!(%error, "The billing connection could not use Chrome");
    }
    if let Ok(reads) = chrome {
        let observed_at = Utc::now();
        let matching_account = if provider == "codex" {
            billing_read_confirms(
                &reads,
                provider,
                &accounts.billing_account_uuid(&account_id).await?,
                observed_at,
            )
        } else {
            true
        };
        let changed = store_chrome_billing(&periods, provider, reads, observed_at)?;
        let windows = app.state::<ChatWindows>();
        windows.billing_requested.notify_one();
        let engine = app.state::<crate::service::BackendService>().engine();
        if changed {
            engine.invalidate_plan_term(&account_id);
        }
        refresh_billing_account(&engine, &account_id).await;
        if matching_account && billing_connected(engine.snapshots().get(&account_id), observed_at) {
            return Ok(true);
        }
        return Err("The browser did not confirm a current paid period for this account. Sign in to the matching account and try again.".into());
    }
    let windows = app.state::<ChatWindows>();
    let _operation = windows.operations.lock().await;
    let session =
        match store.list()?.into_iter().find(|session| {
            session.provider == provider && session.label.eq_ignore_ascii_case(&label)
        }) {
            Some(session) => session,
            None => {
                let session = store.create(provider, Some(label))?;
                let _ = app.emit_to("popup", "chat-sessions-changed", store.list()?);
                session
            }
        };
    let url = if provider == "claude" {
        Url::parse("https://claude.ai/settings/billing").unwrap()
    } else {
        official_url(provider)?
    };
    let window_label = format!("chat-{}", session.id);
    if let Some(window) = app.get_webview_window(&window_label) {
        window.navigate(url).map_err(safe_error)?;
        window.unminimize().map_err(safe_error)?;
        window.show().map_err(safe_error)?;
        window.set_focus().map_err(safe_error)?;
    } else {
        build_chat_window(
            &app,
            &session,
            store.profile_directory(&session.id)?,
            &window_label,
            url,
            None,
            true,
        )?;
    }
    windows.billing_requested.notify_one();
    Ok(false)
}

fn billing_read_confirms(
    reads: &[BillingRead],
    provider: &str,
    account: &str,
    now: chrono::DateTime<Utc>,
) -> bool {
    reads.len() == 1
        && reads[0].state == "ready"
        && reads[0].organizations.iter().any(|org| {
            org.uuid.eq_ignore_ascii_case(account)
                && org.status == 200
                && uc_providers::BillingPeriods::plan_for(
                    provider,
                    &org.plan_type,
                    org.tier.as_deref(),
                )
                .is_some()
                && org
                    .details
                    .as_ref()
                    .and_then(|details| billing_end(provider, details))
                    .is_some_and(|end| end > now)
        })
}

fn billing_end(provider: &str, details: &Value) -> Option<chrono::DateTime<Utc>> {
    if !details["status"]
        .as_str()
        .is_some_and(|status| status.trim().eq_ignore_ascii_case("active"))
    {
        return None;
    }
    let field = if provider == "codex" {
        "expires_at"
    } else if details["plan_ending_at"].is_null() {
        "next_charge_at"
    } else {
        "plan_ending_at"
    };
    details[field]
        .as_str()
        .and_then(|end| chrono::DateTime::parse_from_rfc3339(end).ok())
        .map(|end| end.with_timezone(&Utc))
}

fn billing_connected(
    snapshot: Option<&uc_core::ProviderSnapshot>,
    observed_at: chrono::DateTime<Utc>,
) -> bool {
    snapshot.is_some_and(|snapshot| {
        snapshot.plan_checked_at.is_some()
            && matches!(snapshot.plan_term,
            Some(uc_core::PlanTerm::Stated { ends_at, checked_at: Some(checked_at) })
            if ends_at > observed_at && checked_at >= observed_at)
    })
}

async fn refresh_billing_account(engine: &uc_engine::Engine, account_id: &str) {
    let _ = tokio::time::timeout(Duration::from_secs(35), async {
        loop {
            let outcome = engine.refresh(account_id, true).await;
            if outcome != uc_engine::RefreshOutcome::Skipped
                || engine
                    .state()
                    .providers
                    .get(account_id)
                    .is_none_or(|provider| !provider.refreshing)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BillingRead {
    state: String,
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    organizations: Vec<BillingOrganization>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BillingOrganization {
    uuid: String,
    plan_type: String,
    tier: Option<String>,
    status: u16,
    details: Option<Value>,
}

fn billing_origin(provider: &str, url: &Url) -> bool {
    url.scheme() == "https"
        && matches!(
            (provider, url.host_str()),
            ("claude", Some("claude.ai")) | ("codex", Some("chatgpt.com"))
        )
        && url.username().is_empty()
        && url.password().is_none()
}

fn billing_script(provider: &str, key: &str) -> String {
    let key = serde_json::to_string(key).unwrap();
    if provider == "codex" {
        return format!(
            r#"(() => {{
const key = {key};
window[key] = {{state: 'pending'}};
(async () => {{
try {{
const session = await fetch('/api/auth/session', {{credentials: 'same-origin'}});
if (!session.ok) {{ window[key] = {{state: 'failed', status: session.status}}; return; }}
const authentication = await session.json();
if (typeof authentication.accessToken !== 'string' || !authentication.accessToken) {{ window[key] = {{state: 'failed', status: 401}}; return; }}
const response = await fetch('/backend-api/accounts/check/v4-2023-04-27', {{credentials: 'same-origin', headers: {{Authorization: 'Bearer ' + authentication.accessToken}}}});
if (!response.ok) {{ window[key] = {{state: 'failed', status: response.status}}; return; }}
const data = await response.json();
if (!data.accounts || typeof data.accounts !== 'object' || Array.isArray(data.accounts) || Object.keys(data.accounts).length > 33) {{ window[key] = {{state: 'failed'}}; return; }}
const rows = [];
for (const [uuid, value] of Object.entries(data.accounts)) {{
if (uuid === 'default') continue;
if (!/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/i.test(uuid) || uuid === '00000000-0000-0000-0000-000000000000') {{ window[key] = {{state: 'failed'}}; return; }}
if (typeof value?.account?.plan_type !== 'string' || typeof value?.entitlement?.has_active_subscription !== 'boolean') {{ window[key] = {{state: 'failed'}}; return; }}
rows.push({{uuid, planType: value.account.plan_type, tier: null, status: 200, details: {{status: value.entitlement.has_active_subscription ? 'active' : 'inactive', expires_at: value.entitlement.expires_at ?? null}}}});
}}
window[key] = {{state: 'ready', organizations: rows}};
}} catch {{ window[key] = {{state: 'failed'}}; }}
}})();
}})()"#
        );
    }
    format!(
        r#"(() => {{
const key = {key};
window[key] = {{state: 'pending'}};
(async () => {{
try {{
const response = await fetch('/api/organizations', {{credentials: 'same-origin'}});
if (!response.ok) {{ window[key] = {{state: 'failed', status: response.status}}; return; }}
const organizations = await response.json();
if (!Array.isArray(organizations) || organizations.length > 32) {{ window[key] = {{state: 'failed'}}; return; }}
const rows = [];
for (const org of organizations) {{
const capabilities = Array.isArray(org.capabilities) ? org.capabilities : [];
if (!capabilities.includes('chat')) continue;
if (typeof org.uuid !== 'string' || !/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/i.test(org.uuid)) continue;
const planType = capabilities.find(value => ['claude_max', 'claude_pro', 'claude_team'].includes(value)) || 'claude_free';
const tier = typeof org.rate_limit_tier === 'string' ? org.rate_limit_tier : null;
if (planType === 'claude_free') {{ rows.push({{uuid: org.uuid, planType, tier, status: 200, details: {{status: 'inactive'}}}}); continue; }}
const billing = await fetch('/api/organizations/' + encodeURIComponent(org.uuid) + '/subscription_details', {{credentials: 'same-origin'}});
let details = null;
if (billing.ok) {{
const data = await billing.json();
details = {{status: data.status ?? null, next_charge_at: data.next_charge_at ?? null, plan_ending_at: data.plan_ending_at ?? null}};
}}
rows.push({{uuid: org.uuid, planType, tier, status: billing.status, details}});
}}
window[key] = {{state: 'ready', organizations: rows}};
}} catch {{ window[key] = {{state: 'failed'}}; }}
}})();
}})()"#
    )
}

async fn evaluate_billing(
    window: &WebviewWindow,
    provider: &str,
    key: &str,
) -> Result<Option<BillingRead>, String> {
    if !billing_origin(provider, &window.url().map_err(safe_error)?) {
        return Err("Billing requires the official provider web page".into());
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = std::sync::Mutex::new(Some(sender));
    window
        .eval_with_callback(
            format!("window[{}] ?? null", serde_json::to_string(key).unwrap()),
            move |result| {
                if let Ok(mut sender) = sender.lock()
                    && let Some(sender) = sender.take()
                {
                    let _ = sender.send(result);
                }
            },
        )
        .map_err(safe_error)?;
    let result = tokio::time::timeout(Duration::from_secs(3), receiver)
        .await
        .map_err(|_| "Billing evaluation timed out")?
        .map_err(|_| "Billing evaluation was interrupted")?;
    if result.len() > 64 * 1024 {
        return Err("Billing response is too large".into());
    }
    serde_json::from_str(&result).map_err(|_| "Billing response is invalid".into())
}

async fn read_session_billing(app: &AppHandle, session: &ChatSession) -> Result<bool, String> {
    let (window, temporary) = {
        let windows = app.state::<ChatWindows>();
        let _operation = windows.operations.lock().await;
        if let Some(window) = app.get_webview_window(&format!("chat-{}", session.id)) {
            (window, false)
        } else {
            let label = format!("billing-{}", session.id);
            windows.loaded.lock().remove(&label);
            let window = build_chat_window(
                app,
                session,
                app.state::<ChatStore>().profile_directory(&session.id)?,
                &label,
                official_url(&session.provider)?
                    .join("/robots.txt")
                    .unwrap(),
                None,
                false,
            )?;
            (window, true)
        }
    };
    let result = collect_session_billing(app, session, &window).await;
    if temporary {
        let _ = window.close();
        app.state::<ChatWindows>()
            .loaded
            .lock()
            .remove(window.label());
    }
    result
}

async fn collect_session_billing(
    app: &AppHandle,
    session: &ChatSession,
    window: &WebviewWindow,
) -> Result<bool, String> {
    let key = format!("__ucBilling{}", uuid::Uuid::new_v4().simple());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    while !billing_origin(&session.provider, &window.url().map_err(safe_error)?)
        || !app
            .state::<ChatWindows>()
            .loaded
            .lock()
            .contains(window.label())
    {
        if tokio::time::Instant::now() >= deadline {
            return Err("The subscription web session is unavailable".into());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    window
        .eval(billing_script(&session.provider, &key))
        .map_err(safe_error)?;
    let read = loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("The subscription billing request timed out".into());
        }
        if let Some(read) = evaluate_billing(window, &session.provider, &key).await?
            && read.state != "pending"
        {
            break read;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let _ = window.eval(format!(
        "delete window[{}]",
        serde_json::to_string(&key).unwrap()
    ));
    let periods = uc_providers::BillingPeriods::default_store();
    store_billing_read(
        &periods,
        &app.state::<ChatStore>(),
        session,
        read,
        Utc::now(),
    )
}

fn store_billing_read(
    periods: &uc_providers::BillingPeriods,
    store: &ChatStore,
    session: &ChatSession,
    read: BillingRead,
    now: chrono::DateTime<Utc>,
) -> Result<bool, String> {
    let (changed, organizations) = store_source_billing(
        periods,
        &session.provider,
        &format!("session:{}", session.id),
        &session.billing_organizations,
        read,
        now,
    )?;
    store.set_billing_organizations(&session.id, organizations)?;
    Ok(changed)
}

fn store_source_billing(
    periods: &uc_providers::BillingPeriods,
    provider: &str,
    source: &str,
    previous: &[String],
    read: BillingRead,
    now: chrono::DateTime<Utc>,
) -> Result<(bool, Vec<String>), String> {
    let mut unique_accounts = HashSet::new();
    if !matches!(provider, "claude" | "codex")
        || read.organizations.len() > 32
        || read.organizations.iter().any(|org| {
            uuid::Uuid::parse_str(&org.uuid)
                .ok()
                .is_none_or(|id| id.is_nil() || !unique_accounts.insert(id))
        })
    {
        return Err("Billing account metadata is invalid".into());
    }
    if read.state != "ready" && !matches!(read.status, Some(401 | 403)) {
        return Err("The subscription billing request did not complete".into());
    }
    let mut changed = false;
    let mut organizations = Vec::new();
    if read.state == "ready" {
        for org in read.organizations {
            let uuid = uuid::Uuid::parse_str(&org.uuid).unwrap().to_string();
            organizations.push(uuid.clone());
            let before = periods.peek_for(provider, &uuid, now);
            if org.status == 200 {
                let plan = uc_providers::BillingPeriods::plan_for(
                    provider,
                    &org.plan_type,
                    org.tier.as_deref(),
                );
                let details = org.details.unwrap_or(Value::Null);
                let end = billing_end(provider, &details);
                if plan.is_none()
                    || details["status"].as_str().is_some_and(|status| {
                        matches!(
                            status.trim().to_ascii_lowercase().as_str(),
                            "inactive" | "canceled" | "cancelled" | "expired"
                        )
                    })
                {
                    periods
                        .invalidate_for(provider, &uuid)
                        .map_err(safe_error)?;
                } else {
                    periods
                        .save_from(
                            provider,
                            &uuid,
                            plan.as_deref().unwrap_or(""),
                            end,
                            now,
                            source,
                        )
                        .map_err(safe_error)?;
                }
            } else if matches!(org.status, 401 | 403 | 404) {
                periods
                    .invalidate_source(provider, &uuid, source)
                    .map_err(safe_error)?;
            }
            let after = periods.peek_for(provider, &uuid, now);
            changed |=
                before != after || (matches!(org.status, 401 | 403 | 404) && after.is_none());
        }
    }
    for account in previous {
        if !organizations.contains(account) {
            let before = periods.peek_for(provider, account, now);
            periods
                .invalidate_source(provider, account, source)
                .map_err(safe_error)?;
            let after = periods.peek_for(provider, account, now);
            changed |= before != after || after.is_none();
        }
    }
    Ok((changed, organizations))
}

pub async fn run_billing_reader(app: AppHandle) {
    let windows = app.state::<ChatWindows>();
    let mut ticks = tokio::time::interval(Duration::from_secs(300));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticks.tick() => {},
            _ = windows.billing_requested.notified() => {},
        }
        let sessions = match app.state::<ChatStore>().list() {
            Ok(sessions) => sessions,
            Err(_) => continue,
        };
        let mut changed = HashSet::new();
        let periods = uc_providers::BillingPeriods::default_store();
        for provider in ["claude", "codex"] {
            if !periods.browser_enabled_for(provider) {
                continue;
            }
            match read_chrome_billing(provider).await {
                Ok(reads) => match store_chrome_billing(&periods, provider, reads, Utc::now()) {
                    Ok(true) => {
                        changed.insert(provider.to_string());
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::debug!(%error, provider, "Browser billing could not be saved")
                    }
                },
                Err(error) => tracing::debug!(%error, provider, "Browser billing is unavailable"),
            }
        }
        for session in sessions
            .into_iter()
            .filter(|session| matches!(session.provider.as_str(), "claude" | "codex"))
        {
            match read_session_billing(&app, &session).await {
                Ok(true) => {
                    changed.insert(session.provider.clone());
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::debug!(%error, "Billing is unavailable for a saved web session")
                }
            }
        }
        if !changed.is_empty() {
            let engine = app.state::<crate::service::BackendService>().engine();
            for id in engine.provider_ids().into_iter().filter(|id| {
                id.split_once('@')
                    .is_some_and(|(provider, _)| changed.contains(provider))
            }) {
                engine.invalidate_plan_term(&id);
                refresh_billing_account(&engine, &id).await;
            }
        }
    }
}

fn chrome_data_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(|root| PathBuf::from(root).join("Google/Chrome/User Data"))
    }
    #[cfg(target_os = "macos")]
    {
        Some(uc_core::paths::home_dir().join("Library/Application Support/Google/Chrome"))
    }
    #[cfg(target_os = "linux")]
    {
        Some(
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| uc_core::paths::home_dir().join(".config"))
                .join("google-chrome"),
        )
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

fn chrome_port(text: &str) -> Option<(u16, &str)> {
    let mut lines = text.lines();
    let port: u16 = lines.next()?.parse().ok()?;
    let path = lines.next()?.trim();
    let id = path.strip_prefix("/devtools/browser/")?;
    (port != 0 && uuid::Uuid::parse_str(id).is_ok() && lines.next().is_none())
        .then_some((port, path))
}

fn local_debugger_url(raw: &str, port: u16) -> Option<Url> {
    let mut url = Url::parse(raw).ok()?;
    if url.scheme() != "ws"
        || !matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
        || url.port() != Some(port)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/devtools/")
    {
        return None;
    }
    url.set_host(Some("127.0.0.1")).ok()?;
    Some(url)
}

async fn read_chrome_billing(provider: &str) -> Result<Vec<BillingRead>, String> {
    tokio::time::timeout(Duration::from_secs(30), read_chrome_billing_inner(provider))
        .await
        .map_err(|_| "The Chrome billing request timed out".to_string())?
}

async fn read_chrome_billing_inner(provider: &str) -> Result<Vec<BillingRead>, String> {
    let unavailable =
        || "Open the signed-in Chrome Default profile with remote debugging enabled".to_string();
    let directory = chrome_data_directory().ok_or_else(unavailable)?;
    let state: Value = serde_json::from_slice(
        &tokio::fs::read(directory.join("Local State"))
            .await
            .map_err(|_| unavailable())?,
    )
    .map_err(|_| unavailable())?;
    if !chrome_default_profile(&state) {
        return Err(unavailable());
    }
    let port_file = directory.join("DevToolsActivePort");
    if tokio::fs::metadata(&port_file)
        .await
        .map_err(|_| unavailable())?
        .len()
        > 256
    {
        return Err(unavailable());
    }
    let text = tokio::fs::read_to_string(port_file)
        .await
        .map_err(|_| unavailable())?;
    let (port, browser_path) = chrome_port(&text).ok_or_else(unavailable)?;
    let socket = local_debugger_url(&format!("ws://127.0.0.1:{port}{browser_path}"), port)
        .ok_or_else(unavailable)?;
    read_chrome_billing_socket(provider, &socket).await
}

fn chrome_default_profile(state: &Value) -> bool {
    state["profile"]["last_active_profiles"] == serde_json::json!(["Default"])
        && state["profile"]
            .get("last_used")
            .is_none_or(|last_used| matches!(last_used.as_str(), Some("Default" | "")))
}

async fn read_chrome_billing_socket(
    provider: &str,
    socket: &Url,
) -> Result<Vec<BillingRead>, String> {
    let unavailable =
        || "The signed-in Chrome Default profile could not confirm billing".to_string();
    let host = official_url(provider)?.host_str().unwrap().to_string();
    use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
    let configuration = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let (mut connection, _) =
        tokio_tungstenite::connect_async_with_config(socket.as_str(), Some(configuration), false)
            .await
            .map_err(|_| unavailable())?;
    let version = chrome_request(
        &mut connection,
        serde_json::json!({"id":1,"method":"Browser.getVersion"}),
    )
    .await?;
    if !version["product"]
        .as_str()
        .is_some_and(|product| product.starts_with("Chrome/"))
    {
        return Err(unavailable());
    }
    let contexts = chrome_request(
        &mut connection,
        serde_json::json!({"id":2,"method":"Target.getBrowserContexts"}),
    )
    .await?;
    let default_context = contexts["defaultBrowserContextId"]
        .as_str()
        .filter(|id| !id.is_empty());
    let targets = chrome_request(
        &mut connection,
        serde_json::json!({"id":3,"method":"Target.getTargets"}),
    )
    .await?;
    let target = targets["targetInfos"]
        .as_array()
        .and_then(|pages| {
            pages.iter().find(|page| {
                page["type"] == "page"
                    && default_context.is_some_and(|context| page["browserContextId"] == context)
                    && page["url"]
                        .as_str()
                        .and_then(|raw| Url::parse(raw).ok())
                        .as_ref()
                        .is_some_and(|url| billing_origin(provider, url))
            })
        })
        .and_then(|page| page["targetId"].as_str())
        .map(str::to_owned);
    let temporary = target.is_none();
    let mut cleanup = ChromeTargetGuard {
        socket: socket.clone(),
        target: None,
    };
    let target = match target {
        Some(target) => target,
        None => {
            let mut params = serde_json::json!({"url":format!("https://{host}/robots.txt"),"background":true,"hidden":true});
            if let Some(context) = default_context {
                params["browserContextId"] = context.into();
            }
            let created = chrome_request(
                &mut connection,
                serde_json::json!({"id":8,"method":"Target.createTarget","params":params}),
            )
            .await?;
            let target = created["targetId"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(unavailable)?
                .to_owned();
            cleanup.target = Some(target.clone());
            target
        }
    };
    let attached = chrome_request(
        &mut connection,
        serde_json::json!({"id":4,"method":"Target.attachToTarget","params":{
        "targetId":target,"flatten":true}}),
    )
    .await?;
    let session = attached["sessionId"].as_str().ok_or_else(unavailable)?;
    let key = format!("__ucBilling{}", uuid::Uuid::new_v4().simple());
    let script = billing_script(provider, &key);
    let expression = format!(
        r#"(async () => {{
if (location.protocol !== 'https:' || location.hostname !== {host_literal}) return {{state:'failed'}};
{script};
const key = {};
const deadline = Date.now() + 20000;
while (window[key]?.state === 'pending' && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 100));
const read = window[key] ?? {{state:'failed'}};
delete window[key];
return read;
}})()"#,
        serde_json::to_string(&key).unwrap(),
        host_literal = serde_json::to_string(&host).unwrap()
    );
    let request = serde_json::json!({"id":5,"sessionId":session,"method":"Runtime.evaluate","params":{
        "expression":expression,"awaitPromise":true,"returnByValue":true}});
    let response = async {
        if temporary {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            loop {
                let state = chrome_request(&mut connection, serde_json::json!({"id":9,"sessionId":session,
                    "method":"Runtime.evaluate","params":{"expression":format!("location.protocol === 'https:' && location.hostname === {} && document.readyState === 'complete'", serde_json::to_string(&host).unwrap()),"returnByValue":true}})).await?;
                if state["result"]["value"] == true { break; }
                if tokio::time::Instant::now() >= deadline { return Err(unavailable()); }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        chrome_request(&mut connection, request).await
    }.await;
    let _ = chrome_request(&mut connection, serde_json::json!({"id":6,"method":"Target.detachFromTarget","params":{"sessionId":session}})).await;
    if temporary
        && chrome_request(
            &mut connection,
            serde_json::json!({"id":10,"method":"Target.closeTarget","params":{"targetId":target}}),
        )
        .await
        .is_ok_and(|response| response["success"] == true)
    {
        cleanup.target = None;
    }
    let _ = connection.close(None).await;
    let response = response?;
    if response.get("exceptionDetails").is_some() {
        return Err(unavailable());
    }
    let read: BillingRead =
        serde_json::from_value(response["result"]["value"].clone()).map_err(|_| unavailable())?;
    if read.state != "ready" && !matches!(read.status, Some(401 | 403)) {
        return Err(unavailable());
    }
    Ok(vec![read])
}

type ChromeSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct ChromeTargetGuard {
    socket: Url,
    target: Option<String>,
}

impl Drop for ChromeTargetGuard {
    fn drop(&mut self) {
        let Some(target) = self.target.take() else {
            return;
        };
        let socket = self.socket.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(4), async {
                if let Ok((mut connection, _)) = tokio_tungstenite::connect_async(socket.as_str()).await {
                    let _ = chrome_request(&mut connection, serde_json::json!({"id":1,"method":"Target.closeTarget","params":{"targetId":target}})).await;
                    let _ = connection.close(None).await;
                }
            }).await;
        });
    }
}

async fn chrome_request(connection: &mut ChromeSocket, request: Value) -> Result<Value, String> {
    use tokio_tungstenite::tungstenite::Message;
    let unavailable = || "The signed-in browser session could not confirm billing".to_string();
    let deadline = if request["params"]["awaitPromise"] == true {
        Duration::from_secs(24)
    } else {
        Duration::from_secs(3)
    };
    tokio::time::timeout(deadline, async {
        let id = request["id"].clone();
        connection
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|_| unavailable())?;
        while let Some(message) = connection.next().await {
            let message = message.map_err(|_| unavailable())?;
            let Message::Text(text) = message else {
                continue;
            };
            let response: Value = serde_json::from_str(&text).map_err(|_| unavailable())?;
            if response["id"] != id {
                continue;
            }
            if response.get("error").is_some() {
                return Err(unavailable());
            }
            return Ok(response["result"].clone());
        }
        Err(unavailable())
    })
    .await
    .map_err(|_| unavailable())?
}

fn store_chrome_billing(
    periods: &uc_providers::BillingPeriods,
    provider: &str,
    reads: Vec<BillingRead>,
    now: chrono::DateTime<Utc>,
) -> Result<bool, String> {
    if reads.len() != 1 {
        return Err("Billing account metadata is invalid".into());
    }
    let (changed, organizations) = store_source_billing(
        periods,
        provider,
        "browser",
        &periods.browser_accounts(provider),
        reads.into_iter().next().unwrap(),
        now,
    )?;
    periods
        .enable_browser_for(provider, &organizations)
        .map_err(safe_error)?;
    Ok(changed)
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

    #[test]
    fn billing_reader_saves_only_the_current_organization_and_clears_rejected_sessions() {
        let temporary = tempfile::tempdir().unwrap();
        let store = ChatStore::new(temporary.path().join("chat"));
        let periods = uc_providers::BillingPeriods::new(temporary.path().join("billing"));
        let session = store.create("claude", None).unwrap();
        let org = "00000000-0000-4000-8000-000000000001";
        let now = Utc::now();
        let end = now + chrono::Duration::days(24);
        let read = serde_json::from_value(serde_json::json!({
            "state":"ready", "organizations":[{"uuid":org,"planType":"claude_max",
                "tier":"default_claude_max_20x","status":200,"details":{
                    "status":"active","next_charge_at":end.to_rfc3339()}}]
        }))
        .unwrap();
        assert!(store_billing_read(&periods, &store, &session, read, now).unwrap());
        assert_eq!(
            periods.get(org, Some("Max 20x"), now),
            Some(uc_core::PlanTerm::Stated {
                ends_at: end,
                checked_at: Some(now)
            })
        );
        let session = store.get(&session.id).unwrap();
        assert_eq!(session.billing_organizations, [org]);
        let network =
            serde_json::from_value(serde_json::json!({"state":"failed","status":500})).unwrap();
        assert!(store_billing_read(&periods, &store, &session, network, now).is_err());
        assert!(periods.get(org, Some("Max 20x"), now).is_some());
        let rejected =
            serde_json::from_value(serde_json::json!({"state":"failed","status":403})).unwrap();
        assert!(store_billing_read(&periods, &store, &session, rejected, now).unwrap());
        assert_eq!(periods.get(org, Some("Max 20x"), now), None);
        assert!(
            store
                .get(&session.id)
                .unwrap()
                .billing_organizations
                .is_empty()
        );
    }

    #[test]
    fn chrome_debugging_is_bound_to_the_running_local_browser_and_billing_is_opt_in() {
        assert!(
            chrome_port("9222\n/devtools/browser/00000000-0000-4000-8000-000000000001\n").is_some()
        );
        for invalid in [
            "0\n/devtools/browser/00000000-0000-4000-8000-000000000001",
            "9222\n/devtools/browser/invalid",
            "9222\n/devtools/browser/00000000-0000-4000-8000-000000000001\nextra",
        ] {
            assert!(chrome_port(invalid).is_none());
        }
        for invalid in [
            "ws://evil.example:9222/devtools/page/x",
            "wss://127.0.0.1:9222/devtools/page/x",
            "ws://127.0.0.1:9223/devtools/page/x",
            "ws://user@localhost:9222/devtools/page/x",
            "ws://localhost:9222/devtools/page/x?token=x",
        ] {
            assert!(local_debugger_url(invalid, 9222).is_none());
        }
        let temporary = tempfile::tempdir().unwrap();
        let periods = uc_providers::BillingPeriods::new(temporary.path());
        assert!(!periods.browser_enabled());
        let org = "00000000-0000-4000-8000-000000000001";
        let now = Utc::now();
        let read = serde_json::from_value(serde_json::json!({"state":"ready","organizations":[{
        "uuid":org,"planType":"claude_max","tier":null,"status":200,"details":{
            "status":"active","next_charge_at":(now + chrono::Duration::days(24)).to_rfc3339()
        }}]}))
        .unwrap();
        assert!(store_chrome_billing(&periods, "claude", vec![read], now).unwrap());
        assert!(periods.browser_enabled());
        assert_eq!(periods.browser_organizations(), [org]);
        let removed =
            serde_json::from_value(serde_json::json!({"state":"ready","organizations":[]}))
                .unwrap();
        assert!(store_chrome_billing(&periods, "claude", vec![removed], now).unwrap());
        assert_eq!(periods.get(org, Some("Max"), now), None);
    }

    #[test]
    fn billing_rejects_unsafe_origins_and_cannot_persist_unknown_response_fields() {
        for url in [
            "http://claude.ai/",
            "https://claude.ai.evil.example/",
            "https://user@claude.ai/",
            "https://chatgpt.com/",
        ] {
            assert!(!billing_origin("claude", &Url::parse(url).unwrap()));
        }
        assert!(billing_origin(
            "claude",
            &Url::parse("https://claude.ai/settings/billing").unwrap()
        ));
        assert!(billing_origin(
            "codex",
            &Url::parse("https://chatgpt.com/").unwrap()
        ));
        assert!(!billing_origin(
            "codex",
            &Url::parse("https://claude.ai/").unwrap()
        ));
        assert!(!billing_origin(
            "codex",
            &Url::parse("https://chatgpt.com.evil.example/").unwrap()
        ));
        assert!(
            serde_json::from_value::<BillingRead>(serde_json::json!({
                "state":"ready", "cookies":"must-not-be-stored"
            }))
            .is_err()
        );
    }

    fn paid_read(provider: &str, account: &str, end: chrono::DateTime<Utc>) -> BillingRead {
        serde_json::from_value(serde_json::json!({"state":"ready","organizations":[{
            "uuid":account,"planType":if provider == "claude" { "claude_max" } else { "prolite" },
            "tier":null,"status":200,"details":{"status":"active","next_charge_at":end.to_rfc3339(),"expires_at":end.to_rfc3339()}
        }]})).unwrap()
    }

    #[test]
    fn browser_and_app_session_auth_failures_only_revoke_their_own_evidence() {
        for provider in ["claude", "codex"] {
            let temporary = tempfile::tempdir().unwrap();
            let store = ChatStore::new(temporary.path().join("chat"));
            let periods = uc_providers::BillingPeriods::new(temporary.path().join("billing"));
            let session = store.create(provider, None).unwrap();
            let account = "00000000-0000-4000-8000-000000000001";
            let now = Utc::now();
            let end = now + chrono::Duration::days(24);
            let plan = if provider == "claude" {
                "Max"
            } else {
                "Pro 5x"
            };
            assert!(
                store_billing_read(
                    &periods,
                    &store,
                    &session,
                    paid_read(provider, account, end),
                    now - chrono::Duration::minutes(1)
                )
                .unwrap()
            );
            let session = store.get(&session.id).unwrap();
            assert!(
                store_chrome_billing(
                    &periods,
                    provider,
                    vec![paid_read(provider, account, end)],
                    now
                )
                .unwrap()
            );
            let term = Some(uc_core::PlanTerm::Stated {
                ends_at: end,
                checked_at: Some(now),
            });
            let rejected = || {
                serde_json::from_value(serde_json::json!({"state":"failed","status":403})).unwrap()
            };
            assert!(!store_billing_read(&periods, &store, &session, rejected(), now).unwrap());
            assert_eq!(periods.get_for(provider, account, Some(plan), now), term);
            let limited =
                serde_json::from_value(serde_json::json!({"state":"failed","status":429})).unwrap();
            assert!(store_chrome_billing(&periods, provider, vec![limited], now).is_err());
            assert_eq!(periods.get_for(provider, account, Some(plan), now), term);
            assert!(store_chrome_billing(&periods, provider, vec![rejected()], now).unwrap());
            assert_eq!(periods.get_for(provider, account, Some(plan), now), None);
            assert!(periods.browser_enabled_for(provider));
            assert!(periods.browser_accounts(provider).is_empty());
            assert!(
                store_chrome_billing(
                    &periods,
                    provider,
                    vec![paid_read(provider, account, end)],
                    now
                )
                .unwrap()
            );
            assert_eq!(periods.get_for(provider, account, Some(plan), now), term);
            let free = serde_json::from_value(serde_json::json!({"state":"ready","organizations":[{
                "uuid":account,"planType":"free","tier":null,"status":200,"details":{"status":"inactive","expires_at":end.to_rfc3339()}
            }]})).unwrap();
            assert!(store_chrome_billing(&periods, provider, vec![free], now).unwrap());
            assert_eq!(periods.peek_for(provider, account, now), None);
        }
    }

    #[test]
    fn connection_success_requires_new_verified_evidence_on_the_clicked_account() {
        let now = Utc::now();
        let snapshot: uc_core::ProviderSnapshot = serde_json::from_value(serde_json::json!({
            "providerID":"codex@fixture","displayName":"Codex","plan":"Pro 5x","planCheckedAt":now.to_rfc3339(),
            "planTerm":{"basis":"stated","endsAt":(now + chrono::Duration::days(24)).to_rfc3339(),"checkedAt":now.to_rfc3339()},
            "lines":[],"refreshedAt":now.to_rfc3339()
        })).unwrap();
        assert!(billing_connected(Some(&snapshot), now));
        assert!(!billing_connected(None, now));
        assert!(!billing_connected(
            Some(&snapshot),
            now + chrono::Duration::seconds(1)
        ));
        let mut unconfirmed = snapshot.clone();
        unconfirmed.plan_checked_at = None;
        assert!(!billing_connected(Some(&unconfirmed), now));
        unconfirmed.plan_checked_at = Some(now);
        unconfirmed.plan_term = None;
        assert!(!billing_connected(Some(&unconfirmed), now));
    }

    #[test]
    fn independent_fresh_jwt_cannot_claim_billing_for_a_different_browser_account() {
        let now = Utc::now();
        let first = "00000000-0000-4000-8000-000000000001";
        let second = "00000000-0000-4000-8000-000000000002";
        let read = paid_read("codex", second, now + chrono::Duration::days(7));
        let mut reads = vec![read];
        assert!(billing_read_confirms(&reads, "codex", second, now));
        assert!(!billing_read_confirms(&reads, "codex", first, now));
        reads[0].organizations[0].details.as_mut().unwrap()["status"] =
            serde_json::json!("inactive");
        assert!(!billing_read_confirms(&reads, "codex", second, now));
        reads[0].organizations[0].details.as_mut().unwrap()["status"] = serde_json::json!("active");
        reads[0].organizations[0].details.as_mut().unwrap()["expires_at"] =
            serde_json::json!(now.to_rfc3339());
        assert!(!billing_read_confirms(&reads, "codex", second, now));
    }

    #[tokio::test]
    async fn cancelled_browser_reads_close_hidden_targets_and_never_attach_to_incognito() {
        use tokio_tungstenite::tungstenite::Message;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = Url::parse(&format!(
            "ws://{}/devtools/browser/fixture",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let (attached, attaching) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut connection = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut attached = Some(attached);
            while let Some(Ok(Message::Text(text))) = connection.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "Browser.getVersion" => serde_json::json!({"product":"Chrome/fixture"}),
                    "Target.getBrowserContexts" => {
                        serde_json::json!({"browserContextIds":["cdp-private"],"defaultBrowserContextId":"default-profile"})
                    }
                    "Target.getTargets" => serde_json::json!({"targetInfos":[
                        {"type":"page","targetId":"incognito","browserContextId":"user-incognito","url":"https://claude.ai/new"},
                        {"type":"page","targetId":"other-profile","browserContextId":"other-profile","url":"https://claude.ai/new"}
                    ]}),
                    "Target.createTarget" => {
                        assert_eq!(request["params"]["hidden"], true);
                        assert_eq!(request["params"]["background"], true);
                        assert_eq!(request["params"]["browserContextId"], "default-profile");
                        serde_json::json!({"targetId":"temporary"})
                    }
                    "Target.attachToTarget" => {
                        assert_eq!(request["params"]["targetId"], "temporary");
                        attached.take().unwrap().send(()).unwrap();
                        continue;
                    }
                    method => panic!("Unexpected CDP method: {method}"),
                };
                connection
                    .send(Message::Text(
                        serde_json::json!({"id":request["id"],"result":result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
            let (stream, _) = listener.accept().await.unwrap();
            let mut cleanup = tokio_tungstenite::accept_async(stream).await.unwrap();
            let Message::Text(text) = cleanup.next().await.unwrap().unwrap() else {
                panic!("Missing cleanup request");
            };
            let request: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(request["method"], "Target.closeTarget");
            assert_eq!(request["params"]["targetId"], "temporary");
            cleanup
                .send(Message::Text(
                    serde_json::json!({"id":request["id"],"result":{"success":true}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        });
        let reader =
            tokio::spawn(async move { read_chrome_billing_socket("claude", &socket).await });
        tokio::time::timeout(Duration::from_secs(2), attaching)
            .await
            .unwrap()
            .unwrap();
        reader.abort();
        assert!(reader.await.is_err_and(|error| error.is_cancelled()));
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn default_browser_auth_rejection_is_returned_for_source_invalidation() {
        use tokio_tungstenite::tungstenite::Message;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = Url::parse(&format!(
            "ws://{}/devtools/browser/fixture",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut connection = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut detached = false;
            while let Some(Ok(Message::Text(text))) = connection.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "Browser.getVersion" => serde_json::json!({"product":"Chrome/fixture"}),
                    "Target.getBrowserContexts" => {
                        serde_json::json!({"browserContextIds":[],"defaultBrowserContextId":"default-profile"})
                    }
                    "Target.getTargets" => serde_json::json!({"targetInfos":[
                        {"type":"page","targetId":"incognito","browserContextId":"user-incognito","url":"https://chatgpt.com/"},
                        {"type":"page","targetId":"existing","browserContextId":"default-profile","url":"https://chatgpt.com/"}
                    ]}),
                    "Target.attachToTarget" => {
                        assert_eq!(request["params"]["targetId"], "existing");
                        serde_json::json!({"sessionId":"attached"})
                    }
                    "Runtime.evaluate" => {
                        assert!(
                            request["params"]["expression"]
                                .as_str()
                                .unwrap()
                                .contains("/api/auth/session")
                        );
                        assert!(
                            request["params"]["expression"]
                                .as_str()
                                .unwrap()
                                .contains("Authorization: 'Bearer '")
                        );
                        serde_json::json!({"result":{"value":{"state":"failed","status":401}}})
                    }
                    "Target.detachFromTarget" => {
                        detached = true;
                        serde_json::json!({})
                    }
                    method => panic!("Unexpected CDP method: {method}"),
                };
                connection
                    .send(Message::Text(
                        serde_json::json!({"id":request["id"],"result":result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
            assert!(detached);
        });
        let reads = read_chrome_billing_socket("codex", &socket).await.unwrap();
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].state, "failed");
        assert_eq!(reads[0].status, Some(401));
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn no_tab_reads_create_and_close_only_session_owned_hidden_targets() {
        use tokio_tungstenite::tungstenite::Message;
        for provider in ["claude", "codex"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let socket = Url::parse(&format!(
                "ws://{}/devtools/browser/fixture",
                listener.local_addr().unwrap()
            ))
            .unwrap();
            let host = official_url(provider)
                .unwrap()
                .host_str()
                .unwrap()
                .to_string();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut connection = tokio_tungstenite::accept_async(stream).await.unwrap();
                let mut closed = false;
                while let Some(Ok(Message::Text(text))) = connection.next().await {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    let result = match request["method"].as_str().unwrap() {
                        "Browser.getVersion" => serde_json::json!({"product":"Chrome/fixture"}),
                        "Target.getBrowserContexts" => serde_json::json!({"browserContextIds":[]}),
                        "Target.getTargets" => {
                            serde_json::json!({"targetInfos":[{"type":"page","targetId":"unverified-context","browserContextId":"unverified","url":format!("https://{host}/")}]})
                        }
                        "Target.createTarget" => {
                            assert_eq!(request["params"]["hidden"], true);
                            assert_eq!(request["params"]["background"], true);
                            assert!(request["params"]["browserContextId"].is_null());
                            assert_eq!(
                                request["params"]["url"],
                                format!("https://{host}/robots.txt")
                            );
                            serde_json::json!({"targetId":"temporary"})
                        }
                        "Target.attachToTarget" => {
                            assert_eq!(request["params"]["targetId"], "temporary");
                            serde_json::json!({"sessionId":"attached"})
                        }
                        "Runtime.evaluate" if request["params"]["awaitPromise"] != true => {
                            serde_json::json!({"result":{"value":true}})
                        }
                        "Runtime.evaluate" => {
                            serde_json::json!({"result":{"value":{"state":"ready","organizations":[]}}})
                        }
                        "Target.detachFromTarget" => serde_json::json!({}),
                        "Target.closeTarget" => {
                            assert_eq!(request["params"]["targetId"], "temporary");
                            closed = true;
                            serde_json::json!({"success":true})
                        }
                        method => panic!("Unexpected CDP method: {method}"),
                    };
                    connection
                        .send(Message::Text(
                            serde_json::json!({"id":request["id"],"result":result})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                }
                assert!(closed);
            });
            let reads = read_chrome_billing_socket(provider, &socket).await.unwrap();
            assert_eq!(reads[0].state, "ready");
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    struct BlockingBillingProvider {
        provider: uc_core::Provider,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
        calls: std::sync::atomic::AtomicUsize,
        checked_at: chrono::DateTime<Utc>,
    }

    #[async_trait::async_trait]
    impl uc_core::ProviderRuntime for BlockingBillingProvider {
        fn provider(&self) -> &uc_core::Provider {
            &self.provider
        }
        fn widget_descriptors(&self) -> Vec<uc_core::WidgetDescriptor> {
            Vec::new()
        }
        async fn has_local_credentials(&self) -> bool {
            true
        }
        async fn refresh(&self, _: uc_core::RefreshContext) -> uc_core::ProviderSnapshot {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                self.entered.notify_one();
                self.release.notified().await;
            }
            let mut snapshot = uc_core::ProviderSnapshot::make(
                &self.provider,
                Some("Pro 5x".into()),
                Vec::new(),
                self.checked_at,
            );
            snapshot.plan_checked_at = Some(self.checked_at);
            snapshot.plan_term = Some(uc_core::PlanTerm::Stated {
                ends_at: self.checked_at + chrono::Duration::days(24),
                checked_at: Some(self.checked_at),
            });
            snapshot
        }
    }

    #[tokio::test]
    async fn billing_connection_retries_after_an_overlapping_quota_refresh() {
        let directory = tempfile::tempdir().unwrap();
        let provider = std::sync::Arc::new(BlockingBillingProvider {
            provider: uc_core::Provider::new("codex@fixture", "Codex"),
            entered: Default::default(),
            release: Default::default(),
            calls: Default::default(),
            checked_at: Utc::now(),
        });
        let config = uc_engine::EngineConfig::default();
        let engine = std::sync::Arc::new(uc_engine::Engine::new(
            vec![provider.clone()],
            uc_engine::SnapshotCache::new(
                directory.path().join("cache.json"),
                config.refresh_interval,
            ),
            config,
        ));
        let first_engine = engine.clone();
        let first = tokio::spawn(async move { first_engine.refresh("codex@fixture", true).await });
        provider.entered.notified().await;
        engine.invalidate_plan_term("codex@fixture");
        let retry_engine = engine.clone();
        let retry =
            tokio::spawn(
                async move { refresh_billing_account(&retry_engine, "codex@fixture").await },
            );
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!retry.is_finished());
        provider.release.notify_one();
        first.await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), retry)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(billing_connected(
            engine.snapshots().get("codex@fixture"),
            provider.checked_at
        ));
    }
}
