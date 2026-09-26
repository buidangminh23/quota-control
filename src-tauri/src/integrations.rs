//! Native integrations the core owns: the global shortcut and whether `usagectl` is on PATH. They
//! live in their own document so the popup's settings saves, which merge a stale copy onto the
//! stored one, can never revert them.

use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::service::safe_error;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Integrations {
    /// An accelerator such as `Ctrl+Alt+KeyU`; `None` leaves the popup without a shortcut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_shortcut: Option<String>,
    /// The user installed `usagectl` from Settings; the app keeps that copy current.
    #[serde(default)]
    pub command_line_tool: bool,
}

pub struct IntegrationStore {
    path: PathBuf,
    current: Mutex<Integrations>,
}

impl IntegrationStore {
    pub fn default_store() -> Self {
        Self::open(uc_core::paths::config_dir().join("integrations.json"))
    }

    /// A missing or unreadable document starts from the defaults; the file is left as it was.
    pub fn open(path: PathBuf) -> Self {
        let current = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!("Integration settings are invalid; using defaults: {error}");
                Integrations::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Integrations::default(),
            Err(error) => {
                tracing::warn!("Integration settings could not be read; using defaults: {error}");
                Integrations::default()
            }
        };
        Self {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Integrations {
        self.current.lock().clone()
    }

    /// Apply `change` and persist it; memory only changes once the file is written.
    pub fn update(&self, change: impl FnOnce(&mut Integrations)) -> Result<Integrations, String> {
        let mut current = self.current.lock();
        let mut next = current.clone();
        change(&mut next);
        let bytes = serde_json::to_vec_pretty(&next).map_err(safe_error)?;
        uc_core::paths::write_atomic(&self.path, &bytes).map_err(safe_error)?;
        *current = next.clone();
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_persist_and_invalid_files_fall_back_without_being_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.json");
        let store = IntegrationStore::open(path.clone());
        assert_eq!(store.get(), Integrations::default());
        store
            .update(|integrations| {
                integrations.global_shortcut = Some("Ctrl+Alt+KeyU".into());
                integrations.command_line_tool = true;
            })
            .unwrap();
        let reopened = IntegrationStore::open(path.clone());
        assert_eq!(
            reopened.get().global_shortcut.as_deref(),
            Some("Ctrl+Alt+KeyU")
        );
        assert!(reopened.get().command_line_tool);

        std::fs::write(&path, b"{broken").unwrap();
        assert_eq!(
            IntegrationStore::open(path.clone()).get(),
            Integrations::default()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
    }
}
