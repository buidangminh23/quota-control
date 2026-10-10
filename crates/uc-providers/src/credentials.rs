use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde_json::Value;
use uc_accounts::{AccountRecord, AccountStore, CredentialMode};
use uc_core::{ErrorCategory, SimpleProviderError, paths};

use crate::ProviderKind;
use crate::keychain::{self, KeychainItem};

pub struct Credentials {
    pub(crate) access_token: String,
    pub(crate) account_id: Option<String>,
    pub(crate) billing_account_id: Option<String>,
    pub(crate) plan: Option<String>,
    pub(crate) expires_at: Option<DateTime<Utc>>,
    pub(crate) profile_scope: bool,
    /// The plan's paid period as the login document states or implies it.
    pub(crate) plan_term: Option<uc_core::PlanTerm>,
}

#[derive(Clone)]
pub struct CredentialStore {
    kind: ProviderKind,
    pub(crate) source: CredentialSource,
}

/// Where a CLI keeps its login: a JSON file, or on macOS the keychain item Claude Code writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliLocation {
    File(PathBuf),
    Keychain(KeychainItem),
}

impl CliLocation {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::File(path) => Some(path),
            Self::Keychain(_) => None,
        }
    }

    /// Where the CLI keeps its login on this computer. Claude Code on macOS uses the keychain and
    /// may leave a stale `.credentials.json` behind, so the keychain wins whenever it holds a login.
    pub fn discover(kind: ProviderKind) -> Self {
        let (variable, directory, file) = match kind {
            ProviderKind::Claude => ("CLAUDE_CONFIG_DIR", ".claude", ".credentials.json"),
            ProviderKind::Codex => ("CODEX_HOME", ".codex", "auth.json"),
        };
        let root = paths::env_path(variable).unwrap_or_else(|| paths::home_dir().join(directory));
        let file_location = Self::File(root.join(file));
        if !cfg!(target_os = "macos") || kind != ProviderKind::Claude {
            return file_location;
        }
        match keychain::find_claude() {
            Ok(Some(item)) => Self::Keychain(item),
            Ok(None) => file_location,
            Err(_) => Self::Keychain(KeychainItem::claude()),
        }
    }
}

#[derive(Clone)]
pub(crate) enum CredentialSource {
    Cli(CliLocation),
    Account {
        store: Arc<AccountStore>,
        record: AccountRecord,
    },
}

impl CredentialStore {
    pub fn new(kind: ProviderKind, path: impl Into<PathBuf>) -> Self {
        Self::at(kind, CliLocation::File(path.into()))
    }

    pub fn at(kind: ProviderKind, location: CliLocation) -> Self {
        Self {
            kind,
            source: CredentialSource::Cli(location),
        }
    }

    pub fn discover(kind: ProviderKind) -> Self {
        Self::at(kind, CliLocation::discover(kind))
    }

    pub fn location(&self) -> Option<&CliLocation> {
        match &self.source {
            CredentialSource::Cli(location) => Some(location),
            CredentialSource::Account { .. } => None,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.location().and_then(CliLocation::path)
    }

    /// True for a browser-connected account, whose session this app owns and renews.
    pub(crate) fn is_managed(&self) -> bool {
        matches!(&self.source, CredentialSource::Account { record, .. } if record.credential_mode == CredentialMode::ManagedOauth)
    }

    pub fn for_account(
        kind: ProviderKind,
        store: Arc<AccountStore>,
        record: AccountRecord,
    ) -> Self {
        Self {
            kind,
            source: CredentialSource::Account { store, record },
        }
    }

    pub async fn load(&self) -> Result<Credentials, SimpleProviderError> {
        let store = self.clone();
        uc_core::load_blocking(move || store.load_sync()).await
    }

    pub(crate) fn load_sync(&self) -> Result<Credentials, SimpleProviderError> {
        parse_credentials(self.kind, &self.read_document()?)
    }

    pub(crate) fn read_document(&self) -> Result<Value, SimpleProviderError> {
        if let CredentialSource::Account { store, record } = &self.source {
            return store.credentials(&record.id).map_err(|_| {
                SimpleProviderError::new(
                    ErrorCategory::CredentialAccess,
                    "Saved account credentials are unavailable. Reconnect this account.",
                )
            });
        }
        let location = self.location().ok_or_else(invalid)?;
        read_location(location, self.kind)
    }
}

pub(crate) fn read_location(
    location: &CliLocation,
    kind: ProviderKind,
) -> Result<Value, SimpleProviderError> {
    match location {
        CliLocation::File(path) => read_json_file(path, kind),
        CliLocation::Keychain(item) => match keychain::password(item) {
            Ok(Some(secret)) => keychain::decode_document(&secret).ok_or_else(invalid),
            Ok(None) => Err(not_logged_in(kind)),
            Err(_) => Err(SimpleProviderError::new(
                ErrorCategory::CredentialAccess,
                "Cannot read the login from the macOS keychain. Unlock the keychain and try again.",
            )),
        },
    }
}

fn not_logged_in(kind: ProviderKind) -> SimpleProviderError {
    SimpleProviderError::new(
        ErrorCategory::NotLoggedIn,
        format!("Sign in with {} to load usage.", kind.cli()),
    )
}

pub(crate) fn read_json_file(
    path: &Path,
    kind: ProviderKind,
) -> Result<Value, SimpleProviderError> {
    let file = std::fs::File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            not_logged_in(kind)
        } else {
            SimpleProviderError::new(
                ErrorCategory::CredentialAccess,
                "Cannot read local credentials.",
            )
        }
    })?;
    let mut bytes = Vec::new();
    file.take(1_048_577).read_to_end(&mut bytes).map_err(|_| {
        SimpleProviderError::new(
            ErrorCategory::CredentialAccess,
            "Cannot read local credentials.",
        )
    })?;
    if bytes.len() > 1_048_576 {
        return Err(invalid());
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    Ok(body)
}

