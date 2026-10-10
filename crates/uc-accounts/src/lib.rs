mod keys;
mod protection;
mod storage;

pub use keys::{KeyRecord, KeyStore, LoginSnapshot};

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, AccountError>;

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("Account storage could not be accessed")]
    Storage,
    #[error("Account storage contains an unsafe filesystem entry")]
    UnsafePath,
    #[error("Account registry is invalid; existing data was preserved")]
    InvalidRegistry,
    #[error("Account provider, label, or identity is invalid")]
    InvalidAccount,
    #[error("Account does not exist")]
    NotFound,
    #[error("Account credentials are unavailable; reconnect this account")]
    CredentialsUnavailable,
    #[error("Credentials must be a JSON object no larger than 1 MiB")]
    InvalidCredentials,
    #[error("Account credentials could not be protected")]
    Protection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialMode {
    SharedCli,
    ManagedOauth,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub connected_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub credential_mode: CredentialMode,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAccount {
    #[serde(flatten)]
    record: AccountRecord,
    credential_revision: Uuid,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Registry {
    version: u32,
    accounts: BTreeMap<String, StoredAccount>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            version: 1,
            accounts: BTreeMap::new(),
        }
    }
}

/// Held while one process renews an account's session, so the app and the CLI never spend the
/// same refresh token. Released on drop.
pub struct RenewalLock {
    _file: std::fs::File,
}

#[derive(Clone)]
pub struct AccountStore {
    root: PathBuf,
}

