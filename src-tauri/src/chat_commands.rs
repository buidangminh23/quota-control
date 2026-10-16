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
    browser_billing: SharedChromeConnection,
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
    accounts: State<'_, crate::account_commands::Accounts>,
    account_id: String,
) -> Result<bool, String> {
    accounts.billing_label(&account_id)?;
    if connect_account_billing(&app, &account_id).await? {
        return Ok(true);
    }
    let provider = billing_provider(&account_id).ok_or("Unsupported billing provider")?;
    crate::browser::open_login_page(&app, billing_page(provider)?.as_str())?;
    request_account_billing(&app, &account_id);
    Ok(false)
}

fn billing_provider(account_id: &str) -> Option<&str> {
    account_id
        .split_once('@')
        .filter(|(_, identity)| !identity.is_empty())
        .map(|(provider, _)| provider)
        .filter(|provider| matches!(*provider, "claude" | "codex"))
}

fn browser_billing_enabled(provider: &str, account_ids: &[String]) -> bool {
    account_ids
        .iter()
        .any(|id| billing_provider(id) == Some(provider))
}

fn billing_page(provider: &str) -> Result<Url, String> {
    match provider {
        "claude" => Url::parse("https://claude.ai/settings/billing").map_err(safe_error),
        "codex" => Url::parse("https://chatgpt.com/#settings/Account").map_err(safe_error),
        _ => Err("Unsupported billing provider".into()),
    }
}

pub fn request_account_billing(app: &AppHandle, account_id: &str) {
    if billing_provider(account_id).is_none() {
        return;
    }
    request_billing_refresh(app);
}

pub fn request_billing_refresh(app: &AppHandle) {
    app.state::<ChatWindows>().billing_requested.notify_one();
}

async fn connect_account_billing(app: &AppHandle, account_id: &str) -> Result<bool, String> {
    let accounts = app.state::<crate::account_commands::Accounts>();
    accounts.billing_label(account_id)?;
    let provider =
        billing_provider(account_id).ok_or("Billing requires a supported connected account")?;
    let periods = uc_providers::BillingPeriods::default_store();
    let windows = app.state::<ChatWindows>();
    let chrome = read_chrome_billing(provider, &windows.browser_billing).await;
    if let Err(error) = &chrome {
        tracing::debug!(%error, "The billing connection could not use Chrome");
    }
    if let Ok(reads) = chrome {
        let observed_at = Utc::now();
        let account_uuid = accounts.billing_account_uuid(account_id).await?;
        let matching_account = billing_read_confirms(&reads, provider, &account_uuid, observed_at);
        let without_subscription =
            billing_read_without_subscription(&reads, provider, &account_uuid);
        let changed = store_chrome_billing(&periods, provider, reads, observed_at)?;
        let engine = app.state::<crate::service::BackendService>().engine();
        if changed {
            refresh_billing_provider(&engine, provider).await;
        } else {
            refresh_billing_account(&engine, account_id).await;
        }
        if without_subscription
            || (matching_account
                && billing_connected(engine.snapshots().get(account_id), observed_at))
        {
            return Ok(true);
        }
        tracing::debug!("The existing browser session has not confirmed this account's billing");
    }
    Ok(false)
}

