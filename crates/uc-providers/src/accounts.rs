use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_accounts::{AccountRecord, AccountStore, CredentialMode};
use uc_core::{
    ErrorCategory, HttpRequest, ProviderRuntime, ReqwestHttpClient, SharedHttpClient,
    SimpleProviderError,
};

use crate::credentials::{
    CliLocation, CredentialSource, CredentialStore, Credentials, jwt_claims, parse_credentials,
    read_json_file, read_location,
};
use crate::{LocalProvider, ProviderKind};

pub struct AccountLabelBackfill {
    store: Arc<AccountStore>,
    http: SharedHttpClient,
    attempted: tokio::sync::Mutex<HashSet<String>>,
}

impl AccountLabelBackfill {
    pub fn new(store: Arc<AccountStore>) -> Self {
        Self {
            store,
            http: ReqwestHttpClient::shared(),
            attempted: Default::default(),
        }
    }

    pub fn with_http(mut self, http: SharedHttpClient) -> Self {
        self.http = http;
        self
    }

    pub async fn run(&self) -> Result<bool, SimpleProviderError> {
        let store = self.store.clone();
        let records =
            uc_core::load_blocking(move || store.list().map_err(|_| account_error())).await?;
        let eligible: Vec<_> = {
            let mut attempted = self.attempted.lock().await;
            records
                .into_iter()
                .filter(|record| {
                    record.credential_mode == CredentialMode::ManagedOauth
                        && record.label == record.provider
                        && attempted.insert(record.id.clone())
                })
                .collect()
        };
        let mut changed = false;
        for record in eligible {
            if let Ok(updated) = self.update(record).await {
                changed |= updated;
            }
        }
        Ok(changed)
    }

    async fn update(&self, record: AccountRecord) -> Result<bool, SimpleProviderError> {
        let kind = ProviderKind::parse(&record.provider).ok_or_else(account_error)?;
        let (expected, email) = match kind {
            ProviderKind::Codex => {
                let store = self.store.clone();
                let id = record.id.clone();
                let document = uc_core::load_blocking(move || {
                    store.credentials(&id).map_err(|_| account_error())
                })
                .await?;
                (record, crate::oauth::codex_email_label(&document["tokens"]))
            }
            ProviderKind::Claude => {
                let source = CredentialStore::for_account(kind, self.store.clone(), record.clone());
                let credentials = ready_credentials(
                    &source,
                    kind,
                    &self.http,
                    Utc::now(),
                    refresh_url(kind),
                    None,
                )
                .await?;
                let expected = {
                    let lock = refresh_lock(&record.id);
                    let _guard = lock.lock().await;
                    let current = source.load().await?;
                    if current.access_token != credentials.access_token {
                        return Ok(false);
                    }
                    let store = self.store.clone();
                    let id = record.id.clone();
                    uc_core::load_blocking(move || {
                        store
                            .list()
                            .map_err(|_| account_error())?
                            .into_iter()
                            .find(|record| record.id == id)
                            .ok_or_else(account_error)
                    })
                    .await?
                };
                if expected.credential_mode != CredentialMode::ManagedOauth
                    || expected.label != expected.provider
                {
                    return Ok(false);
                }
                let response = tokio::time::timeout(
                    Duration::from_secs(10),
                    self.http.send(
                        HttpRequest::get("https://api.anthropic.com/api/oauth/profile")
                            .bearer(&credentials.access_token)
                            .header("anthropic-beta", "oauth-2025-04-20")
                            .timeout(Duration::from_secs(10)),
                    ),
                )
                .await
                .map_err(|_| account_error())?
                .map_err(|_| account_error())?;
                if !response.is_success() {
                    return Ok(false);
                }
                let profile: Value = response.json().map_err(|_| account_error())?;
                let document = json!({"oauthAccount": {
                    "accountUuid": profile["account"]["uuid"],
                    "organizationUuid": profile["organization"]["uuid"]
                }});
                if account_id(kind, &identity(kind, &document)?) != record.id {
                    return Ok(false);
                }
                (
                    expected,
                    crate::oauth::email_label(&profile["account"]["email"]),
                )
            }
        };
        let Some(email) = email else {
            return Ok(false);
        };
        let store = self.store.clone();
        uc_core::load_blocking(move || {
            store
                .set_label_if_default(&expected, &email)
                .map_err(|_| account_error())
        })
        .await
    }
}

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

