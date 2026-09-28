use std::sync::Arc;

use serde_json::Value;
use uc_accounts::{AccountStore, KeyRecord, KeyStore};
use uc_core::{Clock, ProviderRuntime, ReqwestHttpClient, SimpleProviderError};
use uc_engine::{Engine, EngineConfig, SnapshotCache};
use uc_logscan::{LocalHistoryRuntime, LogSource};
use uc_providers::CliAccount;
use uc_services::{Detected, Roots};

/// The providers the app shows: every connected account and every CLI login on this computer, the
/// other services' logins and API keys, then this machine's Claude and Codex log history. The app
/// and `usagectl` build the same set, so they share one snapshot cache.
pub fn provider_runtimes(
    store: Arc<AccountStore>,
) -> Result<Vec<Arc<dyn ProviderRuntime>>, SimpleProviderError> {
    let services = ServiceCards::scan(KeyStore::default_store(), Roots::system());
    provider_runtimes_with(store, &uc_providers::cli_accounts(), &services)
}

/// [`provider_runtimes`] for CLI logins and service cards the caller already read.
pub fn provider_runtimes_with(
    store: Arc<AccountStore>,
    cli: &[CliAccount],
    services: &ServiceCards,
) -> Result<Vec<Arc<dyn ProviderRuntime>>, SimpleProviderError> {
    let mut runtimes = uc_providers::account_runtimes(store, cli)?;
    runtimes.extend(services.runtimes());
    runtimes.push(Arc::new(LocalHistoryRuntime::new(LogSource::Claude)));
    runtimes.push(Arc::new(LocalHistoryRuntime::new(LogSource::Codex)));
    Ok(runtimes)
}

/// What the other services' cards are built from: the API keys saved in Quota Control, and the
/// logins and environment keys this computer has.
#[derive(Clone, Debug)]
pub struct ServiceCards {
    pub store: KeyStore,
    pub roots: Roots,
    pub saved: Vec<KeyRecord>,
    pub detected: Vec<Detected>,
    /// Cards found on this computer that the user removed from Quota Control; they get no runtime.
    pub dismissed: Vec<Detected>,
}

impl ServiceCards {
    /// Read the saved keys and look for logins and environment keys under `roots`. It reads other
    /// apps' files and credential entries, so callers on an async runtime run it off the runtime.
    pub fn scan(store: KeyStore, roots: Roots) -> Self {
        let saved = forget_retired(
            &store,
            store.list().unwrap_or_else(|error| {
                tracing::warn!("Saved API keys could not be read: {error}");
                Vec::new()
            }),
        );
        let hidden = store.dismissed().unwrap_or_else(|error| {
            tracing::warn!("Removed cards could not be read: {error}");
            Vec::new()
        });
        let (dismissed, detected) = uc_services::detect(&roots, &saved)
            .into_iter()
            .partition(|found| hidden.contains(&found.id));
        Self {
            store,
            roots,
            saved,
            detected,
            dismissed,
        }
    }

    /// Each card's id and label: the cards are rebuilt when a rescan finds these changed.
    pub fn fingerprint(&self) -> Vec<(String, Option<String>)> {
        let mut cards: Vec<(String, Option<String>)> = self
            .detected
            .iter()
            .map(|found| (found.id.clone(), found.label.clone()))
            .chain(
                self.saved
                    .iter()
                    .map(|record| (record.id.clone(), Some(record.label.clone()))),
            )
            .collect();
        cards.sort();
        cards
    }

    /// A runtime for each card.
    pub fn runtimes(&self) -> Vec<Arc<dyn ProviderRuntime>> {
        uc_services::runtimes(
            &self.detected,
            &self.store,
            &self.saved,
            &self.roots,
            ReqwestHttpClient::shared(),
        )
    }
}

/// `saved` without the sign-ins and keys of services Quota Control no longer reads, which are
/// removed from the store (with their removed-card marks) the first time they are seen.
fn forget_retired(store: &KeyStore, saved: Vec<KeyRecord>) -> Vec<KeyRecord> {
    let retired = |service: &str| uc_services::RETIRED_SERVICES.contains(&service);
    let (gone, kept): (Vec<KeyRecord>, Vec<KeyRecord>) = saved
        .into_iter()
        .partition(|record| retired(&record.service));
    for record in &gone {
        match store.remove(&record.id) {
            Ok(()) => tracing::info!("Forgot a saved {} sign-in or key", record.service),
            Err(error) => tracing::warn!(
                "A saved {} card could not be removed: {error}",
                record.service
            ),
        }
    }
    if let Ok(dismissed) = store.dismissed() {
        let stale: Vec<String> = dismissed
            .into_iter()
            .filter(|id| {
                id.split_once('@')
                    .is_some_and(|(service, _)| retired(service))
            })
            .collect();
        if !stale.is_empty() {
            let _ = store.restore(&stale);
        }
    }
    kept
}

