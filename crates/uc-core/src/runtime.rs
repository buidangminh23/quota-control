//! The provider runtime contract. Port of upstream `ProviderRuntime.swift`.
//!
//! A provider reads credentials already on the machine (auth store), calls the provider's API (usage
//! client) and normalizes the response (mapper) into a `ProviderSnapshot` of `MetricLine` values.

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::{ErrorCategory, SimpleProviderError};
use crate::model::{Provider, ProviderSnapshot, WidgetDescriptor};

/// Per-refresh context. `manual` mirrors upstream's `ProviderRefreshContext.isManual`: true when the
/// user explicitly asked for this refresh (footer click, Ctrl+R, a row's Refresh menu item).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefreshContext {
    pub manual: bool,
}

impl RefreshContext {
    pub fn manual() -> Self {
        Self { manual: true }
    }

    pub fn scheduled() -> Self {
        Self { manual: false }
    }
}

/// A banked limit reset the account can spend (Codex's "Usage limit resets").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitResetCredit {
    pub id: String,
    pub expires_at: Option<DateTime<Utc>>,
}

/// The provider's answer to spending a reset: `code` is `reset` when the limit came back,
/// otherwise why it did not (`already_redeemed`, `no_credit`, `nothing_to_reset`, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitResetReply {
    pub code: String,
    /// Which limit came back, as the provider names it.
    pub reset_type: Option<String>,
}

pub fn no_limit_resets() -> SimpleProviderError {
    SimpleProviderError::new(
        ErrorCategory::NotAvailable,
        "This provider has no limit resets.",
    )
}

/// One AI provider Quota Control can track.
#[async_trait]
pub trait ProviderRuntime: Send + Sync {
    fn provider(&self) -> &Provider;

    /// The widgets this provider feeds, in their default declaration order.
    fn widget_descriptors(&self) -> Vec<WidgetDescriptor>;

    /// Whether stored local spending still has usable account ownership for this card.
    fn allows_cached_local_history(&self) -> bool {
        true
    }

    /// Returns the latest snapshot. Failures return `ProviderSnapshot::error` instead of panicking.
    async fn refresh(&self, context: RefreshContext) -> ProviderSnapshot;

    /// Whether credentials for this provider already exist on this machine: a cheap, local-only probe
    /// (files, credential store, SQLite; never the network) used by first-run detection.
    async fn has_local_credentials(&self) -> bool;

    /// The banked limit resets this account can spend now, soonest to expire first. Only
    /// providers that bank resets override this.
    async fn limit_reset_credits(&self) -> Result<Vec<LimitResetCredit>, SimpleProviderError> {
        Err(no_limit_resets())
    }

    /// Spend `credit_id`. Called only when the user asks; nothing spends resets on its own.
    /// Repeating `request_id` after an attempt that got no answer makes the provider report
    /// `already_redeemed` instead of spending a second reset.
    async fn redeem_limit_reset(
        &self,
        _credit_id: &str,
        _request_id: &str,
    ) -> Result<LimitResetReply, SimpleProviderError> {
        Err(no_limit_resets())
    }
}

/// A source of "now", injectable for deterministic tests.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(Utc::now)
}

pub fn fixed_clock(instant: DateTime<Utc>) -> Clock {
    Arc::new(move || instant)
}

/// Run a blocking credential or file load on the blocking pool so it never stalls the async runtime.
pub async fn load_blocking<T, F>(load: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    match tokio::task::spawn_blocking(load).await {
        Ok(value) => value,
        Err(join_error) => std::panic::resume_unwind(join_error.into_panic()),
    }
}

/// Await `future`, giving up after `limit`. Used for the 120-second per-provider refresh deadline.
pub async fn with_deadline<T>(limit: std::time::Duration, future: impl Future<Output = T>) -> Option<T> {
    tokio::time::timeout(limit, future).await.ok()
}
