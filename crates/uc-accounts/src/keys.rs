//! API keys the user saved for services that keep no login on this computer (OpenRouter, Z.ai,
//! MiniMax, ...). A key is protected like an account's credentials (DPAPI on Windows, owner-only
//! files elsewhere) and is read back only for that service's own requests.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{AccountError, Result, protection, storage};

/// A saved key as the Accounts screen lists it; the key itself is never part of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyRecord {
    /// `<service>@<sha256 of the key>`: the card id, the same for the same key.
    pub id: String,
    pub service: String,
    pub label: String,
    pub added_at: DateTime<Utc>,
    /// The key's last four characters, so two keys of one service can be told apart.
    pub hint: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredKey {
    #[serde(flatten)]
    record: KeyRecord,
    revision: Uuid,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KeyRegistry {
    version: u32,
    keys: BTreeMap<String, StoredKey>,
}

impl Default for KeyRegistry {
    fn default() -> Self {
        Self {
            version: 1,
            keys: BTreeMap::new(),
        }
    }
}

const MAX_KEY_LENGTH: usize = 8192;

#[derive(Clone, Debug)]
pub struct KeyStore {
    root: PathBuf,
}

impl KeyStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_store() -> Self {
        Self::new(uc_core::paths::config_dir().join("api-keys"))
    }

    /// The card id a key of `service` gets.
    pub fn key_id(service: &str, key: &str) -> String {
        let hash: String = Sha256::digest(format!("key:{}", key.trim()).as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!("{service}@{hash}")
    }

    pub fn list(&self) -> Result<Vec<KeyRecord>> {
        let _lock = storage::lock(&self.root)?;
        Ok(self
            .registry()?
            .keys
            .into_values()
            .map(|entry| entry.record)
            .collect())
    }

    /// Save `key` for `service`, with `extra` fields (a region, an account id) kept beside it.
    /// Saving the same key again replaces its label and fields and keeps its card.
    pub fn add(&self, service: &str, label: &str, key: &str, extra: &Value) -> Result<KeyRecord> {
        let key = key.trim();
        validate(service, label)?;
        if key.is_empty()
            || key.len() > MAX_KEY_LENGTH
            || key
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(AccountError::InvalidCredentials);
        }
        let mut secret = match extra {
            Value::Object(fields) => fields.clone(),
            Value::Null => serde_json::Map::new(),
            _ => return Err(AccountError::InvalidCredentials),
        };
        secret.insert("apiKey".into(), Value::String(key.into()));
        let bytes = serde_json::to_vec(&Value::Object(secret))
            .map_err(|_| AccountError::InvalidCredentials)?;
        let id = Self::key_id(service, key);
        let hint: String = key
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let _lock = storage::lock(&self.root)?;
        let mut registry = self.registry()?;
        let now = Utc::now();
        let record = KeyRecord {
            id: id.clone(),
            service: service.into(),
            label: label.trim().into(),
            added_at: registry
                .keys
                .get(&id)
                .map(|entry| entry.record.added_at)
                .unwrap_or(now),
            hint,
        };
        let revision = Uuid::new_v4();
        let encrypted = protection::protect(&bytes, id.as_bytes())?;
        storage::write_atomic(&self.secret_path(&id, revision), &encrypted)?;
        let previous = registry.keys.insert(
            id.clone(),
            StoredKey {
                record: record.clone(),
                revision,
            },
        );
        if let Err(error) = self.save_registry(&registry) {
            let _ = storage::remove(&self.secret_path(&id, revision));
            return Err(error);
        }
        if let Some(previous) = previous {
            let _ = storage::remove(&self.secret_path(&id, previous.revision));
        }
        Ok(record)
    }

    /// The saved key and its extra fields, as `{"apiKey": ..., ...}`.
    pub fn secret(&self, id: &str) -> Result<Value> {
        let _lock = storage::lock(&self.root)?;
        let registry = self.registry()?;
        let entry = registry.keys.get(id).ok_or(AccountError::NotFound)?;
        let encrypted = storage::read(&self.secret_path(id, entry.revision), 64 * 1024)
            .map_err(|_| AccountError::CredentialsUnavailable)?
            .ok_or(AccountError::CredentialsUnavailable)?;
        let bytes = protection::unprotect(&encrypted, id.as_bytes())?;
        let secret: Value =
            serde_json::from_slice(&bytes).map_err(|_| AccountError::CredentialsUnavailable)?;
        if !secret.get("apiKey").is_some_and(Value::is_string) {
            return Err(AccountError::CredentialsUnavailable);
        }
        Ok(secret)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let _lock = storage::lock(&self.root)?;
        let mut registry = self.registry()?;
        let entry = registry.keys.remove(id).ok_or(AccountError::NotFound)?;
        storage::remove(&self.secret_path(id, entry.revision))?;
        self.save_registry(&registry)
    }

    fn secret_path(&self, id: &str, revision: Uuid) -> PathBuf {
        self.root
            .join("credentials")
            .join(format!("{id}.{revision}.bin"))
    }

    fn registry(&self) -> Result<KeyRegistry> {
        let Some(bytes) = storage::read(&self.root.join("keys.json"), 4 * 1_048_576)? else {
            return Ok(KeyRegistry::default());
        };
        let registry: KeyRegistry =
            serde_json::from_slice(&bytes).map_err(|_| AccountError::InvalidRegistry)?;
        if registry.version != 1
            || registry.keys.iter().any(|(id, entry)| {
                let Some(hash) = id.strip_prefix(&format!("{}@", entry.record.service)) else {
                    return true;
                };
                id != &entry.record.id
                    || hash.len() != 64
                    || !hash
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                    || validate(&entry.record.service, &entry.record.label).is_err()
            })
        {
            return Err(AccountError::InvalidRegistry);
        }
        Ok(registry)
    }

    fn save_registry(&self, registry: &KeyRegistry) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(registry).map_err(|_| AccountError::Storage)?;
        storage::write_atomic(&self.root.join("keys.json"), &bytes)
    }
}

