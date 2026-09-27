//! `ServiceRuntime`: one service plus one credential, as the engine's `ProviderRuntime`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uc_accounts::KeyStore;
use uc_core::{
    Clock, ErrorCategory, Provider, ProviderRuntime, ProviderSnapshot, RefreshContext,
    SharedHttpClient, SimpleProviderError, WidgetDescriptor, system_clock,
};

use crate::catalog::identity_hash;
use crate::service::{FetchContext, Memo, Roots, Secret, Service};

/// How long a card waits after the service answered "too many requests".
const RATE_LIMIT_PAUSE_MINUTES: i64 = 5;

/// Where a card's credential comes from. It is read again at every refresh, so a login the app
/// renewed, or a key the user replaced, is followed without rebuilding the card.
#[derive(Clone, Debug)]
pub enum CredentialSource {
    /// A login found on this computer, picked out of a fresh discovery by its identity's hash.
    Login { identity_hash: String, roots: Roots },
    /// A key saved in Quota Control.
    Saved { store: KeyStore, id: String },
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
            CredentialSource::Saved { store, id } => {
                let secret = uc_core::load_blocking(move || store.secret(&id)).await;
                secret.map(Secret::new).map_err(|_| {
                    SimpleProviderError::new(
                        ErrorCategory::CredentialAccess,
                        "The saved API key could not be read. Add it again in Accounts.",
                    )
                })
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
        let reading = self.service.fetch(&context).await?;
        Ok(
            ProviderSnapshot::make(&self.provider, reading.plan, reading.lines, now)
                .with_plan_term(reading.plan_term)
                .with_warning(reading.warning),
        )
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
