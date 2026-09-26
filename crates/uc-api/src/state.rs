use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use uc_core::{Provider, ProviderSnapshot, WidgetDescriptor};
use uc_engine::Engine;

/// Everything one request needs, captured from the engine when the request arrives.
#[derive(Clone, Debug)]
pub struct ApiState {
    /// What the collection routes serve: enabled providers in the dashboard's order.
    pub enabled_ordered_ids: Vec<String>,
    /// Every provider the engine knows; single-provider routes serve disabled ones too.
    pub known_ids: BTreeSet<String>,
    /// Last good snapshot per provider, without raw daily history.
    pub snapshots: HashMap<String, ProviderSnapshot>,
    /// Per provider, only the descriptors that export at least one limit resource. A provider
    /// absent here has nothing to report on `/v1/limits` (local log history cards).
    pub limit_descriptors: HashMap<String, Vec<WidgetDescriptor>>,
    /// The latest refresh failure per provider; the last good snapshot stays beside it.
    pub errors: HashMap<String, String>,
    pub generated_at: DateTime<Utc>,
    /// A limits entry expires this long after it was fetched.
    pub refresh_interval: Duration,
}

impl ApiState {
    /// `saved_order` is the dashboard's provider order; providers it misses follow in catalog order.
    pub fn capture(engine: &Engine, saved_order: &[String], now: DateTime<Utc>) -> Self {
        let catalog = engine.catalog();
        let known_ids: BTreeSet<String> = catalog
            .iter()
            .map(|entry| entry.provider.id.clone())
            .collect();
        let mut seen = HashSet::new();
        let enabled_ordered_ids = saved_order
            .iter()
            .chain(catalog.iter().map(|entry| &entry.provider.id))
            .filter(|id| known_ids.contains(*id) && seen.insert((*id).clone()))
            .filter(|id| engine.is_enabled(id))
            .cloned()
            .collect();
        let limit_descriptors = catalog
            .into_iter()
            .filter_map(|entry| {
                let descriptors: Vec<WidgetDescriptor> = entry
                    .descriptors
                    .into_iter()
                    .filter(|descriptor| !descriptor.limit_resources.is_empty())
                    .collect();
                (!descriptors.is_empty()).then_some((entry.provider.id, descriptors))
            })
            .collect();
        let snapshots = engine
            .snapshots()
            .into_iter()
            .map(|(id, mut snapshot)| {
                snapshot.usage_history = None;
                (id, snapshot)
            })
            .collect();
        let errors = known_ids
            .iter()
            .filter_map(|id| {
                engine
                    .error_message(id)
                    .map(|message| (id.clone(), message))
            })
            .collect();
        Self {
            enabled_ordered_ids,
            known_ids,
            snapshots,
            limit_descriptors,
            errors,
            generated_at: now,
            refresh_interval: engine.config().refresh_interval,
        }
    }

    /// Every known provider `token` names: an exact id, or a family naming all of its cards
    /// (`claude` names every `claude@…` card). Plain string matching, never resolved from runtime
    /// state, so the same token always names the same cards. Empty when it names nothing.
    pub fn matching_ids(&self, token: &str) -> Vec<String> {
        matching_ids(self.known_ids.iter(), token)
    }
}

pub(crate) fn matching_ids<'a>(
    known_ids: impl IntoIterator<Item = &'a String>,
    token: &str,
) -> Vec<String> {
    let mut ids: Vec<String> = known_ids
        .into_iter()
        .filter(|id| id.as_str() == token || Provider::family_of(id) == token)
        .cloned()
        .collect();
    ids.sort();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_name_exact_cards_or_whole_families() {
        let known: BTreeSet<String> = ["claude@b", "claude@a", "claude-local", "codex@c"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(matching_ids(&known, "claude"), ["claude@a", "claude@b"]);
        assert_eq!(matching_ids(&known, "claude-local"), ["claude-local"]);
        assert_eq!(matching_ids(&known, "codex@c"), ["codex@c"]);
        assert!(matching_ids(&known, "cursor").is_empty());
        assert!(matching_ids(&known, "claud").is_empty());
    }
}
