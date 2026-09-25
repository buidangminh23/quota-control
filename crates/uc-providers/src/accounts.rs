use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_accounts::{AccountRecord, AccountStore, CredentialMode};
use uc_core::{
    ErrorCategory, HttpRequest, ProviderRuntime, ReqwestHttpClient, SharedHttpClient,
    SimpleProviderError,
};

use crate::credentials::{
    CredentialSource, CredentialStore, Credentials, jwt_claims, parse_credentials, read_json_file,
};
use crate::{LocalProvider, ProviderKind};

pub(crate) const CODEX_CLIENT: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(crate) const CLAUDE_CLIENT: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub(crate) const CLAUDE_SCOPE: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

pub(crate) fn auth_error(message: &str) -> SimpleProviderError {
    SimpleProviderError::new(ErrorCategory::AuthInvalid, message)
}

pub(crate) fn account_error() -> SimpleProviderError {
    SimpleProviderError::new(
        ErrorCategory::CredentialAccess,
        "Cannot access the saved account. Try connecting it again.",
    )
}

fn required(value: &Value) -> Result<&str, SimpleProviderError> {
    value.as_str().map(str::trim).filter(|value| !value.is_empty()).ok_or_else(|| auth_error("A stable account identity is unavailable. Connect this account through the browser."))
}

pub(crate) fn identity(
    kind: ProviderKind,
    document: &Value,
) -> Result<String, SimpleProviderError> {
    match kind {
        ProviderKind::Codex => {
            let claims: Vec<Value> = ["id_token", "access_token"]
                .into_iter()
                .filter_map(|field| document["tokens"][field].as_str().and_then(jwt_claims))
                .collect();
            let stored_account = document["tokens"]["account_id"]
                .as_str()
                .filter(|value| !value.is_empty());
            let claimed_account = claims.iter().find_map(|claim| {
                claim["https://api.openai.com/auth"]["chatgpt_account_id"].as_str()
            });
            let account = stored_account.or(claimed_account).ok_or_else(|| auth_error("The ChatGPT workspace identity is unavailable. Connect this account through the browser."))?;
            let user = claims.iter().find_map(|claim| {
                let auth = &claim["https://api.openai.com/auth"];
                auth["chatgpt_user_id"].as_str().or_else(|| auth["user_id"].as_str()).filter(|value| !value.is_empty())
            }).ok_or_else(|| auth_error("The ChatGPT user identity is unavailable. Connect this account through the browser."))?;
            Ok(format!("{account}|{user}"))
        }
        ProviderKind::Claude => {
            let account = required(&document["oauthAccount"]["accountUuid"])?;
            let organization = required(&document["oauthAccount"]["organizationUuid"])?;
            Ok(format!(
                "{}|{}",
                account.to_lowercase(),
                organization.to_lowercase()
            ))
        }
    }
}

pub async fn import_current_account(
    store: Arc<AccountStore>,
    kind: ProviderKind,
    label: String,
) -> Result<AccountRecord, SimpleProviderError> {
    let credentials = CredentialStore::discover(kind);
    let profile = if kind == ProviderKind::Claude {
        Some(
            uc_core::paths::env_path("CLAUDE_CONFIG_DIR")
                .map(|root| root.join(".claude.json"))
                .unwrap_or_else(|| uc_core::paths::home_dir().join(".claude.json")),
        )
    } else {
        None
    };
    import_account_from_file(
        store,
        kind,
        label,
        credentials.path().ok_or_else(account_error)?.to_path_buf(),
        profile,
    )
    .await
}

pub async fn import_account_from_file(
    store: Arc<AccountStore>,
    kind: ProviderKind,
    label: String,
    path: std::path::PathBuf,
    profile_path: Option<std::path::PathBuf>,
) -> Result<AccountRecord, SimpleProviderError> {
    let (document, key) = uc_core::load_blocking(move || {
        let mut document = read_json_file(&path, kind)?;
        parse_credentials(kind, &document)?;
        if kind == ProviderKind::Claude {
            let profile = read_json_file(
                profile_path.as_deref().ok_or_else(|| {
                    auth_error(
                        "Claude account metadata is unavailable. Connect through the browser.",
                    )
                })?,
                kind,
            )?;
            document["oauthAccount"] = profile["oauthAccount"].clone();
        }
        let key = identity(kind, &document)?;
        Ok::<_, SimpleProviderError>((document, key))
    })
    .await?;
    let lock = refresh_lock(&account_id(kind, &key));
    let _guard = lock.lock().await;
    uc_core::load_blocking(move || {
        store
            .import(
                kind.cli(),
                &label,
                &key,
                &document,
                CredentialMode::SharedCli,
            )
            .map_err(|_| account_error())
    })
    .await
}

