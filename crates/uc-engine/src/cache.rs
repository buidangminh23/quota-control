//! Persisted snapshot cache. Port of upstream `ProviderSnapshotCache.swift`.
//!
//! Every stored snapshot loads at launch for instant paint (stale-while-revalidate), but a snapshot
//! only gates a refresh as *fresh* when it was written during this running session and is younger
//! than one refresh interval. A one-shot reader (the CLI) opts into timestamp-only freshness.

use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
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
    disk_version: Option<DiskVersion>,
}

#[derive(PartialEq, Eq)]
struct DiskVersion {
    generation: u64,
    metadata: Option<(std::time::SystemTime, u64)>,
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
        if !self.refresh(&mut memo) {
            return None;
        }
        let snapshot = memo.payload.as_ref()?.snapshots.get(provider_id)?.clone();
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
        let mut lock = match self.lock(true) {
            Ok(lock) => lock,
            Err(error) => {
                tracing::warn!(target: "cache", "snapshot not persisted: {error}");
                return;
            }
        };
        let mut payload = match self.decode_stored() {
            Ok(payload) => payload,
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                memo.payload.clone().unwrap_or_default()
            }
            Err(error) => {
                tracing::warn!(target: "cache", "snapshot not persisted: {error}");
                return;
            }
        };
        Self::accept_payload(&mut memo, payload.clone());
        if payload
            .produced_by_identity_keys
            .get(&snapshot.provider_id)
            .map(String::as_str)
            == produced_by_identity_key
            && payload
                .snapshots
                .get(&snapshot.provider_id)
                .is_some_and(|stored| {
                    stored.refreshed_at > snapshot.refreshed_at
                        && stored.refreshed_at <= (self.clock)()
                })
        {
            return;
        }
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
        if self.save(&payload, &mut lock) {
            memo.payload = Some(payload);
            memo.disk_version = self.disk_version(&mut lock).ok();
            memo.session_writes.insert(snapshot.provider_id.clone());
        }
    }

    /// Replace a stored snapshot without touching its freshness or account stamp.
    pub fn replace_silently(&self, snapshot: &ProviderSnapshot) {
        let mut memo = self.memo.lock();
        let mut lock = match self.lock(true) {
            Ok(lock) => lock,
            Err(error) => {
                tracing::warn!(target: "cache", "snapshot not replaced: {error}");
                return;
            }
        };
        let mut payload = match self.decode_stored() {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(target: "cache", "snapshot not replaced: {error}");
                return;
            }
        };
        let id = &snapshot.provider_id;
        let unchanged = memo.payload.as_ref().is_some_and(|observed| {
            observed.snapshots.get(id) == payload.snapshots.get(id)
                && observed.produced_by_identity_keys.get(id)
                    == payload.produced_by_identity_keys.get(id)
        });
        Self::accept_payload(&mut memo, payload.clone());
        if !unchanged
            || !payload
                .snapshots
                .get(id)
                .is_some_and(|stored| stored.refreshed_at == snapshot.refreshed_at)
        {
            return;
        }
        payload.snapshots.insert(id.clone(), snapshot.clone());
        if self.save(&payload, &mut lock) {
            memo.payload = Some(payload);
            memo.disk_version = self.disk_version(&mut lock).ok();
        }
    }

    pub fn produced_by_identity_key(&self, provider_id: &str) -> Option<String> {
        let mut memo = self.memo.lock();
        self.refresh(&mut memo);
        memo.payload
            .as_ref()?
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
        let mut memo = self.memo.lock();
        if !self.refresh(&mut memo) {
            return true;
        }
        let Some(payload) = &memo.payload else {
            return false;
        };
        payload.snapshots.contains_key(provider_id)
            && payload
                .produced_by_identity_keys
                .get(provider_id)
                .map(String::as_str)
                != Some(current)
    }

    fn payload(&self) -> Payload {
        let mut memo = self.memo.lock();
        self.refresh(&mut memo);
        memo.payload.clone().unwrap_or_default()
    }

    fn lock(&self, exclusive: bool) -> std::io::Result<File> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut lock_path = self.path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(PathBuf::from(lock_path))?;
        if exclusive {
            lock.lock()?;
        } else {
            lock.lock_shared()?;
        }
        Ok(lock)
    }

    fn refresh(&self, memo: &mut CacheState) -> bool {
        let result = (|| {
            let mut lock = self.lock(false)?;
            let version = self.disk_version(&mut lock)?;
            if memo.disk_version.as_ref() == Some(&version) {
                return Ok(None);
            }
            let payload = self.decode_stored()?;
            Ok::<_, std::io::Error>(Some((payload, version)))
        })();
        match result {
            Ok(Some((payload, version))) => {
                Self::accept_payload(memo, payload);
                memo.disk_version = Some(version);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(target: "cache", "cache read failed, keeping last snapshot: {error}");
                return false;
            }
        }
        true
    }

    fn accept_payload(memo: &mut CacheState, payload: Payload) {
        if let Some(previous) = &memo.payload {
            memo.session_writes.retain(|id| {
                previous.snapshots.get(id) == payload.snapshots.get(id)
                    && previous.produced_by_identity_keys.get(id)
                        == payload.produced_by_identity_keys.get(id)
            });
        }
        memo.payload = Some(payload);
        memo.disk_version = None;
    }

    fn generation(lock: &mut File) -> std::io::Result<u64> {
        lock.seek(SeekFrom::Start(0))?;
        if lock.metadata()?.len() == 0 {
            return Ok(0);
        }
        let mut bytes = [0; 8];
        lock.read_exact(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn disk_version(&self, lock: &mut File) -> std::io::Result<DiskVersion> {
        let metadata = match std::fs::metadata(&self.path) {
            Ok(metadata) => Some((metadata.modified()?, metadata.len())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        Ok(DiskVersion {
            generation: Self::generation(lock)?,
            metadata,
        })
    }

    fn decode_stored(&self) -> std::io::Result<Payload> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Payload::default());
            }
            Err(error) => return Err(error),
        };
        serde_json::from_slice(&bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }

    fn save(&self, payload: &Payload, lock: &mut File) -> bool {
        let written = (|| {
            let bytes = serde_json::to_vec(payload).map_err(std::io::Error::other)?;
            let generation = Self::generation(lock)?.wrapping_add(1);
            lock.seek(SeekFrom::Start(0))?;
            lock.write_all(&generation.to_le_bytes())?;
            lock.sync_data()?;
            uc_core::paths::write_atomic(&self.path, &bytes)
        })();
        if let Err(error) = written {
            tracing::warn!(target: "cache", "snapshot not persisted: {error}");
            return false;
        }
        true
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

    #[test]
    fn independent_instances_merge_writes_and_refresh_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let app = SnapshotCache::with_options(
            path.clone(),
            Duration::from_secs(300),
            false,
            fixed_clock(now()),
        );
        let cli = SnapshotCache::with_options(
            path.clone(),
            Duration::from_secs(300),
            true,
            fixed_clock(now()),
        );
        let ids = ["claude".into(), "codex".into()];
        assert!(app.load_snapshots(&ids).is_empty());
        assert!(cli.load_snapshots(&ids).is_empty());
        cli.store(&snapshot("claude", 0, now()), Some("claude-user"));
        app.store(&snapshot("codex", 0, now()), Some("codex-user"));
        assert_eq!(app.load_snapshots(&ids), cli.load_snapshots(&ids));
        assert_eq!(cli.load_snapshots(&ids).len(), 2);
        assert_eq!(
            app.produced_by_identity_key("claude").as_deref(),
            Some("claude-user")
        );
        assert_eq!(
            cli.produced_by_identity_key("codex").as_deref(),
            Some("codex-user")
        );
        assert!(app.fresh_snapshot("claude").is_none());
        assert!(app.fresh_snapshot("codex").is_some());
        assert!(cli.fresh_snapshot("codex").is_some());
    }

    #[test]
    fn simultaneous_independent_instances_preserve_all_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let ids: Vec<String> = (0..16).map(|index| format!("provider-{index}")).collect();
        let barrier = std::sync::Barrier::new(ids.len());
        std::thread::scope(|scope| {
            for id in &ids {
                let path = &path;
                let barrier = &barrier;
                scope.spawn(move || {
                    let cache = SnapshotCache::new(path.clone(), Duration::from_secs(300));
                    assert!(cache.load_snapshots(std::slice::from_ref(id)).is_empty());
                    barrier.wait();
                    cache.store(&snapshot(id, 0, now()), Some(id));
                });
            }
        });
        let cache = SnapshotCache::new(path, Duration::from_secs(300));
        assert_eq!(cache.load_snapshots(&ids).len(), ids.len());
        for id in ids {
            assert_eq!(cache.produced_by_identity_key(&id), Some(id));
        }
    }

    #[test]
    fn external_identity_update_invalidates_session_freshness() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let app = SnapshotCache::with_options(
            path.clone(),
            Duration::from_secs(300),
            false,
            fixed_clock(now()),
        );
        let cli = SnapshotCache::new(path, Duration::from_secs(300));
        let value = snapshot("claude", 0, now());
        app.store(&value, Some("user-a"));
        assert!(app.fresh_snapshot("claude").is_some());
        cli.store(&value, Some("user-b"));
        assert!(app.has_stale_account_stamp("claude", Some("user-a")));
        assert_eq!(
            app.produced_by_identity_key("claude").as_deref(),
            Some("user-b")
        );
        assert!(app.fresh_snapshot("claude").is_none());
        cli.store(&value, None);
        assert_eq!(app.produced_by_identity_key("claude"), None);
    }

    #[test]
    fn delayed_store_cannot_replace_a_fresher_snapshot_for_the_same_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let first = SnapshotCache::new(path.clone(), Duration::from_secs(300));
        let second = SnapshotCache::new(path, Duration::from_secs(300));
        let latest = snapshot("claude", 0, now());
        first.store(&latest, Some("new-user"));
        second.store(&snapshot("claude", 1, now()), Some("new-user"));
        assert_eq!(second.load_snapshots(&["claude".into()])["claude"], latest);
        assert_eq!(
            second.produced_by_identity_key("claude").as_deref(),
            Some("new-user")
        );
    }

    #[test]
    fn silent_replace_preserves_other_entries_stamp_and_freshness() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let app = SnapshotCache::with_options(
            path.clone(),
            Duration::from_secs(300),
            false,
            fixed_clock(now()),
        );
        let cli = SnapshotCache::new(path.clone(), Duration::from_secs(300));
        let mut value = snapshot("claude", 0, now());
        app.store(&value, Some("user-a"));
        cli.store(&snapshot("codex", 0, now()), Some("user-b"));
        value.plan = Some("Updated plan".into());
        app.replace_silently(&value);
        assert_eq!(app.fresh_snapshot("claude"), Some(value.clone()));
        assert_eq!(
            app.produced_by_identity_key("claude").as_deref(),
            Some("user-a")
        );
        assert_eq!(
            app.load_snapshots(&["claude".into(), "codex".into()]).len(),
            2
        );
        let relaunched =
            SnapshotCache::with_options(path, Duration::from_secs(300), false, fixed_clock(now()));
        relaunched.load_snapshots(&["claude".into()]);
        value.plan = Some("Another plan".into());
        relaunched.replace_silently(&value);
        assert!(relaunched.fresh_snapshot("claude").is_none());
        assert_eq!(
            relaunched.load_snapshots(&["claude".into()])["claude"],
            value
        );
    }

    #[test]
    fn silent_replace_rejects_external_account_change_with_same_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let app = SnapshotCache::new(path.clone(), Duration::from_secs(300));
        let cli = SnapshotCache::new(path, Duration::from_secs(300));
        let mut value = snapshot("claude", 0, now());
        app.store(&value, Some("user-a"));
        cli.store(&value, Some("user-b"));
        let latest = value.clone();
        value.plan = Some("Old account plan".into());
        app.replace_silently(&value);
        assert_eq!(app.load_snapshots(&["claude".into()])["claude"], latest);
        assert_eq!(
            app.produced_by_identity_key("claude").as_deref(),
            Some("user-b")
        );
    }

    #[test]
    fn silent_replace_cannot_roll_back_or_extend_freshness() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let app = SnapshotCache::new(path.clone(), Duration::from_secs(300));
        let cli = SnapshotCache::new(path, Duration::from_secs(300));
        let old = snapshot("claude", 1, now());
        let latest = snapshot("claude", 0, now());
        app.store(&old, Some("user-a"));
        cli.store(&latest, Some("user-b"));
        app.replace_silently(&old);
        assert_eq!(app.load_snapshots(&["claude".into()])["claude"], latest);
        app.replace_silently(&snapshot("claude", -1, now()));
        assert_eq!(app.load_snapshots(&["claude".into()])["claude"], latest);
    }

    #[test]
    fn failed_cache_reads_preserve_last_display_but_cannot_skip_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cache = SnapshotCache::with_options(
            path.clone(),
            Duration::from_secs(300),
            false,
            fixed_clock(now()),
        );
        let value = snapshot("claude", 0, now());
        cache.store(&value, Some("user-a"));
        std::fs::write(&path, b"invalid JSON").unwrap();
        assert_eq!(cache.load_snapshots(&["claude".into()])["claude"], value);
        assert!(cache.fresh_snapshot("claude").is_none());
        assert!(cache.has_stale_account_stamp("claude", Some("user-a")));
        cache.store(&snapshot("codex", 0, now()), None);
        let recovered = cache.load_snapshots(&["claude".into(), "codex".into()]);
        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered["claude"], value);
        assert!(serde_json::from_slice::<Payload>(&std::fs::read(&path).unwrap()).is_ok());
    }

    #[test]
    fn account_switch_and_clock_rollback_can_replace_a_newer_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SnapshotCache::with_options(
            dir.path().join("cache.json"),
            Duration::from_secs(300),
            false,
            fixed_clock(now()),
        );
        cache.store(&snapshot("claude", 0, now()), Some("old-account"));
        let current = snapshot("claude", 1, now());
        cache.store(&current, Some("new-account"));
        assert_eq!(cache.load_snapshots(&["claude".into()])["claude"], current);
        assert_eq!(
            cache.produced_by_identity_key("claude").as_deref(),
            Some("new-account")
        );
        cache.store(&snapshot("claude", -1, now()), Some("new-account"));
        cache.store(&current, Some("new-account"));
        assert_eq!(cache.load_snapshots(&["claude".into()])["claude"], current);
    }

    #[test]
    fn failed_lock_does_not_overwrite_persisted_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cache = SnapshotCache::new(path.clone(), Duration::from_secs(300));
        cache.store(&snapshot("claude", 0, now()), Some("user-a"));
        let before = std::fs::read(&path).unwrap();
        let lock_path = dir.path().join("cache.json.lock");
        std::fs::remove_file(&lock_path).unwrap();
        std::fs::create_dir(&lock_path).unwrap();
        cache.store(&snapshot("codex", 0, now()), None);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(cache.fresh_snapshot("codex").is_none());
    }
}
