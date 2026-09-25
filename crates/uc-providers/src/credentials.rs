use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde_json::Value;
use uc_accounts::{AccountRecord, AccountStore};
use uc_core::{ErrorCategory, SimpleProviderError, paths};

use crate::ProviderKind;

pub struct Credentials {
    pub(crate) access_token: String,
    pub(crate) account_id: Option<String>,
    pub(crate) plan: Option<String>,
    pub(crate) expires_at: Option<DateTime<Utc>>,
    pub(crate) profile_scope: bool,
}

#[derive(Clone)]
pub struct CredentialStore {
    kind: ProviderKind,
    pub(crate) source: CredentialSource,
}

#[derive(Clone)]
pub(crate) enum CredentialSource {
    File(PathBuf),
    Account {
        store: Arc<AccountStore>,
        record: AccountRecord,
    },
}

impl CredentialStore {
    pub fn new(kind: ProviderKind, path: impl Into<PathBuf>) -> Self {
        Self {
            kind,
            source: CredentialSource::File(path.into()),
        }
    }

    pub fn discover(kind: ProviderKind) -> Self {
        let (variable, directory, file) = match kind {
            ProviderKind::Claude => ("CLAUDE_CONFIG_DIR", ".claude", ".credentials.json"),
            ProviderKind::Codex => ("CODEX_HOME", ".codex", "auth.json"),
        };
        let root = paths::env_path(variable).unwrap_or_else(|| paths::home_dir().join(directory));
        Self::new(kind, root.join(file))
    }

    pub fn path(&self) -> Option<&Path> {
        match &self.source {
            CredentialSource::File(path) => Some(path),
            _ => None,
        }
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
        let path = self.path().ok_or_else(invalid)?;
        read_json_file(path, self.kind)
    }
}

pub(crate) fn read_json_file(
    path: &Path,
    kind: ProviderKind,
) -> Result<Value, SimpleProviderError> {
    let file = std::fs::File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SimpleProviderError::new(
                ErrorCategory::NotLoggedIn,
                format!("Sign in with {} to load usage.", kind.cli()),
            )
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
            let mut plan =
                text(&oauth["subscriptionType"]).map(|value| crate::mapping::title_case(&value));
            if let (Some(plan), Some(tier)) = (&mut plan, text(&oauth["rateLimitTier"]))
                && let Some(multiplier) = tier.split('_').find(|part| {
                    part.ends_with('x') && part[..part.len() - 1].parse::<u32>().is_ok()
                })
            {
                plan.push(' ');
                plan.push_str(multiplier);
            }
            Ok(Credentials {
                access_token,
                account_id: None,
                plan,
                expires_at,
                profile_scope,
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
                plan: None,
                expires_at,
                profile_scope: true,
            })
        }
    }
}
