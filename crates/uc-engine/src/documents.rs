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
        store
            .save(
                DocumentName::Settings,
                &serde_json::json!({"theme": "dark"}),
            )
            .unwrap();
        assert_eq!(
            store.load(DocumentName::Settings).unwrap().unwrap()["theme"],
            "dark"
        );
    }

    #[test]
    fn corrupt_documents_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("layout.json"), b"{not json").unwrap();
        let store = DocumentStore::new(dir.path().to_path_buf());
        assert!(store.load(DocumentName::Layout).is_err());
    }

    #[test]
    fn concurrent_saves_leave_one_complete_document() {
        let dir = tempfile::tempdir().unwrap();
        let store = DocumentStore::new(dir.path().to_path_buf());
        let barrier = std::sync::Barrier::new(16);
        std::thread::scope(|scope| {
            for index in 0..16 {
                let store = &store;
                let barrier = &barrier;
                scope.spawn(move || {
                    let value = serde_json::json!({"writer": index, "values": vec![index; 4096]});
                    barrier.wait();
                    store.save(DocumentName::Settings, &value).unwrap();
                });
            }
        });
        let value = store.load(DocumentName::Settings).unwrap().unwrap();
        let values = value["values"].as_array().unwrap();
        assert_eq!(values.len(), 4096);
        assert!(values.iter().all(|item| *item == value["writer"]));
    }
}