fn validate(service: &str, label: &str) -> Result<()> {
    let service_ok = !service.is_empty()
        && service.len() <= 40
        && service.starts_with(|character: char| character.is_ascii_lowercase())
        && service
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !service_ok
        || label.trim().is_empty()
        || label.len() > 256
        || label.chars().any(char::is_control)
    {
        return Err(AccountError::InvalidAccount);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> (KeyStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (KeyStore::new(dir.path().join("api-keys")), dir)
    }

    #[test]
    fn a_saved_key_is_listed_without_the_key_and_read_back_with_its_fields() {
        let (store, _dir) = store();
        let record = store
            .add(
                "openrouter",
                "Work",
                " sk-or-v1-abcd1234 ",
                &json!({"region": "global"}),
            )
            .unwrap();
        assert_eq!(
            record.id,
            KeyStore::key_id("openrouter", "sk-or-v1-abcd1234")
        );
        assert_eq!(record.hint, "1234");
        assert_eq!(store.list().unwrap(), vec![record.clone()]);
        let secret = store.secret(&record.id).unwrap();
        assert_eq!(secret["apiKey"], "sk-or-v1-abcd1234");
        assert_eq!(secret["region"], "global");
        let registry = std::fs::read_to_string(store.root.join("keys.json")).unwrap();
        assert!(!registry.contains("sk-or-v1-abcd1234"));
    }

    #[test]
    fn saving_the_same_key_again_keeps_one_card_and_its_first_date() {
        let (store, _dir) = store();
        let first = store.add("zai", "Z.ai", "key-1", &Value::Null).unwrap();
        let second = store.add("zai", "Renamed", "key-1", &Value::Null).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.added_at, first.added_at);
        assert_eq!(store.list().unwrap(), vec![second]);
        let files = std::fs::read_dir(store.root.join("credentials"))
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "bin")
            })
            .count();
        assert_eq!(files, 1);
    }

    #[test]
    fn a_removed_key_is_gone_with_its_secret() {
        let (store, _dir) = store();
        let record = store
            .add("deepseek", "DeepSeek", "sk-1", &Value::Null)
            .unwrap();
        store.remove(&record.id).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(matches!(
            store.secret(&record.id),
            Err(AccountError::NotFound)
        ));
        assert!(matches!(
            store.remove(&record.id),
            Err(AccountError::NotFound)
        ));
    }

    #[test]
    fn keys_services_and_labels_are_validated() {
        let (store, _dir) = store();
        assert!(store.add("Bad", "x", "k", &Value::Null).is_err());
        assert!(store.add("-x", "x", "k", &Value::Null).is_err());
        assert!(store.add("zai", " ", "k", &Value::Null).is_err());
        assert!(store.add("zai", "x", "", &Value::Null).is_err());
        assert!(store.add("zai", "x", "has space", &Value::Null).is_err());
        assert!(store.add("zai", "x", "k", &json!("not an object")).is_err());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn a_tampered_registry_is_rejected() {
        let (store, _dir) = store();
        store.add("zai", "x", "k", &Value::Null).unwrap();
        let path = store.root.join("keys.json");
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\"zai\"", "\"groq\"");
        std::fs::write(&path, text).unwrap();
        assert!(matches!(store.list(), Err(AccountError::InvalidRegistry)));
    }
}