/// Whether a card starts hidden the first time it appears: the service reads a login that every
/// install of its app has, signed in or not (Ollama), so the card waits to be turned on.
pub fn starts_hidden(id: &str) -> bool {
    let family = id.split('@').next().unwrap_or(id);
    uc_services::service(family).is_some_and(|service| service.starts_hidden())
}

/// Which cards are enabled, and which card ids to remember as seen (`knownProviders`).
#[derive(Debug, PartialEq, Eq)]
pub struct ProviderSelection {
    pub enabled: Vec<String>,
    pub known: Vec<String>,
}

/// Decide the enabled cards among `ids`. A card listed in `enabled` stays enabled, and a card never
/// seen before starts enabled, so a CLI login that appeared while the app was closed shows up. Seen
/// cards are remembered while present; a hidden card is also remembered while absent, so it returns
/// hidden, and a visible one is forgotten, so it returns visible. `known` of `None` predates that
/// record; `fallback_known` then stands in for it. Without an `enabled` list every card is enabled.
/// A card that [`starts_hidden`] is enabled only once listed.
pub fn select_providers(
    ids: &[String],
    enabled: Option<&[String]>,
    known: Option<&[String]>,
    fallback_known: &[String],
) -> ProviderSelection {
    let known_before = known.unwrap_or(fallback_known);
    let Some(enabled) = enabled else {
        return ProviderSelection {
            enabled: ids
                .iter()
                .filter(|id| !starts_hidden(id))
                .cloned()
                .collect(),
            known: ids.to_vec(),
        };
    };
    let selected = ids
        .iter()
        .filter(|id| enabled.contains(id) || (!known_before.contains(id) && !starts_hidden(id)))
        .cloned()
        .collect();
    let mut remembered = ids.to_vec();
    for id in known_before {
        if !enabled.contains(id) && !remembered.contains(id) {
            remembered.push(id.clone());
        }
    }
    ProviderSelection {
        enabled: selected,
        known: remembered,
    }
}

/// A string list stored under `key` in the settings document, when it is one.
pub fn settings_list(settings: Option<&Value>, key: &str) -> Option<Vec<String>> {
    settings
        .and_then(|settings| settings.get(key))
        .and_then(|value| serde_json::from_value::<Vec<String>>(value.clone()).ok())
}

