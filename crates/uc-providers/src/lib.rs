pub mod accounts;
pub mod credentials;
pub mod mapping;
pub mod oauth;

pub use accounts::{import_current_account, managed_runtimes};

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use credentials::CredentialStore;
use uc_core::{
    Clock, ErrorCategory, HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind,
    Provider, ProviderLink, ProviderRuntime, ProviderSnapshot, RefreshContext, ReqwestHttpClient,
    SessionStartSignal, SharedHttpClient, SimpleProviderError, WidgetDescriptor, system_clock,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Claude,
    Codex,
}

impl ProviderKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
    pub fn cli(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn endpoint(self) -> &'static str {
        match self {
            Self::Claude => "https://api.anthropic.com/api/oauth/usage",
            Self::Codex => "https://chatgpt.com/backend-api/wham/usage",
        }
    }
}

pub struct LocalProvider {
    kind: ProviderKind,
    provider: Provider,
    credentials: CredentialStore,
    http: SharedHttpClient,
    clock: Clock,
    endpoint: String,
    refresh_endpoint: String,
    cooldown: tokio::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
}

impl LocalProvider {
    pub fn new(kind: ProviderKind, credentials: CredentialStore, http: SharedHttpClient) -> Self {
        let (name, status) = match kind {
            ProviderKind::Claude => ("Claude", "https://status.anthropic.com"),
            ProviderKind::Codex => ("Codex", "https://status.openai.com"),
        };
        Self {
            kind,
            provider: Provider::new(kind.cli(), name)
                .with_links(vec![ProviderLink::new("Status", status)]),
            credentials,
            http,
            clock: system_clock(),
            endpoint: kind.endpoint().into(),
            refresh_endpoint: accounts::refresh_url(kind).into(),
            cooldown: tokio::sync::Mutex::new(None),
        }
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_account(mut self, record: &uc_accounts::AccountRecord) -> Self {
        self.provider.id = record.id.clone();
        self.provider.display_name = format!("{} · {}", self.provider.display_name, record.label);
        self
    }

    pub fn with_refresh_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.refresh_endpoint = endpoint.into();
        self
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    async fn fetch(&self) -> Result<ProviderSnapshot, SimpleProviderError> {
        let now = (self.clock)();
        let mut credentials = accounts::ready_credentials(
            &self.credentials,
            self.kind,
            &self.http,
            now,
            &self.refresh_endpoint,
            None,
        )
        .await?;
        if credentials.expires_at.is_some_and(|expires| expires <= now) {
            return Err(SimpleProviderError::new(
                ErrorCategory::AuthExpired,
                format!("Session expired. Sign in with {} again.", self.kind.cli()),
            ));
        }
        if !credentials.profile_scope {
            return Err(SimpleProviderError::new(
                ErrorCategory::NotAvailable,
                "Sign in with claude again to grant access to live usage.",
            ));
        }
        if self.cooldown.lock().await.is_some_and(|until| until > now) {
            return Err(SimpleProviderError::new(
                ErrorCategory::RateLimited,
                "Usage updates are rate limited. Waiting before retrying.",
            ));
        }
        let mut response = self.request_usage(&credentials).await?;
        if response.status == 401 {
            let renewed = accounts::ready_credentials(
                &self.credentials,
                self.kind,
                &self.http,
                now,
                &self.refresh_endpoint,
                Some(&credentials.access_token),
            )
            .await?;
            if renewed.access_token != credentials.access_token {
                credentials = renewed;
                response = self.request_usage(&credentials).await?;
            }
        }
        if response.status == 429 {
            let seconds = response
                .header("retry-after")
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(300)
                .min(86400);
            *self.cooldown.lock().await =
                now.checked_add_signed(chrono::Duration::seconds(seconds));
        }
        let mapped = mapping::map_response(self.kind, &response, now)?;
        Ok(ProviderSnapshot::make(
            &self.provider,
            mapped.plan.or(credentials.plan),
            mapped.lines,
            now,
        ))
    }

    async fn request_usage(
        &self,
        credentials: &credentials::Credentials,
    ) -> Result<uc_core::HttpResponse, SimpleProviderError> {
        let mut request = HttpRequest::get(&self.endpoint)
            .bearer(&credentials.access_token)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(15));
        match self.kind {
            ProviderKind::Claude => {
                request = request.header("anthropic-beta", "oauth-2025-04-20");
            }
            ProviderKind::Codex => {
                if let Some(account_id) = &credentials.account_id {
                    request = request.header("ChatGPT-Account-Id", account_id);
                }
            }
        }
        self.http.send(request).await.map_err(|_| {
            SimpleProviderError::new(
                ErrorCategory::Network,
                "Cannot connect to the usage service.",
            )
        })
    }
}