fn billing_read_without_subscription(reads: &[BillingRead], provider: &str, account: &str) -> bool {
    reads.len() == 1
        && reads[0].state == "ready"
        && reads[0].organizations.iter().any(|org| {
            org.uuid.eq_ignore_ascii_case(account)
                && org.status == 200
                && matches!(
                    (provider, org.plan_type.as_str()),
                    ("claude", "claude_free") | ("codex", "free")
                )
                && org
                    .details
                    .as_ref()
                    .is_some_and(|details| details["status"] == "inactive")
        })
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

pub(crate) async fn refresh_billing_provider(engine: &uc_engine::Engine, provider: &str) {
    let ids: Vec<_> = engine
        .provider_ids()
        .into_iter()
        .filter(|id| billing_provider(id) == Some(provider))
        .collect();
    refresh_synced_billing_accounts(engine, &ids).await;
}

pub(crate) async fn refresh_synced_billing_accounts(engine: &uc_engine::Engine, ids: &[String]) {
    let mut ids: Vec<_> = ids
        .iter()
        .map(String::as_str)
        .filter(|id| billing_provider(id).is_some() && engine.runtime(id).is_some())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    for id in &ids {
        engine.invalidate_plan_term(id);
    }
    futures::future::join_all(ids.iter().map(|id| refresh_billing_account(engine, id))).await;
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BillingRead {
    state: String,
    #[serde(default)]
    stage: Option<String>,
    #[serde(default)]
    account_count: Option<usize>,
    #[serde(default)]
    temporary_target: Option<bool>,
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

fn billing_stage(stage: Option<&str>) -> &'static str {
    match stage {
        Some("session_http") => "session_http",
        Some("session_json") => "session_json",
        Some("session_token") => "session_token",
        Some("accounts_http") => "accounts_http",
        Some("accounts_json") => "accounts_json",
        Some("accounts_schema") => "accounts_schema",
        Some("account_id_schema") => "account_id_schema",
        Some("account_plan_schema") => "account_plan_schema",
        Some("account_subscription_schema") => "account_subscription_schema",
        Some("subscription_http") => "subscription_http",
        Some("subscription_json") => "subscription_json",
        Some("origin_guard") => "origin_guard",
        Some("read_missing") => "read_missing",
        Some("read_timeout") => "read_timeout",
        Some("ready") => "ready",
        _ => "unknown",
    }
}

fn billing_http_status(status: Option<u16>) -> Option<u16> {
    status.filter(|status| (100..=599).contains(status))
}

fn billing_read_error(read: &BillingRead) -> String {
    format!(
        "Billing diagnostic: stage={}, http_status={:?}, account_count={:?}, pending={}, temporary_target={:?}",
        billing_stage(read.stage.as_deref()),
        billing_http_status(read.status),
        read.account_count.filter(|count| *count <= 33),
        read.state == "pending",
        read.temporary_target
    )
}

fn billing_failure(stage: &'static str) -> String {
    format!("Billing diagnostic: {stage}")
}

fn billing_account_stage<'a>(
    reads: &'a [BillingRead],
    provider: &str,
    account: &str,
    now: chrono::DateTime<Utc>,
) -> (&'static str, Option<u16>, Option<&'a BillingOrganization>) {
    let Some(read) = reads.first().filter(|_| reads.len() == 1) else {
        return ("read_schema", None, None);
    };
    if read.state != "ready" {
        return (
            billing_stage(read.stage.as_deref()),
            billing_http_status(read.status),
            None,
        );
    }
    let Some(org) = read
        .organizations
        .iter()
        .find(|org| org.uuid.eq_ignore_ascii_case(account))
    else {
        return ("exact_account_missing", None, None);
    };
    let stage = if org.status != 200 {
        "subscription_http"
    } else if matches!(
        (provider, org.plan_type.as_str()),
        ("claude", "claude_free") | ("codex", "free")
    ) {
        "free_plan"
    } else if uc_providers::BillingPeriods::plan_for(provider, &org.plan_type, org.tier.as_deref())
        .is_none()
    {
        "unsupported_plan"
    } else if let Some(end) = org
        .details
        .as_ref()
        .and_then(|details| billing_end(provider, details))
    {
        if end > now {
            "confirmed"
        } else {
            "expired_end"
        }
    } else if org.details.as_ref().is_some_and(|details| {
        details["status"]
            .as_str()
            .is_some_and(|status| status.trim().eq_ignore_ascii_case("active"))
    }) {
        "active_missing_end"
    } else {
        "subscription_inactive_or_unknown"
    };
    (stage, billing_http_status(Some(org.status)), Some(org))
}

#[cfg(test)]
fn billing_plan_label(plan: &str) -> &'static str {
    match plan {
        "claude_max" => "claude_max",
        "claude_pro" => "claude_pro",
        "claude_team" => "claude_team",
        "claude_free" => "claude_free",
        "free" => "free",
        "plus" => "plus",
        "pro" => "pro",
        "prolite" => "prolite",
        "team" => "team",
        "business" => "business",
        "enterprise" => "enterprise",
        "edu" => "edu",
        "go" => "go",
        _ => "other",
    }
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
let stage = 'session_http';
let accountCount;
try {{
window[key].stage = stage;
const session = await fetch('/api/auth/session', {{credentials: 'same-origin'}});
if (!session.ok) {{ window[key] = {{state: 'failed', stage, status: session.status}}; return; }}
stage = 'session_json';
window[key].stage = stage;
const authentication = await session.json();
if (typeof authentication.accessToken !== 'string' || !authentication.accessToken) {{ window[key] = {{state: 'failed', stage: 'session_token', status: 401}}; return; }}
stage = 'accounts_http';
window[key].stage = stage;
const response = await fetch('/backend-api/accounts/check/v4-2023-04-27', {{credentials: 'same-origin', headers: {{Authorization: 'Bearer ' + authentication.accessToken}}}});
if (!response.ok) {{ window[key] = {{state: 'failed', stage, status: response.status}}; return; }}
stage = 'accounts_json';
window[key].stage = stage;
const data = await response.json();
stage = 'accounts_schema';
if (!data.accounts || typeof data.accounts !== 'object' || Array.isArray(data.accounts) || Object.keys(data.accounts).length > 33) {{ window[key] = {{state: 'failed', stage}}; return; }}
accountCount = Object.keys(data.accounts).length;
const rows = [];
for (const [uuid, value] of Object.entries(data.accounts)) {{
if (uuid === 'default') continue;
if (!/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/i.test(uuid) || uuid === '00000000-0000-0000-0000-000000000000') {{ window[key] = {{state: 'failed', stage: 'account_id_schema', accountCount}}; return; }}
if (typeof value?.account?.plan_type !== 'string') {{ window[key] = {{state: 'failed', stage: 'account_plan_schema', accountCount}}; return; }}
if (typeof value?.entitlement?.has_active_subscription !== 'boolean') {{ window[key] = {{state: 'failed', stage: 'account_subscription_schema', accountCount}}; return; }}
rows.push({{uuid, planType: value.account.plan_type, tier: null, status: 200, details: {{status: value.entitlement.has_active_subscription ? 'active' : 'inactive', expires_at: value.entitlement.expires_at ?? null}}}});
}}
window[key] = {{state: 'ready', stage: 'ready', accountCount, organizations: rows}};
}} catch {{ window[key] = {{state: 'failed', stage, accountCount}}; }}
}})();
}})()"#
        );
    }
    format!(
        r#"(() => {{
const key = {key};
window[key] = {{state: 'pending'}};
(async () => {{
let stage = 'accounts_http';
try {{
window[key].stage = stage;
const response = await fetch('/api/organizations', {{credentials: 'same-origin'}});
if (!response.ok) {{ window[key] = {{state: 'failed', stage, status: response.status}}; return; }}
stage = 'accounts_json';
window[key].stage = stage;
const organizations = await response.json();
stage = 'accounts_schema';
if (!Array.isArray(organizations) || organizations.length > 32) {{ window[key] = {{state: 'failed', stage}}; return; }}
const rows = [];
for (const org of organizations) {{
const capabilities = Array.isArray(org.capabilities) ? org.capabilities : [];
if (!capabilities.includes('chat')) continue;
if (typeof org.uuid !== 'string' || !/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$/i.test(org.uuid)) continue;
const planType = capabilities.find(value => ['claude_max', 'claude_pro', 'claude_team'].includes(value)) || 'claude_free';
const tier = typeof org.rate_limit_tier === 'string' ? org.rate_limit_tier : null;
if (planType === 'claude_free') {{ rows.push({{uuid: org.uuid, planType, tier, status: 200, details: {{status: 'inactive'}}}}); continue; }}
stage = 'subscription_http';
window[key].stage = stage;
const billing = await fetch('/api/organizations/' + encodeURIComponent(org.uuid) + '/subscription_details', {{credentials: 'same-origin'}});
let details = null;
if (billing.ok) {{
stage = 'subscription_json';
window[key].stage = stage;
const data = await billing.json();
details = {{status: data.status ?? null, next_charge_at: data.next_charge_at ?? null, plan_ending_at: data.plan_ending_at ?? null}};
}}
rows.push({{uuid: org.uuid, planType, tier, status: billing.status, details}});
}}
window[key] = {{state: 'ready', stage: 'ready', accountCount: organizations.length, organizations: rows}};
}} catch {{ window[key] = {{state: 'failed', stage}}; }}
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
    if read.state != "ready" {
        return Err("The subscription billing request did not complete".into());
    }
    let mut changed = false;
    let mut organizations = Vec::new();
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
            if matches!(
                (provider, org.plan_type.as_str()),
                ("claude", "claude_free") | ("codex", "free")
            ) || end.is_some_and(|end| end <= now)
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
                changed = true;
            } else if let (Some(plan), Some(end)) = (plan, end) {
                periods
                    .save_from(provider, &uuid, &plan, Some(end), now, source)
                    .map_err(safe_error)?;
            }
        }
        let after = periods.peek_for(provider, &uuid, now);
        changed |= before != after;
    }
    Ok((changed, organizations))
}

pub async fn run_billing_reader(app: AppHandle) {
    let windows = app.state::<ChatWindows>();
    let mut delay = Duration::ZERO;
    let mut retries = 0_u32;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(delay) => {},
            _ = windows.billing_requested.notified() => { retries = 0; },
        }
        let sessions = match app.state::<ChatStore>().list() {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::debug!(%error, "Saved billing sessions could not be read");
                Vec::new()
            }
        };
        let mut changed = HashSet::new();
        let periods = uc_providers::BillingPeriods::default_store();
        let account_ids = app
            .state::<crate::service::BackendService>()
            .engine()
            .provider_ids();
        for provider in ["claude", "codex"] {
            if !browser_billing_enabled(provider, &account_ids) {
                continue;
            }
            match read_chrome_billing(provider, &windows.browser_billing).await {
                Ok(reads) => {
                    report_browser_billing(&app, provider, &account_ids, &reads).await;
                    match store_chrome_billing(&periods, provider, reads, Utc::now()) {
                        Ok(true) => {
                            tracing::info!(
                                provider,
                                "Billing refreshed from the existing browser login"
                            );
                            changed.insert(provider.to_string());
                        }
                        Ok(false) => {}
                        Err(error) => {
                            tracing::debug!(%error, provider, "Browser billing could not be saved")
                        }
                    }
                }
                Err(error) => tracing::info!(%error, provider, "Browser billing diagnostic"),
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
            for provider in changed {
                refresh_billing_provider(&engine, &provider).await;
            }
        }
        let engine = app.state::<crate::service::BackendService>().engine();
        let snapshots = engine.snapshots();
        let now = Utc::now();
        let pending = engine.provider_ids().iter().any(|id| {
            billing_provider(id).is_some() && !billing_refresh_complete(snapshots.get(id), now)
        });
        delay = billing_refresh_delay(pending, retries);
        retries = if pending {
            retries.saturating_add(1)
        } else {
            0
        };
    }
}

