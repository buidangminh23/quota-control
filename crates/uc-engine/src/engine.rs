//! Refresh orchestration. Port of the refresh half of upstream `WidgetDataStore.swift` (the
//! presentation half, `data(for:)`, lives in the popup).
//!
//! - All enabled providers refresh concurrently: once at launch, then every refresh interval.
//! - A reading goes out of date when a limit window in it resets, so that provider is asked again a
//!   few seconds after the reset instead of at the next interval.
//! - A failed refresh never wipes data: the last good snapshot stays, the error is kept beside it.
//! - A failing provider is backed off for 60 s so a wake burst can't re-probe it in a tight loop.
//! - A provider that refuses a reading for asking too often is left alone for five minutes.
//! - A provider that never returns is abandoned after 120 s and reported as timed out.
//! - Scheduling compares wall-clock time, so a machine waking from sleep refreshes right away.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{Notify, broadcast};
use uc_core::{
    Clock, ErrorCategory, MetricLine, Provider, ProviderRuntime, ProviderSnapshot,
    ProviderUsageHistory, RefreshContext, UsageHistoryDescriptor, WidgetDescriptor, system_clock,
};

use crate::cache::SnapshotCache;

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub refresh_interval: Duration,
    pub provider_timeout: Duration,
    pub failure_backoff: Duration,
    /// How long a provider that refused a reading for asking too often is left alone.
    pub rate_limit_backoff: Duration,
    pub slow_threshold: Duration,
    /// How often the scheduler checks whether a batch is due.
    pub tick: Duration,
    /// How long after a limit window resets its provider is asked again, so the provider has
    /// rolled the window over too even when its clock runs a little behind this machine's.
    pub reset_settle: Duration,
    /// How soon a reading that still shows a reset which has already passed is asked again...
    pub reset_recheck: Duration,
    /// ...for up to this long after that reset.
    pub reset_recheck_window: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            refresh_interval: Duration::from_secs(5 * 60),
            provider_timeout: Duration::from_secs(120),
            failure_backoff: Duration::from_secs(60),
            rate_limit_backoff: Duration::from_secs(5 * 60),
            slow_threshold: Duration::from_secs(10),
            tick: Duration::from_secs(15),
            reset_settle: Duration::from_secs(5),
            reset_recheck: Duration::from_secs(30),
            reset_recheck_window: Duration::from_secs(2 * 60),
        }
    }
}

/// The scheduler's shortest sleep, so a reset it cannot act on yet never turns into a busy loop.
const MIN_PAUSE: Duration = Duration::from_secs(1);

fn delta(duration: Duration) -> chrono::Duration {
    chrono::Duration::from_std(duration).unwrap_or_else(|_| chrono::Duration::zero())
}

/// Which span of `every` on the clock `at` falls in. The spans are counted from 1970, so every
/// computer keeps the same ones.
fn span(at: DateTime<Utc>, every: Duration) -> i64 {
    let seconds = i64::try_from(every.as_secs()).unwrap_or(i64::MAX).max(1);
    at.timestamp().div_euclid(seconds)
}

/// When a limit window in `snapshot` puts it out of date: the first reset after the reading was
/// taken, plus `reset_settle`. A reading that already shows a passed reset (the provider had not
/// rolled that window over yet) is due `reset_recheck` after it was taken, for up to
/// `reset_recheck_window` after that reset.
fn reset_deadline(snapshot: &ProviderSnapshot, config: &EngineConfig) -> Option<DateTime<Utc>> {
    let read = snapshot.refreshed_at;
    snapshot
        .lines
        .iter()
        .filter_map(|line| match line {
            MetricLine::Progress(progress) => progress.resets_at,
            _ => None,
        })
        .filter_map(|reset| {
            if reset > read {
                reset.checked_add_signed(delta(config.reset_settle))
            } else if reset
                .checked_add_signed(delta(config.reset_recheck_window))
                .is_some_and(|end| read < end)
            {
                read.checked_add_signed(delta(config.reset_recheck))
            } else {
                None
            }
        })
        .min()
}

/// Re-renders a snapshot's spend rows from preserved daily history (upstream
/// `UsageHistorySnapshotRenderer`). Injected so the engine stays independent of the log scanners.
pub trait HistoryRenderer: Send + Sync {
    fn render(
        &self,
        snapshot: ProviderSnapshot,
        history: &ProviderUsageHistory,
        descriptor: &UsageHistoryDescriptor,
        now: DateTime<Utc>,
    ) -> ProviderSnapshot;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshOutcome {
    Refreshed,
    Failed,
    CacheHit,
    Skipped,
    BackedOff,
}

/// One provider the engine knows, with its widgets in declaration order.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEntry {
    pub provider: Provider,
    pub descriptors: Vec<WidgetDescriptor>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRuntimeState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<ProviderSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub refreshing: bool,
}

/// What the popup renders from: per-provider state plus the footer's schedule.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineState {
    pub providers: BTreeMap<String, ProviderRuntimeState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<DateTime<Utc>>,
    pub refresh_interval_ms: u64,
}

#[derive(Default)]
struct Inner {
    snapshots: HashMap<String, ProviderSnapshot>,
    refreshing: HashSet<String>,
    errors: HashMap<String, String>,
    /// Providers whose last answer refused the reading for asking too often.
    rate_limited: HashSet<String>,
    retry_after: HashMap<String, DateTime<Utc>>,
    /// When each provider was last asked, whatever came of it.
    attempted_at: HashMap<String, DateTime<Utc>>,
    last_refresh_at: Option<DateTime<Utc>>,
    enabled: HashSet<String>,
    batch_in_flight: bool,
}

/// Outcome hook (for logs and diagnostics): provider id, outcome, error category, manual.
pub type OutcomeHook = Arc<dyn Fn(&str, RefreshOutcome, Option<ErrorCategory>, bool) + Send + Sync>;

