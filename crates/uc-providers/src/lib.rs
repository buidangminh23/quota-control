pub mod accounts;
pub mod credentials;
pub mod mapping;
pub mod oauth;

pub use accounts::{CliAccount, VisibleAccount, account_runtimes, cli_accounts, visible_accounts};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use credentials::CredentialStore;
use sha2::{Digest, Sha256};
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
    cli: Option<CliBinding>,
}

/// The CLI login a card follows, checked on every refresh so that a CLI that switched accounts
/// never shows the other account's limits under this card.
struct CliBinding {
    id: String,
    path: PathBuf,
    profile: Option<PathBuf>,
    verified_token: tokio::sync::Mutex<Option<[u8; 32]>>,
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
            cli: None,
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

    /// Follow the CLI login `account`; the card takes that account's id, so it keeps its place
    /// when the same account is later connected through the browser.
    pub fn with_cli_login(mut self, account: &CliAccount) -> Self {
        self.provider.id = account.id.clone();
        self.cli = Some(CliBinding {
            id: account.id.clone(),
            path: account.path.clone(),
            profile: account.profile.clone(),
            verified_token: tokio::sync::Mutex::new(None),
        });
        self
    }

    async fn ready_credentials(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        rejected_token: Option<&str>,
    ) -> Result<credentials::Credentials, SimpleProviderError> {
        let Some(binding) = &self.cli else {
            return accounts::ready_credentials(
                &self.credentials,
                self.kind,
                &self.http,
                now,
                &self.refresh_endpoint,
                rejected_token,
            )
            .await;
        };
        let (kind, path, profile) = (self.kind, binding.path.clone(), binding.profile.clone());
        let (current, credentials) =
            uc_core::load_blocking(move || accounts::read_cli_account(kind, path, profile)).await?;
        if current.id != binding.id {
            return Err(SimpleProviderError::new(
                ErrorCategory::NotAvailable,
                format!(
                    "The {} CLI on this computer is now signed in to another account.",
                    self.kind.cli()
                ),
            ));
        }
        Ok(credentials)
    }

    async fn verify_cli_identity(
        &self,
        credentials: &credentials::Credentials,
    ) -> Result<(), SimpleProviderError> {
        let Some(binding) = &self.cli else {
            return Ok(());
        };
        if self.kind != ProviderKind::Claude {
            return Ok(());
        }
        let fingerprint: [u8; 32] = Sha256::digest(credentials.access_token.as_bytes()).into();
        let mut verified = binding.verified_token.lock().await;
        if verified.as_ref() == Some(&fingerprint) {
            return Ok(());
        }
        *verified = None;
        let profile = self
            .http
            .send(
                HttpRequest::get("https://api.anthropic.com/api/oauth/profile")
                    .bearer(&credentials.access_token)
                    .header("anthropic-beta", "oauth-2025-04-20")
                    .timeout(Duration::from_secs(15)),
            )
            .await
            .map_err(|_| {
                SimpleProviderError::new(
                    ErrorCategory::Network,
                    "Cannot verify the Claude CLI account. Try again later.",
                )
            })?;
        if !profile.is_success() {
            self.record_cooldown(&profile, (self.clock)()).await;
            return Err(SimpleProviderError::new(
                if matches!(profile.status, 401 | 403) {
                    ErrorCategory::AuthExpired
                } else {
                    ErrorCategory::http(profile.status)
                },
                "The Claude CLI account could not be verified. Sign in with claude again if the problem persists.",
            ));
        }
        let profile: serde_json::Value = profile
            .json()
            .map_err(|_| accounts::auth_error("The Claude CLI account profile is invalid."))?;
        let document = serde_json::json!({"oauthAccount": {
            "accountUuid": profile["account"]["uuid"],
            "organizationUuid": profile["organization"]["uuid"]
        }});
        let identity = accounts::identity(self.kind, &document)?;
        if accounts::account_id(self.kind, &identity) != binding.id {
            return Err(SimpleProviderError::new(
                ErrorCategory::NotAvailable,
                "The Claude CLI session belongs to another account. Waiting for its account metadata to update.",
            ));
        }
        *verified = Some(fingerprint);
        Ok(())
    }

    async fn record_cooldown(
        &self,
        response: &uc_core::HttpResponse,
        now: chrono::DateTime<chrono::Utc>,
    ) {
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
    }

    pub fn with_refresh_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.refresh_endpoint = endpoint.into();
        self
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    fn validate_credentials(
        &self,
        credentials: &credentials::Credentials,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), SimpleProviderError> {
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
        Ok(())
    }

    async fn fetch(&self) -> Result<ProviderSnapshot, SimpleProviderError> {
        let now = (self.clock)();
        let mut credentials = self.ready_credentials(now, None).await?;
        self.validate_credentials(&credentials, now)?;
        if self.cooldown.lock().await.is_some_and(|until| until > now) {
            return Err(SimpleProviderError::new(
                ErrorCategory::RateLimited,
                "Usage updates are rate limited. Waiting before retrying.",
            ));
        }
        let mut response = self.request_usage(&credentials).await?;
        if response.status == 401 {
            let renewed = self
                .ready_credentials(now, Some(&credentials.access_token))
                .await?;
            if renewed.access_token != credentials.access_token {
                credentials = renewed;
                self.validate_credentials(&credentials, now)?;
                response = self.request_usage(&credentials).await?;
            }
        }
        self.record_cooldown(&response, now).await;
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
        self.verify_cli_identity(credentials).await?;
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
