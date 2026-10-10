//! `ServiceRuntime`: one service plus one credential, as the engine's `ProviderRuntime`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uc_accounts::KeyStore;
use uc_core::{
    Clock, ErrorCategory, MetricLine, PlanTerm, ProgressFormat, Provider, ProviderRuntime,
    ProviderSnapshot, RefreshContext, SharedHttpClient, SimpleProviderError, WidgetDescriptor,
    system_clock,
};

use crate::catalog::identity_hash;
use crate::service::{FetchContext, Memo, Reading, Roots, Secret, Service};

/// How long a card waits after the service answered "too many requests".
const RATE_LIMIT_PAUSE_MINUTES: i64 = 5;

/// Where a card's credential comes from. It is read again at every refresh, so a login the app
/// renewed, or a key the user replaced, is followed without rebuilding the card.
#[derive(Clone, Debug)]
pub enum CredentialSource {
    /// A login found on this computer, picked out of a fresh discovery by its identity's hash.
    Login { identity_hash: String, roots: Roots },
    /// A key saved in Quota Control, or an account signed in to from it (`signed_in`), whose token
    /// document the card owns.
    Saved {
        store: KeyStore,
        id: String,
        signed_in: bool,
    },
    /// A key in an environment variable.
    Env {
        variable: &'static str,
        roots: Roots,
    },
}

pub struct ServiceRuntime {
    service: &'static dyn Service,
    provider: Provider,
    source: CredentialSource,
    http: SharedHttpClient,
    clock: Clock,
    memo: Memo,
    paused_until: tokio::sync::Mutex<Option<DateTime<Utc>>>,
}

impl ServiceRuntime {
    pub fn new(
        service: &'static dyn Service,
        provider: Provider,
        source: CredentialSource,
        http: SharedHttpClient,
    ) -> Self {
        Self {
            service,
            provider,
            source,
            http,
            clock: system_clock(),
            memo: Memo::default(),
            paused_until: tokio::sync::Mutex::new(None),
        }
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn service(&self) -> &'static dyn Service {
        self.service
    }

    async fn secret(&self) -> Result<Secret, SimpleProviderError> {
        let service = self.service;
        match self.source.clone() {
            CredentialSource::Login {
                identity_hash: wanted,
                roots,
            } => {
                let found = uc_core::load_blocking(move || {
                    service
                        .discover(&roots)
                        .into_iter()
                        .find(|login| identity_hash(service.id(), &login.identity) == wanted)
                        .map(|login| login.secret)
                })
                .await;
                found.ok_or_else(|| {
                    let app = service.connection().login_from.unwrap_or(service.name());
                    SimpleProviderError::new(
                        ErrorCategory::NotLoggedIn,
                        format!("This account is no longer signed in to {app} on this computer."),
                    )
                })
            }
            CredentialSource::Saved {
                store,
                id,
                signed_in,
            } => {
                let secret = uc_core::load_blocking(move || store.secret(&id)).await;
                match (secret, signed_in) {
                    (Ok(value), true) => Ok(Secret::owned(value)),
                    (Ok(value), false) => Ok(Secret::new(value)),
                    (Err(_), true) => Err(SimpleProviderError::new(
                        ErrorCategory::CredentialAccess,
                        "The saved sign-in could not be read. Sign in again in Accounts.",
                    )),
                    (Err(_), false) => Err(SimpleProviderError::new(
                        ErrorCategory::CredentialAccess,
                        "The saved API key could not be read. Add it again in Accounts.",
                    )),
                }
            }
            CredentialSource::Env { variable, roots } => roots
                .var(variable)
                .map(|key| Secret::api_key(&key))
                .ok_or_else(|| {
                    SimpleProviderError::new(
                        ErrorCategory::NotLoggedIn,
                        format!("{variable} is no longer set."),
                    )
                }),
        }
    }

