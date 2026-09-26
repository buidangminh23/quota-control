use std::collections::HashSet;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const STORAGE_ERROR: &str = "Chat session storage could not be accessed";
const UNSAFE_PATH: &str = "Chat session storage contains an unsafe filesystem entry";
const INVALID_REGISTRY: &str = "Chat session metadata is invalid; existing data was preserved";
const MAX_METADATA_BYTES: u64 = 1_048_576;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatSession {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub created_at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Registry {
    version: u32,
    sessions: Vec<ChatSession>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            version: 1,
            sessions: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct ChatStore {
    root: PathBuf,
}

impl ChatStore {
    pub fn default_store() -> Self {
        Self::new(uc_core::paths::config_dir().join("web-chat"))
    }

    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn list(&self) -> Result<Vec<ChatSession>, String> {
        let _lock = self.lock()?;
        Ok(self.read_registry()?.sessions)
    }

    pub fn create(&self, provider: &str, label: Option<String>) -> Result<ChatSession, String> {
        let default_label = provider_label(provider)?;
        let label = checked_label(label.as_deref().unwrap_or(default_label))?.to_owned();
        let _lock = self.lock()?;
        let mut registry = self.read_registry()?;
        let id = loop {
            let candidate = Uuid::new_v4().to_string();
            if !registry
                .sessions
                .iter()
                .any(|session| session.id == candidate)
            {
                break candidate;
            }
        };
        let session = ChatSession {
            id,
            provider: provider.to_owned(),
            label,
            created_at: Utc::now().to_rfc3339(),
        };
        registry.sessions.push(session.clone());
        self.write_registry(&registry)?;
        Ok(session)
    }

    pub fn get(&self, id: &str) -> Result<ChatSession, String> {
        checked_id(id)?;
        let _lock = self.lock()?;
        self.read_registry()?
            .sessions
            .into_iter()
            .find(|session| session.id == id)
            .ok_or_else(|| "Chat session does not exist".into())
    }

    pub fn profile_directory(&self, id: &str) -> Result<PathBuf, String> {
        checked_id(id)?;
        let _lock = self.lock()?;
        if !self
            .read_registry()?
            .sessions
            .iter()
            .any(|session| session.id == id)
        {
            return Err("Chat session does not exist".into());
        }
        let profiles = self.root.join("profiles");
        ensure_directory(&profiles)?;
        let directory = profiles.join(id);
        ensure_directory(&directory)?;
        Ok(directory)
    }

    fn lock(&self) -> Result<File, String> {
        ensure_directory(&self.root)?;
        let path = self.root.join("sessions.lock");
        reject_links(&path)?;
        let file = file_options()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .map_err(|_| STORAGE_ERROR.to_owned())?;
        reject_links(&path)?;
        verify_file(&file)?;
        file.lock().map_err(|_| STORAGE_ERROR.to_owned())?;
        Ok(file)
    }

    fn read_registry(&self) -> Result<Registry, String> {
        let path = self.root.join("sessions.json");
        reject_links(&path)?;
        let file = match file_options().read(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Registry::default());
            }
            Err(_) => return Err(STORAGE_ERROR.into()),
        };
        reject_links(&path)?;
        verify_file(&file)?;
        let mut bytes = Vec::new();
        file.take(MAX_METADATA_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| STORAGE_ERROR.to_owned())?;
        if bytes.len() as u64 > MAX_METADATA_BYTES {
            return Err(INVALID_REGISTRY.into());
        }
        let registry: Registry =
            serde_json::from_slice(&bytes).map_err(|_| INVALID_REGISTRY.to_owned())?;
        let mut ids = HashSet::new();
        if registry.version != 1
            || registry.sessions.iter().any(|session| {
                checked_id(&session.id).is_err()
                    || !ids.insert(session.id.as_str())
                    || provider_label(&session.provider).is_err()
                    || checked_label(&session.label).is_err()
                    || session.label.trim() != session.label
                    || DateTime::parse_from_rfc3339(&session.created_at).is_err()
            })
        {
            return Err(INVALID_REGISTRY.into());
        }
        Ok(registry)
    }

    fn write_registry(&self, registry: &Registry) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(registry).map_err(|_| STORAGE_ERROR.to_owned())?;
        if bytes.len() as u64 > MAX_METADATA_BYTES {
            return Err("Chat session metadata has reached its storage limit".into());
        }
        let path = self.root.join("sessions.json");
        reject_links(&path)?;
        let temporary = self.root.join(format!(".sessions.{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = file_options()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| STORAGE_ERROR.to_owned())?;
            verify_file(&file)?;
            file.write_all(&bytes)
                .map_err(|_| STORAGE_ERROR.to_owned())?;
            file.sync_all().map_err(|_| STORAGE_ERROR.to_owned())?;
            drop(file);
            reject_links(&path)?;
            std::fs::rename(&temporary, &path).map_err(|_| STORAGE_ERROR.to_owned())?;
            #[cfg(unix)]
            let _ = File::open(&self.root).and_then(|directory| directory.sync_all());
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

fn provider_label(provider: &str) -> Result<&'static str, String> {
    match provider {
        "claude" => Ok("Claude"),
        "codex" => Ok("ChatGPT"),
        _ => Err("Unsupported chat provider".into()),
    }
}

fn checked_label(label: &str) -> Result<&str, String> {
    if label.chars().any(char::is_control) {
        return Err(
            "Chat label must contain 1 to 120 characters without control characters".into(),
        );
    }
    let label = label.trim();
    if label.is_empty() || label.chars().count() > 120 {
        return Err(
            "Chat label must contain 1 to 120 characters without control characters".into(),
        );
    }
    Ok(label)
}

fn checked_id(id: &str) -> Result<(), String> {
    match Uuid::parse_str(id) {
        Ok(uuid) if uuid.get_version_num() == 4 && uuid.to_string() == id => Ok(()),
        _ => Err("Invalid chat session ID".into()),
    }
}

fn ensure_directory(path: &Path) -> Result<(), String> {
    reject_links(path)?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| STORAGE_ERROR.to_owned())?;
    reject_links(path)?;
    if !std::fs::symlink_metadata(path)
        .map_err(|_| STORAGE_ERROR.to_owned())?
        .is_dir()
    {
        return Err(UNSAFE_PATH.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| STORAGE_ERROR.to_owned())?;
    }
    Ok(())
}

fn file_options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    options
}