pub(crate) fn account_id(kind: ProviderKind, identity: &str) -> String {
    let hash: String = Sha256::digest(identity.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{}@{hash}", kind.cli())
}

pub fn managed_runtimes(
    store: Arc<AccountStore>,
) -> Result<Vec<Arc<dyn ProviderRuntime>>, SimpleProviderError> {
    runtimes_with_client(store, ReqwestHttpClient::shared())
}

pub fn runtimes_with_client(
    store: Arc<AccountStore>,
    http: SharedHttpClient,
) -> Result<Vec<Arc<dyn ProviderRuntime>>, SimpleProviderError> {
    store
        .list()
        .map_err(|_| account_error())?
        .into_iter()
        .map(|record| {
            let kind = ProviderKind::parse(&record.provider)
                .ok_or_else(|| auth_error("Unsupported account provider."))?;
            let credentials = CredentialStore::for_account(kind, store.clone(), record.clone());
            Ok(
                Arc::new(LocalProvider::new(kind, credentials, http.clone()).with_account(&record))
                    as Arc<dyn ProviderRuntime>,
            )
        })
        .collect()
}

pub(crate) fn refresh_lock(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    locks.entry(id.to_owned()).or_default().clone()
}

pub(crate) fn refresh_url(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Claude => "https://platform.claude.com/v1/oauth/token",
        ProviderKind::Codex => "https://auth.openai.com/oauth/token",
    }
}

pub(crate) async fn ready_credentials(
    source: &CredentialStore,
    kind: ProviderKind,
    http: &SharedHttpClient,
    now: DateTime<Utc>,
    endpoint: &str,
    rejected_token: Option<&str>,
) -> Result<Credentials, SimpleProviderError> {
    if !matches!(&source.source, CredentialSource::Account { record, .. } if record.credential_mode == CredentialMode::ManagedOauth)
    {
        return source.load().await;
    }
    let source = source.clone();
    let http = http.clone();
    let endpoint = endpoint.to_owned();
    let rejected_token = rejected_token.map(str::to_owned);
    tokio::spawn(async move {
        ready_credentials_inner(
            &source,
            kind,
            &http,
            now,
            &endpoint,
            rejected_token.as_deref(),
        )
        .await
    })
    .await
    .map_err(|_| account_error())?
}