#[async_trait]
impl ProviderRuntime for LocalProvider {
    fn provider(&self) -> &Provider {
        &self.provider
    }

    fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
        let labels = match self.kind {
            ProviderKind::Claude => vec![
                ("session", "Session"),
                ("weekly", "Weekly"),
                ("sonnet", "Sonnet"),
                ("fable", "Fable"),
            ],
            ProviderKind::Codex => vec![
                ("session", "Session"),
                ("weekly", "Weekly"),
                ("spark", "Spark"),
                ("sparkWeekly", "Spark Weekly"),
            ],
        };
        let mut descriptors: Vec<_> = labels
            .into_iter()
            .map(|(suffix, label)| {
                let signal = (self.kind == ProviderKind::Claude && suffix == "session")
                    .then_some(SessionStartSignal::MissingResetDate);
                WidgetDescriptor::percent(
                    format!("{}.{suffix}", self.provider.id),
                    &self.provider,
                    label,
                    None,
                    signal,
                )
                .exporting_progress(suffix, "percent")
            })
            .collect();
        match self.kind {
            ProviderKind::Claude => descriptors.push(
                WidgetDescriptor::bounded_dollars(
                    format!("{}.extra", self.provider.id),
                    &self.provider,
                    "Extra Usage",
                    Some("Extra usage spent"),
                    100.0,
                    None,
                    Some("spent"),
                )
                .exporting_limit(
                    "extraUsage",
                    LimitResourceKind::Consumption,
                    "usd",
                    LimitResourceSource::ProgressOrValue {
                        kind: MetricKind::Dollars,
                        label: None,
                    },
                    false,
                ),
            ),
            ProviderKind::Codex => {
                descriptors.push(
                    WidgetDescriptor::combined(
                        format!("{}.credits", self.provider.id),
                        &self.provider,
                        "Credits",
                        None,
                        false,
                    )
                    .exporting_limit(
                        "credits",
                        LimitResourceKind::Balance,
                        "credits",
                        LimitResourceSource::Value {
                            kind: MetricKind::Count,
                            label: Some("credits".into()),
                        },
                        false,
                    )
                    .exporting_limit(
                        "creditValue",
                        LimitResourceKind::Balance,
                        "usd",
                        LimitResourceSource::Value {
                            kind: MetricKind::Dollars,
                            label: None,
                        },
                        false,
                    ),
                );
                descriptors.push(
                    WidgetDescriptor::values(
                        format!("{}.rateLimitResets", self.provider.id),
                        &self.provider,
                        "Rate Limit Resets",
                        None,
                        Some(MetricKind::Count),
                        Some("available"),
                        false,
                        Some("resets"),
                        true,
                    )
                    .exporting_limit(
                        "rateLimitResets",
                        LimitResourceKind::Balance,
                        "resets",
                        LimitResourceSource::Value {
                            kind: MetricKind::Count,
                            label: Some("available".into()),
                        },
                        false,
                    ),
                );
            }
        }
        descriptors
    }

    fn allows_cached_local_history(&self) -> bool {
        false
    }

    async fn refresh(&self, _: RefreshContext) -> ProviderSnapshot {
        self.fetch().await.unwrap_or_else(|error| {
            let mut snapshot = ProviderSnapshot::error(&self.provider, &error);
            snapshot.refreshed_at = (self.clock)();
            snapshot
        })
    }

    async fn has_local_credentials(&self) -> bool {
        self.credentials.load().await.is_ok()
    }
}

pub fn default_runtimes() -> Vec<Arc<dyn ProviderRuntime>> {
    let http = ReqwestHttpClient::shared();
    [ProviderKind::Claude, ProviderKind::Codex]
        .into_iter()
        .map(|kind| {
            Arc::new(LocalProvider::new(
                kind,
                CredentialStore::discover(kind),
                http.clone(),
            )) as Arc<dyn ProviderRuntime>
        })
        .collect()
}
