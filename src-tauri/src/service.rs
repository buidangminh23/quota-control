use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use uc_core::ProviderRuntime;
use uc_engine::{DocumentName, DocumentStore, Engine, EngineConfig, SnapshotCache};

pub struct BackendService {
    engine: RwLock<Arc<Engine>>,
    cache: SnapshotCache,
    documents: Mutex<DocumentStore>,
    tasks: Mutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub platform: &'static str,
    pub log_file: String,
}

impl BackendService {
    pub fn new(runtimes: Vec<Arc<dyn ProviderRuntime>>) -> anyhow::Result<Self> {
        let config = EngineConfig::default();
        let cache = SnapshotCache::new(SnapshotCache::default_path(), config.refresh_interval);
        let engine = Arc::new(uc_api::build_engine(
            runtimes,
            cache.clone(),
            config,
            uc_core::system_clock(),
        ));
        let documents = DocumentStore::default_store();
        Self::with_storage(engine, documents, cache)
    }

    pub fn with_storage(
        engine: Arc<Engine>,
        documents: DocumentStore,
        cache: SnapshotCache,
    ) -> anyhow::Result<Self> {
        match documents.load(DocumentName::Settings) {
            Ok(Some(settings)) => match enabled_ids(&settings) {
                Ok(Some(ids)) => engine.set_enabled(&ids),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!("Stored settings are invalid; using defaults: {error}")
                }
            },
            Ok(None) => {}
            Err(_) => tracing::warn!(
                "Stored settings could not be read; using defaults and preserving the file"
            ),
        }
        Ok(Self {
            engine: RwLock::new(engine),
            cache,
            documents: Mutex::new(documents),
            tasks: Mutex::new(Vec::new()),
        })
    }

    pub fn engine(&self) -> Arc<Engine> {
        self.engine.read().clone()
    }

    pub fn replace_runtimes<R: tauri::Runtime>(
        &self,
        runtimes: Vec<Arc<dyn ProviderRuntime>>,
        app: &AppHandle<R>,
    ) -> Result<(), String> {
        let documents = self.documents.lock();
        let prior = self.engine();
        let previously_known = prior.provider_ids();
        let config = prior.config().clone();
        let next = Arc::new(uc_api::build_engine(
            runtimes,
            self.cache.clone(),
            config,
            uc_core::system_clock(),
        ));
        let mut enabled: Vec<_> = next
            .provider_ids()
            .into_iter()
            .filter(|id| !previously_known.contains(id) || prior.is_enabled(id))
            .collect();
        enabled.sort();
        next.set_enabled(&enabled);
        if let Ok(Some(mut settings)) = documents.load(DocumentName::Settings)
            && let Some(object) = settings.as_object_mut() {
                object.insert("enabledProviders".into(), serde_json::json!(enabled));
                if documents.save(DocumentName::Settings, &settings).is_err() {
                    tracing::warn!("Account catalog updated, but provider selection could not be saved");
                }
            }
        for task in self.tasks.lock().drain(..) {
            task.abort();
        }
        *self.engine.write() = next;
        self.start(app);
        if app.emit_to("popup", "catalog-changed", self.engine().catalog()).is_err() {
            tracing::warn!("Could not publish the updated account catalog");
        }
        Ok(())
    }

    pub fn start<R: tauri::Runtime>(&self, app: &AppHandle<R>) {
        let mut tasks = self.tasks.lock();
        for task in tasks.drain(..) {
            task.abort();
        }
        let engine = self.engine();
        let mut events = engine.subscribe();
        let handle = app.clone();
        let event_engine = engine.clone();
        let _ = app.emit_to("popup", "engine-state", engine.state());
        tasks.push(tauri::async_runtime::spawn(async move {
            loop {
                let state = match events.recv().await {
                    Ok(state) => state,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        event_engine.state()
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if let Err(error) = handle.emit_to("popup", "engine-state", state) {
                    tracing::warn!("Engine event delivery failed: {error}");
                }
            }
        }));
        tasks.push(tauri::async_runtime::spawn(engine.run()));
    }

    pub fn info(&self) -> AppInfo {
        AppInfo {
            name: "Quota Control",
            version: env!("CARGO_PKG_VERSION"),
            platform: std::env::consts::OS,
            log_file: uc_core::paths::log_file().to_string_lossy().into_owned(),
        }
    }

    /// What the local HTTP API serves right now, in the dashboard's provider order.
    pub fn api_state(&self) -> uc_api::ApiState {
        let order = self
            .documents
            .lock()
            .load(DocumentName::Layout)
            .ok()
            .flatten()
            .and_then(|layout| layout.get("providerOrder").cloned())
            .and_then(|order| serde_json::from_value::<Vec<String>>(order).ok())
            .unwrap_or_default();
        uc_api::ApiState::capture(&self.engine(), &order, chrono::Utc::now())
    }

    pub fn load(&self, name: &str) -> Result<Option<Value>, String> {
        let name = document_name(name)?;
        self.documents.lock().load(name).map_err(safe_error)
    }

    pub fn save(&self, name: &str, value: &Value) -> Result<(), String> {
        let name = document_name(name)?;
        if serde_json::to_vec(value).map_err(safe_error)?.len() > 1_048_576 {
            return Err("Document exceeds the 1 MiB limit".into());
        }
        let ids = if name == DocumentName::Settings {
            enabled_ids(value)?
        } else {
            None
        };
        let documents = self.documents.lock();
        if let Some(ids) = &ids {
            self.validate_provider_ids(ids)?;
        }
        documents.save(name, value).map_err(safe_error)?;
        if let Some(ids) = ids {
            self.engine().set_enabled(&ids);
        }
        Ok(())
    }

    pub fn set_enabled(&self, ids: &[String]) -> Result<(), String> {
        let documents = self.documents.lock();
        self.validate_provider_ids(ids)?;
        let mut settings = documents
            .load(DocumentName::Settings)
            .map_err(safe_error)?
            .unwrap_or_else(|| serde_json::json!({}));
        let object = settings
            .as_object_mut()
            .ok_or("Settings must be an object")?;
        object.insert("enabledProviders".into(), serde_json::json!(ids));
        documents
            .save(DocumentName::Settings, &settings)
            .map_err(safe_error)?;
        self.engine().set_enabled(ids);
        Ok(())
    }

    pub fn validate_provider_ids(&self, ids: &[String]) -> Result<(), String> {
        if ids.iter().any(|id| self.engine().runtime(id).is_none()) {
            return Err("Unknown provider ID".into());
        }
        Ok(())
    }
}