async fn report_browser_billing(
    app: &AppHandle,
    provider: &str,
    account_ids: &[String],
    reads: &[BillingRead],
) {
    let accounts = app.state::<crate::account_commands::Accounts>();
    let now = Utc::now();
    let account_count = reads
        .first()
        .map_or(0, |read| read.organizations.len().min(32));
    for id in account_ids
        .iter()
        .filter(|id| billing_provider(id) == Some(provider))
    {
        let Ok(uuid) = accounts.billing_account_uuid(id).await else {
            tracing::info!(
                provider,
                stage = "local_account_identity",
                "Browser billing diagnostic"
            );
            continue;
        };
        let (stage, http_status, _) = billing_account_stage(reads, provider, &uuid, now);
        tracing::info!(
            provider,
            stage,
            ?http_status,
            account_count,
            "Browser billing diagnostic"
        );
    }
}

fn billing_refresh_complete(
    snapshot: Option<&uc_core::ProviderSnapshot>,
    now: chrono::DateTime<Utc>,
) -> bool {
    snapshot.is_some_and(|snapshot| {
        snapshot
            .plan
            .as_deref()
            .is_some_and(|plan| plan.eq_ignore_ascii_case("free"))
            || matches!(snapshot.plan_term,
                Some(uc_core::PlanTerm::Stated { ends_at, checked_at: Some(_) })
                if ends_at > now)
    })
}