    async fn read(&self, now: DateTime<Utc>) -> Result<ProviderSnapshot, SimpleProviderError> {
        if let Some(until) = *self.paused_until.lock().await
            && until > now
        {
            return Err(SimpleProviderError::new(
                ErrorCategory::RateLimited,
                "Usage updates are rate limited. Waiting before retrying.",
            ));
        }
        let reading = if let CredentialSource::Saved {
            store,
            id,
            signed_in: true,
        } = self.source.clone()
        {
            let service = self.service;
            let http = self.http.clone();
            let memo = self.memo.clone();
            tokio::spawn(async move {
                let (lock, snapshot) = uc_core::load_blocking(move || {
                    let lock = store.renewal_lock(&id)?;
                    let snapshot = store.login_snapshot(&id)?;
                    Ok::<_, uc_accounts::AccountError>((lock, snapshot))
                })
                .await
                .map_err(|_| credential_error())?;
                let secret = Secret::owned(snapshot.document().clone());
                let memo = memo.with_renewal(snapshot, lock);
                fetch(service, &secret, &http, now, &memo).await
            })
            .await
            .map_err(|_| credential_error())??
        } else {
            let secret = self.secret().await?;
            fetch(self.service, &secret, &self.http, now, &self.memo).await?
        };
        Ok(snapshot(&self.provider, reading, now))
    }
}

fn snapshot(provider: &Provider, reading: Reading, now: DateTime<Utc>) -> ProviderSnapshot {
    let paid = reading.plan.as_deref().is_some_and(|plan| {
        !matches!(
            plan.trim().to_ascii_lowercase().as_str(),
            "free" | "free plan" | "free tier" | "unknown" | "inactive"
        )
    });
    let term = reading.plan_term.and_then(|term| match term {
        PlanTerm::Stated {
            ends_at,
            checked_at,
        } if paid && ends_at > now => {
            let checked_at = checked_at.or(reading.plan_checked_at).unwrap_or(now);
            (checked_at <= now).then_some(PlanTerm::Stated {
                ends_at,
                checked_at: Some(checked_at),
            })
        }
        _ => None,
    });
    let checked_at = reading
        .plan_checked_at
        .filter(|checked_at| *checked_at <= now)
        .or(match &term {
            Some(PlanTerm::Stated { checked_at, .. }) => *checked_at,
            _ => None,
        });
    let mut snapshot = ProviderSnapshot::make(provider, reading.plan, reading.lines, now)
        .with_plan_term(term)
        .with_account(reading.account)
        .with_warning(reading.warning);
    snapshot.plan_checked_at = checked_at;
    snapshot
}

async fn fetch(
    service: &'static dyn Service,
    secret: &Secret,
    http: &SharedHttpClient,
    now: DateTime<Utc>,
    memo: &Memo,
) -> Result<Reading, SimpleProviderError> {
    service
        .fetch(&FetchContext {
            secret,
            http,
            now,
            memo,
        })
        .await
}

fn credential_error() -> SimpleProviderError {
    SimpleProviderError::new(
        ErrorCategory::CredentialAccess,
        "The saved sign-in could not be read. Sign in again in Accounts.",
    )
}

#[async_trait]
impl ProviderRuntime for ServiceRuntime {
    fn provider(&self) -> &Provider {
        &self.provider
    }

    fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
        self.service.descriptors(&self.provider)
    }

    fn descriptors_for(&self, snapshot: Option<&ProviderSnapshot>) -> Vec<WidgetDescriptor> {
        let mut descriptors = self.service.descriptors(&self.provider);
        if let Some(snapshot) = snapshot {
            let extra = model_descriptors(&self.provider, &descriptors, snapshot);
            descriptors.extend(extra);
        }
        descriptors
    }

    fn allows_cached_local_history(&self) -> bool {
        false
    }

    async fn refresh(&self, _: RefreshContext) -> ProviderSnapshot {
        let now = (self.clock)();
        match self.read(now).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if error.category == ErrorCategory::RateLimited {
                    let mut paused = self.paused_until.lock().await;
                    if paused.is_none_or(|until| until <= now) {
                        *paused = Some(now + chrono::Duration::minutes(RATE_LIMIT_PAUSE_MINUTES));
                    }
                }
                let mut snapshot = ProviderSnapshot::error(&self.provider, &error);
                snapshot.refreshed_at = now;
                snapshot
            }
        }
    }

    async fn has_local_credentials(&self) -> bool {
        self.secret().await.is_ok()
    }
}

/// The id suffix of a row the service did not declare: `m-` and the label's lowercase words.
pub fn model_suffix(label: &str) -> String {
    let mut slug = String::new();
    for character in label.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(48).collect();
    format!("m-{}", slug.trim_end_matches('-'))
}