impl AccountStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_store() -> Self {
        Self::new(uc_core::paths::config_dir().join("accounts"))
    }

    pub fn directory(&self) -> &std::path::Path {
        &self.root
    }

    pub fn list(&self) -> Result<Vec<AccountRecord>> {
        let _lock = storage::lock(&self.root)?;
        Ok(self
            .registry()?
            .accounts
            .into_values()
            .map(|entry| entry.record)
            .collect())
    }

    pub fn import(
        &self,
        provider: &str,
        label: &str,
        identity: &str,
        secret: &Value,
        mode: CredentialMode,
    ) -> Result<AccountRecord> {
        validate_input(provider, label, identity)?;
        let bytes = credential_bytes(secret)?;
        let hash: String = Sha256::digest(identity.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let id = format!("{provider}@{hash}");
        let _lock = storage::lock(&self.root)?;
        let mut registry = self.registry()?;
        let now = Utc::now();
        if let Some(existing) = registry.accounts.get_mut(&id)
            && existing.record.credential_mode == CredentialMode::ManagedOauth
            && mode == CredentialMode::SharedCli
        {
            existing.record.label = label.trim().into();
            existing.record.updated_at = now;
            let record = existing.record.clone();
            self.save_registry(&registry)?;
            return Ok(record);
        }
        let record = AccountRecord {
            id: id.clone(),
            provider: provider.into(),
            label: label.trim().into(),
            connected_at: registry
                .accounts
                .get(&id)
                .map(|entry| entry.record.connected_at)
                .unwrap_or(now),
            updated_at: now,
            credential_mode: mode,
        };
        let revision = self.write_credentials(&id, &bytes)?;
        let previous = registry.accounts.insert(
            id.clone(),
            StoredAccount {
                record: record.clone(),
                credential_revision: revision,
            },
        );
        if let Err(error) = self.save_registry(&registry) {
            let _ = storage::remove(&self.secret_path(&id, revision));
            return Err(error);
        }
        if let Some(previous) = previous {
            let _ = storage::remove(&self.secret_path(&id, previous.credential_revision));
        }
        Ok(record)
    }

    pub fn credentials(&self, id: &str) -> Result<Value> {
        let _lock = storage::lock(&self.root)?;
        let registry = self.registry()?;
        let entry = registry.accounts.get(id).ok_or(AccountError::NotFound)?;
        let encrypted = storage::read(
            &self.secret_path(id, entry.credential_revision),
            2 * 1_048_576,
        )
        .map_err(|_| AccountError::CredentialsUnavailable)?
        .ok_or(AccountError::CredentialsUnavailable)?;
        let bytes = protection::unprotect(&encrypted, id.as_bytes())?;
        let secret: Value =
            serde_json::from_slice(&bytes).map_err(|_| AccountError::CredentialsUnavailable)?;
        if !secret.is_object() {
            return Err(AccountError::CredentialsUnavailable);
        }
        Ok(secret)
    }

    pub fn update_credentials(&self, id: &str, secret: &Value) -> Result<()> {
        let bytes = credential_bytes(secret)?;
        let _lock = storage::lock(&self.root)?;
        let mut registry = self.registry()?;
        let entry = registry
            .accounts
            .get_mut(id)
            .ok_or(AccountError::NotFound)?;
        let previous_revision = entry.credential_revision;
        let revision = self.write_credentials(id, &bytes)?;
        entry.credential_revision = revision;
        entry.record.updated_at = Utc::now();
        if let Err(error) = self.save_registry(&registry) {
            let _ = storage::remove(&self.secret_path(id, revision));
            return Err(error);
        }
        let _ = storage::remove(&self.secret_path(id, previous_revision));
        Ok(())
    }

    pub fn set_label_if_default(&self, expected: &AccountRecord, label: &str) -> Result<bool> {
        let _lock = storage::lock(&self.root)?;
        let mut registry = self.registry()?;
        let entry = registry
            .accounts
            .get_mut(&expected.id)
            .ok_or(AccountError::NotFound)?;
        validate_input(&entry.record.provider, label, "stored")?;
        if entry.record != *expected
            || entry.record.credential_mode != CredentialMode::ManagedOauth
            || entry.record.label != entry.record.provider
            || entry.record.label == label.trim()
        {
            return Ok(false);
        }
        entry.record.label = label.trim().into();
        entry.record.updated_at = Utc::now();
        self.save_registry(&registry)?;
        Ok(true)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let _lock = storage::lock(&self.root)?;
        let mut registry = self.registry()?;
        let entry = registry.accounts.remove(id).ok_or(AccountError::NotFound)?;
        storage::remove(&self.secret_path(id, entry.credential_revision))?;
        self.save_registry(&registry)?;
        let _ = storage::remove(&self.renewal_path(id));
        Ok(())
    }

    /// Wait until no other process is renewing `id`'s session, then hold that right until the
    /// returned guard drops.
    pub fn renewal_lock(&self, id: &str) -> Result<RenewalLock> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'@' || byte == b'-')
        {
            return Err(AccountError::InvalidAccount);
        }
        storage::lock_file(&self.root, &self.renewal_path(id))
            .map(|file| RenewalLock { _file: file })
    }

    fn renewal_path(&self, id: &str) -> PathBuf {
        self.root
            .join("credentials")
            .join(format!("{id}.renewal.lock"))
    }

    fn secret_path(&self, id: &str, revision: Uuid) -> PathBuf {
        self.root
            .join("credentials")
            .join(format!("{id}.{revision}.bin"))
    }

    fn write_credentials(&self, id: &str, bytes: &[u8]) -> Result<Uuid> {
        let encrypted = protection::protect(bytes, id.as_bytes())?;
        let revision = Uuid::new_v4();
        storage::write_atomic(&self.secret_path(id, revision), &encrypted)?;
        Ok(revision)
    }

    fn registry(&self) -> Result<Registry> {
        let Some(bytes) = storage::read(&self.root.join("registry.json"), 8 * 1_048_576)? else {
            return Ok(Registry::default());
        };
        let registry: Registry =
            serde_json::from_slice(&bytes).map_err(|_| AccountError::InvalidRegistry)?;
        if registry.version != 1
            || registry.accounts.iter().any(|(id, entry)| {
                let Some(hash) = id.strip_prefix(&format!("{}@", entry.record.provider)) else {
                    return true;
                };
                id != &entry.record.id
                    || hash.len() != 64
                    || !hash
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                    || validate_input(&entry.record.provider, &entry.record.label, "stored")
                        .is_err()
            })
        {
            return Err(AccountError::InvalidRegistry);
        }
        Ok(registry)
    }

    fn save_registry(&self, registry: &Registry) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(registry).map_err(|_| AccountError::Storage)?;
        storage::write_atomic(&self.root.join("registry.json"), &bytes)
    }
}

fn validate_input(provider: &str, label: &str, identity: &str) -> Result<()> {
    if !matches!(provider, "claude" | "codex")
        || label.trim().is_empty()
        || label.len() > 256
        || label.chars().any(char::is_control)
        || identity.trim().is_empty()
        || identity.len() > 4096
    {
        return Err(AccountError::InvalidAccount);
    }
    Ok(())
}

fn credential_bytes(secret: &Value) -> Result<Vec<u8>> {
    if !secret.is_object() {
        return Err(AccountError::InvalidCredentials);
    }
    let bytes = serde_json::to_vec(secret).map_err(|_| AccountError::InvalidCredentials)?;
    if bytes.len() > 1_048_576 {
        return Err(AccountError::InvalidCredentials);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