async fn ready_credentials_inner(
    source: &CredentialStore,
    kind: ProviderKind,
    http: &SharedHttpClient,
    now: DateTime<Utc>,
    endpoint: &str,
    rejected_token: Option<&str>,
) -> Result<Credentials, SimpleProviderError> {
    let CredentialSource::Account { store, record } = &source.source else {
        return source.load().await;
    };
    if record.credential_mode != CredentialMode::ManagedOauth {
        return source.load().await;
    }
    let lock = refresh_lock(&record.id);
    let _guard = lock.lock().await;
    let source_copy = source.clone();
    let mut document = uc_core::load_blocking(move || source_copy.read_document()).await?;
    let credentials = parse_credentials(kind, &document)?;
    let rejected = rejected_token.is_some_and(|token| token == credentials.access_token);
    if !rejected
        && !credentials
            .expires_at
            .is_some_and(|expiry| expiry <= now + chrono::Duration::seconds(60))
    {
        return Ok(credentials);
    }
    let refresh = match kind {
        ProviderKind::Claude => &document["claudeAiOauth"]["refreshToken"],
        ProviderKind::Codex => &document["tokens"]["refresh_token"],
    };
    let refresh = refresh
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SimpleProviderError::new(
                ErrorCategory::AuthExpired,
                "This account needs to be connected again.",
            )
        })?;
    let request = match kind {
        ProviderKind::Claude => HttpRequest::post(endpoint).json_body(&json!({"grant_type":"refresh_token","refresh_token":refresh,"client_id":CLAUDE_CLIENT,"scope":CLAUDE_SCOPE})),
        ProviderKind::Codex => {
            let body = url::form_urlencoded::Serializer::new(String::new()).append_pair("grant_type", "refresh_token").append_pair("refresh_token", refresh).append_pair("client_id", CODEX_CLIENT).finish();
            HttpRequest::post(endpoint).header("Content-Type", "application/x-www-form-urlencoded").body(body.into_bytes())
        }
    };
    let response = http.send(request).await.map_err(|_| {
        SimpleProviderError::new(
            ErrorCategory::Network,
            "Cannot renew this account session. Try again later.",
        )
    })?;
    if !response.is_success() {
        let category = if matches!(response.status, 400 | 401 | 403) {
            ErrorCategory::AuthExpired
        } else {
            ErrorCategory::http(response.status)
        };
        return Err(SimpleProviderError::new(
            category,
            "This account session could not be renewed. Reconnect it if the problem persists.",
        ));
    }
    let refreshed: Value = response
        .json()
        .map_err(|_| auth_error("The login service returned an invalid token response."))?;
    let old_identity = identity(kind, &document)?;
    apply_tokens(kind, &mut document, &refreshed, now)?;
    if identity(kind, &document)? != old_identity {
        return Err(auth_error(
            "The renewed session belongs to a different account. Reconnect this account.",
        ));
    }
    let parsed = parse_credentials(kind, &document)?;
    let store = store.clone();
    let id = record.id.clone();
    uc_core::load_blocking(move || {
        store
            .update_credentials(&id, &document)
            .map_err(|_| account_error())
    })
    .await?;
    Ok(parsed)
}

pub(crate) fn apply_tokens(
    kind: ProviderKind,
    document: &mut Value,
    tokens: &Value,
    now: DateTime<Utc>,
) -> Result<(), SimpleProviderError> {
    let access = required(&tokens["access_token"])?;
    if access.bytes().any(|byte| byte < 32 || byte == 127) {
        return Err(auth_error("The login service returned an invalid token."));
    }
    let expires = tokens["expires_in"]
        .as_i64()
        .filter(|seconds| *seconds > 0 && *seconds <= 31_536_000)
        .and_then(|seconds| now.checked_add_signed(chrono::Duration::seconds(seconds)))
        .map(|date| date.timestamp_millis());
    match kind {
        ProviderKind::Claude => {
            document["claudeAiOauth"]["accessToken"] = json!(access);
            document["claudeAiOauth"]["expiresAt"] = json!(
                expires
                    .ok_or_else(|| auth_error("The login service omitted the session expiry."))?
            );
            if let Some(refresh) = tokens["refresh_token"]
                .as_str()
                .filter(|value| !value.is_empty())
            {
                document["claudeAiOauth"]["refreshToken"] = json!(refresh);
            }
            if let Some(scope) = tokens["scope"].as_str() {
                document["claudeAiOauth"]["scopes"] =
                    json!(scope.split_whitespace().collect::<Vec<_>>());
            }
            if let Some(account) = tokens["account"]["uuid"].as_str() {
                document["oauthAccount"]["accountUuid"] = json!(account);
            }
            if let Some(org) = tokens["organization"]["uuid"].as_str() {
                document["oauthAccount"]["organizationUuid"] = json!(org);
            }
        }
        ProviderKind::Codex => {
            document["tokens"]["access_token"] = json!(access);
            for key in ["refresh_token", "id_token"] {
                if let Some(value) = tokens[key].as_str().filter(|value| !value.is_empty()) {
                    document["tokens"][key] = json!(value);
                }
            }
            if let Some(id) = document["tokens"]["id_token"]
                .as_str()
                .and_then(jwt_claims)
                .and_then(|claims| {
                    claims["https://api.openai.com/auth"]["chatgpt_account_id"]
                        .as_str()
                        .map(str::to_owned)
                })
            {
                document["tokens"]["account_id"] = json!(id);
            }
            if let Some(expires) = expires {
                document["_usageControl"]["expiresAt"] = json!(expires);
            }
            document["last_refresh"] = json!(now.to_rfc3339());
        }
    }
    Ok(())
}
