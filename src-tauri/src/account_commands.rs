use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use uc_accounts::{AccountStore, CredentialMode, KeyRecord, KeyStore};
use uc_core::ProviderRuntime;
use uc_providers::oauth::{LoginLanguage, OAuthManager, is_cancelled, is_expired};
use uc_providers::{CliAccount, ProviderKind, VisibleAccount};
use uc_services::{Detected, KeyFormat, Roots, ServiceInfo};

use crate::browser::{LoginBrowser, open_login_page};
use crate::service::{BackendService, safe_error};

pub struct Accounts {
    pub store: Arc<AccountStore>,
    oauth: OAuthManager,
    label_backfill: uc_providers::accounts::AccountLabelBackfill,
    changes: tokio::sync::Mutex<()>,
    /// The CLI logins the current cards were built from.
    cli: parking_lot::Mutex<Vec<CliAccount>>,
    /// The other services' logins and keys the current cards were built from.
    services: parking_lot::Mutex<uc_api::ServiceCards>,
}

impl Accounts {
    /// Reads the services' logins and keys once, so the first cards include them.
    pub fn new(store: Arc<AccountStore>, keys: KeyStore) -> Self {
        Self {
            oauth: OAuthManager::new(store.clone()),
            label_backfill: uc_providers::accounts::AccountLabelBackfill::new(store.clone()),
            store,
            changes: tokio::sync::Mutex::new(()),
            cli: parking_lot::Mutex::new(Vec::new()),
            services: parking_lot::Mutex::new(uc_api::ServiceCards::scan(keys, Roots::system())),
        }
    }

    /// Every card's runtime, with the CLI logins read afresh. A login the keychain refuses to read
    /// keeps its card, which then shows the keychain error.
    pub fn runtimes(&self) -> Result<Vec<Arc<dyn ProviderRuntime>>, String> {
        let previous = self.cli.lock().clone();
        let cli = uc_providers::cli_accounts_keeping(&previous);
        let services = self.services.lock().clone();
        let runtimes = uc_api::provider_runtimes_with(self.store.clone(), &cli, &services)
            .map_err(safe_error)?;
        *self.cli.lock() = cli;
        Ok(runtimes)
    }

    /// Cards that settings written by an older version cannot have hidden, because those cards did
    /// not exist yet: CLI logins no stored account covers, and the other services' cards.
    pub fn new_card_ids(&self) -> Vec<String> {
        let records = self.store.list().unwrap_or_default();
        let mut ids: Vec<String> = self
            .cli
            .lock()
            .iter()
            .filter(|login| !records.iter().any(|record| record.id == login.id))
            .map(|login| login.id.clone())
            .collect();
        let services = self.services.lock();
        ids.extend(services.detected.iter().map(|found| found.id.clone()));
        ids.extend(services.saved.iter().map(|record| record.id.clone()));
        ids
    }

    fn keys(&self) -> KeyStore {
        self.services.lock().store.clone()
    }

    fn entries(&self) -> Result<Vec<AccountEntry>, String> {
        let records = self.store.list().map_err(safe_error)?;
        let cli = self.cli.lock().clone();
        Ok(uc_providers::visible_accounts(&records, &cli)
            .into_iter()
            .map(AccountEntry::from)
            .collect())
    }

    /// Each CLI card's account and where its login is read from: a card is rebuilt when either
    /// changes, so a login that moved from the file into the keychain is followed.
    fn cli_bindings(&self) -> Vec<(String, uc_providers::CliLocation)> {
        bindings(&self.cli.lock())
    }
}

/// An account card as the Accounts screen lists it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountEntry {
    id: String,
    provider: String,
    label: String,
    connected_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    credential_mode: &'static str,
}