fn verify_file(file: &File) -> Result<(), String> {
    let metadata = file.metadata().map_err(|_| STORAGE_ERROR.to_owned())?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(UNSAFE_PATH.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.nlink() != 1 {
            return Err(UNSAFE_PATH.into());
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| STORAGE_ERROR.to_owned())?;
    }
    Ok(())
}

fn reject_links(path: &Path) -> Result<(), String> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        if component == Component::ParentDir {
            return Err(UNSAFE_PATH.into());
        }
        prefix.push(component);
        match std::fs::symlink_metadata(&prefix) {
            Ok(metadata) if is_link(&metadata) && !system_link(&metadata) => {
                return Err(UNSAFE_PATH.into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(STORAGE_ERROR.into()),
        }
    }
    Ok(())
}

/// A link root owns, such as macOS's `/var` -> `private/var`, cannot be planted by the user's
/// processes, so it does not make the path unsafe.
fn system_link(metadata: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.uid() == 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (ChatStore, tempfile::TempDir) {
        let temporary = tempfile::tempdir().unwrap();
        (ChatStore::new(temporary.path().join("web-chat")), temporary)
    }

    #[test]
    fn sessions_and_isolated_profiles_survive_restart() {
        let (store, _temporary) = store();
        let personal = store.create("codex", Some(" Personal ".into())).unwrap();
        let work = store.create("codex", Some("Work".into())).unwrap();
        let claude = store.create("claude", None).unwrap();
        assert_eq!(personal.label, "Personal");
        assert_eq!(claude.label, "Claude");
        let personal_profile = store.profile_directory(&personal.id).unwrap();
        let work_profile = store.profile_directory(&work.id).unwrap();
        assert_ne!(personal_profile, work_profile);
        std::fs::write(personal_profile.join("session-data"), b"personal-session").unwrap();
        assert!(!work_profile.join("session-data").exists());
        let reopened = ChatStore::new(store.root.clone());
        assert_eq!(
            reopened.list().unwrap(),
            vec![personal.clone(), work, claude]
        );
        assert_eq!(reopened.get(&personal.id).unwrap(), personal);
        assert_eq!(
            reopened.profile_directory(&personal.id).unwrap(),
            personal_profile
        );
        assert_eq!(
            std::fs::read(personal_profile.join("session-data")).unwrap(),
            b"personal-session"
        );
    }

    #[test]
    fn only_registered_uuid_profiles_are_accessible() {
        let (store, _temporary) = store();
        for invalid in [
            "../outside",
            "..\\outside",
            "C:\\other",
            "",
            "/tmp/test",
            "not-a-uuid",
        ] {
            assert!(store.get(invalid).is_err());
            assert!(store.profile_directory(invalid).is_err());
        }
        assert!(
            store
                .profile_directory(&Uuid::new_v4().to_string())
                .is_err()
        );
        assert!(!store.root.join("profiles").exists());
    }

    #[test]
    fn providers_and_labels_are_validated() {
        let (store, _temporary) = store();
        assert!(store.create("other", None).is_err());
        for invalid in [
            "".to_owned(),
            "   ".into(),
            "line\nbreak".into(),
            "\tName".into(),
            "a".repeat(121),
        ] {
            assert!(store.create("codex", Some(invalid)).is_err());
        }
        assert_eq!(store.create("codex", None).unwrap().label, "ChatGPT");
        assert!(store.create("claude", Some("ệ".repeat(120))).is_ok());
    }

    #[test]
    fn corrupted_metadata_is_never_replaced() {
        let (store, _temporary) = store();
        store.create("codex", None).unwrap();
        let path = store.root.join("sessions.json");
        let corrupt = b"{not valid JSON";
        std::fs::write(&path, corrupt).unwrap();
        assert_eq!(store.list().unwrap_err(), INVALID_REGISTRY);
        assert_eq!(store.create("claude", None).unwrap_err(), INVALID_REGISTRY);
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
    }

    #[test]
    fn duplicate_ids_and_invalid_stored_fields_are_rejected() {
        let (store, _temporary) = store();
        let session = store.create("codex", None).unwrap();
        let mut invalid = serde_json::to_value(Registry {
            version: 1,
            sessions: vec![session.clone(), session],
        })
        .unwrap();
        let path = store.root.join("sessions.json");
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(store.list().is_err());
        invalid["sessions"].as_array_mut().unwrap().truncate(1);
        invalid["sessions"][0]["id"] = "../outside".into();
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(store.list().is_err());
    }

    #[test]
    fn independent_store_instances_serialize_creates() {
        let (store, _temporary) = store();
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for index in 0..8 {
                let root = store.root.clone();
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    ChatStore::new(root)
                        .create("codex", Some(format!("Session {index}")))
                        .unwrap();
                });
            }
        });
        let sessions = store.list().unwrap();
        assert_eq!(sessions.len(), 8);
        assert_eq!(
            sessions
                .iter()
                .map(|session| &session.id)
                .collect::<HashSet<_>>()
                .len(),
            8
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_profiles_are_private_and_links_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (store, temporary) = store();
        let session = store.create("codex", None).unwrap();
        let profile = store.profile_directory(&session.id).unwrap();
        for directory in [&store.root, &store.root.join("profiles"), &profile] {
            assert_eq!(
                std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        for file in [
            store.root.join("sessions.json"),
            store.root.join("sessions.lock"),
        ] {
            assert_eq!(
                std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let linked_root = temporary.path().join("linked-root");
        symlink(&store.root, &linked_root).unwrap();
        assert_eq!(ChatStore::new(linked_root).list().unwrap_err(), UNSAFE_PATH);
        let another = store.create("codex", None).unwrap();
        symlink(
            temporary.path(),
            store.root.join("profiles").join(&another.id),
        )
        .unwrap();
        assert_eq!(
            store.profile_directory(&another.id).unwrap_err(),
            UNSAFE_PATH
        );
        assert_eq!(store.list().unwrap().len(), 2);
    }
}
