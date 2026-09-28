//! API keys the user saved for services that keep no login on this computer (OpenRouter, Z.ai,
//! MiniMax, ...), and the accounts of such services signed in to through the browser (the token
//! document the sign-in returned). Both are protected like an account's credentials (DPAPI on
//! Windows, owner-only files elsewhere) and are read back only for that service's own requests.

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
    /// The key's last four characters, so two keys of one service can be told apart; empty for a
    /// signed-in account.
    pub hint: String,
    /// How a signed-in account was added (`google`, `github`, `browser`); absent for a pasted key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<String>,
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
/// The largest token document a sign-in may save; the stored file is read back up to 64 KiB.
const MAX_DOCUMENT_BYTES: usize = 32 * 1024;

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
            || key.chars().any(|character| {
                character.is_control() || (character.is_whitespace() && character != ' ')
            })
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
        let hint: String = key
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        self.save(
            &Self::key_id(service, key),
            service,
            label,
            hint,
            None,
            &bytes,
        )
    }

    /// Save the token document a browser sign-in returned for one account of `service`, as card
    /// `id` (`<service>@<64 hex digits>`, derived by the caller from the account's identity).
    /// Signing in to the same account again replaces the document and keeps its card and date.
    pub fn add_login(
        &self,
        id: &str,
        service: &str,
        label: &str,
        method: &str,
        document: &Value,
    ) -> Result<KeyRecord> {
        validate(service, label)?;
        if !valid_id(id, service) || !valid_method(method) {
            return Err(AccountError::InvalidAccount);
        }
        let bytes = document_bytes(document)?;
        self.save(
            id,
            service,
            label,
            String::new(),
            Some(method.into()),
            &bytes,
        )
    }

    /// Replace a signed-in account's token document, when a renewal rotated its refresh token.
    pub fn replace_login(&self, id: &str, document: &Value) -> Result<()> {
        let bytes = document_bytes(document)?;
        let _lock = storage::lock(&self.root)?;
        let mut registry = self.registry()?;
        let entry = registry.keys.get_mut(id).ok_or(AccountError::NotFound)?;
        if entry.record.sign_in.is_none() {
            return Err(AccountError::InvalidAccount);
        }
        let previous = entry.revision;
        let revision = Uuid::new_v4();
        let encrypted = protection::protect(&bytes, id.as_bytes())?;
        storage::write_atomic(&self.secret_path(id, revision), &encrypted)?;
        entry.revision = revision;
        if let Err(error) = self.save_registry(&registry) {
            let _ = storage::remove(&self.secret_path(id, revision));
            return Err(error);
        }
        let _ = storage::remove(&self.secret_path(id, previous));
        Ok(())
    }

    fn save(
        &self,
        id: &str,
        service: &str,
        label: &str,
        hint: String,
        sign_in: Option<String>,
        bytes: &[u8],
    ) -> Result<KeyRecord> {
        let _lock = storage::lock(&self.root)?;
        let mut registry = self.registry()?;
        let now = Utc::now();
        let record = KeyRecord {
            id: id.into(),
            service: service.into(),
            label: label.trim().into(),
            added_at: registry
                .keys
                .get(id)
                .map(|entry| entry.record.added_at)
                .unwrap_or(now),
            hint,
            sign_in,
        };
        let revision = Uuid::new_v4();
        let encrypted = protection::protect(bytes, id.as_bytes())?;
        storage::write_atomic(&self.secret_path(id, revision), &encrypted)?;
        let previous = registry.keys.insert(
            id.into(),
            StoredKey {
                record: record.clone(),
                revision,
            },
        );
        if let Err(error) = self.save_registry(&registry) {
            let _ = storage::remove(&self.secret_path(id, revision));
            return Err(error);
        }
        if let Some(previous) = previous {
            let _ = storage::remove(&self.secret_path(id, previous.revision));
        }
        Ok(record)
    }

    /// A saved key and its extra fields, as `{"apiKey": ..., ...}`, or a signed-in account's token
    /// document as its sign-in returned it.
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
        let usable = match entry.record.sign_in {
            Some(_) => secret.is_object(),
            None => secret.get("apiKey").is_some_and(Value::is_string),
        };
        if !usable {
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

    /// Cards found on this computer (another app's login, an environment key) that the user
    /// removed here. They are kept apart from the saved keys, so an older version still reads
    /// `keys.json`, and the logins themselves are never touched.
    pub fn dismissed(&self) -> Result<Vec<String>> {
        let _lock = storage::lock(&self.root)?;
        Ok(self.dismissed_list()?.ids)
    }

    /// Stop showing the found card `id`.
    pub fn dismiss(&self, id: &str) -> Result<()> {
        if !valid_card_id(id) {
            return Err(AccountError::InvalidAccount);
        }
        let _lock = storage::lock(&self.root)?;
        let mut list = self.dismissed_list()?;
        if list.ids.iter().any(|known| known == id) {
            return Ok(());
        }
        if list.ids.len() >= MAX_DISMISSED {
            list.ids.remove(0);
        }
        list.ids.push(id.into());
        self.save_dismissed(&list)
    }

    /// Show the found cards `ids` again.
    pub fn restore(&self, ids: &[String]) -> Result<()> {
        let _lock = storage::lock(&self.root)?;
        let mut list = self.dismissed_list()?;
        let before = list.ids.len();
        list.ids.retain(|id| !ids.contains(id));
        if list.ids.len() == before {
            return Ok(());
        }
        self.save_dismissed(&list)
    }

    fn dismissed_list(&self) -> Result<DismissedList> {
        let Some(bytes) = storage::read(&self.root.join(DISMISSED_FILE), 256 * 1024)? else {
            return Ok(DismissedList::default());
        };
        let list: DismissedList =
            serde_json::from_slice(&bytes).map_err(|_| AccountError::InvalidRegistry)?;
        if list.version != 1 || list.ids.iter().any(|id| !valid_card_id(id)) {
            return Err(AccountError::InvalidRegistry);
        }
        Ok(list)
    }

    fn save_dismissed(&self, list: &DismissedList) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(list).map_err(|_| AccountError::Storage)?;
        storage::write_atomic(&self.root.join(DISMISSED_FILE), &bytes)
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
                    || entry
                        .record
                        .sign_in
                        .as_deref()
                        .is_some_and(|method| !valid_method(method))
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DismissedList {
    version: u32,
    ids: Vec<String>,
}

impl Default for DismissedList {
    fn default() -> Self {
        Self {
            version: 1,
            ids: Vec::new(),
        }
    }
}

const DISMISSED_FILE: &str = "dismissed.json";
const MAX_DISMISSED: usize = 512;

/// A found card's id: `<service>@<64 lowercase hex digits>` for any valid service.
fn valid_card_id(id: &str) -> bool {
    id.split_once('@')
        .is_some_and(|(service, _)| validate(service, "x").is_ok() && valid_id(id, service))
}

/// `<service>@<64 lowercase hex digits>`.
fn valid_id(id: &str, service: &str) -> bool {
    id.strip_prefix(service)
        .and_then(|rest| rest.strip_prefix('@'))
        .is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

/// A sign-in method is one short lowercase word.
fn valid_method(method: &str) -> bool {
    (1..=16).contains(&method.len()) && method.bytes().all(|byte| byte.is_ascii_lowercase())
}

/// A token document as saved: a JSON object of bounded size.
fn document_bytes(document: &Value) -> Result<Vec<u8>> {
    if !document.is_object() {
        return Err(AccountError::InvalidCredentials);
    }
    let bytes = serde_json::to_vec(document).map_err(|_| AccountError::InvalidCredentials)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(AccountError::InvalidCredentials);
    }
    Ok(bytes)
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
        assert!(store.add("zai", "x", "has\ttab", &Value::Null).is_err());
        assert!(store.add("zai", "x", "line\nbreak", &Value::Null).is_err());
        assert!(store.add("zai", "x", "k", &json!("not an object")).is_err());
        assert!(store.list().unwrap().is_empty());
        let header = store
            .add("longcat", "x", "session=a1; theme=dark", &Value::Null)
            .unwrap();
        assert_eq!(
            store.secret(&header.id).unwrap()["apiKey"],
            "session=a1; theme=dark"
        );
    }

    fn login_id(service: &str, digit: char) -> String {
        format!("{service}@{}", digit.to_string().repeat(64))
    }

    #[test]
    fn a_signed_in_account_keeps_its_whole_token_document() {
        let (store, _dir) = store();
        let id = login_id("gemini", 'a');
        let document = json!({"access_token": "at", "refresh_token": "rt", "expiry_date": 1});
        let record = store
            .add_login(&id, "gemini", "me@example.com", "google", &document)
            .unwrap();
        assert_eq!(record.id, id);
        assert_eq!(record.sign_in.as_deref(), Some("google"));
        assert_eq!(record.hint, "");
        assert_eq!(store.secret(&id).unwrap(), document);
        let registry = std::fs::read_to_string(store.root.join("keys.json")).unwrap();
        assert!(registry.contains("\"signIn\": \"google\""));
        assert!(!registry.contains("refresh_token") && !registry.contains("\"rt\""));
        let again = store
            .add_login(
                &id,
                "gemini",
                "me@example.com",
                "google",
                &json!({"access_token": "new"}),
            )
            .unwrap();
        assert_eq!(again.added_at, record.added_at);
        assert_eq!(store.list().unwrap(), vec![again]);
        assert_eq!(store.secret(&id).unwrap()["access_token"], "new");
    }

    #[test]
    fn a_rotated_login_replaces_its_document_and_leaves_one_file() {
        let (store, _dir) = store();
        let id = login_id("kiro", 'b');
        store
            .add_login(
                &id,
                "kiro",
                "Kiro",
                "github",
                &json!({"refreshToken": "r1"}),
            )
            .unwrap();
        store
            .replace_login(&id, &json!({"refreshToken": "r2"}))
            .unwrap();
        assert_eq!(store.secret(&id).unwrap()["refreshToken"], "r2");
        let files = std::fs::read_dir(store.root.join("credentials"))
            .unwrap()
            .count();
        assert_eq!(files, 1);
        let key = store.add("zai", "Z.ai", "key-1", &Value::Null).unwrap();
        assert!(matches!(
            store.replace_login(&key.id, &json!({})),
            Err(AccountError::InvalidAccount)
        ));
        assert!(matches!(
            store.replace_login(&login_id("kiro", 'c'), &json!({})),
            Err(AccountError::NotFound)
        ));
    }

    #[test]
    fn sign_ins_are_validated() {
        let (store, _dir) = store();
        let document = json!({"token": "t"});
        let id = login_id("copilot", 'd');
        let add = |id: &str, service: &str, method: &str, document: &Value| {
            store.add_login(id, service, "x", method, document)
        };
        assert!(add("copilot@abc", "copilot", "github", &document).is_err());
        assert!(add(&login_id("gemini", 'd'), "copilot", "github", &document).is_err());
        assert!(add(&login_id("copilot", 'D'), "copilot", "github", &document).is_err());
        assert!(add(&id, "copilot", "Git Hub", &document).is_err());
        assert!(add(&id, "copilot", "", &document).is_err());
        assert!(add(&id, "copilot", "github", &json!("token")).is_err());
        let huge = json!({"token": "t".repeat(MAX_DOCUMENT_BYTES)});
        assert!(add(&id, "copilot", "github", &huge).is_err());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn dismissed_cards_are_kept_apart_from_the_keys_and_restored() {
        let (store, _dir) = store();
        let key = store.add("zai", "Z.ai", "key-1", &Value::Null).unwrap();
        let gemini = login_id("gemini", 'a');
        let copilot = login_id("copilot", 'b');
        assert!(store.dismissed().unwrap().is_empty());
        store.dismiss(&gemini).unwrap();
        store.dismiss(&gemini).unwrap();
        store.dismiss(&copilot).unwrap();
        assert_eq!(
            store.dismissed().unwrap(),
            vec![gemini.clone(), copilot.clone()]
        );
        assert_eq!(store.list().unwrap(), vec![key]);
        let registry = std::fs::read_to_string(store.root.join("keys.json")).unwrap();
        assert!(!registry.contains(&gemini));
        store.restore(std::slice::from_ref(&gemini)).unwrap();
        assert_eq!(store.dismissed().unwrap(), vec![copilot]);
        assert!(matches!(
            store.dismiss("gemini@abc"),
            Err(AccountError::InvalidAccount)
        ));
        assert!(matches!(
            store.dismiss("../x@0000"),
            Err(AccountError::InvalidAccount)
        ));
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