/// Claude Code keeps the signed-in account's identity in `.claude.json`, beside its credentials.
pub(crate) fn claude_profile_path() -> PathBuf {
    uc_core::paths::env_path("CLAUDE_CONFIG_DIR")
        .map(|root| root.join(".claude.json"))
        .unwrap_or_else(|| uc_core::paths::home_dir().join(".claude.json"))
}

/// A login that the Claude Code or Codex CLI on this computer holds right now. Quota Control reads
/// it on every refresh and never renews it, so the CLI stays the only owner of the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliAccount {
    pub kind: ProviderKind,
    pub id: String,
    pub email: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub location: CliLocation,
    pub profile: Option<PathBuf>,
}

/// The CLI logins on this computer that can show live limits. A CLI that is signed out, or whose
/// login holds no token (Claude Code inside Claude Desktop keeps its sign-in elsewhere), is left
/// out. Reads files and the macOS keychain, so keep it off the async runtime's worker threads where
/// that matters.
pub fn cli_accounts() -> Vec<CliAccount> {
    cli_accounts_keeping(&[])
}

/// `cli_accounts`, except that a login the keychain refused to read (locked, denied) keeps the
/// account `previous` knew for that CLI: its card stays and shows the keychain error instead of
/// disappearing until the keychain opens again.
pub fn cli_accounts_keeping(previous: &[CliAccount]) -> Vec<CliAccount> {
    [ProviderKind::Claude, ProviderKind::Codex]
        .into_iter()
        .filter_map(|kind| {
            let location = CliLocation::discover(kind);
            let profile = (kind == ProviderKind::Claude).then(claude_profile_path);
            match cli_account_at(kind, location, profile) {
                Ok(account) => Some(account),
                Err(error) => kept_through(&error, kind, previous),
            }
        })
        .collect()
}

fn kept_through(
    error: &SimpleProviderError,
    kind: ProviderKind,
    previous: &[CliAccount],
) -> Option<CliAccount> {
    (error.category == ErrorCategory::CredentialAccess)
        .then(|| {
            previous
                .iter()
                .find(|account| account.kind == kind)
                .cloned()
        })
        .flatten()
}

pub fn cli_account_from(
    kind: ProviderKind,
    path: PathBuf,
    profile: Option<PathBuf>,
) -> Result<CliAccount, SimpleProviderError> {
    cli_account_at(kind, CliLocation::File(path), profile)
}

pub fn cli_account_at(
    kind: ProviderKind,
    location: CliLocation,
    profile: Option<PathBuf>,
) -> Result<CliAccount, SimpleProviderError> {
    read_cli_account(kind, location, profile).map(|(account, _)| account)
}

pub(crate) fn read_cli_account(
    kind: ProviderKind,
    location: CliLocation,
    profile: Option<PathBuf>,
) -> Result<(CliAccount, Credentials), SimpleProviderError> {
    let mut document = read_location(&location, kind)?;
    let mut credentials = parse_credentials(kind, &document)?;
    if kind == ProviderKind::Claude {
        let profile = profile.as_deref().ok_or_else(|| {
            auth_error("Claude account metadata is unavailable. Connect through the browser.")
        })?;
        document["oauthAccount"] = read_json_file(profile, kind)?["oauthAccount"].clone();
        credentials.plan_term = crate::plan_term::from_document(kind, &document);
    }
    let key = identity(kind, &document)?;
    let email = match kind {
        ProviderKind::Codex => crate::oauth::codex_email_label(&document["tokens"]),
        ProviderKind::Claude => {
            crate::oauth::email_label(&document["oauthAccount"]["emailAddress"])
        }
    };
    let updated_at = login_updated_at(&location, profile.as_deref());
    Ok((
        CliAccount {
            kind,
            id: account_id(kind, &key),
            email,
            updated_at,
            location,
            profile,
        },
        credentials,
    ))
}

