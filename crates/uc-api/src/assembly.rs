use std::sync::Arc;

use serde_json::Value;
use uc_accounts::AccountStore;
use uc_core::{Clock, ProviderRuntime, SimpleProviderError};
use uc_engine::{Engine, EngineConfig, SnapshotCache};
use uc_logscan::{LocalHistoryRuntime, LogSource};
use uc_providers::CliAccount;

/// The providers the app shows: every connected account and every CLI login on this computer, then
/// this machine's Claude and Codex log history. The app and `usagectl` build the same set, so they
/// share one snapshot cache.
pub fn provider_runtimes(
    store: Arc<AccountStore>,
) -> Result<Vec<Arc<dyn ProviderRuntime>>, SimpleProviderError> {
    provider_runtimes_with(store, &uc_providers::cli_accounts())
}

/// [`provider_runtimes`] for CLI logins the caller already read.
pub fn provider_runtimes_with(
    store: Arc<AccountStore>,
    cli: &[CliAccount],
) -> Result<Vec<Arc<dyn ProviderRuntime>>, SimpleProviderError> {
    let mut runtimes = uc_providers::account_runtimes(store, cli)?;
    runtimes.push(Arc::new(LocalHistoryRuntime::new(LogSource::Claude)));
    runtimes.push(Arc::new(LocalHistoryRuntime::new(LogSource::Codex)));
    Ok(runtimes)
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
pub fn select_providers(
    ids: &[String],
    enabled: Option<&[String]>,
    known: Option<&[String]>,
    fallback_known: &[String],
) -> ProviderSelection {
    let known_before = known.unwrap_or(fallback_known);
    let Some(enabled) = enabled else {
        return ProviderSelection {
            enabled: ids.to_vec(),
            known: ids.to_vec(),
        };
    };
    let selected = ids
        .iter()
        .filter(|id| enabled.contains(id) || !known_before.contains(id))
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