impl From<VisibleAccount<'_>> for AccountEntry {
    fn from(account: VisibleAccount<'_>) -> Self {
        match account {
            VisibleAccount::Stored(record) => Self {
                id: record.id.clone(),
                provider: record.provider.clone(),
                label: record.label.clone(),
                connected_at: record.connected_at,
                updated_at: record.updated_at,
                credential_mode: match record.credential_mode {
                    CredentialMode::SharedCli => "shared_cli",
                    CredentialMode::ManagedOauth => "managed_oauth",
                },
            },
            VisibleAccount::Cli(login) => Self {
                id: login.id.clone(),
                provider: login.kind.cli().into(),
                label: login.kind.cli().into(),
                connected_at: login.updated_at,
                updated_at: login.updated_at,
                credential_mode: "cli",
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginOpened {
    flow_id: String,
    authorization_url: String,
    expires_in_seconds: u64,
    browser: LoginBrowser,
}

/// How a browser sign-in ended, sent to the popup as `account-login`. A connected or failed login
/// brings the popup back; a cancelled or abandoned (`expired`) one does not.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginOutcome {
    flow_id: String,
    provider: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn kind(provider: &str) -> Result<ProviderKind, String> {
    ProviderKind::parse(provider).ok_or_else(|| "Unsupported account provider".into())
}

#[tauri::command]
pub async fn list_accounts(accounts: State<'_, Accounts>) -> Result<Vec<AccountEntry>, String> {
    accounts.entries()
}

/// A service as the Accounts screen lists it, with the cards it has.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceEntry {
    #[serde(flatten)]
    info: ServiceInfo,
    /// Logins other apps keep on this computer, and keys in environment variables.
    detected: Vec<Detected>,
    /// Keys saved in Quota Control; the keys themselves never leave the store.
    keys: Vec<KeyRecord>,
}

#[tauri::command]
pub async fn list_services(accounts: State<'_, Accounts>) -> Result<Vec<ServiceEntry>, String> {
    let cards = accounts.services.lock().clone();
    Ok(service_entries(&cards))
}

fn service_entries(cards: &uc_api::ServiceCards) -> Vec<ServiceEntry> {
    uc_services::service_infos()
        .into_iter()
        .map(|info| ServiceEntry {
            detected: cards
                .detected
                .iter()
                .filter(|found| found.service == info.id)
                .cloned()
                .collect(),
            keys: cards
                .saved
                .iter()
                .filter(|record| record.service == info.id)
                .cloned()
                .collect(),
            info,
        })
        .collect()
}

const MAX_FIELD_LENGTH: usize = 512;
const MAX_LABEL_LENGTH: usize = 256;

/** A pasted key as the service takes it, or what to paste instead. */
fn api_key(raw: &str, format: KeyFormat) -> Result<String, String> {
    format.normalize(raw).ok_or_else(|| {
        match format {
            KeyFormat::Token => "Paste the whole API key, without spaces or line breaks",
            KeyFormat::Cookie => "Paste only the cookie's value, without spaces or line breaks",
            KeyFormat::CookieHeader => {
                "Paste the whole Cookie header: name=value pairs separated by semicolons"
            }
        }
        .into()
    })
}

/** The card's label: what the user typed, or the service's name. */
fn key_label(raw: Option<&str>, service_name: &str) -> Result<String, String> {
    let label = raw.map(str::trim).unwrap_or_default();
    if label.len() > MAX_LABEL_LENGTH || label.chars().any(char::is_control) {
        return Err("The label is too long or has control characters".into());
    }
    Ok(if label.is_empty() {
        service_name.to_string()
    } else {
        label.to_string()
    })
}

/// The extra values a service asks for beside its key (an xAI team id): only the fields it
/// declares, trimmed, blanks left out.
fn key_fields(
    declared: &[(&'static str, &'static str)],
    fields: Option<serde_json::Map<String, Value>>,
) -> Result<Value, String> {
    let mut extra = serde_json::Map::new();
    for (name, value) in fields.unwrap_or_default() {
        if !declared.iter().any(|(field, _)| *field == name) {
            return Err("This service does not ask for that value".into());
        }
        let text = match &value {
            Value::String(text) => text.trim(),
            Value::Null => "",
            _ => return Err("Values beside the key must be text".into()),
        };
        if text.len() > MAX_FIELD_LENGTH || text.chars().any(char::is_control) {
            return Err("A value beside the key is too long or has control characters".into());
        }
        if !text.is_empty() {
            extra.insert(name, Value::String(text.into()));
        }
    }
    Ok(Value::Object(extra))
}

/// Save an API key for a service and add its card.
#[tauri::command]
pub async fn add_api_key(
    app: AppHandle,
    accounts: State<'_, Accounts>,
    service: State<'_, BackendService>,
    service_id: String,
    label: Option<String>,
    key: String,
    fields: Option<serde_json::Map<String, Value>>,
) -> Result<KeyRecord, String> {
    let known = uc_services::service(&service_id).ok_or("Unsupported service")?;
    let help = known
        .connection()
        .api_key
        .ok_or("This service connects through its app's login, not an API key")?;
    let extra = key_fields(help.fields, fields)?;
    let key = api_key(&key, known.key_format())?;
    let label = key_label(label.as_deref(), known.name())?;
    let _changes = accounts.changes.lock().await;
    let keys = accounts.keys();
    let record =
        tauri::async_runtime::spawn_blocking(move || keys.add(&service_id, &label, &key, &extra))
            .await
            .map_err(safe_error)?
            .map_err(safe_error)?;
    rescan_services(&accounts).await;
    service.replace_runtimes(accounts.runtimes()?, &app)?;
    Ok(record)
}

/// Forget a saved API key and remove its card.
#[tauri::command]
pub async fn remove_api_key(
    app: AppHandle,
    accounts: State<'_, Accounts>,
    service: State<'_, BackendService>,
    key_id: String,
) -> Result<(), String> {
    let _changes = accounts.changes.lock().await;
    let keys = accounts.keys();
    tauri::async_runtime::spawn_blocking(move || keys.remove(&key_id))
        .await
        .map_err(safe_error)?
        .map_err(safe_error)?;
    rescan_services(&accounts).await;
    service.replace_runtimes(accounts.runtimes()?, &app)
}

/// Read the services' logins and keys again; true when the cards they give changed.
async fn rescan_services(accounts: &Accounts) -> bool {
    let keys = accounts.keys();
    let Ok(scanned) = tauri::async_runtime::spawn_blocking(move || {
        uc_api::ServiceCards::scan(keys, Roots::system())
    })
    .await
    else {
        return false;
    };
    let mut current = accounts.services.lock();
    let changed = scanned.fingerprint() != current.fingerprint();
    *current = scanned;
    changed
}

/// Open the provider's sign-in page and finish the login in the background: it keeps going while
/// the popup is hidden behind the browser, and ends by showing the popup again.
#[tauri::command]
pub async fn begin_account_login(
    app: AppHandle,
    accounts: State<'_, Accounts>,
    provider: String,
    language: Option<String>,
) -> Result<LoginOpened, String> {
    let kind = kind(&provider)?;
    let language = LoginLanguage::parse(language.as_deref().unwrap_or_default());
    let start = accounts
        .oauth
        .begin_login_in(kind, kind.cli().into(), language)
        .await
        .map_err(safe_error)?;
    let browser = match open_login_page(&app, &start.authorization_url) {
        Ok(browser) => browser,
        Err(error) => {
            let _ = accounts.oauth.cancel_login(&start.flow_id).await;
            return Err(error);
        }
    };
    tauri::async_runtime::spawn(finish_login(app.clone(), start.flow_id.clone(), kind));
    Ok(LoginOpened {
        flow_id: start.flow_id,
        authorization_url: start.authorization_url,
        expires_in_seconds: start.expires_in_seconds,
        browser,
    })
}

async fn finish_login(app: AppHandle, flow_id: String, kind: ProviderKind) {
    let accounts = app.state::<Accounts>();
    let outcome = match accounts.oauth.complete_login(&flow_id).await {
        Ok(record) => {
            let _changes = accounts.changes.lock().await;
            let rebuilt = accounts.runtimes().and_then(|runtimes| {
                app.state::<BackendService>()
                    .replace_runtimes(runtimes, &app)
            });
            if let Err(error) = rebuilt {
                tracing::warn!("Account connected, but the cards were not rebuilt: {error}");
            }
            LoginOutcome {
                flow_id,
                provider: kind.cli(),
                status: "connected",
                account_id: Some(record.id),
                error: None,
            }
        }
        Err(error) if is_cancelled(&error) => LoginOutcome {
            flow_id,
            provider: kind.cli(),
            status: "cancelled",
            account_id: None,
            error: None,
        },
        Err(error) if is_expired(&error) => LoginOutcome {
            flow_id,
            provider: kind.cli(),
            status: "expired",
            account_id: None,
            error: Some(error.message),
        },
        Err(error) => LoginOutcome {
            flow_id,
            provider: kind.cli(),
            status: "failed",
            account_id: None,
            error: Some(safe_error(error.message)),
        },
    };
    if matches!(outcome.status, "connected" | "failed")
        && let Err(error) = crate::show_popup(&app)
    {
        tracing::warn!("{error}");
    }
    if app.emit_to("popup", "account-login", outcome).is_err() {
        tracing::warn!("Could not report how the sign-in ended");
    }
}

/// Show the sign-in page of a login that is still waiting, in the same browser as before.
#[tauri::command]
pub async fn reopen_account_login(
    app: AppHandle,
    accounts: State<'_, Accounts>,
    flow_id: String,
) -> Result<LoginBrowser, String> {
    let url = accounts
        .oauth
        .authorization_url(&flow_id)
        .await
        .ok_or("This login is no longer active. Start again.")?;
    open_login_page(&app, &url)
}

#[tauri::command]
pub async fn cancel_account_login(
    accounts: State<'_, Accounts>,
    flow_id: String,
) -> Result<(), String> {
    accounts
        .oauth
        .cancel_login(&flow_id)
        .await
        .map_err(safe_error)
}

#[tauri::command]
pub async fn remove_account(
    app: AppHandle,
    accounts: State<'_, Accounts>,
    service: State<'_, BackendService>,
    account_id: String,
) -> Result<(), String> {
    let _changes = accounts.changes.lock().await;
    accounts.store.remove(&account_id).map_err(safe_error)?;
    service.replace_runtimes(accounts.runtimes()?, &app)
}

fn bindings(logins: &[CliAccount]) -> Vec<(String, uc_providers::CliLocation)> {
    logins
        .iter()
        .map(|login| (login.id.clone(), login.location.clone()))
        .collect()
}

/// Rebuild the cards when a CLI signed in, out, into another account, or moved its login since
/// they were built, or when another service's login or key came or went.
pub async fn sync_logins(app: &AppHandle) {
    let accounts = app.state::<Accounts>();
    let previous = accounts.cli.lock().clone();
    let Ok(detected) = tauri::async_runtime::spawn_blocking(move || {
        bindings(&uc_providers::cli_accounts_keeping(&previous))
    })
    .await
    else {
        return;
    };
    let cli_changed = detected != accounts.cli_bindings();
    let _changes = accounts.changes.lock().await;
    let services_changed = rescan_services(&accounts).await;
    if !cli_changed && !services_changed {
        return;
    }
    if let Err(error) = accounts.runtimes().and_then(|runtimes| {
        app.state::<BackendService>()
            .replace_runtimes(runtimes, app)
    }) {
        tracing::warn!("Logins changed, but the cards were not rebuilt: {error}");
    }
}

pub async fn backfill_account_labels(app: &AppHandle) {
    let accounts = app.state::<Accounts>();
    if !matches!(accounts.label_backfill.run().await, Ok(true)) {
        return;
    }
    let _changes = accounts.changes.lock().await;
    let _ = accounts.runtimes().and_then(|runtimes| {
        app.state::<BackendService>()
            .replace_runtimes(runtimes, app)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_commands_only_accept_supported_providers() {
        assert!(kind("codex").is_ok());
        assert!(kind("claude").is_ok());
        assert!(kind("../accounts").is_err());
    }

    #[test]
    fn a_pasted_key_is_trimmed_and_a_broken_one_refused() {
        assert_eq!(
            api_key("  sk-abc123\n", KeyFormat::Token).unwrap(),
            "sk-abc123"
        );
        assert!(api_key("", KeyFormat::Token).is_err());
        assert!(api_key("sk-abc 123", KeyFormat::Token).is_err());
        assert!(api_key("sk-abc\n123", KeyFormat::Token).is_err());
        assert_eq!(
            api_key("Cookie: session=a1; theme=dark", KeyFormat::CookieHeader).unwrap(),
            "session=a1; theme=dark"
        );
        assert_eq!(
            api_key("sk-abc", KeyFormat::CookieHeader).unwrap_err(),
            "Paste the whole Cookie header: name=value pairs separated by semicolons"
        );
        assert_eq!(key_label(Some("  Work "), "OpenRouter").unwrap(), "Work");
        assert_eq!(key_label(Some(" "), "OpenRouter").unwrap(), "OpenRouter");
        assert_eq!(key_label(None, "OpenRouter").unwrap(), "OpenRouter");
        assert!(key_label(Some("a\u{7}b"), "OpenRouter").is_err());
    }

    #[test]
    fn only_the_values_a_service_declares_are_kept_beside_its_key() {
        let declared = [("teamId", "Team ID")];
        let fields = |value: Value| value.as_object().cloned();
        assert_eq!(
            key_fields(
                &declared,
                fields(serde_json::json!({"teamId": "  team-1 "}))
            )
            .unwrap(),
            serde_json::json!({"teamId": "team-1"})
        );
        assert_eq!(
            key_fields(&declared, fields(serde_json::json!({"teamId": ""}))).unwrap(),
            serde_json::json!({})
        );
        assert!(key_fields(&declared, fields(serde_json::json!({"apiKey": "x"}))).is_err());
        assert!(key_fields(&declared, fields(serde_json::json!({"teamId": 3}))).is_err());
        assert!(key_fields(&declared, fields(serde_json::json!({"teamId": "a\nb"}))).is_err());
        assert_eq!(key_fields(&declared, None).unwrap(), serde_json::json!({}));
    }

    #[test]
    fn cli_logins_are_listed_as_automatic_cards() {
        let login = CliAccount {
            kind: ProviderKind::Codex,
            id: "codex@aa".into(),
            email: None,
            updated_at: Utc::now(),
            location: uc_providers::CliLocation::File("auth.json".into()),
            profile: None,
        };
        let entry = serde_json::to_value(AccountEntry::from(VisibleAccount::Cli(&login))).unwrap();
        assert_eq!(entry["credentialMode"], "cli");
        assert_eq!(entry["provider"], "codex");
        assert_eq!(entry["label"], "codex");
    }
}