pub struct Engine {
    runtimes: Vec<Arc<dyn ProviderRuntime>>,
    by_id: HashMap<String, Arc<dyn ProviderRuntime>>,
    history_descriptors: HashMap<String, UsageHistoryDescriptor>,
    identity_keys: HashMap<String, String>,
    cache: SnapshotCache,
    config: EngineConfig,
    renderer: Option<Arc<dyn HistoryRenderer>>,
    clock: Clock,
    inner: Mutex<Inner>,
    events: broadcast::Sender<EngineState>,
    wake: Notify,
    outcome_hook: Mutex<Option<OutcomeHook>>,
}

struct BatchGuard<'a>(&'a Engine);

impl Drop for BatchGuard<'_> {
    fn drop(&mut self) {
        self.0.inner.lock().batch_in_flight = false;
        self.0.publish();
    }
}

struct RefreshGuard<'a> {
    engine: &'a Engine,
    provider_id: &'a str,
}

impl Drop for RefreshGuard<'_> {
    fn drop(&mut self) {
        self.engine.inner.lock().refreshing.remove(self.provider_id);
        self.engine.publish();
    }
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Engine {
    pub fn new(
        runtimes: Vec<Arc<dyn ProviderRuntime>>,
        cache: SnapshotCache,
        config: EngineConfig,
    ) -> Self {
        Self::with_options(
            runtimes,
            cache,
            config,
            HashMap::new(),
            None,
            system_clock(),
        )
    }

    pub fn with_options(
        runtimes: Vec<Arc<dyn ProviderRuntime>>,
        cache: SnapshotCache,
        config: EngineConfig,
        identity_keys: HashMap<String, String>,
        renderer: Option<Arc<dyn HistoryRenderer>>,
        clock: Clock,
    ) -> Self {
        let by_id: HashMap<_, _> = runtimes
            .iter()
            .map(|r| (r.provider().id.clone(), r.clone()))
            .collect();
        let history_descriptors = runtimes
            .iter()
            .filter_map(|runtime| {
                runtime
                    .widget_descriptors()
                    .into_iter()
                    .find_map(|d| d.history_resource)
                    .map(|descriptor| (runtime.provider().id.clone(), descriptor))
            })
            .collect();
        let ids: Vec<String> = runtimes.iter().map(|r| r.provider().id.clone()).collect();
        let loaded = cache
            .load_snapshots(&ids)
            .into_iter()
            .filter(|(id, _)| {
                let stale =
                    cache.has_stale_account_stamp(id, identity_keys.get(id).map(String::as_str));
                if stale {
                    tracing::info!(target: "cache", "stale account cache discarded for {id}");
                }
                !stale
            })
            .collect();
        let (events, _) = broadcast::channel(32);
        Self {
            runtimes,
            by_id,
            history_descriptors,
            identity_keys,
            cache,
            config,
            renderer,
            clock,
            inner: Mutex::new(Inner {
                snapshots: loaded,
                enabled: ids.into_iter().collect(),
                ..Inner::default()
            }),
            events,
            wake: Notify::new(),
            outcome_hook: Mutex::new(None),
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn set_outcome_hook(&self, hook: OutcomeHook) {
        *self.outcome_hook.lock() = Some(hook);
    }

    pub fn catalog(&self) -> Vec<ProviderEntry> {
        let inner = self.inner.lock();
        self.runtimes
            .iter()
            .map(|runtime| ProviderEntry {
                provider: runtime.provider().clone(),
                descriptors: runtime.descriptors_for(inner.snapshots.get(&runtime.provider().id)),
            })
            .collect()
    }

    pub fn provider_ids(&self) -> Vec<String> {
        self.runtimes
            .iter()
            .map(|r| r.provider().id.clone())
            .collect()
    }

    pub fn runtime(&self, provider_id: &str) -> Option<Arc<dyn ProviderRuntime>> {
        self.by_id.get(provider_id).cloned()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EngineState> {
        self.events.subscribe()
    }

    /// The popup's view: snapshots without their raw daily history (only spend rows are rendered).
    pub fn state(&self) -> EngineState {
        let inner = self.inner.lock();
        let providers = self
            .runtimes
            .iter()
            .map(|runtime| {
                let id = runtime.provider().id.clone();
                let snapshot = inner.snapshots.get(&id).cloned().map(|mut s| {
                    s.usage_history = None;
                    s
                });
                let state = ProviderRuntimeState {
                    snapshot,
                    error: inner.errors.get(&id).cloned(),
                    refreshing: inner.refreshing.contains(&id),
                };
                (id, state)
            })
            .collect();
        EngineState {
            providers,
            last_refresh_at: inner.last_refresh_at,
            refresh_interval_ms: self.config.refresh_interval.as_millis() as u64,
        }
    }

    /// Full snapshots including history, for the local API and CLI.
    pub fn snapshots(&self) -> HashMap<String, ProviderSnapshot> {
        self.inner.lock().snapshots.clone()
    }

    pub fn error_message(&self, provider_id: &str) -> Option<String> {
        self.inner.lock().errors.get(provider_id).cloned()
    }

    /// Whether the provider's last answer refused the reading for asking too often.
    pub fn rate_limited(&self, provider_id: &str) -> bool {
        self.inner.lock().rate_limited.contains(provider_id)
    }

    fn publish(&self) {
        let _ = self.events.send(self.state());
    }

    pub fn is_enabled(&self, provider_id: &str) -> bool {
        self.inner.lock().enabled.contains(provider_id)
    }

    /// Replace the enabled set. Newly enabled providers lose any stale backoff and fetch promptly.
    pub fn set_enabled(&self, provider_ids: &[String]) {
        let newly_enabled = {
            let mut inner = self.inner.lock();
            let next: HashSet<String> = provider_ids
                .iter()
                .filter(|id| self.by_id.contains_key(*id))
                .cloned()
                .collect();
            let added: Vec<String> = next.difference(&inner.enabled).cloned().collect();
            for id in &added {
                inner.retry_after.remove(id);
                inner.rate_limited.remove(id);
            }
            inner.enabled = next;
            !added.is_empty()
        };
        if newly_enabled {
            self.wake.notify_one();
        }
        self.publish();
    }

    /// Refresh every enabled provider concurrently.
    pub async fn refresh_all(&self, force: bool) -> Vec<RefreshOutcome> {
        let ids: Vec<String> = {
            let mut inner = self.inner.lock();
            let ids: Vec<String> = self
                .runtimes
                .iter()
                .map(|r| r.provider().id.clone())
                .filter(|id| inner.enabled.contains(id))
                .collect();
            if inner.batch_in_flight {
                return vec![RefreshOutcome::Skipped; ids.len()];
            }
            inner.batch_in_flight = true;
            ids
        };
        let _batch = BatchGuard(self);
        let started = Instant::now();
        tracing::info!(target: "refresh", "batch start ({} providers, force={force})", ids.len());
        let outcomes =
            futures::future::join_all(ids.iter().map(|id| self.refresh(id, force))).await;
        {
            let mut inner = self.inner.lock();
            inner.last_refresh_at = Some((self.clock)());
        }
        let count = |outcome: RefreshOutcome| outcomes.iter().filter(|o| **o == outcome).count();
        tracing::info!(
            target: "refresh",
            "batch end ({}ms, {} ok / {} failed / {} cached / {} backed off)",
            started.elapsed().as_millis(),
            count(RefreshOutcome::Refreshed),
            count(RefreshOutcome::Failed),
            count(RefreshOutcome::CacheHit),
            count(RefreshOutcome::BackedOff)
        );
        outcomes
    }

    /// The longest time a failed reading holds a provider back. A provider held back for longer
    /// was held back before the clock was set back.
    fn longest_backoff(&self) -> Duration {
        self.config
            .rate_limit_backoff
            .max(self.config.failure_backoff)
    }

    /// Refresh one provider. `force` bypasses the cache and the failure backoff; a reading that a
    /// limit window reset has put out of date bypasses the cache too.
    pub async fn refresh(&self, provider_id: &str, force: bool) -> RefreshOutcome {
        self.refresh_with(provider_id, force, None).await
    }

    /// Read one provider again, past its cached reading, unless it was read already in the span
    /// of `every` the clock is in. The spans are the same on every computer, so computers that
    /// watch one account read it at the same moments. A provider that asked to be left alone
    /// (rate limited, backing off) is not asked.
    pub async fn refresh_on_the_clock(&self, provider_id: &str, every: Duration) -> RefreshOutcome {
        self.refresh_with(provider_id, false, Some(every)).await
    }

    /// With `every`, a reading taken in the clock's current span of it decides; without it the
    /// cached reading does.
    async fn refresh_with(
        &self,
        provider_id: &str,
        force: bool,
        every: Option<Duration>,
    ) -> RefreshOutcome {
        let identity = self.identity_keys.get(provider_id).map(String::as_str);
        {
            let mut inner = self.inner.lock();
            if !inner.enabled.contains(provider_id) {
                return RefreshOutcome::Skipped;
            }
            if !self.by_id.contains_key(provider_id) || inner.refreshing.contains(provider_id) {
                return RefreshOutcome::Skipped;
            }
            let now = (self.clock)();
            if !force
                && let Some(retry_after) = inner.retry_after.get(provider_id).copied()
                && now < retry_after
            {
                if retry_after - now <= delta(self.longest_backoff()) {
                    tracing::debug!(target: "refresh", "backoff skip {provider_id}");
                    return RefreshOutcome::BackedOff;
                }
                tracing::info!(target: "refresh", "{provider_id} was held back before the clock was set back; asking again");
                inner.retry_after.remove(provider_id);
            }
            let stale_stamp = self.cache.has_stale_account_stamp(provider_id, identity);
            let window_reset = !force
                && self
                    .reset_refresh_at(&inner, provider_id)
                    .is_some_and(|due| due <= now);
            if let Some(every) = every
                && !stale_stamp
                && !window_reset
                && inner
                    .snapshots
                    .get(provider_id)
                    .is_some_and(|reading| span(reading.refreshed_at, every) == span(now, every))
            {
                tracing::debug!(target: "refresh", "{provider_id} was read in this span of {}s", every.as_secs());
                return RefreshOutcome::CacheHit;
            }
            if !force
                && every.is_none()
                && !stale_stamp
                && !window_reset
                && let Some(cached) = self.cache.fresh_snapshot(provider_id)
            {
                tracing::debug!(target: "refresh", "cache hit {provider_id}");
                if inner.snapshots.get(provider_id) != Some(&cached) {
                    inner.snapshots.insert(provider_id.to_string(), cached);
                }
                return RefreshOutcome::CacheHit;
            }
            if window_reset {
                tracing::info!(target: "refresh", "{provider_id} limit window reset, refreshing ahead of the interval");
            }
            inner.refreshing.insert(provider_id.to_string());
            inner.attempted_at.insert(provider_id.to_string(), now);
        }
        let _refresh = RefreshGuard {
            engine: self,
            provider_id,
        };
        self.publish();

        self.run_refresh(provider_id, force).await
    }

    async fn run_refresh(&self, provider_id: &str, force: bool) -> RefreshOutcome {
        let runtime = self.by_id[provider_id].clone();
        let context = if force {
            RefreshContext::manual()
        } else {
            RefreshContext::scheduled()
        };
        let started = Instant::now();
        let work = tokio::spawn(async move { runtime.refresh(context).await });
        let _abort = AbortOnDrop(work.abort_handle());
        let result = tokio::time::timeout(self.config.provider_timeout, work).await;
        let snapshot = match result {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(join_error)) => {
                let message = if join_error.is_panic() {
                    "Refresh crashed"
                } else {
                    "Refresh cancelled"
                };
                tracing::error!(target: "refresh", "{provider_id} refresh task failed: {join_error}");
                return self.record_failure(
                    provider_id,
                    message.to_string(),
                    Some(ErrorCategory::Other),
                    force,
                );
            }
            Err(_) => {
                let seconds = self.config.provider_timeout.as_secs();
                tracing::warn!(target: "refresh", "{provider_id} timed out after {seconds}s");
                return self.record_failure(
                    provider_id,
                    format!("Refresh timed out after {seconds}s"),
                    Some(ErrorCategory::Network),
                    force,
                );
            }
        };
        if snapshot.provider_id != provider_id {
            return self.record_failure(
                provider_id,
                "Refresh returned a snapshot for another provider".to_string(),
                Some(ErrorCategory::Other),
                force,
            );
        }
        let elapsed = started.elapsed();
        if elapsed >= self.config.slow_threshold {
            tracing::warn!(
                target: "refresh",
                "{provider_id} slow refresh ({}ms, threshold={}ms)",
                elapsed.as_millis(),
                self.config.slow_threshold.as_millis()
            );
        }
        if let Some(message) = Self::error_message_in(&snapshot) {
            return self.record_failure(provider_id, message, snapshot.error_category, force);
        }
        self.record_success(provider_id, snapshot, elapsed, force)
    }

    fn record_failure(
        &self,
        provider_id: &str,
        message: String,
        category: Option<ErrorCategory>,
        force: bool,
    ) -> RefreshOutcome {
        tracing::warn!(target: "refresh", "{provider_id} failed: {}", uc_core::redact::log_message(&message));
        {
            let mut inner = self.inner.lock();
            inner.errors.insert(provider_id.to_string(), message);
            let backoff = if category == Some(ErrorCategory::RateLimited) {
                inner.rate_limited.insert(provider_id.to_string());
                self.longest_backoff()
            } else {
                inner.rate_limited.remove(provider_id);
                self.config.failure_backoff
            };
            let retry = (self.clock)()
                + chrono::Duration::from_std(backoff).unwrap_or(chrono::Duration::seconds(60));
            inner.retry_after.insert(provider_id.to_string(), retry);
        }
        self.notify_outcome(provider_id, RefreshOutcome::Failed, category, force);
        RefreshOutcome::Failed
    }

    fn record_success(
        &self,
        provider_id: &str,
        mut snapshot: ProviderSnapshot,
        elapsed: Duration,
        force: bool,
    ) -> RefreshOutcome {
        {
            let mut inner = self.inner.lock();
            inner.errors.remove(provider_id);
            inner.rate_limited.remove(provider_id);
            inner.retry_after.remove(provider_id);
            if snapshot.usage_history.is_none() {
                let previous = inner
                    .snapshots
                    .get(provider_id)
                    .and_then(|s| s.usage_history.clone());
                if let (Some(history), Some(descriptor), Some(renderer)) = (
                    previous,
                    self.history_descriptors.get(provider_id),
                    self.renderer.as_ref(),
                ) {
                    snapshot.usage_history = Some(history.clone());
                    snapshot = renderer.render(snapshot, &history, descriptor, (self.clock)());
                    tracing::debug!(target: "refresh", "preserved last-good history for {provider_id} after scan miss");
                }
            }
            inner
                .snapshots
                .insert(provider_id.to_string(), snapshot.clone());
        }
        self.cache.store(
            &snapshot,
            self.identity_keys.get(provider_id).map(String::as_str),
        );
        tracing::info!(target: "refresh", "{provider_id} ok ({}ms)", elapsed.as_millis());
        self.notify_outcome(provider_id, RefreshOutcome::Refreshed, None, force);
        RefreshOutcome::Refreshed
    }

    fn notify_outcome(
        &self,
        provider_id: &str,
        outcome: RefreshOutcome,
        category: Option<ErrorCategory>,
        force: bool,
    ) {
        let hook = self.outcome_hook.lock().clone();
        if let Some(hook) = hook {
            hook(provider_id, outcome, category, force);
        }
    }

    /// A snapshot that carries only error lines is a failed refresh; its message comes from the badge.
    fn error_message_in(snapshot: &ProviderSnapshot) -> Option<String> {
        if snapshot.lines.is_empty() || !snapshot.lines.iter().all(|line| line.is_error()) {
            return None;
        }
        Some(
            snapshot
                .error_text()
                .unwrap_or("Refresh failed")
                .to_string(),
        )
    }

    /// Ask the scheduler to run a pass now (without forcing past the cache).
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    fn batch_due(&self) -> bool {
        let inner = self.inner.lock();
        if inner.batch_in_flight {
            return false;
        }
        match inner.last_refresh_at {
            None => true,
            Some(last) => {
                let elapsed = (self.clock)().signed_duration_since(last);
                elapsed.num_milliseconds() < 0
                    || elapsed.num_milliseconds() as u128
                        >= self.config.refresh_interval.as_millis()
            }
        }
    }

    /// When `provider_id` should be asked again because a limit window in its reading has reset:
    /// `None` while no window resets, while the provider is being asked, or once it has been asked
    /// since. A backed-off provider waits for its backoff.
    fn reset_refresh_at(&self, inner: &Inner, provider_id: &str) -> Option<DateTime<Utc>> {
        if inner.refreshing.contains(provider_id) {
            return None;
        }
        let deadline = reset_deadline(inner.snapshots.get(provider_id)?, &self.config)?;
        if inner
            .attempted_at
            .get(provider_id)
            .is_some_and(|attempt| *attempt >= deadline)
        {
            return None;
        }
        Some(match inner.retry_after.get(provider_id) {
            Some(retry) if *retry > deadline => *retry,
            _ => deadline,
        })
    }

    /// Ask every enabled provider whose reading a limit window reset has put out of date, ahead of
    /// the interval. The interval schedule stays as it was.
    async fn refresh_reset_windows(&self) -> Vec<RefreshOutcome> {
        let due: Vec<String> = {
            let inner = self.inner.lock();
            let now = (self.clock)();
            self.runtimes
                .iter()
                .map(|runtime| runtime.provider().id.clone())
                .filter(|id| inner.enabled.contains(id))
                .filter(|id| {
                    self.reset_refresh_at(&inner, id)
                        .is_some_and(|due| due <= now)
                })
                .collect()
        };
        futures::future::join_all(due.iter().map(|id| self.refresh(id, false))).await
    }

    /// How long the scheduler sleeps: one tick, or until the next limit window reset is due.
    fn pause(&self) -> Duration {
        let inner = self.inner.lock();
        let now = (self.clock)();
        inner
            .enabled
            .iter()
            .filter_map(|id| self.reset_refresh_at(&inner, id))
            .map(|due| (due - now).to_std().unwrap_or(Duration::ZERO))
            .fold(self.config.tick, Duration::min)
            .max(MIN_PAUSE)
    }

    /// The periodic loop: a pass at launch, then whenever one interval of wall-clock time has passed
    /// since the last batch finished, or when woken (a provider was just enabled). In between, each
    /// provider is asked again right after a limit window in its reading resets.
    pub async fn run(self: Arc<Self>) {
        loop {
            if self.batch_due() {
                self.refresh_all(false).await;
            } else {
                self.refresh_reset_windows().await;
            }
            tokio::select! {
                () = tokio::time::sleep(self.pause()) => {}
                () = self.wake.notified() => {
                    self.refresh_all(false).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use async_trait::async_trait;
    use uc_core::{MetricLine, ProgressFormat};

    struct FakeProvider {
        provider: Provider,
        calls: AtomicUsize,
        active: AtomicUsize,
        fail: AtomicBool,
        hang: AtomicBool,
    }

    impl FakeProvider {
        fn new(id: &str, fail: bool, hang: bool) -> Arc<Self> {
            Arc::new(Self {
                provider: Provider::new(id, id),
                calls: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                fail: AtomicBool::new(fail),
                hang: AtomicBool::new(hang),
            })
        }
    }

    struct ActiveCall<'a>(&'a AtomicUsize);

    impl Drop for ActiveCall<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl ProviderRuntime for FakeProvider {
        fn provider(&self) -> &Provider {
            &self.provider
        }

        fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
            vec![WidgetDescriptor::percent(
                format!("{}.session", self.provider.id),
                &self.provider,
                "Session",
                None,
                None,
            )]
        }

        async fn refresh(&self, _context: RefreshContext) -> ProviderSnapshot {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = ActiveCall(&self.active);
            if self.hang.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
            if self.fail.load(Ordering::SeqCst) {
                return ProviderSnapshot::error_message(
                    &self.provider,
                    "Not logged in",
                    Some(ErrorCategory::NotLoggedIn),
                );
            }
            let line = MetricLine::progress("Session", 10.0, 100.0, ProgressFormat::Percent).into();
            ProviderSnapshot::make(&self.provider, Some("Pro".into()), vec![line], Utc::now())
        }

        async fn has_local_credentials(&self) -> bool {
            true
        }
    }

    fn engine(
        runtimes: Vec<Arc<dyn ProviderRuntime>>,
        config: EngineConfig,
    ) -> (Engine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cache = SnapshotCache::new(dir.path().join("cache.json"), config.refresh_interval);
        (Engine::new(runtimes, cache, config), dir)
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// A clock the test moves by hand, shared by the engine, its cache and the provider.
    struct TestClock(Arc<Mutex<DateTime<Utc>>>);

    impl TestClock {
        fn starting(text: &str) -> Self {
            Self(Arc::new(Mutex::new(at(text))))
        }

        fn clock(&self) -> Clock {
            let now = self.0.clone();
            Arc::new(move || *now.lock())
        }

        fn set(&self, text: &str) {
            *self.0.lock() = at(text);
        }
    }

    /// A Claude-like provider whose five-hour Session window resets at a set moment.
    struct WindowProvider {
        provider: Provider,
        clock: Clock,
        used: Mutex<f64>,
        resets_at: Mutex<Option<DateTime<Utc>>>,
        fail: AtomicBool,
        refuse: AtomicBool,
        calls: AtomicUsize,
    }

    impl WindowProvider {
        fn new(clock: Clock, used: f64, resets_at: &str) -> Arc<Self> {
            Arc::new(Self {
                provider: Provider::new("claude", "Claude"),
                clock,
                used: Mutex::new(used),
                resets_at: Mutex::new(Some(at(resets_at))),
                fail: AtomicBool::new(false),
                refuse: AtomicBool::new(false),
                calls: AtomicUsize::new(0),
            })
        }

        fn roll_over(&self, used: f64, resets_at: &str) {
            *self.used.lock() = used;
            *self.resets_at.lock() = Some(at(resets_at));
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ProviderRuntime for WindowProvider {
        fn provider(&self) -> &Provider {
            &self.provider
        }

        fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
            Vec::new()
        }

        async fn refresh(&self, _context: RefreshContext) -> ProviderSnapshot {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.refuse.load(Ordering::SeqCst) {
                let mut snapshot = ProviderSnapshot::error_message(
                    &self.provider,
                    "Usage updates are rate limited. Try again later.",
                    Some(ErrorCategory::RateLimited),
                );
                snapshot.refreshed_at = (self.clock)();
                return snapshot;
            }
            if self.fail.load(Ordering::SeqCst) {
                let mut snapshot = ProviderSnapshot::error_message(
                    &self.provider,
                    "Network unavailable",
                    Some(ErrorCategory::Network),
                );
                snapshot.refreshed_at = (self.clock)();
                return snapshot;
            }
            let line =
                MetricLine::progress("Session", *self.used.lock(), 100.0, ProgressFormat::Percent)
                    .period_ms(Some(5 * 3_600_000))
                    .resets_at(*self.resets_at.lock())
                    .into();
            ProviderSnapshot::make(
                &self.provider,
                Some("Max 20x".into()),
                vec![line],
                (self.clock)(),
            )
        }

        async fn has_local_credentials(&self) -> bool {
            true
        }
    }

    fn clocked_engine(
        runtimes: Vec<Arc<dyn ProviderRuntime>>,
        clock: &TestClock,
    ) -> (Engine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let config = EngineConfig::default();
        let cache = SnapshotCache::with_options(
            dir.path().join("cache.json"),
            config.refresh_interval,
            false,
            clock.clock(),
        );
        let engine =
            Engine::with_options(runtimes, cache, config, HashMap::new(), None, clock.clock());
        (engine, dir)
    }

    fn session_used(engine: &Engine) -> f64 {
        match engine.snapshots()["claude"].line("Session") {
            Some(MetricLine::Progress(line)) => line.used,
            other => panic!("no Session meter: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_window_reset_is_read_again_seconds_after_it_instead_of_at_the_interval() {
        let clock = TestClock::starting("2026-09-27T06:09:08Z");
        let claude = WindowProvider::new(clock.clock(), 100.0, "2026-09-27T06:10:00.119Z");
        let (engine, _dir) = clocked_engine(vec![claude.clone()], &clock);
        assert_eq!(
            engine.refresh_all(false).await,
            vec![RefreshOutcome::Refreshed]
        );

        clock.set("2026-09-27T06:10:01Z");
        assert_eq!(
            engine.refresh_all(false).await,
            vec![RefreshOutcome::CacheHit]
        );
        assert_eq!(engine.pause(), Duration::from_millis(4_119));

        claude.roll_over(2.0, "2026-09-27T11:09:59.612Z");
        clock.set("2026-09-27T06:10:05.119Z");
        assert_eq!(
            engine.refresh_reset_windows().await,
            vec![RefreshOutcome::Refreshed]
        );
        assert_eq!(session_used(&engine), 2.0);
        assert_eq!(claude.calls(), 2);

        clock.set("2026-09-27T06:10:20Z");
        assert!(engine.refresh_reset_windows().await.is_empty());
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::CacheHit
        );
        assert_eq!(engine.pause(), EngineConfig::default().tick);
        assert_eq!(
            engine.state().last_refresh_at,
            Some(at("2026-09-27T06:10:01Z")),
            "a reset refresh leaves the interval schedule alone"
        );
    }

    #[tokio::test]
    async fn a_card_in_use_is_read_past_its_cache_but_not_past_a_backoff() {
        let clock = TestClock::starting("2026-09-29T02:30:00Z");
        let claude = WindowProvider::new(clock.clock(), 60.0, "2026-09-29T04:50:00Z");
        let (engine, _dir) = clocked_engine(vec![claude.clone()], &clock);
        engine.refresh_all(false).await;

        claude.roll_over(70.0, "2026-09-29T04:50:00Z");
        clock.set("2026-09-29T02:30:30Z");
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::CacheHit
        );
        assert_eq!(
            engine
                .refresh_on_the_clock("claude", Duration::from_secs(30))
                .await,
            RefreshOutcome::Refreshed
        );
        assert_eq!(session_used(&engine), 70.0);

        claude.fail.store(true, Ordering::SeqCst);
        clock.set("2026-09-29T02:31:00Z");
        assert_eq!(
            engine
                .refresh_on_the_clock("claude", Duration::from_secs(30))
                .await,
            RefreshOutcome::Failed
        );
        clock.set("2026-09-29T02:31:30Z");
        assert_eq!(
            engine
                .refresh_on_the_clock("claude", Duration::from_secs(30))
                .await,
            RefreshOutcome::BackedOff
        );
        assert_eq!(claude.calls(), 3);
        assert_eq!(session_used(&engine), 70.0, "the last good reading stays");
    }

    #[tokio::test]
    async fn a_card_refused_for_asking_too_often_is_left_alone_for_five_minutes() {
        let clock = TestClock::starting("2026-09-29T03:35:00Z");
        let claude = WindowProvider::new(clock.clock(), 60.0, "2026-09-29T07:50:00Z");
        let (engine, _dir) = clocked_engine(vec![claude.clone()], &clock);
        engine.refresh_all(false).await;
        assert!(!engine.rate_limited("claude"));

        claude.refuse.store(true, Ordering::SeqCst);
        clock.set("2026-09-29T03:38:02Z");
        assert_eq!(
            engine
                .refresh_on_the_clock("claude", Duration::from_secs(30))
                .await,
            RefreshOutcome::Failed
        );
        assert!(engine.rate_limited("claude"));

        for later in [
            "2026-09-29T03:39:03Z",
            "2026-09-29T03:41:00Z",
            "2026-09-29T03:43:01Z",
        ] {
            clock.set(later);
            assert_eq!(
                engine
                    .refresh_on_the_clock("claude", Duration::from_secs(30))
                    .await,
                RefreshOutcome::BackedOff
            );
            assert_eq!(
                engine.refresh("claude", false).await,
                RefreshOutcome::BackedOff
            );
        }
        assert_eq!(
            claude.calls(),
            2,
            "a refused card is not asked again within five minutes"
        );
        assert_eq!(session_used(&engine), 60.0, "the last good reading stays");

        claude.refuse.store(false, Ordering::SeqCst);
        claude.roll_over(64.0, "2026-09-29T07:50:00Z");
        clock.set("2026-09-29T03:43:02Z");
        assert_eq!(
            engine
                .refresh_on_the_clock("claude", Duration::from_secs(30))
                .await,
            RefreshOutcome::Refreshed
        );
        assert!(!engine.rate_limited("claude"));
        assert_eq!(session_used(&engine), 64.0);
    }

    #[tokio::test]
    async fn a_card_held_back_before_the_clock_was_set_back_is_asked_again() {
        let clock = TestClock::starting("2026-09-29T03:35:00Z");
        let claude = WindowProvider::new(clock.clock(), 60.0, "2026-09-29T07:50:00Z");
        let (engine, _dir) = clocked_engine(vec![claude.clone()], &clock);
        engine.refresh_all(false).await;
        claude.refuse.store(true, Ordering::SeqCst);
        clock.set("2026-09-29T03:38:00Z");
        let every = Duration::from_secs(30);
        assert_eq!(
            engine.refresh_on_the_clock("claude", every).await,
            RefreshOutcome::Failed
        );

        clock.set("2026-09-29T03:40:00Z");
        assert_eq!(
            engine.refresh_on_the_clock("claude", every).await,
            RefreshOutcome::BackedOff
        );
        assert_eq!(claude.calls(), 2);

        claude.refuse.store(false, Ordering::SeqCst);
        clock.set("2026-09-29T01:38:00Z");
        assert_eq!(
            engine.refresh_on_the_clock("claude", every).await,
            RefreshOutcome::Refreshed,
            "two hours of waiting were never asked for"
        );
        assert_eq!(claude.calls(), 3);
    }

    #[tokio::test]
    async fn a_card_is_read_once_in_each_span_of_the_clock() {
        let clock = TestClock::starting("2026-09-29T05:00:40Z");
        let claude = WindowProvider::new(clock.clock(), 60.0, "2026-09-29T07:50:00Z");
        let (engine, _dir) = clocked_engine(vec![claude.clone()], &clock);
        let every = Duration::from_secs(3 * 60);
        engine.refresh_all(false).await;

        claude.roll_over(61.0, "2026-09-29T07:50:00Z");
        clock.set("2026-09-29T05:02:59Z");
        assert_eq!(
            engine.refresh_on_the_clock("claude", every).await,
            RefreshOutcome::CacheHit,
            "it was read in the three minutes from 05:00 on"
        );
        assert_eq!(claude.calls(), 1);
        clock.set("2026-09-29T05:03:00Z");
        assert_eq!(
            engine.refresh_on_the_clock("claude", every).await,
            RefreshOutcome::Refreshed
        );
        assert_eq!(session_used(&engine), 61.0);

        clock.set("2026-09-29T05:05:30Z");
        assert_eq!(
            engine.refresh_all(false).await,
            vec![RefreshOutcome::CacheHit],
            "the interval keeps to its cache"
        );
        assert_eq!(claude.calls(), 2);
    }

    #[tokio::test]
    async fn a_card_switched_on_again_is_no_longer_held_as_refused() {
        let clock = TestClock::starting("2026-09-29T05:00:00Z");
        let claude = WindowProvider::new(clock.clock(), 60.0, "2026-09-29T07:50:00Z");
        let (engine, _dir) = clocked_engine(vec![claude.clone()], &clock);
        claude.refuse.store(true, Ordering::SeqCst);
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::Failed
        );
        assert!(engine.rate_limited("claude"));

        engine.set_enabled(&[]);
        engine.set_enabled(&["claude".to_string()]);
        assert!(!engine.rate_limited("claude"));
    }

    #[tokio::test]
    async fn another_failure_after_a_refusal_keeps_the_short_backoff() {
        let clock = TestClock::starting("2026-09-29T03:35:00Z");
        let claude = WindowProvider::new(clock.clock(), 60.0, "2026-09-29T07:50:00Z");
        let (engine, _dir) = clocked_engine(vec![claude.clone()], &clock);
        claude.refuse.store(true, Ordering::SeqCst);
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::Failed
        );
        assert!(engine.rate_limited("claude"));

        claude.refuse.store(false, Ordering::SeqCst);
        claude.fail.store(true, Ordering::SeqCst);
        clock.set("2026-09-29T03:40:00Z");
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::Failed
        );
        assert!(
            !engine.rate_limited("claude"),
            "the network failed; nothing was refused"
        );
        clock.set("2026-09-29T03:40:59Z");
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::BackedOff
        );
        clock.set("2026-09-29T03:41:00Z");
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::Failed
        );
        assert_eq!(claude.calls(), 3);
    }

    #[tokio::test]
    async fn a_failed_reset_read_falls_back_to_the_interval() {
        let clock = TestClock::starting("2026-09-27T06:09:08Z");
        let claude = WindowProvider::new(clock.clock(), 100.0, "2026-09-27T06:10:00Z");
        let (engine, _dir) = clocked_engine(vec![claude.clone()], &clock);
        engine.refresh_all(false).await;

        claude.fail.store(true, Ordering::SeqCst);
        clock.set("2026-09-27T06:10:05Z");
        assert_eq!(
            engine.refresh_reset_windows().await,
            vec![RefreshOutcome::Failed]
        );
        assert_eq!(session_used(&engine), 100.0, "the last good reading stays");

        clock.set("2026-09-27T06:11:10Z");
        assert!(engine.refresh_reset_windows().await.is_empty());
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::CacheHit
        );
        assert_eq!(engine.pause(), EngineConfig::default().tick);

        claude.fail.store(false, Ordering::SeqCst);
        claude.roll_over(0.0, "2026-09-27T11:14:08Z");
        clock.set("2026-09-27T06:14:08Z");
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::Refreshed
        );
        assert_eq!(claude.calls(), 3);
    }

    #[tokio::test]
    async fn a_reading_that_still_shows_a_passed_reset_is_asked_again_for_two_minutes() {
        let clock = TestClock::starting("2026-09-27T06:09:08Z");
        let claude = WindowProvider::new(clock.clock(), 100.0, "2026-09-27T06:10:00Z");
        let (engine, _dir) = clocked_engine(vec![claude.clone()], &clock);
        engine.refresh_all(false).await;

        for asked in [
            "2026-09-27T06:10:05Z",
            "2026-09-27T06:10:35Z",
            "2026-09-27T06:11:05Z",
            "2026-09-27T06:11:35Z",
            "2026-09-27T06:12:05Z",
        ] {
            clock.set(asked);
            assert_eq!(
                engine.refresh_reset_windows().await,
                vec![RefreshOutcome::Refreshed],
                "asked at {asked}"
            );
        }
        clock.set("2026-09-27T06:12:35Z");
        assert!(engine.refresh_reset_windows().await.is_empty());
        assert_eq!(claude.calls(), 6);
    }

    #[tokio::test]
    async fn a_backed_off_provider_waits_for_its_backoff_before_a_reset_read() {
        let clock = TestClock::starting("2026-09-27T06:09:08Z");
        let claude = WindowProvider::new(clock.clock(), 100.0, "2026-09-27T06:10:00Z");
        let (engine, _dir) = clocked_engine(vec![claude.clone()], &clock);
        engine.refresh_all(false).await;

        claude.fail.store(true, Ordering::SeqCst);
        clock.set("2026-09-27T06:09:40Z");
        assert_eq!(engine.refresh("claude", true).await, RefreshOutcome::Failed);
        claude.fail.store(false, Ordering::SeqCst);

        clock.set("2026-09-27T06:10:05Z");
        assert!(engine.refresh_reset_windows().await.is_empty());
        assert_eq!(engine.pause(), Duration::from_secs(15));
        clock.set("2026-09-27T06:10:30Z");
        assert_eq!(engine.pause(), Duration::from_secs(10));
        clock.set("2026-09-27T06:10:40Z");
        assert_eq!(
            engine.refresh_reset_windows().await,
            vec![RefreshOutcome::Refreshed]
        );
    }

    #[tokio::test]
    async fn success_is_cached_for_the_interval() {
        let fake = FakeProvider::new("claude", false, false);
        let (engine, _dir) = engine(vec![fake.clone()], EngineConfig::default());
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::Refreshed
        );
        assert_eq!(
            engine.refresh("claude", false).await,
            RefreshOutcome::CacheHit
        );
        assert_eq!(
            engine.refresh("claude", true).await,
            RefreshOutcome::Refreshed
        );
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
        let state = engine.state();
        assert_eq!(
            state.providers["claude"]
                .snapshot
                .as_ref()
                .unwrap()
                .plan
                .as_deref(),
            Some("Pro")
        );
    }

    #[tokio::test]
    async fn failures_keep_last_good_snapshot_and_back_off() {
        let fake = FakeProvider::new("codex", false, false);
        let (engine, _dir) = engine(vec![fake.clone()], EngineConfig::default());
        assert_eq!(
            engine.refresh("codex", false).await,
            RefreshOutcome::Refreshed
        );
        let last_good = engine.snapshots()["codex"].clone();
        fake.fail.store(true, Ordering::SeqCst);
        assert_eq!(engine.refresh("codex", true).await, RefreshOutcome::Failed);
        assert_eq!(
            engine.refresh("codex", false).await,
            RefreshOutcome::BackedOff
        );
        assert_eq!(engine.refresh("codex", true).await, RefreshOutcome::Failed);
        assert_eq!(
            engine.state().providers["codex"].error.as_deref(),
            Some("Not logged in")
        );
        assert_eq!(
            engine.state().providers["codex"].snapshot.as_ref(),
            Some(&last_good)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn hung_providers_time_out() {
        let fake = FakeProvider::new("cursor", false, true);
        let config = EngineConfig {
            provider_timeout: Duration::from_secs(120),
            ..EngineConfig::default()
        };
        let (engine, _dir) = engine(vec![fake], config);
        assert_eq!(engine.refresh("cursor", true).await, RefreshOutcome::Failed);
        let state = engine.state();
        assert_eq!(
            state.providers["cursor"].error.as_deref(),
            Some("Refresh timed out after 120s")
        );
        assert!(!state.providers["cursor"].refreshing);
    }

    #[tokio::test]
    async fn disabled_providers_are_skipped() {
        let fake = FakeProvider::new("grok", false, false);
        let (engine, _dir) = engine(vec![fake.clone()], EngineConfig::default());
        engine.set_enabled(&[]);
        assert_eq!(engine.refresh("grok", true).await, RefreshOutcome::Skipped);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn state_strips_usage_history() {
        let fake = FakeProvider::new("claude", false, false);
        let (engine, _dir) = engine(vec![fake], EngineConfig::default());
        engine.refresh_all(false).await;
        let state = engine.state();
        assert!(state.last_refresh_at.is_some());
        assert!(
            state.providers["claude"]
                .snapshot
                .as_ref()
                .unwrap()
                .usage_history
                .is_none()
        );
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["refreshIntervalMs"], 300_000);
    }

    #[tokio::test(start_paused = true)]
    async fn overlapping_batches_do_not_clear_the_running_batch() {
        let fake = FakeProvider::new("codex", false, true);
        let (engine, _dir) = engine(vec![fake.clone()], EngineConfig::default());
        let engine = Arc::new(engine);
        let running = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.refresh_all(true).await })
        };
        while fake.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            engine.refresh_all(true).await,
            vec![RefreshOutcome::Skipped]
        );
        assert!(engine.inner.lock().batch_in_flight);
        assert!(engine.state().last_refresh_at.is_none());
        assert!(!engine.batch_due());
        running.abort();
        assert!(running.await.unwrap_err().is_cancelled());
        tokio::task::yield_now().await;
        assert!(!engine.inner.lock().batch_in_flight);
        assert!(!engine.state().providers["codex"].refreshing);
        assert_eq!(fake.active.load(Ordering::SeqCst), 0);
        assert!(engine.batch_due());
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_refresh_releases_provider_and_aborts_work() {
        let fake = FakeProvider::new("codex", false, true);
        let (engine, _dir) = engine(vec![fake.clone()], EngineConfig::default());
        let engine = Arc::new(engine);
        let running = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.refresh("codex", true).await })
        };
        while fake.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        running.abort();
        assert!(running.await.unwrap_err().is_cancelled());
        tokio::task::yield_now().await;
        assert!(!engine.state().providers["codex"].refreshing);
        assert_eq!(fake.active.load(Ordering::SeqCst), 0);
        fake.hang.store(false, Ordering::SeqCst);
        assert_eq!(
            engine.refresh("codex", true).await,
            RefreshOutcome::Refreshed
        );
    }
}