/// One row per line of `snapshot` that no declared widget reads (a model the account can use),
/// after the declared ones. Error snapshots add none.
fn model_descriptors(
    provider: &Provider,
    declared: &[WidgetDescriptor],
    snapshot: &ProviderSnapshot,
) -> Vec<WidgetDescriptor> {
    if snapshot.is_error() {
        return Vec::new();
    }
    let mut seen: std::collections::HashSet<String> = declared
        .iter()
        .map(|descriptor| descriptor.id.clone())
        .collect();
    let labels: std::collections::HashSet<&str> = declared
        .iter()
        .map(|descriptor| descriptor.metric_label.as_str())
        .collect();
    let mut rows = Vec::new();
    for line in &snapshot.lines {
        let label = line.label();
        if label.trim().is_empty() || labels.contains(label) {
            continue;
        }
        let suffix = model_suffix(label);
        let id = format!("{}.{suffix}", provider.id);
        if suffix == "m-" || !seen.insert(id.clone()) {
            continue;
        }
        let descriptor = match line {
            MetricLine::Progress(progress) => match &progress.format {
                ProgressFormat::Percent => {
                    WidgetDescriptor::percent(id, provider, label, None, None)
                        .exporting_progress(&suffix, "percent")
                }
                ProgressFormat::Count { suffix: unit } => WidgetDescriptor::bounded_count(
                    id,
                    provider,
                    label,
                    None,
                    progress.limit,
                    unit,
                    progress.period_duration_ms,
                ),
                ProgressFormat::Dollars => WidgetDescriptor::bounded_dollars(
                    id,
                    provider,
                    label,
                    None,
                    progress.limit,
                    None,
                    None,
                ),
            },
            MetricLine::Values(values) => {
                let first = values.values.first();
                WidgetDescriptor::values(
                    id,
                    provider,
                    label,
                    None,
                    first.map(|value| value.kind),
                    first.and_then(|value| value.label.as_deref()),
                    false,
                    None,
                    false,
                )
            }
            MetricLine::Badge(_) | MetricLine::Text(_) | MetricLine::Chart(_) => continue,
        };
        rows.push(descriptor);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Connection, Reading};
    use crate::support::lines;
    use chrono::TimeZone;
    use serde_json::{Value, json};

    fn provider() -> Provider {
        Provider::new("antigravity@abc", "Antigravity")
    }

    #[test]
    fn paid_periods_get_a_verification_stamp_without_refreshing_a_cached_confirmation() {
        let now = Utc.with_ymd_and_hms(2026, 10, 10, 10, 0, 0).unwrap();
        let ends_at = now + chrono::Duration::days(10);
        let original = Some(now - chrono::Duration::hours(6));
        for (checked_at, plan_checked_at) in [(None, None), (original, None), (None, original)] {
            let reading = Reading::new(
                Some("Pro".into()),
                vec![lines::percent("Monthly", 40.0, Some(ends_at), None)],
            )
            .with_plan_checked_at(plan_checked_at)
            .with_plan_term(Some(PlanTerm::Stated {
                ends_at,
                checked_at,
            }));
            let current = snapshot(&provider(), reading, now);
            let confirmed = checked_at.or(plan_checked_at).unwrap_or(now);
            assert_eq!(current.plan_checked_at, Some(confirmed));
            assert_eq!(
                current.plan_term,
                Some(PlanTerm::Stated {
                    ends_at,
                    checked_at: Some(confirmed)
                })
            );
            assert_eq!(current.refreshed_at, now);
        }
    }

    #[test]
    fn free_unknown_expired_and_inferred_periods_keep_usage_without_paid_dates() {
        let now = Utc.with_ymd_and_hms(2026, 10, 10, 10, 0, 0).unwrap();
        let lines = vec![lines::percent("Weekly", 40.0, None, None)];
        let future = now + chrono::Duration::days(10);
        let stated = Some(PlanTerm::Stated {
            ends_at: future,
            checked_at: None,
        });
        for (plan, term) in [
            (Some("Free"), stated.clone()),
            (Some("Free Plan"), stated.clone()),
            (Some("Free Tier"), stated.clone()),
            (Some("Unknown"), stated.clone()),
            (Some("Inactive"), stated),
            (
                None,
                Some(PlanTerm::Stated {
                    ends_at: future,
                    checked_at: None,
                }),
            ),
            (
                Some("Pro"),
                Some(PlanTerm::Stated {
                    ends_at: now,
                    checked_at: None,
                }),
            ),
            (
                Some("Pro"),
                Some(PlanTerm::Stated {
                    ends_at: future,
                    checked_at: Some(future),
                }),
            ),
            (
                Some("Pro"),
                Some(PlanTerm::MonthlyFrom {
                    started_at: now - chrono::Duration::days(7),
                    checked_at: Some(now),
                }),
            ),
        ] {
            let reading =
                Reading::new(plan.map(str::to_string), lines.clone()).with_plan_term(term);
            let current = snapshot(&provider(), reading, now);
            assert_eq!(current.plan_term, None, "{plan:?}");
            assert_eq!(current.lines, lines);
        }
        let checked_at = now - chrono::Duration::hours(6);
        let current = snapshot(
            &provider(),
            Reading::new(Some("Free".into()), lines).with_plan_checked_at(Some(checked_at)),
            now,
        );
        assert_eq!(current.plan_checked_at, Some(checked_at));
        assert_eq!(current.plan_term, None);
    }

    /// A service whose token renews at every fetch and whose usage request then fails.
    struct Rotating;

    #[async_trait]
    impl Service for Rotating {
        fn id(&self) -> &'static str {
            "rotating"
        }

        fn name(&self) -> &'static str {
            "Rotating"
        }

        fn connection(&self) -> Connection {
            Connection::default()
        }

        fn descriptors(&self, _: &Provider) -> Vec<WidgetDescriptor> {
            Vec::new()
        }

        async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
            let generation = context.secret.value()["generation"].as_u64().unwrap_or(0);
            context
                .keep_renewed(json!({"refresh": "rotated", "generation": generation + 1}))
                .await?;
            Err(SimpleProviderError::new(ErrorCategory::Network, "down"))
        }
    }

    static ROTATING: Rotating = Rotating;

    struct GatedRotating {
        calls: std::sync::atomic::AtomicUsize,
        started: tokio::sync::Notify,
        response: tokio::sync::Notify,
        persisted: tokio::sync::Notify,
        usage: tokio::sync::Notify,
        finished: tokio::sync::Notify,
    }

    impl GatedRotating {
        fn new() -> &'static Self {
            Box::leak(Box::new(Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
                started: tokio::sync::Notify::new(),
                response: tokio::sync::Notify::new(),
                persisted: tokio::sync::Notify::new(),
                usage: tokio::sync::Notify::new(),
                finished: tokio::sync::Notify::new(),
            }))
        }
    }

    #[async_trait]
    impl Service for GatedRotating {
        fn id(&self) -> &'static str {
            "rotating"
        }
        fn name(&self) -> &'static str {
            "Rotating"
        }
        fn connection(&self) -> Connection {
            Connection::default()
        }
        fn descriptors(&self, _: &Provider) -> Vec<WidgetDescriptor> {
            Vec::new()
        }

        async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
            let first = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
            if first {
                self.started.notify_one();
                self.response.notified().await;
            }
            let generation = context.secret.value()["generation"].as_u64().unwrap_or(0);
            context
                .keep_renewed(json!({"generation": generation + 1}))
                .await?;
            if first {
                self.persisted.notify_one();
                self.usage.notified().await;
                self.finished.notify_one();
            }
            Err(SimpleProviderError::new(ErrorCategory::Network, "down"))
        }
    }

    async fn signal(notify: &tokio::sync::Notify) {
        tokio::time::timeout(std::time::Duration::from_secs(5), notify.notified())
            .await
            .unwrap();
    }

    fn login() -> (tempfile::TempDir, KeyStore, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::new(dir.path().join("api-keys"));
        let id = format!("rotating@{}", "a".repeat(64));
        store
            .add_login(&id, "rotating", "me", "google", &json!({"generation": 0}))
            .unwrap();
        (dir, store, id)
    }

    #[tokio::test]
    async fn cancellation_during_rotation_persists_before_usage_and_unblocks_replacement() {
        let (_dir, store, id) = login();
        let service = GatedRotating::new();
        let mut card = runtime(&store, &id, true);
        card.service = service;
        let first = tokio::spawn(async move { card.refresh(RefreshContext::manual()).await });
        signal(&service.started).await;
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        service.response.notify_one();
        signal(&service.persisted).await;
        assert_eq!(store.secret(&id).unwrap()["generation"], 1);

        let mut replacement = runtime(&store, &id, true);
        replacement.service = service;
        let snapshot = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            replacement.refresh(RefreshContext::manual()),
        )
        .await
        .unwrap();
        assert_eq!(snapshot.error_category, Some(ErrorCategory::Network));
        assert_eq!(store.secret(&id).unwrap()["generation"], 2);
        service.usage.notify_one();
        signal(&service.finished).await;
        assert_eq!(store.secret(&id).unwrap()["generation"], 2);
    }

    #[tokio::test]
    async fn cancellation_after_rotation_keeps_the_saved_document() {
        let (_dir, store, id) = login();
        let service = GatedRotating::new();
        let mut card = runtime(&store, &id, true);
        card.service = service;
        let first = tokio::spawn(async move { card.refresh(RefreshContext::manual()).await });
        signal(&service.started).await;
        service.response.notify_one();
        signal(&service.persisted).await;
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        assert_eq!(store.secret(&id).unwrap()["generation"], 1);
        service.usage.notify_one();
        signal(&service.finished).await;
    }

    #[tokio::test]
    async fn concurrent_runtimes_reload_credentials_after_the_renewal_lock() {
        let (_dir, store, id) = login();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let card = runtime(&store, &id, true);
            tasks.push(tokio::spawn(async move {
                card.refresh(RefreshContext::manual()).await
            }));
        }
        for task in tasks {
            let snapshot = tokio::time::timeout(std::time::Duration::from_secs(10), task)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(snapshot.error_category, Some(ErrorCategory::Network));
        }
        assert_eq!(store.secret(&id).unwrap()["generation"], 8);
    }

    fn runtime(store: &KeyStore, id: &str, signed_in: bool) -> ServiceRuntime {
        ServiceRuntime::new(
            &ROTATING,
            Provider::new(id, "Rotating"),
            CredentialSource::Saved {
                store: store.clone(),
                id: id.to_string(),
                signed_in,
            },
            uc_core::ReqwestHttpClient::shared(),
        )
    }

    #[tokio::test]
    async fn a_renewed_sign_in_is_saved_even_when_the_fetch_then_fails() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::new(dir.path().join("api-keys"));
        let id = format!("rotating@{}", "a".repeat(64));
        store
            .add_login(
                &id,
                "rotating",
                "me",
                "google",
                &json!({"refresh": "first"}),
            )
            .unwrap();
        let card = runtime(&store, &id, true);
        for generation in 1..=2 {
            let snapshot = card.refresh(RefreshContext::manual()).await;
            assert_eq!(snapshot.error_category, Some(ErrorCategory::Network));
            let saved: Value = store.secret(&id).unwrap();
            assert_eq!(
                saved,
                json!({"refresh": "rotated", "generation": generation})
            );
        }
    }

    #[tokio::test]
    async fn a_saved_api_key_is_never_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::new(dir.path().join("api-keys"));
        let record = store
            .add("rotating", "key", "sk-test-1234", &Value::Null)
            .unwrap();
        let card = runtime(&store, &record.id, false);
        card.refresh(RefreshContext::manual()).await;
        assert_eq!(
            store.secret(&record.id).unwrap(),
            json!({"apiKey": "sk-test-1234"})
        );
    }

    #[test]
    fn model_suffixes_are_stable_slugs_without_dots() {
        assert_eq!(
            model_suffix("Gemini 3.1 Pro (High)"),
            "m-gemini-3-1-pro-high"
        );
        assert_eq!(model_suffix("  Claude Sonnet 4.6 "), "m-claude-sonnet-4-6");
        assert_eq!(model_suffix("!!!"), "m-");
    }

    #[test]
    fn undeclared_lines_become_rows_after_the_declared_ones() {
        let provider = provider();
        let declared = vec![WidgetDescriptor::percent(
            format!("{}.weekly", provider.id),
            &provider,
            "Weekly",
            None,
            None,
        )];
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let snapshot = ProviderSnapshot::make(
            &provider,
            None,
            vec![
                lines::percent("Weekly", 10.0, None, None),
                lines::percent("Gemini 3.1 Pro (High)", 25.0, None, None),
                lines::count("Claude Opus 4.6", 3.0, 10.0, "requests", None, None),
                lines::dollar_value("Balance Left", 4.0),
                lines::badge("Status", "ok"),
                lines::percent("Gemini 3.1 Pro (High)", 30.0, None, None),
            ],
            now,
        );
        let rows = model_descriptors(&provider, &declared, &snapshot);
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "antigravity@abc.m-gemini-3-1-pro-high",
                "antigravity@abc.m-claude-opus-4-6",
                "antigravity@abc.m-balance-left",
            ]
        );
        assert_eq!(rows[0].metric_label, "Gemini 3.1 Pro (High)");
        assert_eq!(rows[1].template.limit, Some(10.0));
        let error = ProviderSnapshot::error_message(&provider, "down", None);
        assert!(model_descriptors(&provider, &declared, &error).is_empty());
    }
}
