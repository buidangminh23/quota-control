use std::sync::Arc;

use tauri::{AppHandle, State};
use uc_accounts::{AccountRecord, AccountStore};
use uc_core::ProviderRuntime;
use uc_providers::ProviderKind;
use uc_providers::oauth::{LoginStart, OAuthManager};

use crate::service::{BackendService, safe_error};

pub struct Accounts {
    pub store: Arc<AccountStore>,
    oauth: OAuthManager,
    changes: tokio::sync::Mutex<()>,
}

impl Accounts {
    pub fn new(store: Arc<AccountStore>) -> Self {
        Self {
            oauth: OAuthManager::new(store.clone()),
            store,
            changes: tokio::sync::Mutex::new(()),
        }
    }

    pub fn runtimes(&self) -> Result<Vec<Arc<dyn ProviderRuntime>>, String> {
        uc_api::provider_runtimes(self.store.clone()).map_err(safe_error)
    }
}

fn kind(provider: &str) -> Result<ProviderKind, String> {
    ProviderKind::parse(provider).ok_or_else(|| "Unsupported account provider".into())
}

fn account_label(label: Option<String>, provider: ProviderKind) -> Result<String, String> {
    let label = label.unwrap_or_else(|| provider.cli().into());
    let label = label.trim();
    if label.is_empty() || label.len() > 120 || label.chars().any(char::is_control) {
        return Err("Account label must contain 1 to 120 characters".into());
    }
    Ok(label.into())
}

#[tauri::command]
pub async fn list_accounts(accounts: State<'_, Accounts>) -> Result<Vec<AccountRecord>, String> {
    accounts.store.list().map_err(safe_error)
}

#[tauri::command]
pub async fn import_current_account(
    app: AppHandle,
    accounts: State<'_, Accounts>,
    service: State<'_, BackendService>,
    provider: String,
    label: Option<String>,
) -> Result<AccountRecord, String> {
    let kind = kind(&provider)?;
    let label = account_label(label, kind)?;
    let _changes = accounts.changes.lock().await;
    let record = uc_providers::import_current_account(accounts.store.clone(), kind, label)
        .await
        .map_err(safe_error)?;
    service.replace_runtimes(accounts.runtimes()?, &app)?;
    Ok(record)
}

#[tauri::command]
pub async fn begin_account_login(
    accounts: State<'_, Accounts>,
    provider: String,
    label: Option<String>,
) -> Result<LoginStart, String> {
    let kind = kind(&provider)?;
    accounts
        .oauth
        .begin_login(kind, account_label(label, kind)?)
        .await
        .map_err(safe_error)
}

#[tauri::command]
pub async fn complete_account_login(
    app: AppHandle,
    accounts: State<'_, Accounts>,
    service: State<'_, BackendService>,
    flow_id: String,
    callback: Option<String>,
) -> Result<AccountRecord, String> {
    let record = accounts
        .oauth
        .complete_login(&flow_id, callback)
        .await
        .map_err(safe_error)?;
    let _changes = accounts.changes.lock().await;
    service.replace_runtimes(accounts.runtimes()?, &app)?;
    Ok(record)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_commands_only_accept_supported_providers_and_labels() {
        assert!(kind("codex").is_ok());
        assert!(kind("../accounts").is_err());
        assert!(account_label(Some("".into()), ProviderKind::Claude).is_err());
        assert!(account_label(Some("work\nsecret".into()), ProviderKind::Claude).is_err());
    }
}