/// When the login last changed: the file's modification time, or the keychain item's.
fn login_updated_at(location: &CliLocation, profile: Option<&std::path::Path>) -> DateTime<Utc> {
    let modified = |path: &std::path::Path| {
        std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .map(DateTime::<Utc>::from)
            .ok()
    };
    match location {
        CliLocation::File(path) => modified(path),
        CliLocation::Keychain(item) => {
            crate::keychain::modified_at(item).or_else(|| profile.and_then(modified))
        }
    }
    .unwrap_or_else(Utc::now)
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

/// An account card the app shows, with where its session comes from.
#[derive(Clone, Copy, Debug)]
pub enum VisibleAccount<'a> {
    Stored(&'a AccountRecord),
    Cli(&'a CliAccount),
}

/// The accounts to show. A browser-connected session wins over the same account's CLI login,
/// because it renews itself; a live CLI login wins over an older imported copy of it.
pub fn visible_accounts<'a>(
    records: &'a [AccountRecord],
    cli: &'a [CliAccount],
) -> Vec<VisibleAccount<'a>> {
    let mut visible: Vec<_> = records
        .iter()
        .filter(|record| {
            record.credential_mode != CredentialMode::SharedCli
                || !cli.iter().any(|login| login.id == record.id)
        })
        .map(VisibleAccount::Stored)
        .collect();
    visible.extend(
        cli.iter()
            .filter(|login| {
                !records.iter().any(|record| {
                    record.id == login.id && record.credential_mode == CredentialMode::ManagedOauth
                })
            })
            .map(VisibleAccount::Cli),
    );
    visible
}

pub fn account_runtimes(
    store: Arc<AccountStore>,
    cli: &[CliAccount],
) -> Result<Vec<Arc<dyn ProviderRuntime>>, SimpleProviderError> {
    account_runtimes_with(store, cli, ReqwestHttpClient::shared())
}

pub fn account_runtimes_with(
    store: Arc<AccountStore>,
    cli: &[CliAccount],
    http: SharedHttpClient,
) -> Result<Vec<Arc<dyn ProviderRuntime>>, SimpleProviderError> {
    let records = store.list().map_err(|_| account_error())?;
    visible_accounts(&records, cli)
        .into_iter()
        .map(|account| match account {
            VisibleAccount::Stored(record) => {
                let kind = ProviderKind::parse(&record.provider)
                    .ok_or_else(|| auth_error("Unsupported account provider."))?;
                let credentials = CredentialStore::for_account(kind, store.clone(), record.clone());
                Ok(Arc::new(
                    LocalProvider::new(kind, credentials, http.clone()).with_account(record),
                ) as Arc<dyn ProviderRuntime>)
            }
            VisibleAccount::Cli(login) => {
                let credentials = CredentialStore::at(login.kind, login.location.clone());
                Ok(Arc::new(
                    LocalProvider::new(login.kind, credentials, http.clone()).with_cli_login(login),
                ) as Arc<dyn ProviderRuntime>)
            }
        })
        .collect()
}

/// Runtimes for the stored accounts alone.
pub fn runtimes_with_client(
    store: Arc<AccountStore>,
    http: SharedHttpClient,
) -> Result<Vec<Arc<dyn ProviderRuntime>>, SimpleProviderError> {
    account_runtimes_with(store, &[], http)
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
    let document = uc_core::load_blocking(move || source_copy.read_document()).await?;
    let credentials = parse_credentials(kind, &document)?;
    if !needs_renewal(&credentials, rejected_token, now) {
        return Ok(credentials);
    }
    let renewal_store = store.clone();
    let renewal_id = record.id.clone();
    let source_copy = source.clone();
    let (_renewal, mut document) = uc_core::load_blocking(move || {
        let renewal = renewal_store
            .renewal_lock(&renewal_id)
            .map_err(|_| account_error())?;
        Ok::<_, SimpleProviderError>((renewal, source_copy.read_document()?))
    })
    .await?;
    let credentials = parse_credentials(kind, &document)?;
    if !needs_renewal(&credentials, rejected_token, now) {
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

/// True when the stored session was just rejected or is about to expire. Re-checked after the
/// cross-process lock, because another process may have renewed it while this one waited.
fn needs_renewal(
    credentials: &Credentials,
    rejected_token: Option<&str>,
    now: DateTime<Utc>,
) -> bool {
    rejected_token.is_some_and(|token| token == credentials.access_token)
        || credentials
            .expires_at
            .is_some_and(|expiry| expiry <= now + chrono::Duration::seconds(60))
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
