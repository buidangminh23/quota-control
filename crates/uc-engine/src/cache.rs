//! Persisted snapshot cache. Port of upstream `ProviderSnapshotCache.swift`.
//!
//! Every stored snapshot loads at launch for instant paint (stale-while-revalidate), but a snapshot
//! only gates a refresh as *fresh* when it was written during this running session and is younger
//! than one refresh interval. A one-shot reader (the CLI) opts into timestamp-only freshness.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uc_core::{Clock, ProviderSnapshot, system_clock};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    snapshots: BTreeMap<String, ProviderSnapshot>,
    /// Card id → the account identity key that produced the stored snapshot.
    #[serde(default)]
    produced_by_identity_keys: BTreeMap<String, String>,
}

#[derive(Default)]
struct CacheState {
    payload: Option<Payload>,
    session_writes: HashSet<String>,
}

#[derive(Clone)]
pub struct SnapshotCache {
    path: PathBuf,
    ttl: Duration,
    allows_persisted_freshness: bool,
    clock: Clock,
    memo: Arc<Mutex<CacheState>>,
}

impl SnapshotCache {
    /// Bump the file version when the snapshot shape changes so stale caches are dropped once.
    pub const FILE_NAME: &'static str = "provider-snapshots.v1.json";

    pub fn new(path: PathBuf, ttl: Duration) -> Self {
        Self::with_options(path, ttl, false, system_clock())
    }

    pub fn default_path() -> PathBuf {
        uc_core::paths::cache_dir().join(Self::FILE_NAME)
    }

    pub fn with_options(
        path: PathBuf,
        ttl: Duration,
        allows_persisted_freshness: bool,
        clock: Clock,
    ) -> Self {
        Self {
            path,
            ttl,
            allows_persisted_freshness,
            clock,
            memo: Arc::new(Mutex::new(CacheState::default())),
        }
    }

    /// Every stored snapshot for the given providers, including expired ones (for display).
    pub fn load_snapshots(&self, provider_ids: &[String]) -> BTreeMap<String, ProviderSnapshot> {
        let wanted: HashSet<&String> = provider_ids.iter().collect();
        let payload = self.payload();
        let loaded: BTreeMap<_, _> = payload
            .snapshots
            .into_iter()
            .filter(|(id, _)| wanted.contains(id))
            .collect();
        tracing::debug!(target: "cache", "loaded {} snapshots from disk", loaded.len());
        loaded
    }

    /// The snapshot only when it is fresh enough to skip a refresh.
    pub fn fresh_snapshot(&self, provider_id: &str) -> Option<ProviderSnapshot> {
        let mut memo = self.memo.lock();
        let snapshot = memo
            .payload
            .get_or_insert_with(|| self.decode_stored())
            .snapshots
            .get(provider_id)?
            .clone();
        let written_this_session = memo.session_writes.contains(provider_id);
        let age = (self.clock)().signed_duration_since(snapshot.refreshed_at);
        let trusted = self.allows_persisted_freshness || written_this_session;
        let fresh = trusted
            && age.num_milliseconds() >= 0
            && (age.num_milliseconds() as u128) < self.ttl.as_millis();
        tracing::debug!(
            target: "cache",
            "{provider_id} staleness {}s vs ttl {}s -> {}",
            age.num_seconds(),
            self.ttl.as_secs(),
            if !trusted { "stale (not written this session)" } else if fresh { "fresh" } else { "stale" }
        );
        fresh.then_some(snapshot)
    }