fn billing_refresh_delay(pending: bool, retries: u32) -> Duration {
    Duration::from_secs(if pending {
        (30_u64 << retries.min(4)).min(300)
    } else {
        300
    })
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

async fn read_chrome_billing(
    provider: &str,
    shared: &SharedChromeConnection,
) -> Result<Vec<BillingRead>, String> {
    let socket = chrome_billing_endpoint().await?;
    read_chrome_billing_shared(provider, &socket, shared).await
}

async fn chrome_billing_endpoint() -> Result<Url, String> {
    let unavailable = || billing_failure("chrome_endpoint_unavailable");
    let directory = chrome_data_directory().ok_or_else(unavailable)?;
    let state: Value = serde_json::from_slice(
        &tokio::fs::read(directory.join("Local State"))
            .await
            .map_err(|_| unavailable())?,
    )
    .map_err(|_| unavailable())?;
    if !chrome_default_profile(&state) {
        return Err(billing_failure("chrome_default_profile_guard"));
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
    local_debugger_url(&format!("ws://127.0.0.1:{port}{browser_path}"), port)
        .ok_or_else(unavailable)
}

fn chrome_default_profile(state: &Value) -> bool {
    state["profile"]["last_active_profiles"] == serde_json::json!(["Default"])
        && state["profile"]
            .get("last_used")
            .is_none_or(|last_used| matches!(last_used.as_str(), Some("Default" | "")))
}

#[cfg(test)]
async fn read_chrome_billing_socket(
    provider: &str,
    socket: &Url,
) -> Result<Vec<BillingRead>, String> {
    read_chrome_billing_shared(provider, socket, &Default::default()).await
}

async fn read_chrome_billing_shared(
    provider: &str,
    socket: &Url,
    shared: &SharedChromeConnection,
) -> Result<Vec<BillingRead>, String> {
    let mut stored = shared.lock().await;
    if stored
        .as_ref()
        .is_some_and(|connection| connection.endpoint != *socket || connection.broken)
    {
        stored.take();
    }
    if stored.is_none() {
        use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
        let configuration = WebSocketConfig::default()
            .max_message_size(Some(1024 * 1024))
            .max_frame_size(Some(1024 * 1024));
        let connect_timeout = Duration::from_secs(120);
        #[cfg(test)]
        let connect_timeout = if matches!(
            std::env::var("UC_BILLING_PROBE_PROVIDER").as_deref(),
            Ok("claude" | "codex")
        ) {
            Duration::from_secs(600)
        } else {
            connect_timeout
        };
        let (connection, _) = tokio::time::timeout(
            connect_timeout,
            tokio_tungstenite::connect_async_with_config(
                socket.as_str(),
                Some(configuration),
                false,
            ),
        )
        .await
        .map_err(|_| billing_failure("chrome_connect_timeout"))?
        .map_err(|_| billing_failure("chrome_connect_failed"))?;
        *stored = Some(ChromeConnection {
            id: uuid::Uuid::new_v4(),
            endpoint: socket.clone(),
            socket: connection,
            next_id: 0,
            broken: false,
        });
    }
    let connection = stored.as_mut().unwrap();
    tokio::time::timeout(
        Duration::from_secs(30),
        collect_chrome_billing(provider, connection, shared),
    )
    .await
    .map_err(|_| billing_failure("chrome_collection_timeout"))?
}

async fn collect_chrome_billing(
    provider: &str,
    connection: &mut ChromeConnection,
    shared: &SharedChromeConnection,
) -> Result<Vec<BillingRead>, String> {
    let host = official_url(provider)?.host_str().unwrap().to_string();
    let version = chrome_request(
        connection,
        serde_json::json!({"id":1,"method":"Browser.getVersion"}),
    )
    .await?;
    if !version["product"]
        .as_str()
        .is_some_and(|product| product.starts_with("Chrome/"))
    {
        return Err(billing_failure("chrome_product_guard"));
    }
    let contexts = chrome_request(
        connection,
        serde_json::json!({"id":2,"method":"Target.getBrowserContexts"}),
    )
    .await?;
    let default_context = contexts["defaultBrowserContextId"]
        .as_str()
        .filter(|id| !id.is_empty());
    let targets = chrome_request(
        connection,
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
    tracing::info!(
        provider,
        temporary_target = temporary,
        "Browser billing target"
    );
    let mut cleanup = ChromeTargetGuard {
        connection: shared.clone(),
        connection_id: connection.id,
        endpoint: connection.endpoint.clone(),
        target: None,
        session: None,
        attaching_target: None,
        creating_target: false,
    };
    let target = match target {
        Some(target) => target,
        None => {
            cleanup.creating_target = true;
            let mut params = serde_json::json!({"url":format!("https://{host}/robots.txt"),"background":true,"hidden":true});
            if let Some(context) = default_context {
                params["browserContextId"] = context.into();
            }
            let created = chrome_request(
                connection,
                serde_json::json!({"id":8,"method":"Target.createTarget","params":params}),
            )
            .await?;
            let target = created["targetId"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| billing_failure("chrome_create_target_schema"))?
                .to_owned();
            cleanup.target = Some(target.clone());
            cleanup.creating_target = false;
            target
        }
    };
    cleanup.attaching_target = Some(target.clone());
    let attached = chrome_request(
        connection,
        serde_json::json!({"id":4,"method":"Target.attachToTarget","params":{
        "targetId":target,"flatten":true}}),
    )
    .await?;
    let session = attached["sessionId"]
        .as_str()
        .ok_or_else(|| billing_failure("chrome_attach_schema"))?;
    cleanup.session = Some(session.to_string());
    cleanup.attaching_target = None;
    let key = format!("__ucBilling{}", uuid::Uuid::new_v4().simple());
    let script = billing_script(provider, &key);
    let key_literal = serde_json::to_string(&key).unwrap();
    let origin_guard = format!(
        "if (location.protocol !== 'https:' || location.hostname !== {}) return {{state:'failed',stage:'origin_guard'}};",
        serde_json::to_string(&host).unwrap()
    );
    let expression = format!(
        r#"(() => {{
{origin_guard}
{script};
return window[{key_literal}];
}})()"#
    );
    let poll_expression = format!(
        "(() => {{ {origin_guard} return window[{key_literal}] ?? {{state:'failed',stage:'read_missing'}}; }})()"
    );
    let request = serde_json::json!({"id":5,"sessionId":session,"method":"Runtime.evaluate","params":{
        "expression":expression,"returnByValue":true}});
    let response = async {
        if temporary {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            loop {
                let state = chrome_request(connection, serde_json::json!({"id":9,"sessionId":session,
                    "method":"Runtime.evaluate","params":{"expression":format!("location.protocol === 'https:' && location.hostname === {} && document.readyState === 'complete'", serde_json::to_string(&host).unwrap()),"returnByValue":true}})).await?;
                if state["result"]["value"] == true { break; }
                if tokio::time::Instant::now() >= deadline { return Err(billing_failure("chrome_target_ready_timeout")); }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        let response = chrome_request(connection, request).await?;
        let mut read = parse_chrome_billing_response(&response)?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while read.state == "pending" && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let response = chrome_request(connection, serde_json::json!({"sessionId":session,"method":"Runtime.evaluate","params":{
                "expression":poll_expression,"returnByValue":true}})).await?;
            read = parse_chrome_billing_response(&response)?;
        }
        Ok::<_, String>(read)
    }.await;
    let _ = chrome_request(connection, serde_json::json!({"sessionId":session,"method":"Runtime.evaluate","params":{
        "expression":format!("(() => {{ {origin_guard} delete window[{key_literal}]; }})()"),"returnByValue":true}})).await;
    if chrome_request(connection, serde_json::json!({"id":6,"method":"Target.detachFromTarget","params":{"sessionId":session}})).await.is_ok() {
        cleanup.session = None;
    }
    if temporary
        && chrome_request(
            connection,
            serde_json::json!({"id":10,"method":"Target.closeTarget","params":{"targetId":target}}),
        )
        .await
        .is_ok_and(|response| response["success"] == true)
    {
        cleanup.target = None;
    }
    let mut read = response.map_err(|error| format!("{error}, temporary_target={temporary}"))?;
    read.temporary_target = Some(temporary);
    if read.state != "ready" && !matches!(read.status, Some(401 | 403)) {
        return Err(billing_read_error(&read));
    }
    Ok(vec![read])
}

fn parse_chrome_billing_response(response: &Value) -> Result<BillingRead, String> {
    if response.get("exceptionDetails").is_some() {
        return Err(billing_failure("chrome_evaluate_exception"));
    }
    serde_json::from_value(response["result"]["value"].clone())
        .map_err(|_| billing_failure("chrome_read_schema"))
}

type ChromeSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

type SharedChromeConnection = std::sync::Arc<tokio::sync::Mutex<Option<ChromeConnection>>>;

struct ChromeConnection {
    id: uuid::Uuid,
    endpoint: Url,
    socket: ChromeSocket,
    next_id: u64,
    broken: bool,
}

struct ChromeTargetGuard {
    connection: SharedChromeConnection,
    connection_id: uuid::Uuid,
    endpoint: Url,
    target: Option<String>,
    session: Option<String>,
    attaching_target: Option<String>,
    creating_target: bool,
}

impl Drop for ChromeTargetGuard {
    fn drop(&mut self) {
        let target = self.target.take();
        let session = self.session.take();
        let attaching_target = self.attaching_target.take();
        let creating_target = self.creating_target;
        if target.is_none() && session.is_none() && attaching_target.is_none() && !creating_target {
            return;
        }
        let shared = self.connection.clone();
        let connection_id = self.connection_id;
        let endpoint = self.endpoint.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            let mut stored = shared.lock().await;
            if stored.as_ref().is_some_and(|connection| connection.id == connection_id && connection.endpoint == endpoint && (connection.broken || creating_target)) {
                stored.take();
                return;
            }
            if let Some(connection) = stored.as_mut()
                && connection.id == connection_id
                && connection.endpoint == endpoint
                && !connection.broken
            {
                let detach = session.map(|session| serde_json::json!({"sessionId":session}))
                    .or_else(|| attaching_target.map(|target| serde_json::json!({"targetId":target})));
                if let Some(params) = detach
                    && chrome_request(connection, serde_json::json!({"method":"Target.detachFromTarget","params":params})).await.is_err()
                {
                    connection.broken = true;
                }
                if let Some(target) = target {
                    let _ = chrome_request(connection,
                        serde_json::json!({"method":"Target.closeTarget","params":{"targetId":target}}),
                    ).await;
                }
            }
            if stored.as_ref().is_some_and(|connection| connection.id == connection_id && connection.broken) {
                stored.take();
            }
        });
    }
}

