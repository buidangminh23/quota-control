//! `ServiceRuntime`: one service plus one credential, as the engine's `ProviderRuntime`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uc_accounts::KeyStore;
use uc_core::{
    Clock, ErrorCategory, MetricLine, ProgressFormat, Provider, ProviderRuntime, ProviderSnapshot,
    RefreshContext, SharedHttpClient, SimpleProviderError, WidgetDescriptor, system_clock,
};

use crate::catalog::identity_hash;
use crate::service::{FetchContext, Memo, RENEWED, Roots, Secret, Service};

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
        let secret = self.secret().await?;
        let context = FetchContext {
            secret: &secret,
            http: &self.http,
            now,
            memo: &self.memo,
        };
        let reading = self.service.fetch(&context).await;
        if let Some(document) = self.memo.get(RENEWED, now).await {
            self.memo.remove(RENEWED).await;
            self.keep_renewed(document).await;
        }
        let reading = reading?;
        Ok(
            ProviderSnapshot::make(&self.provider, reading.plan, reading.lines, now)
                .with_plan_term(reading.plan_term)
                .with_account(reading.account)
                .with_warning(reading.warning),
        )
    }
}

impl ServiceRuntime {
    /// Save a signed-in account's renewed token document, so the rotated refresh token is the one
    /// the next refresh uses. Another app's login is never written.
    async fn keep_renewed(&self, document: serde_json::Value) {
        let CredentialSource::Saved {
            store,
            id,
            signed_in: true,
        } = self.source.clone()
        else {
            return;
        };
        let saved = uc_core::load_blocking(move || store.replace_login(&id, &document)).await;
        if saved.is_err() {
            tracing::warn!(
                "A renewed sign-in of {} could not be saved",
                self.service.name()
            );
        }
    }
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
                .await;
            Err(SimpleProviderError::new(ErrorCategory::Network, "down"))
        }
    }

    static ROTATING: Rotating = Rotating;

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
