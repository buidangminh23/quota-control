use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use uc_accounts::{AccountStore, CredentialMode};
use uc_core::ProviderRuntime;
use uc_providers::oauth::{LoginLanguage, OAuthManager, is_cancelled, is_expired};
use uc_providers::{CliAccount, ProviderKind, VisibleAccount};

use crate::browser::{LoginBrowser, open_login_page};
use crate::service::{BackendService, safe_error};

pub struct Accounts {
    pub store: Arc<AccountStore>,
    oauth: OAuthManager,
    changes: tokio::sync::Mutex<()>,
    /// The CLI logins the current cards were built from.
    cli: parking_lot::Mutex<Vec<CliAccount>>,
}

impl Accounts {
    pub fn new(store: Arc<AccountStore>) -> Self {
        Self {
            oauth: OAuthManager::new(store.clone()),
            store,
            changes: tokio::sync::Mutex::new(()),
            cli: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Every card's runtime, with the CLI logins read afresh.
    pub fn runtimes(&self) -> Result<Vec<Arc<dyn ProviderRuntime>>, String> {
        let cli = uc_providers::cli_accounts();
        let runtimes =
            uc_api::provider_runtimes_with(self.store.clone(), &cli).map_err(safe_error)?;
        *self.cli.lock() = cli;
        Ok(runtimes)
    }

    /// Cards of CLI logins that no stored account covers. Before CLI logins became cards, none of
    /// them existed, so settings written by an older version cannot have hidden them.
    pub fn cli_only_ids(&self) -> Vec<String> {
        let records = self.store.list().unwrap_or_default();
        self.cli
            .lock()
            .iter()
            .filter(|login| !records.iter().any(|record| record.id == login.id))
            .map(|login| login.id.clone())
            .collect()
    }

    fn entries(&self) -> Result<Vec<AccountEntry>, String> {
        let records = self.store.list().map_err(safe_error)?;
        let cli = self.cli.lock().clone();
        Ok(uc_providers::visible_accounts(&records, &cli)
            .into_iter()
            .map(AccountEntry::from)
            .collect())
    }

    fn cli_ids(&self) -> Vec<String> {
        self.cli
            .lock()
            .iter()
            .map(|login| login.id.clone())
            .collect()
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

/// Rebuild the cards when a CLI signed in, out, or into another account since they were built.
pub async fn sync_cli_logins(app: &AppHandle) {
    let accounts = app.state::<Accounts>();
    let detected = tauri::async_runtime::spawn_blocking(|| {
        uc_providers::cli_accounts()
            .into_iter()
            .map(|login| login.id)
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    if detected == accounts.cli_ids() {
        return;
    }
    let _changes = accounts.changes.lock().await;
    if let Err(error) = accounts.runtimes().and_then(|runtimes| {
        app.state::<BackendService>()
            .replace_runtimes(runtimes, app)
    }) {
        tracing::warn!("CLI logins changed, but the cards were not rebuilt: {error}");
    }
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
    fn cli_logins_are_listed_as_automatic_cards() {
        let login = CliAccount {
            kind: ProviderKind::Codex,
            id: "codex@aa".into(),
            updated_at: Utc::now(),
            path: "auth.json".into(),
            profile: None,
        };
        let entry = serde_json::to_value(AccountEntry::from(VisibleAccount::Cli(&login))).unwrap();
        assert_eq!(entry["credentialMode"], "cli");
        assert_eq!(entry["provider"], "codex");
        assert_eq!(entry["label"], "codex");
    }
}