/// An engine whose cache entries are stamped with each card's id. The app and `usagectl` must stamp
/// alike: an entry stamped by another identity is discarded at the next launch.
pub fn build_engine(
    runtimes: Vec<Arc<dyn ProviderRuntime>>,
    cache: SnapshotCache,
    config: EngineConfig,
    clock: Clock,
) -> Engine {
    let identities = runtimes
        .iter()
        .map(|runtime| (runtime.provider().id.clone(), runtime.provider().id.clone()))
        .collect();
    Engine::with_options(runtimes, cache, config, identities, None, clock)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn saved_cards_of_a_retired_service_are_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::new(dir.path().join("api-keys"));
        let gemini = format!("gemini@{}", "a".repeat(64));
        store
            .add_login(
                &gemini,
                "gemini",
                "me@example.com",
                "google",
                &serde_json::json!({"t": 1}),
            )
            .unwrap();
        store
            .dismiss(&format!("gemini@{}", "b".repeat(64)))
            .unwrap();
        let kept = store.add("zai", "Z.ai", "key-1", &Value::Null).unwrap();
        let cards = ServiceCards::scan(store.clone(), Roots::under(dir.path()));
        assert_eq!(cards.saved, vec![kept.clone()]);
        assert_eq!(store.list().unwrap(), vec![kept]);
        assert!(store.dismissed().unwrap().is_empty());
    }

    #[test]
    fn a_removed_found_card_gets_no_runtime_until_restored() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::new(dir.path().join("api-keys"));
        let roots = Roots::under(dir.path()).with_var("ELEVENLABS_API_KEY", "sk_test_key");
        let elevenlabs = |cards: &[Detected]| -> Vec<String> {
            cards
                .iter()
                .filter(|found| found.service == "elevenlabs")
                .map(|found| found.id.clone())
                .collect()
        };
        let cards = ServiceCards::scan(store.clone(), roots.clone());
        let found = elevenlabs(&cards.detected);
        assert_eq!(found.len(), 1);
        assert!(cards.dismissed.is_empty());
        let before = cards.runtimes().len();
        store.dismiss(&found[0]).unwrap();
        let cards = ServiceCards::scan(store.clone(), roots.clone());
        assert!(elevenlabs(&cards.detected).is_empty());
        assert_eq!(elevenlabs(&cards.dismissed), found);
        assert!(!cards.fingerprint().iter().any(|(id, _)| *id == found[0]));
        assert_eq!(cards.runtimes().len(), before - 1);
        store.restore(&found).unwrap();
        assert_eq!(
            elevenlabs(&ServiceCards::scan(store, roots).detected),
            found
        );
    }

    #[test]
    fn without_a_list_every_card_is_enabled_and_remembered() {
        let present = ids(&["codex@cli", "claude-local"]);
        let selection = select_providers(&present, None, None, &[]);
        assert_eq!(selection.enabled, present);
        assert_eq!(selection.known, present);
    }

    #[test]
    fn a_card_never_seen_starts_enabled_and_a_hidden_one_stays_hidden() {
        let present = ids(&["codex@new", "claude-local", "codex-local"]);
        let selection = select_providers(
            &present,
            Some(&ids(&["claude-local"])),
            Some(&ids(&["claude-local", "codex-local"])),
            &[],
        );
        assert_eq!(selection.enabled, ids(&["codex@new", "claude-local"]));
        assert_eq!(selection.known, present);
    }

    #[test]
    fn an_absent_card_is_remembered_only_while_hidden() {
        let present = ids(&["claude-local"]);
        let selection = select_providers(
            &present,
            Some(&ids(&["claude-local", "codex@shown"])),
            Some(&ids(&["claude-local", "codex@shown", "codex@hidden"])),
            &[],
        );
        assert_eq!(selection.known, ids(&["claude-local", "codex@hidden"]));
        let back = ids(&["claude-local", "codex@shown", "codex@hidden"]);
        let returned =
            select_providers(&back, Some(&selection.enabled), Some(&selection.known), &[]);
        assert_eq!(returned.enabled, ids(&["claude-local", "codex@shown"]));
    }

    #[test]
    fn settings_from_before_the_record_use_the_fallback() {
        let present = ids(&["codex@cli", "claude-local", "codex-local"]);
        let enabled = ids(&["claude-local"]);
        let selection = select_providers(
            &present,
            Some(&enabled),
            None,
            &ids(&["claude-local", "codex-local"]),
        );
        assert_eq!(selection.enabled, ids(&["codex@cli", "claude-local"]));
        let unchanged = select_providers(&present, Some(&enabled), None, &present);
        assert_eq!(unchanged.enabled, enabled);
    }

    #[test]
    fn a_card_that_starts_hidden_waits_to_be_turned_on() {
        let ollama = format!("ollama@{}", "a".repeat(64));
        if !starts_hidden(&ollama) {
            return;
        }
        let present = vec![ollama.clone(), "claude-local".to_string()];
        let fresh = select_providers(&present, None, None, &[]);
        assert_eq!(fresh.enabled, ids(&["claude-local"]));
        let appeared = select_providers(
            &present,
            Some(&ids(&["claude-local"])),
            Some(&ids(&["claude-local"])),
            &[],
        );
        assert_eq!(appeared.enabled, ids(&["claude-local"]));
        assert_eq!(appeared.known, present);
        let turned_on = select_providers(&present, Some(&present), Some(&present), &[]);
        assert_eq!(turned_on.enabled, present);
        assert!(!starts_hidden("claude@abc"));
    }

    #[test]
    fn settings_lists_ignore_other_shapes() {
        let settings = serde_json::json!({"enabledProviders": ["a"], "knownProviders": "a"});
        assert_eq!(
            settings_list(Some(&settings), "enabledProviders"),
            Some(ids(&["a"]))
        );
        assert_eq!(settings_list(Some(&settings), "knownProviders"), None);
        assert_eq!(settings_list(None, "enabledProviders"), None);
    }
}