fn invalid() -> SimpleProviderError {
    SimpleProviderError::new(
        ErrorCategory::AuthInvalid,
        "Local credentials are invalid. Sign in again.",
    )
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn header_text(value: &Value) -> Result<Option<String>, SimpleProviderError> {
    let text = text(value);
    if text
        .as_ref()
        .is_some_and(|value| value.bytes().any(|byte| byte < 32 || byte == 127))
    {
        return Err(invalid());
    }
    Ok(text)
}

pub(crate) fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn jwt_expiry(token: &str) -> Option<DateTime<Utc>> {
    let claims = jwt_claims(token)?;
    DateTime::from_timestamp(claims["exp"].as_i64()?, 0)
}

pub(crate) fn parse_credentials(
    kind: ProviderKind,
    body: &Value,
) -> Result<Credentials, SimpleProviderError> {
    if !body.is_object() {
        return Err(invalid());
    }
    match kind {
        ProviderKind::Claude => {
            let oauth = body
                .get("claudeAiOauth")
                .filter(|value| value.is_object())
                .ok_or_else(invalid)?;
            let access_token = header_text(&oauth["accessToken"])?.ok_or_else(invalid)?;
            let expires_at = match oauth.get("expiresAt").filter(|value| !value.is_null()) {
                Some(value) => Some(
                    DateTime::from_timestamp_millis(value.as_i64().ok_or_else(invalid)?)
                        .ok_or_else(invalid)?,
                ),
                None => jwt_expiry(&access_token),
            };
            let profile_scope = match oauth.get("scopes").filter(|value| !value.is_null()) {
                Some(Value::Array(scopes)) => {
                    scopes.is_empty() || scopes.iter().any(|value| value == "user:profile")
                }
                Some(_) => return Err(invalid()),
                None => true,
            };
            let plan = text(&oauth["subscriptionType"])
                .map(|value| crate::mapping::claude_plan(&value, oauth["rateLimitTier"].as_str()));
            Ok(Credentials {
                access_token,
                account_id: None,
                billing_account_id: body["oauthAccount"]["organizationUuid"]
                    .as_str()
                    .and_then(|id| uuid::Uuid::parse_str(id.trim()).ok())
                    .filter(|id| !id.is_nil())
                    .map(|id| id.to_string()),
                plan,
                expires_at,
                profile_scope,
                plan_term: crate::plan_term::from_document(kind, body),
            })
        }
        ProviderKind::Codex => {
            let tokens = &body["tokens"];
            let access_token = header_text(&tokens["access_token"])?;
            let Some(access_token) = access_token else {
                if text(&body["OPENAI_API_KEY"]).is_some() {
                    return Err(SimpleProviderError::new(
                        ErrorCategory::NotAvailable,
                        "Usage limits require a ChatGPT login; API keys do not provide subscription limits.",
                    ));
                }
                return Err(invalid());
            };
            let expires_at = jwt_expiry(&access_token).or_else(|| {
                body["_usageControl"]["expiresAt"]
                    .as_i64()
                    .and_then(DateTime::from_timestamp_millis)
            });
            Ok(Credentials {
                access_token,
                account_id: header_text(&tokens["account_id"])?,
                billing_account_id: crate::plan_term::codex_billing_account(body),
                plan: crate::plan_term::codex_plan(body),
                expires_at,
                profile_scope: true,
                plan_term: crate::plan_term::from_document(kind, body),
            })
        }
    }
}
