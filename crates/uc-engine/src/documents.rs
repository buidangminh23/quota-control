//! Named JSON documents the popup owns (settings, layout), persisted atomically in the config dir.

use std::path::PathBuf;

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentName {
    Settings,
    Layout,
}

impl DocumentName {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "settings" => Some(Self::Settings),
            "layout" => Some(Self::Layout),
            _ => None,
        }
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Settings => "settings.json",
            Self::Layout => "layout.json",
        }
    }
}

pub struct DocumentStore {
    root: PathBuf,
}

impl DocumentStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_store() -> Self {
        Self::new(uc_core::paths::config_dir())
    }

    fn path(&self, name: DocumentName) -> PathBuf {
        self.root.join(name.file_name())
    }

    /// `Ok(None)` when the document was never saved. A corrupt file is reported, not silently dropped.
    pub fn load(&self, name: DocumentName) -> anyhow::Result<Option<Value>> {
        match std::fs::read(self.path(name)) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, name: DocumentName, value: &Value) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(value)?;
        uc_core::paths::write_atomic(&self.path(name), &bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_documents() {
        let dir = tempfile::tempdir().unwrap();
        let store = DocumentStore::new(dir.path().to_path_buf());
        assert!(store.load(DocumentName::Settings).unwrap().is_none());
        store.save(DocumentName::Settings, &serde_json::json!({"theme": "dark"})).unwrap();
        assert_eq!(store.load(DocumentName::Settings).unwrap().unwrap()["theme"], "dark");
    }

    #[test]
    fn corrupt_documents_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("layout.json"), b"{not json").unwrap();
        let store = DocumentStore::new(dir.path().to_path_buf());
        assert!(store.load(DocumentName::Layout).is_err());
    }
}