pub fn safe_error(error: impl std::fmt::Display) -> String {
    uc_core::redact::log_message(&error.to_string())
}

fn document_name(name: &str) -> Result<DocumentName, String> {
    DocumentName::parse(name).ok_or_else(|| "Unknown document name".into())
}

fn enabled_ids(value: &Value) -> Result<Option<Vec<String>>, String> {
    let object = value.as_object().ok_or("Settings must be an object")?;
    object
        .get("enabledProviders")
        .map(|ids| {
            serde_json::from_value::<Vec<String>>(ids.clone())
                .map_err(|_| "enabledProviders must be an array of strings".into())
        })
        .transpose()
}

pub fn validate_url(raw: &str) -> Result<(), String> {
    let url = url::Url::parse(raw).map_err(|_| "Invalid URL")?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Only HTTP and HTTPS URLs without credentials are allowed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use uc_core::{Provider, ProviderSnapshot, RefreshContext, WidgetDescriptor};

    struct FixtureProvider(Provider);

    #[async_trait]
    impl ProviderRuntime for FixtureProvider {
        fn provider(&self) -> &Provider {
            &self.0
        }

        fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
            Vec::new()
        }

        async fn refresh(&self, _: RefreshContext) -> ProviderSnapshot {
            std::future::pending().await
        }

        async fn has_local_credentials(&self) -> bool {
            false
        }
    }

    fn fixture_runtimes(ids: &[&str]) -> Vec<Arc<dyn ProviderRuntime>> {
        ids.iter()
            .map(|id| Arc::new(FixtureProvider(Provider::new(*id, *id))) as Arc<dyn ProviderRuntime>)
            .collect()
    }

    fn fixture_service(root: &std::path::Path) -> BackendService {
        let config = EngineConfig::default();
        let cache = SnapshotCache::new(root.join("cache.json"), config.refresh_interval);
        let engine = Arc::new(Engine::new(
            fixture_runtimes(&["codex@removed", "codex@retained", "claude@disabled"]),
            cache.clone(),
            config,
        ));
        BackendService::with_storage(engine, DocumentStore::new(root.join("config")), cache).unwrap()
    }

    struct StopTasks<'a>(&'a BackendService);

    impl Drop for StopTasks<'_> {
        fn drop(&mut self) {
            for task in self.0.tasks.lock().drain(..) {
                task.abort();
            }
        }
    }

    #[cfg(windows)]
    struct RestorePermissions {
        path: std::path::PathBuf,
        permissions: std::fs::Permissions,
    }

    #[cfg(windows)]
    impl Drop for RestorePermissions {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.path, self.permissions.clone());
        }
    }

    #[cfg(windows)]
    #[test]
    fn catalog_reconciliation_survives_read_only_settings_write_failure() {
        let directory = tempfile::tempdir().unwrap();
        let documents = DocumentStore::new(directory.path().join("config"));
        let original = serde_json::json!({
            "enabledProviders": ["codex@removed", "codex@retained"],
            "theme": "dark"
        });
        documents.save(DocumentName::Settings, &original).unwrap();
        let settings_path = directory.path().join("config/settings.json");
        let original_bytes = std::fs::read(&settings_path).unwrap();
        let permissions = std::fs::metadata(&settings_path).unwrap().permissions();
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        let _restore = RestorePermissions {
            path: settings_path.clone(),
            permissions,
        };
        std::fs::set_permissions(&settings_path, readonly).unwrap();
        assert!(documents.save(DocumentName::Settings, &original).is_err());
        let service = fixture_service(directory.path());
        let _stop = StopTasks(&service);
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();

        service
            .replace_runtimes(
                fixture_runtimes(&["codex@added", "codex@retained", "claude@disabled"]),
                app.handle(),
            )
            .unwrap();

        let engine = service.engine();
        assert_eq!(
            engine.provider_ids(),
            vec!["codex@added", "codex@retained", "claude@disabled"]
        );
        assert!(engine.runtime("codex@removed").is_none());
        assert!(!engine.state().providers.contains_key("codex@removed"));
        assert!(engine.is_enabled("codex@added"));
        assert!(engine.is_enabled("codex@retained"));
        assert!(!engine.is_enabled("claude@disabled"));
        assert_eq!(std::fs::read(&settings_path).unwrap(), original_bytes);
        assert_eq!(documents.load(DocumentName::Settings).unwrap(), Some(original));
    }

    #[test]
    fn catalog_reconciliation_survives_unreadable_settings_document() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("config/settings.json");
        std::fs::create_dir_all(&settings_path).unwrap();
        let marker = settings_path.join("preserve");
        std::fs::write(&marker, b"existing-data").unwrap();
        let service = fixture_service(directory.path());
        let _stop = StopTasks(&service);
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        assert!(service.load("settings").is_err());

        service
            .replace_runtimes(fixture_runtimes(&["codex@added"]), app.handle())
            .unwrap();

        assert_eq!(service.engine().provider_ids(), vec!["codex@added"]);
        assert!(service.engine().runtime("codex@removed").is_none());
        assert!(service.engine().is_enabled("codex@added"));
        assert!(service.load("settings").is_err());
        assert_eq!(std::fs::read(marker).unwrap(), b"existing-data");
    }

    #[test]
    fn document_names_cannot_escape_config_directory() {
        for invalid in ["../auth", "settings.json", "", "C:\\secret"] {
            assert!(document_name(invalid).is_err());
        }
        assert_eq!(document_name("settings").unwrap(), DocumentName::Settings);
    }

    #[test]
    fn provider_selection_distinguishes_absent_and_disabled() {
        assert_eq!(enabled_ids(&serde_json::json!({})).unwrap(), None);
        assert_eq!(
            enabled_ids(&serde_json::json!({"enabledProviders":[]})).unwrap(),
            Some(vec![])
        );
        assert!(enabled_ids(&serde_json::json!({"enabledProviders":null})).is_err());
        assert!(enabled_ids(&serde_json::json!([])).is_err());
    }

    #[test]
    fn external_links_reject_local_protocols_and_embedded_secrets() {
        for invalid in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://user:secret@example.com",
            "not a url",
        ] {
            assert!(validate_url(invalid).is_err());
        }
        assert!(validate_url("https://status.openai.com").is_ok());
    }
}