async fn chrome_request(
    connection: &mut ChromeConnection,
    mut request: Value,
) -> Result<Value, String> {
    use tokio_tungstenite::tungstenite::Message;
    let operation = match request["method"].as_str() {
        Some("Browser.getVersion") => "version",
        Some("Target.getBrowserContexts") => "contexts",
        Some("Target.getTargets") => "targets",
        Some("Target.createTarget") => "create_target",
        Some("Target.attachToTarget") => "attach",
        Some("Target.detachFromTarget") => "detach",
        Some("Target.closeTarget") => "close_target",
        Some("Runtime.evaluate") => "evaluate",
        _ => "unknown",
    };
    let unavailable =
        |reason: &'static str| format!("Billing diagnostic: chrome_{operation}_{reason}");
    let deadline = if request["params"]["awaitPromise"] == true {
        Duration::from_secs(24)
    } else {
        Duration::from_secs(3)
    };
    connection.next_id += 1;
    let id = Value::from(connection.next_id);
    request["id"] = id.clone();
    tokio::time::timeout(deadline, async {
        if connection
            .socket
            .send(Message::Text(request.to_string().into()))
            .await
            .is_err()
        {
            connection.broken = true;
            return Err(unavailable("send"));
        }
        while let Some(message) = connection.socket.next().await {
            let message = message.map_err(|_| {
                connection.broken = true;
                unavailable("receive")
            })?;
            let Message::Text(text) = message else {
                continue;
            };
            let response: Value =
                serde_json::from_str(&text).map_err(|_| unavailable("protocol_json"))?;
            if response["id"] != id {
                continue;
            }
            if response.get("error").is_some() {
                return Err(unavailable("protocol_error"));
            }
            return Ok(response["result"].clone());
        }
        connection.broken = true;
        Err(unavailable("closed"))
    })
    .await
    .map_err(|_| unavailable("timeout"))?
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

    #[tokio::test]
    #[ignore = "Read-only probe using an already signed-in Chrome Default profile"]
    async fn existing_browser_login_confirms_connected_account_billing() {
        let provider =
            std::env::var("UC_BILLING_PROBE_PROVIDER").unwrap_or_else(|_| "codex".into());
        assert!(matches!(provider.as_str(), "claude" | "codex"));
        let started = std::time::Instant::now();
        let result = read_chrome_billing(&provider, &Default::default()).await;
        eprintln!(
            "Billing diagnostic: elapsed_ms={}",
            started.elapsed().as_millis()
        );
        let reads = result.unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(reads.len(), 1);
        eprintln!("{}", billing_read_error(&reads[0]));
        assert!(
            reads[0].state == "ready",
            "The browser billing read did not complete"
        );
        let store = std::sync::Arc::new(uc_accounts::AccountStore::default_store());
        let records = store.list().unwrap();
        let cli = uc_providers::cli_accounts_keeping(&[]);
        let ids: Vec<_> = uc_providers::visible_accounts(&records, &cli)
            .into_iter()
            .map(|account| match account {
                uc_providers::VisibleAccount::Stored(record) => record.id.clone(),
                uc_providers::VisibleAccount::Cli(login) => login.id.clone(),
            })
            .filter(|id| billing_provider(id) == Some(provider.as_str()))
            .collect();
        let mut matched = 0;
        let mut paid_accounts = 0;
        let mut confirmed_periods = 0;
        for id in ids {
            let Ok(uuid) = uc_providers::accounts::billing_account_uuid(store.clone(), &cli, &id)
            else {
                eprintln!("Billing diagnostic: stage=local_account_identity");
                continue;
            };
            let (stage, http_status, org) =
                billing_account_stage(&reads, &provider, &uuid, Utc::now());
            eprintln!("Billing diagnostic: stage={stage}, http_status={http_status:?}");
            if let Some(org) = org {
                matched += 1;
                paid_accounts += usize::from(
                    uc_providers::BillingPeriods::plan_for(
                        &provider,
                        &org.plan_type,
                        org.tier.as_deref(),
                    )
                    .is_some(),
                );
                confirmed_periods +=
                    usize::from(billing_read_confirms(&reads, &provider, &uuid, Utc::now()));
                eprintln!(
                    "Matched {provider} billing: plan={}, confirmed_end={:?}",
                    billing_plan_label(&org.plan_type),
                    org.details
                        .as_ref()
                        .and_then(|details| billing_end(&provider, details))
                );
            }
        }
        assert!(
            matched > 0,
            "The browser session must match a connected provider account"
        );
        eprintln!(
            "Existing {provider} browser login: {matched} matched accounts, {paid_accounts} paid accounts, {confirmed_periods} confirmed billing periods"
        );
        assert_eq!(confirmed_periods, paid_accounts);
    }

    #[test]
    fn billing_diagnostics_redact_untrusted_values_and_classify_exact_matches() {
        let untrusted = "https://example.invalid/?token=secret@example.invalid";
        let read: BillingRead = serde_json::from_value(serde_json::json!({
            "state":"failed", "stage":untrusted, "status":65535, "accountCount":100000
        }))
        .unwrap();
        assert_eq!(
            billing_read_error(&read),
            "Billing diagnostic: stage=unknown, http_status=None, account_count=None, pending=false, temporary_target=None"
        );
        assert_eq!(billing_plan_label(untrusted), "other");
        let now = Utc::now();
        let account = "00000000-0000-4000-8000-000000000001";
        let mut reads = vec![paid_read("codex", account, now + chrono::Duration::days(1))];
        assert_eq!(
            billing_account_stage(&reads, "codex", account, now).0,
            "confirmed"
        );
        assert_eq!(
            billing_account_stage(&reads, "codex", "00000000-0000-4000-8000-000000000002", now).0,
            "exact_account_missing"
        );
        reads[0].organizations[0].details.as_mut().unwrap()["expires_at"] = Value::Null;
        assert_eq!(
            billing_account_stage(&reads, "codex", account, now).0,
            "active_missing_end"
        );
        reads[0].organizations[0].status = 503;
        assert_eq!(
            billing_account_stage(&reads, "codex", account, now).0,
            "subscription_http"
        );
        reads[0].organizations[0].status = 200;
        reads[0].organizations[0].plan_type = "free".into();
        assert_eq!(
            billing_account_stage(&reads, "codex", account, now).0,
            "free_plan"
        );
        assert_eq!(
            billing_account_stage(&reads, "claude", account, now).0,
            "unsupported_plan"
        );
    }

    #[tokio::test]
    async fn browser_diagnostics_keep_http_schema_and_protocol_failures_distinct() {
        use tokio_tungstenite::tungstenite::Message;
        for (payload, expected) in [
            (
                serde_json::json!({"state":"failed","stage":"session_http","status":401}),
                "stage=session_http, http_status=Some(401)",
            ),
            (
                serde_json::json!({"state":"failed","stage":"accounts_http","status":403}),
                "stage=accounts_http, http_status=Some(403)",
            ),
            (
                serde_json::json!({"state":"failed","stage":"account_subscription_schema","accountCount":3}),
                "stage=account_subscription_schema, http_status=None, account_count=Some(3)",
            ),
            (
                serde_json::json!({"state":"failed","stage":"secret@example.invalid","status":503}),
                "stage=unknown, http_status=Some(503)",
            ),
            (Value::Null, "chrome_version_protocol_error"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let socket = Url::parse(&format!(
                "ws://{}/devtools/browser/fixture",
                listener.local_addr().unwrap()
            ))
            .unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut connection = tokio_tungstenite::accept_async(stream).await.unwrap();
                while let Some(Ok(Message::Text(text))) = connection.next().await {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    let response = if payload.is_null() {
                        serde_json::json!({"id":request["id"],"error":{"message":"secret@example.invalid", "code":-32000}})
                    } else {
                        let result = match request["method"].as_str().unwrap() {
                            "Browser.getVersion" => serde_json::json!({"product":"Chrome/fixture"}),
                            "Target.getBrowserContexts" => {
                                serde_json::json!({"defaultBrowserContextId":"default"})
                            }
                            "Target.getTargets" => {
                                serde_json::json!({"targetInfos":[{"type":"page","targetId":"existing","browserContextId":"default","url":"https://chatgpt.com/"}]})
                            }
                            "Target.attachToTarget" => serde_json::json!({"sessionId":"attached"}),
                            "Runtime.evaluate" => serde_json::json!({"result":{"value":payload}}),
                            "Target.detachFromTarget" => serde_json::json!({}),
                            _ => panic!("Unexpected CDP method"),
                        };
                        serde_json::json!({"id":request["id"],"result":result})
                    };
                    connection
                        .send(Message::Text(response.to_string().into()))
                        .await
                        .unwrap();
                }
            });
            let diagnostic = match read_chrome_billing_socket("codex", &socket).await {
                Ok(reads) => billing_read_error(&reads[0]),
                Err(error) => error,
            };
            assert!(
                diagnostic.contains(expected),
                "Unexpected sanitized diagnostic: {diagnostic}"
            );
            assert!(!diagnostic.contains("secret"));
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn browser_billing_polls_natively_without_waiting_for_background_page_timers() {
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
            let mut started = 0;
            let mut polls = 0;
            let mut cleared = false;
            while let Some(Ok(Message::Text(text))) = connection.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "Browser.getVersion" => serde_json::json!({"product":"Chrome/fixture"}),
                    "Target.getBrowserContexts" => {
                        serde_json::json!({"defaultBrowserContextId":"default"})
                    }
                    "Target.getTargets" => {
                        serde_json::json!({"targetInfos":[{"type":"page","targetId":"existing","browserContextId":"default","url":"https://chatgpt.com/"}]})
                    }
                    "Target.attachToTarget" => serde_json::json!({"sessionId":"attached"}),
                    "Runtime.evaluate" => {
                        assert_ne!(request["params"]["awaitPromise"], true);
                        let expression = request["params"]["expression"].as_str().unwrap();
                        assert!(!expression.contains("setTimeout"));
                        assert!(expression.contains("location.hostname"));
                        assert!(expression.contains("location.protocol"));
                        if expression.contains("delete window[") {
                            cleared = true;
                            serde_json::json!({"result":{}})
                        } else if expression.contains("/api/auth/session") {
                            started += 1;
                            serde_json::json!({"result":{"value":{"state":"pending","stage":"session_http"}}})
                        } else {
                            polls += 1;
                            if polls == 1 {
                                serde_json::json!({"result":{"value":{"state":"pending","stage":"accounts_http"}}})
                            } else {
                                serde_json::json!({"result":{"value":{"state":"ready","stage":"ready","organizations":[]}}})
                            }
                        }
                    }
                    "Target.detachFromTarget" => {
                        assert!(cleared);
                        serde_json::json!({})
                    }
                    _ => panic!("Unexpected CDP method"),
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
            assert_eq!(started, 1);
            assert_eq!(polls, 2);
            assert!(cleared);
        });
        let reads = tokio::time::timeout(
            Duration::from_secs(2),
            read_chrome_billing_socket("codex", &socket),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reads[0].state, "ready");
        assert_eq!(reads[0].temporary_target, Some(false));
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

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
    fn billing_reader_keeps_confirmed_dates_through_rejected_sessions() {
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
        assert!(store_billing_read(&periods, &store, &session, rejected, now).is_err());
        assert_eq!(
            periods.get(org, Some("Max 20x"), now),
            Some(uc_core::PlanTerm::Stated {
                ends_at: end,
                checked_at: Some(now)
            })
        );
        assert_eq!(store.get(&session.id).unwrap().billing_organizations, [org]);
    }

    #[test]
    fn chrome_debugging_is_bound_to_the_running_local_browser() {
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
        assert!(!store_chrome_billing(&periods, "claude", vec![removed], now).unwrap());
        assert!(periods.get(org, Some("Max"), now).is_some());
        assert!(periods.browser_organizations().is_empty());
    }

    #[test]
    fn connected_accounts_enable_browser_billing_without_a_second_connection() {
        let temporary = tempfile::tempdir().unwrap();
        let periods = uc_providers::BillingPeriods::new(temporary.path());
        let accounts = vec![
            "claude@first".into(),
            "codex@second".into(),
            "codex@third".into(),
        ];
        for provider in ["claude", "codex"] {
            assert!(!periods.browser_enabled_for(provider));
            assert!(browser_billing_enabled(provider, &accounts));
            assert!(!browser_billing_enabled(provider, &[]));
            assert!(billing_origin(provider, &billing_page(provider).unwrap()));
        }
        for id in ["claude@", "codex", "other@first", "claude.ai@first"] {
            assert!(billing_provider(id).is_none());
            assert!(!browser_billing_enabled("claude", &[id.into()]));
        }
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
    fn browser_and_app_session_failures_preserve_dates_until_authoritative_cancellation() {
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
            assert!(store_billing_read(&periods, &store, &session, rejected(), now).is_err());
            assert_eq!(periods.get_for(provider, account, Some(plan), now), term);
            let limited =
                serde_json::from_value(serde_json::json!({"state":"failed","status":429})).unwrap();
            assert!(store_chrome_billing(&periods, provider, vec![limited], now).is_err());
            assert_eq!(periods.get_for(provider, account, Some(plan), now), term);
            assert!(store_chrome_billing(&periods, provider, vec![rejected()], now).is_err());
            assert_eq!(periods.get_for(provider, account, Some(plan), now), term);
            assert!(periods.browser_enabled_for(provider));
            assert_eq!(periods.browser_accounts(provider), [account]);
            assert!(
                !store_chrome_billing(
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
    fn incomplete_billing_responses_keep_only_the_previously_confirmed_date() {
        for provider in ["claude", "codex"] {
            let temporary = tempfile::tempdir().unwrap();
            let periods = uc_providers::BillingPeriods::new(temporary.path());
            let account = "00000000-0000-4000-8000-000000000001";
            let other = "00000000-0000-4000-8000-000000000002";
            let now = Utc::now();
            let end = now + chrono::Duration::days(24);
            store_chrome_billing(
                &periods,
                provider,
                vec![paid_read(provider, account, end)],
                now,
            )
            .unwrap();
            let before = periods.peek_for(provider, account, now);
            for status in [401, 403, 404, 429, 500] {
                let mut read = paid_read(provider, account, end);
                read.organizations[0].status = status;
                read.organizations[0].details = None;
                assert!(!store_chrome_billing(&periods, provider, vec![read], now).unwrap());
                assert_eq!(periods.peek_for(provider, account, now), before);
            }
            for details in [
                serde_json::json!({"status":"active"}),
                serde_json::json!({"next_charge_at":end.to_rfc3339(),"expires_at":end.to_rfc3339()}),
                serde_json::json!({"status":"active","next_charge_at":"invalid","expires_at":"invalid"}),
            ] {
                let mut read = paid_read(provider, account, end);
                read.organizations[0].details = Some(details);
                assert!(!store_chrome_billing(&periods, provider, vec![read], now).unwrap());
                assert_eq!(periods.peek_for(provider, account, now), before);
            }
            let mut read = paid_read(provider, account, end);
            read.organizations[0].plan_type = String::new();
            assert!(!store_chrome_billing(&periods, provider, vec![read], now).unwrap());
            assert_eq!(periods.peek_for(provider, account, now), before);
            assert_eq!(periods.peek_for(provider, other, now), None);
            assert_eq!(periods.peek_for(provider, account, end), None);
            let mut read = paid_read(provider, account, end);
            read.organizations[0].details = Some(serde_json::json!({"status":"cancelled"}));
            assert!(store_chrome_billing(&periods, provider, vec![read], now).unwrap());
            assert_eq!(periods.peek_for(provider, account, now), None);
        }
    }

    #[test]
    fn authoritative_expiry_clears_older_confirmations_from_every_source() {
        for provider in ["claude", "codex"] {
            let temporary = tempfile::tempdir().unwrap();
            let periods = uc_providers::BillingPeriods::new(temporary.path());
            let account = "00000000-0000-4000-8000-000000000001";
            let now = Utc::now();
            let end = now + chrono::Duration::days(24);
            store_source_billing(
                &periods,
                provider,
                "session:fixture",
                paid_read(provider, account, end),
                now - chrono::Duration::minutes(1),
            )
            .unwrap();
            store_chrome_billing(
                &periods,
                provider,
                vec![paid_read(provider, account, end)],
                now,
            )
            .unwrap();
            assert!(periods.peek_for(provider, account, now).is_some());
            assert!(
                store_chrome_billing(
                    &periods,
                    provider,
                    vec![paid_read(provider, account, now)],
                    now + chrono::Duration::seconds(1),
                )
                .unwrap()
            );
            assert_eq!(periods.peek_for(provider, account, now), None);
        }
    }

    #[test]
    fn automatic_billing_retries_unconfirmed_accounts_with_bounded_backoff() {
        let now = Utc::now();
        let provider = uc_core::Provider::new("claude@fixture", "Claude");
        let mut snapshot =
            uc_core::ProviderSnapshot::make(&provider, Some("Max 20x".into()), Vec::new(), now);
        assert!(!billing_refresh_complete(None, now));
        assert!(!billing_refresh_complete(Some(&snapshot), now));
        snapshot.plan_term = Some(uc_core::PlanTerm::Stated {
            ends_at: now + chrono::Duration::days(7),
            checked_at: None,
        });
        assert!(!billing_refresh_complete(Some(&snapshot), now));
        snapshot.plan_term = Some(uc_core::PlanTerm::Stated {
            ends_at: now + chrono::Duration::days(7),
            checked_at: Some(now - chrono::Duration::days(1)),
        });
        assert!(billing_refresh_complete(Some(&snapshot), now));
        assert!(!billing_refresh_complete(
            Some(&snapshot),
            now + chrono::Duration::days(7)
        ));
        snapshot.plan_term = None;
        snapshot.plan = Some("Free".into());
        assert!(billing_refresh_complete(Some(&snapshot), now));
        for (attempt, seconds) in [
            (0, 30),
            (1, 60),
            (2, 120),
            (3, 240),
            (4, 300),
            (u32::MAX, 300),
        ] {
            assert_eq!(
                billing_refresh_delay(true, attempt),
                Duration::from_secs(seconds)
            );
            assert_eq!(
                billing_refresh_delay(false, attempt),
                Duration::from_secs(300)
            );
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
    fn a_fresh_plan_cannot_claim_billing_for_a_different_browser_account() {
        let now = Utc::now();
        let first = "00000000-0000-4000-8000-000000000001";
        let second = "00000000-0000-4000-8000-000000000002";
        for provider in ["claude", "codex"] {
            let read = paid_read(provider, second, now + chrono::Duration::days(7));
            let mut reads = vec![read];
            assert!(billing_read_confirms(&reads, provider, second, now));
            assert!(!billing_read_confirms(&reads, provider, first, now));
            reads[0].organizations[0].details.as_mut().unwrap()["status"] =
                serde_json::json!("inactive");
            assert!(!billing_read_confirms(&reads, provider, second, now));
            reads[0].organizations[0].details.as_mut().unwrap()["status"] =
                serde_json::json!("active");
            let field = if provider == "codex" {
                "expires_at"
            } else {
                "next_charge_at"
            };
            reads[0].organizations[0].details.as_mut().unwrap()[field] =
                serde_json::json!(now.to_rfc3339());
            assert!(!billing_read_confirms(&reads, provider, second, now));
        }
    }

    #[test]
    fn a_verified_free_account_does_not_need_another_billing_login() {
        let first = "00000000-0000-4000-8000-000000000001";
        let second = "00000000-0000-4000-8000-000000000002";
        for (provider, plan) in [("claude", "claude_free"), ("codex", "free")] {
            let read = serde_json::from_value(serde_json::json!({"state":"ready","organizations":[{
                "uuid":first,"planType":plan,"tier":null,"status":200,"details":{"status":"inactive"}
            }]})).unwrap();
            let mut reads = vec![read];
            assert!(billing_read_without_subscription(&reads, provider, first));
            assert!(!billing_read_without_subscription(&reads, provider, second));
            assert!(!billing_read_confirms(&reads, provider, first, Utc::now()));
            reads[0].organizations[0].status = 401;
            assert!(!billing_read_without_subscription(&reads, provider, first));
        }
    }

    #[tokio::test]
    async fn billing_polls_and_providers_reuse_one_approved_browser_connection() {
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
            let mut reads = 0;
            let mut previous_id = 0;
            while let Some(Ok(Message::Text(text))) = connection.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let id = request["id"].as_u64().unwrap();
                assert!(id > previous_id);
                previous_id = id;
                let result = match request["method"].as_str().unwrap() {
                    "Browser.getVersion" => serde_json::json!({"product":"Chrome/fixture"}),
                    "Target.getBrowserContexts" => {
                        serde_json::json!({"defaultBrowserContextId":"default-profile"})
                    }
                    "Target.getTargets" => serde_json::json!({"targetInfos":[
                        {"type":"page","targetId":"claude","browserContextId":"default-profile","url":"https://claude.ai/new"},
                        {"type":"page","targetId":"codex","browserContextId":"default-profile","url":"https://chatgpt.com/"}
                    ]}),
                    "Target.attachToTarget" => {
                        let expected = if reads % 2 == 0 { "claude" } else { "codex" };
                        assert_eq!(request["params"]["targetId"], expected);
                        serde_json::json!({"sessionId":"attached"})
                    }
                    "Runtime.evaluate" => {
                        serde_json::json!({"result":{"value":{"state":"ready","organizations":[]}}})
                    }
                    "Target.detachFromTarget" => {
                        reads += 1;
                        serde_json::json!({})
                    }
                    method => panic!("Unexpected CDP method: {method}"),
                };
                connection
                    .send(Message::Text(
                        serde_json::json!({"id":id,"result":result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                if reads == 4 {
                    return;
                }
            }
            panic!("The approved browser connection was closed between billing polls");
        });
        let shared = Default::default();
        for (index, provider) in ["claude", "codex", "claude", "codex"]
            .into_iter()
            .enumerate()
        {
            let reads = read_chrome_billing_shared(provider, &socket, &shared)
                .await
                .unwrap();
            assert_eq!(reads[0].state, "ready");
            if index == 0 {
                let current_id = shared.lock().await.as_ref().unwrap().id;
                drop(ChromeTargetGuard {
                    connection: shared.clone(),
                    connection_id: uuid::Uuid::new_v4(),
                    endpoint: socket.clone(),
                    target: None,
                    session: None,
                    attaching_target: Some("stale-session-target".into()),
                    creating_target: true,
                });
                tokio::task::yield_now().await;
                assert_eq!(shared.lock().await.as_ref().unwrap().id, current_id);
            }
        }
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
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
                    "Target.detachFromTarget" => {
                        assert_eq!(request["params"]["targetId"], "temporary");
                        serde_json::json!({})
                    }
                    "Target.closeTarget" => {
                        assert_eq!(request["params"]["targetId"], "temporary");
                        connection
                            .send(Message::Text(
                                serde_json::json!({"id":request["id"],"result":{"success":true}})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap();
                        return;
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
            panic!("The approved browser connection closed before target cleanup");
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
    async fn delayed_attach_reply_detaches_existing_tab_without_losing_browser_approval() {
        use tokio_tungstenite::tungstenite::Message;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = Url::parse(&format!(
            "ws://{}/devtools/browser/fixture",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let shared: SharedChromeConnection = Default::default();
        let (cleaned, cleanup) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut connection = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut cleaned = Some(cleaned);
            let mut detaches = 0;
            while let Some(Ok(Message::Text(text))) = connection.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "Browser.getVersion" => serde_json::json!({"product":"Chrome/fixture"}),
                    "Target.getBrowserContexts" => {
                        serde_json::json!({"defaultBrowserContextId":"default-profile"})
                    }
                    "Target.getTargets" => serde_json::json!({"targetInfos":[
                        {"type":"page","targetId":"existing","browserContextId":"default-profile","url":"https://claude.ai/new"}
                    ]}),
                    "Target.attachToTarget" => {
                        if detaches == 0 {
                            tokio::time::sleep(Duration::from_millis(3500)).await;
                        }
                        serde_json::json!({"sessionId":"attached"})
                    }
                    "Runtime.evaluate" => {
                        serde_json::json!({"result":{"value":{"state":"ready","organizations":[]}}})
                    }
                    "Target.detachFromTarget" => {
                        if detaches == 0 {
                            assert_eq!(request["params"]["targetId"], "existing");
                        } else {
                            assert_eq!(request["params"]["sessionId"], "attached");
                        }
                        detaches += 1;
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
                if detaches == 1
                    && let Some(cleaned) = cleaned.take()
                {
                    cleaned.send(()).unwrap();
                }
                if detaches == 2 {
                    return;
                }
            }
            panic!("The existing user tab's browser connection was closed");
        });
        assert!(
            read_chrome_billing_shared("claude", &socket, &shared)
                .await
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(2), cleanup)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            read_chrome_billing_shared("claude", &socket, &shared)
                .await
                .unwrap()[0]
                .state,
            "ready"
        );
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
                        let expression = request["params"]["expression"].as_str().unwrap();
                        if !expression.contains("delete window[") {
                            assert!(expression.contains("/api/auth/session"));
                            assert!(expression.contains("Authorization: 'Bearer '"));
                        }
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
                        "Runtime.evaluate"
                            if request["params"]["expression"]
                                .as_str()
                                .unwrap()
                                .contains("document.readyState") =>
                        {
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

    struct StoredBillingProvider {
        provider: uc_core::Provider,
        account: String,
        periods: uc_providers::BillingPeriods,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl uc_core::ProviderRuntime for StoredBillingProvider {
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
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let now = Utc::now();
            let mut snapshot = uc_core::ProviderSnapshot::make(
                &self.provider,
                Some("Pro 5x".into()),
                Vec::new(),
                now,
            );
            snapshot.plan_checked_at = Some(now);
            snapshot.plan_term = self.periods.peek_for("codex", &self.account, now);
            snapshot
        }
    }

    #[tokio::test]
    async fn switching_browser_accounts_keeps_each_accounts_confirmed_billing_date() {
        let directory = tempfile::tempdir().unwrap();
        let periods = uc_providers::BillingPeriods::new(directory.path().join("billing"));
        let first = "00000000-0000-4000-8000-000000000001";
        let second = "00000000-0000-4000-8000-000000000002";
        let now = Utc::now();
        store_chrome_billing(
            &periods,
            "codex",
            vec![paid_read("codex", first, now + chrono::Duration::days(7))],
            now,
        )
        .unwrap();
        let providers: Vec<std::sync::Arc<dyn uc_core::ProviderRuntime>> = [first, second]
            .into_iter()
            .map(|account| {
                std::sync::Arc::new(StoredBillingProvider {
                    provider: uc_core::Provider::new(format!("codex@{account}"), "Codex"),
                    account: account.into(),
                    periods: periods.clone(),
                    calls: Default::default(),
                }) as std::sync::Arc<dyn uc_core::ProviderRuntime>
            })
            .collect();
        let config = uc_engine::EngineConfig::default();
        let engine = uc_engine::Engine::new(
            providers,
            uc_engine::SnapshotCache::new(
                directory.path().join("cache.json"),
                config.refresh_interval,
            ),
            config,
        );
        refresh_billing_provider(&engine, "codex").await;
        assert!(
            engine.snapshots()[&format!("codex@{first}")]
                .plan_term
                .is_some()
        );
        assert!(
            engine.snapshots()[&format!("codex@{second}")]
                .plan_term
                .is_none()
        );
        assert!(
            store_chrome_billing(
                &periods,
                "codex",
                vec![paid_read("codex", second, now + chrono::Duration::days(14))],
                now,
            )
            .unwrap()
        );
        refresh_billing_provider(&engine, "codex").await;
        assert!(
            engine.snapshots()[&format!("codex@{first}")]
                .plan_term
                .is_some()
        );
        assert!(
            engine.snapshots()[&format!("codex@{second}")]
                .plan_term
                .is_some()
        );
    }

    #[tokio::test]
    async fn synchronized_billing_only_refreshes_changed_local_accounts() {
        let directory = tempfile::tempdir().unwrap();
        let periods = uc_providers::BillingPeriods::new(directory.path().join("billing"));
        let first = "00000000-0000-4000-8000-000000000001";
        let second = "00000000-0000-4000-8000-000000000002";
        let now = Utc::now();
        for account in [first, second] {
            periods
                .save_from(
                    "codex",
                    account,
                    "Pro 5x",
                    Some(now + chrono::Duration::days(7)),
                    now,
                    "fixture",
                )
                .unwrap();
        }
        let providers: Vec<_> = [("codex", first), ("codex", second), ("other", first)]
            .into_iter()
            .map(|(family, account)| {
                std::sync::Arc::new(StoredBillingProvider {
                    provider: uc_core::Provider::new(format!("{family}@{account}"), family),
                    account: account.into(),
                    periods: periods.clone(),
                    calls: Default::default(),
                })
            })
            .collect();
        let config = uc_engine::EngineConfig::default();
        let engine = uc_engine::Engine::new(
            providers
                .iter()
                .map(|provider| provider.clone() as std::sync::Arc<dyn uc_core::ProviderRuntime>)
                .collect(),
            uc_engine::SnapshotCache::new(
                directory.path().join("cache.json"),
                config.refresh_interval,
            ),
            config,
        );
        let ids = engine.provider_ids();
        for id in &ids {
            engine.refresh(id, true).await;
        }
        for account in [first, second] {
            periods.invalidate_for("codex", account).unwrap();
        }

        refresh_synced_billing_accounts(&engine, &[]).await;
        for provider in &providers {
            assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert!(
                engine.snapshots()[&provider.provider.id]
                    .plan_term
                    .is_some()
            );
        }

        let changed = format!("codex@{first}");
        refresh_synced_billing_accounts(
            &engine,
            &[
                changed.clone(),
                changed.clone(),
                format!("other@{first}"),
                "codex@unknown".into(),
                "codex@".into(),
            ],
        )
        .await;
        assert!(engine.snapshots()[&changed].plan_term.is_none());
        assert_eq!(
            providers[0].calls.load(std::sync::atomic::Ordering::SeqCst),
            2
        );
        for provider in &providers[1..] {
            assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert!(
                engine.snapshots()[&provider.provider.id]
                    .plan_term
                    .is_some()
            );
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
