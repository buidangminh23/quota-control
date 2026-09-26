use std::sync::Arc;

use uc_accounts::AccountStore;
use uc_core::{Clock, ProviderRuntime, SimpleProviderError};
use uc_engine::{Engine, EngineConfig, SnapshotCache};
use uc_logscan::{LocalHistoryRuntime, LogSource};

/// The providers the app shows: every connected account, then this machine's Claude and Codex log
/// history. The app and `usagectl` build the same set, so they share one snapshot cache.
pub fn provider_runtimes(
    store: Arc<AccountStore>,
) -> Result<Vec<Arc<dyn ProviderRuntime>>, SimpleProviderError> {
    let mut runtimes = uc_providers::managed_runtimes(store)?;
    runtimes.push(Arc::new(LocalHistoryRuntime::new(LogSource::Claude)));
    runtimes.push(Arc::new(LocalHistoryRuntime::new(LogSource::Codex)));
    Ok(runtimes)
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