    /// Store a successful snapshot. Error snapshots are never cached.
    pub fn store(&self, snapshot: &ProviderSnapshot, produced_by_identity_key: Option<&str>) {
        if snapshot.lines.iter().any(|line| line.is_error()) {
            tracing::debug!(target: "cache", "skip write {} (error snapshot)", snapshot.provider_id);
            return;
        }
        let mut memo = self.memo.lock();
        memo.session_writes.insert(snapshot.provider_id.clone());
        let payload = memo.payload.get_or_insert_with(|| self.decode_stored());
        payload
            .snapshots
            .insert(snapshot.provider_id.clone(), snapshot.clone());
        match produced_by_identity_key {
            Some(key) => {
                payload
                    .produced_by_identity_keys
                    .insert(snapshot.provider_id.clone(), key.to_string());
            }
            None => {
                payload
                    .produced_by_identity_keys
                    .remove(&snapshot.provider_id);
            }
        }
        self.save(payload);
    }

    /// Replace a stored snapshot without touching its freshness or account stamp.
    pub fn replace_silently(&self, snapshot: &ProviderSnapshot) {
        let mut memo = self.memo.lock();
        let payload = memo.payload.get_or_insert_with(|| self.decode_stored());
        if payload.snapshots.contains_key(&snapshot.provider_id) {
            payload
                .snapshots
                .insert(snapshot.provider_id.clone(), snapshot.clone());
            self.save(payload);
        }
    }

    pub fn produced_by_identity_key(&self, provider_id: &str) -> Option<String> {
        self.payload()
            .produced_by_identity_keys
            .get(provider_id)
            .cloned()
    }

    /// True when the stored entry exists, the card's current identity is known, and the stamp is
    /// missing or names another account.
    pub fn has_stale_account_stamp(
        &self,
        provider_id: &str,
        current_identity_key: Option<&str>,
    ) -> bool {
        let Some(current) = current_identity_key else {
            return false;
        };
        let payload = self.payload();
        payload.snapshots.contains_key(provider_id)
            && payload
                .produced_by_identity_keys
                .get(provider_id)
                .map(String::as_str)
                != Some(current)
    }

    fn payload(&self) -> Payload {
        let mut memo = self.memo.lock();
        memo.payload
            .get_or_insert_with(|| self.decode_stored())
            .clone()
    }

    fn decode_stored(&self) -> Payload {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Payload::default();
            }
            Err(error) => {
                tracing::warn!(target: "cache", "cache read failed, starting empty: {error}");
                return Payload::default();
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(target: "cache", "cache decode failed, dropping stored snapshots: {error}");
                Payload::default()
            }
        }
    }

    fn save(&self, payload: &Payload) {
        let written = serde_json::to_vec(payload)
            .map_err(std::io::Error::other)
            .and_then(|bytes| uc_core::paths::write_atomic(&self.path, &bytes));
        if let Err(error) = written {
            tracing::warn!(target: "cache", "snapshot not persisted: {error}");
        }
    }

    /// Age helper for tests and diagnostics.
    pub fn now(&self) -> chrono::DateTime<Utc> {
        (self.clock)()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use uc_core::{MetricLine, Provider, fixed_clock};

    fn snapshot(id: &str, minutes_ago: i64, now: chrono::DateTime<Utc>) -> ProviderSnapshot {
        let provider = Provider::new(id, id);
        ProviderSnapshot::make(
            &provider,
            None,
            vec![MetricLine::no_usage_data()],
            now - chrono::Duration::minutes(minutes_ago),
        )
    }

    fn now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 25, 12, 0, 0).unwrap()
    }

    #[test]
    fn launch_loaded_snapshots_are_never_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let first = SnapshotCache::with_options(
            path.clone(),
            Duration::from_secs(300),
            false,
            fixed_clock(now()),
        );
        first.store(&snapshot("claude", 1, now()), None);
        assert!(first.fresh_snapshot("claude").is_some());

        let relaunched =
            SnapshotCache::with_options(path, Duration::from_secs(300), false, fixed_clock(now()));
        assert!(relaunched.fresh_snapshot("claude").is_none());
        assert_eq!(relaunched.load_snapshots(&["claude".into()]).len(), 1);
    }

    #[test]
    fn persisted_freshness_trusts_disk_age() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        SnapshotCache::with_options(
            path.clone(),
            Duration::from_secs(300),
            false,
            fixed_clock(now()),
        )
        .store(&snapshot("codex", 2, now()), None);
        let cli =
            SnapshotCache::with_options(path, Duration::from_secs(300), true, fixed_clock(now()));
        assert!(cli.fresh_snapshot("codex").is_some());
    }

    #[test]
    fn expired_snapshots_are_not_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SnapshotCache::with_options(
            dir.path().join("c.json"),
            Duration::from_secs(300),
            false,
            fixed_clock(now()),
        );
        cache.store(&snapshot("cursor", 6, now()), None);
        assert!(cache.fresh_snapshot("cursor").is_none());
    }

    #[test]
    fn error_snapshots_are_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SnapshotCache::new(dir.path().join("c.json"), Duration::from_secs(300));
        let provider = Provider::new("grok", "Grok");
        cache.store(
            &ProviderSnapshot::error_message(&provider, "Not logged in", None),
            None,
        );
        assert!(cache.load_snapshots(&["grok".into()]).is_empty());
    }

    #[test]
    fn account_stamps_detect_swaps() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SnapshotCache::new(dir.path().join("c.json"), Duration::from_secs(300));
        cache.store(&snapshot("claude", 0, Utc::now()), Some("user-a|org"));
        assert!(!cache.has_stale_account_stamp("claude", Some("user-a|org")));
        assert!(cache.has_stale_account_stamp("claude", Some("user-b|org")));
        assert!(!cache.has_stale_account_stamp("claude", None));
    }

    #[test]
    fn concurrent_stores_preserve_every_provider_and_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cache = SnapshotCache::new(path.clone(), Duration::from_secs(300));
        let barrier = std::sync::Barrier::new(16);
        std::thread::scope(|scope| {
            for index in 0..16 {
                let cache = &cache;
                let barrier = &barrier;
                scope.spawn(move || {
                    let id = format!("provider-{index}");
                    barrier.wait();
                    cache.store(&snapshot(&id, 0, now()), Some(&id));
                });
            }
        });
        let ids: Vec<String> = (0..16).map(|index| format!("provider-{index}")).collect();
        assert_eq!(cache.load_snapshots(&ids).len(), ids.len());
        let persisted = SnapshotCache::new(path, Duration::from_secs(300));
        assert_eq!(persisted.load_snapshots(&ids).len(), ids.len());
        for id in ids {
            assert_eq!(persisted.produced_by_identity_key(&id), Some(id));
        }
    }

    #[test]
    fn cloned_caches_share_concurrent_updates_and_session_freshness() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cache = SnapshotCache::with_options(
            path.clone(),
            Duration::from_secs(300),
            false,
            fixed_clock(now()),
        );
        let observer = cache.clone();
        let ids: Vec<String> = (0..16).map(|index| format!("provider-{index}")).collect();
        assert!(observer.load_snapshots(&ids).is_empty());
        let barrier = std::sync::Barrier::new(ids.len());
        std::thread::scope(|scope| {
            for id in &ids {
                let writer = cache.clone();
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    writer.store(&snapshot(id, 0, now()), Some(id));
                });
            }
        });
        assert_eq!(observer.load_snapshots(&ids).len(), ids.len());
        let persisted =
            SnapshotCache::with_options(path, Duration::from_secs(300), false, fixed_clock(now()));
        assert_eq!(
            persisted.load_snapshots(&ids),
            observer.load_snapshots(&ids)
        );
        for id in ids {
            assert!(observer.fresh_snapshot(&id).is_some());
            assert_eq!(
                observer.produced_by_identity_key(&id).as_deref(),
                Some(id.as_str())
            );
            assert_eq!(
                persisted.produced_by_identity_key(&id).as_deref(),
                Some(id.as_str())
            );
            assert!(persisted.fresh_snapshot(&id).is_none());
        }
    }
}
