//! Browser sign-in for Claude and Codex accounts: OAuth 2 authorization code with PKCE. Both
//! providers redirect to a listener on this computer, as their own CLIs do, so the browser hands the
//! code back by itself and shows the outcome; nobody copies a code by hand.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, oneshot};
use uc_accounts::{AccountRecord, AccountStore, CredentialMode};
use uc_core::loopback::{
    self, CallbackContext, CallbackResult, LoginPage, PageBrand, SignInAction,
};
use uc_core::{
    ErrorCategory, HttpRequest, ReqwestHttpClient, SharedHttpClient, SimpleProviderError,
};
use url::Url;
use uuid::Uuid;

use crate::ProviderKind;
use crate::accounts::{
    CLAUDE_CLIENT, CLAUDE_SCOPE, CODEX_CLIENT, account_error, apply_tokens, auth_error, identity,
};
use crate::credentials::{jwt_claims, parse_credentials};

const FLOW_LIFETIME: Duration = Duration::from_secs(600);

pub use uc_core::loopback::{is_cancelled, is_expired};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStart {
    pub flow_id: String,
    pub authorization_url: String,
    pub expires_in_seconds: u64,
}

pub use uc_core::loopback::LoginLanguage;

struct PendingLogin {
    kind: ProviderKind,
    label: String,
    state: String,
    verifier: String,
    redirect: String,
    expires: Instant,
    callback: Option<oneshot::Receiver<CallbackResult>>,
    listener: tokio::task::AbortHandle,
    page: Option<oneshot::Sender<LoginPage>>,
}

impl Drop for PendingLogin {
    fn drop(&mut self) {
        self.listener.abort();
    }
}

struct FlowHandle {
    kind: ProviderKind,
    expires: Instant,
    url: String,
    cancel: Arc<Notify>,
    listener: tokio::task::JoinHandle<()>,
}

struct PreparedAccount {
    kind: ProviderKind,
    label: String,
    identity: String,
    document: Value,
}

enum Stop {
    Cancelled,
    Finished(Result<PreparedAccount, SimpleProviderError>),
}

#[derive(Clone)]
pub struct OAuthEndpoints {
    pub claude_authorize: String,
    pub claude_token: String,
    pub claude_profile: String,
    /// 0 picks a free port, as Claude Code does.
    pub claude_port: u16,
    pub codex_authorize: String,
    pub codex_token: String,
    pub codex_ports: Vec<u16>,
}

impl Default for OAuthEndpoints {
    fn default() -> Self {
        Self {
            claude_authorize: "https://claude.com/cai/oauth/authorize".into(),
            claude_token: "https://platform.claude.com/v1/oauth/token".into(),
            claude_profile: "https://api.anthropic.com/api/oauth/profile".into(),
            claude_port: 0,
            codex_authorize: "https://auth.openai.com/oauth/authorize".into(),
            codex_token: "https://auth.openai.com/oauth/token".into(),
            codex_ports: vec![1455, 1457],
        }
    }
}

pub struct OAuthManager {
    store: Arc<AccountStore>,
    http: SharedHttpClient,
    endpoints: OAuthEndpoints,
    pending: Mutex<HashMap<String, PendingLogin>>,
    flows: Mutex<HashMap<String, FlowHandle>>,
    cancelled: Mutex<HashMap<String, Instant>>,
}

impl OAuthManager {
    pub fn new(store: Arc<AccountStore>) -> Self {
        Self {
            store,
            http: ReqwestHttpClient::shared(),
            endpoints: OAuthEndpoints::default(),
            pending: Mutex::new(HashMap::new()),
            flows: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_http(mut self, http: SharedHttpClient) -> Self {
        self.http = http;
        self
    }

    pub fn with_endpoints(mut self, endpoints: OAuthEndpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    pub async fn begin_login(
        &self,
        kind: ProviderKind,
        label: String,
    ) -> Result<LoginStart, SimpleProviderError> {
        self.begin_login_in(kind, label, LoginLanguage::default())
            .await
    }

    /// Start a browser sign-in whose result page speaks `language`. An older sign-in for the same
    /// provider is cancelled first, which releases its callback port and leaves one tab that can
    /// finish.
    pub async fn begin_login_in(
        &self,
        kind: ProviderKind,
        label: String,
        language: LoginLanguage,
    ) -> Result<LoginStart, SimpleProviderError> {
        let now = Instant::now();
        let mut pending = self.pending.lock().await;
        pending.retain(|_, flow| flow.expires > now && flow.kind != kind);
        let stopped: Vec<_> = {
            let mut flows = self.flows.lock().await;
            let ids: Vec<_> = flows
                .iter()
                .filter(|(_, flow)| flow.kind == kind || flow.expires <= now)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| flows.remove(&id).map(|flow| (id, flow)))
                .collect()
        };
        for (id, flow) in stopped {
            if flow.expires > now {
                self.remember_cancelled(id, flow.expires).await;
                flow.cancel.notify_one();
            }
            flow.listener.abort();
            let _ = flow.listener.await;
        }
        if pending.len() >= 8 {
            return Err(auth_error(
                "Too many pending logins. Cancel an existing login first.",
            ));
        }
        let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let flow_id = Uuid::new_v4().to_string();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let (listeners, redirect) = self.bind_callback(kind).await?;
        let context = CallbackContext {
            redirect: Url::parse(&redirect).map_err(|_| callback_unavailable())?,
            state: state.clone(),
            brand: PageBrand::new(brand(kind), SignInAction::Google),
            language,
        };
        let url = self.authorization_url_for(kind, &redirect, &challenge, &state)?;
        let (sender, receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(FLOW_LIFETIME, loopback::serve(listeners, context))
                .await
                .unwrap_or_else(|_| Err(loopback::expired()));
            let _ = sender.send(result);
        });
        pending.insert(
            flow_id.clone(),
            PendingLogin {
                kind,
                label,
                state,
                verifier,
                redirect,
                expires: now + FLOW_LIFETIME,
                callback: Some(receiver),
                listener: task.abort_handle(),
                page: None,
            },
        );
        self.flows.lock().await.insert(
            flow_id.clone(),
            FlowHandle {
                kind,
                expires: now + FLOW_LIFETIME,
                url: url.clone(),
                cancel: Arc::new(Notify::new()),
                listener: task,
            },
        );
        Ok(LoginStart {
            flow_id,
            authorization_url: url,
            expires_in_seconds: FLOW_LIFETIME.as_secs(),
        })
    }

    /// The sign-in page of a login that is still open, to show it again.
    pub async fn authorization_url(&self, flow_id: &str) -> Option<String> {
        self.flows
            .lock()
            .await
            .get(flow_id)
            .filter(|flow| flow.expires > Instant::now())
            .map(|flow| flow.url.clone())
    }

    pub async fn cancel_login(&self, flow_id: &str) -> Result<(), SimpleProviderError> {
        let mut pending = self.pending.lock().await;
        pending.remove(flow_id);
        let flow = self.flows.lock().await.remove(flow_id);
        if let Some(flow) = flow {
            self.remember_cancelled(flow_id.to_owned(), flow.expires)
                .await;
            flow.cancel.notify_one();
            flow.listener.abort();
            let _ = flow.listener.await;
        }
        Ok(())
    }

    async fn remember_cancelled(&self, flow_id: String, expires: Instant) {
        let mut cancelled = self.cancelled.lock().await;
        cancelled.retain(|_, expiry| *expiry > Instant::now());
        cancelled.insert(flow_id, expires);
        while cancelled.len() > 64 {
            if let Some(oldest) = cancelled
                .iter()
                .min_by_key(|(_, expiry)| **expiry)
                .map(|(id, _)| id.clone())
            {
                cancelled.remove(&oldest);
            }
        }
    }

    /// Wait for the browser to come back, then exchange the code and save the account. Once the
    /// save has begun, a cancellation no longer undoes it.
    pub async fn complete_login(
        &self,
        flow_id: &str,
    ) -> Result<AccountRecord, SimpleProviderError> {
        let flow = self.pending.lock().await.remove(flow_id);
        let mut flow = match flow {
            Some(flow) => flow,
            None => {
                let mut cancelled = self.cancelled.lock().await;
                cancelled.retain(|_, expires| *expires > Instant::now());
                return Err(if cancelled.remove(flow_id).is_some() {
                    loopback::cancelled()
                } else {
                    auth_error("This login is no longer active. Start again.")
                });
            }
        };
        if flow.expires <= Instant::now() {
            return Err(loopback::expired());
        }
        let cancellation = self
            .flows
            .lock()
            .await
            .get(flow_id)
            .map(|flow| flow.cancel.clone())
            .ok_or_else(loopback::cancelled)?;
        let remaining = flow.expires.saturating_duration_since(Instant::now());
        let stop = tokio::select! {
            _ = cancellation.notified() => Stop::Cancelled,
            result = tokio::time::timeout(remaining, self.complete_flow(&mut flow)) => Stop::Finished(
                result.unwrap_or_else(|_| Err(loopback::expired())),
            ),
        };
        self.flows.lock().await.remove(flow_id);
        let (outcome, page) = match stop {
            Stop::Cancelled => (Err(loopback::cancelled()), LoginPage::Cancelled),
            Stop::Finished(Ok(prepared)) => {
                let saved = self.save(prepared).await;
                let page = if saved.is_ok() {
                    LoginPage::Connected
                } else {
                    LoginPage::Failed
                };
                (saved, page)
            }
            Stop::Finished(Err(error)) => (Err(error), LoginPage::Failed),
        };
        if let Some(sender) = flow.page.take() {
            let _ = sender.send(page);
        }
        outcome
    }

    async fn bind_callback(
        &self,
        kind: ProviderKind,
    ) -> Result<(Vec<TcpListener>, String), SimpleProviderError> {
        match kind {
            ProviderKind::Claude => {
                let listeners = loopback::bind_localhost(self.endpoints.claude_port).await?;
                let port = loopback::port(&listeners[0])?;
                Ok((listeners, format!("http://localhost:{port}/callback")))
            }
            ProviderKind::Codex => {
                let listener = loopback::bind_ipv4(&self.endpoints.codex_ports).await?;
                let port = loopback::port(&listener)?;
                Ok((
                    vec![listener],
                    format!("http://127.0.0.1:{port}/auth/callback"),
                ))
            }
        }
    }

    fn authorization_url_for(
        &self,
        kind: ProviderKind,
        redirect: &str,
        challenge: &str,
        state: &str,
    ) -> Result<String, SimpleProviderError> {
        let endpoint = match kind {
            ProviderKind::Claude => &self.endpoints.claude_authorize,
            ProviderKind::Codex => &self.endpoints.codex_authorize,
        };
        let mut url = Url::parse(endpoint)
            .map_err(|_| auth_error("The authorization endpoint is invalid."))?;
        let (client, scope) = match kind {
            ProviderKind::Claude => (CLAUDE_CLIENT, CLAUDE_SCOPE),
            ProviderKind::Codex => (CODEX_CLIENT, "openid profile email offline_access"),
        };
        url.query_pairs_mut()
            .append_pair("client_id", client)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect)
            .append_pair("scope", scope)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state);
        match kind {
            ProviderKind::Claude => {
                url.query_pairs_mut().append_pair("code", "true");
            }
            ProviderKind::Codex => {
                url.query_pairs_mut()
                    .append_pair("id_token_add_organizations", "true")
                    .append_pair("codex_cli_simplified_flow", "true")
                    .append_pair("originator", "codex_cli_rs");
            }
        }
        Ok(url.to_string())
    }

    async fn complete_flow(
        &self,
        flow: &mut PendingLogin,
    ) -> Result<PreparedAccount, SimpleProviderError> {
        let receiver = flow
            .callback
            .take()
            .ok_or_else(|| auth_error("The login callback is unavailable."))?;
        let callback = receiver.await.map_err(|_| loopback::cancelled())??;
        flow.page = Some(callback.page);
        let code = loopback::validate(&callback.url, &flow.redirect, &flow.state)?;
        let request = match flow.kind {
            ProviderKind::Claude => HttpRequest::post(&self.endpoints.claude_token).json_body(&json!({"grant_type":"authorization_code","code":code,"redirect_uri":flow.redirect,"client_id":CLAUDE_CLIENT,"code_verifier":flow.verifier,"state":flow.state})),
            ProviderKind::Codex => {
                let body = url::form_urlencoded::Serializer::new(String::new()).append_pair("grant_type", "authorization_code").append_pair("code", &code).append_pair("redirect_uri", &flow.redirect).append_pair("client_id", CODEX_CLIENT).append_pair("code_verifier", &flow.verifier).finish();
                HttpRequest::post(&self.endpoints.codex_token).header("Content-Type", "application/x-www-form-urlencoded").body(body.into_bytes())
            }
        };
        let response = self.http.send(request).await.map_err(|_| {
            SimpleProviderError::new(
                ErrorCategory::Network,
                "The login could not be completed. Start a new login and try again.",
            )
        })?;
        if !response.is_success() {
            return Err(SimpleProviderError::new(
                ErrorCategory::http(response.status),
                "The login service rejected this code. Start a new login.",
            ));
        }
        let tokens: Value = response
            .json()
            .map_err(|_| auth_error("The login service returned invalid credentials."))?;
        if !tokens["refresh_token"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
        {
            return Err(auth_error(
                "This login did not grant an independent renewable session.",
            ));
        }
        let mut document = match flow.kind {
            ProviderKind::Claude => json!({"claudeAiOauth":{}}),
            ProviderKind::Codex => json!({"tokens":{}}),
        };
        apply_tokens(flow.kind, &mut document, &tokens, chrono::Utc::now())?;
        let credentials = parse_credentials(flow.kind, &document)?;
        let email = if flow.kind == ProviderKind::Claude {
            let profile = self
                .http
                .send(
                    HttpRequest::get(&self.endpoints.claude_profile)
                        .bearer(&credentials.access_token)
                        .header("anthropic-beta", "oauth-2025-04-20"),
                )
                .await
                .map_err(|_| {
                    SimpleProviderError::new(
                        ErrorCategory::Network,
                        "Cannot verify the signed-in account. Try connecting again.",
                    )
                })?;
            if !profile.is_success() {
                return Err(auth_error("The signed-in account could not be verified."));
            }
            let profile: Value = profile
                .json()
                .map_err(|_| auth_error("The account profile is invalid."))?;
            document["oauthAccount"] = json!({"accountUuid":profile["account"]["uuid"],"organizationUuid":profile["organization"]["uuid"]});
            if let Some(plan) = profile["organization"]["organization_type"].as_str() {
                document["claudeAiOauth"]["subscriptionType"] =
                    json!(plan.strip_prefix("claude_").unwrap_or(plan));
            }
            if let Some(tier) = profile["organization"]["rate_limit_tier"].as_str() {
                document["claudeAiOauth"]["rateLimitTier"] = json!(tier);
            }
            email_label(&profile["account"]["email"])
        } else {
            codex_email_label(&tokens)
        };
        let key = identity(flow.kind, &document)?;
        let label = if flow.label.trim().is_empty() || flow.label.trim() == flow.kind.cli() {
            email.unwrap_or_else(|| flow.kind.cli().into())
        } else {
            flow.label.clone()
        };
        Ok(PreparedAccount {
            kind: flow.kind,
            label,
            identity: key,
            document,
        })
    }

    /// Saved on its own task, so dropping the caller midway still leaves a whole account.
    async fn save(&self, prepared: PreparedAccount) -> Result<AccountRecord, SimpleProviderError> {
        let store = self.store.clone();
        tokio::spawn(async move {
            let lock = crate::accounts::refresh_lock(&crate::accounts::account_id(
                prepared.kind,
                &prepared.identity,
            ));
            let _guard = lock.lock().await;
            uc_core::load_blocking(move || {
                store
                    .import(
                        prepared.kind.cli(),
                        &prepared.label,
                        &prepared.identity,
                        &prepared.document,
                        CredentialMode::ManagedOauth,
                    )
                    .map_err(|_| account_error())
            })
            .await
        })
        .await
        .map_err(|_| account_error())?
    }
}

pub(crate) fn codex_email_label(tokens: &Value) -> Option<String> {
    tokens["id_token"]
        .as_str()
        .and_then(jwt_claims)
        .and_then(|claims| {
            email_label(&claims["email"])
                .or_else(|| email_label(&claims["https://api.openai.com/profile"]["email"]))
        })
}

pub(crate) fn email_label(value: &Value) -> Option<String> {
    let raw = value.as_str()?;
    if raw.chars().any(char::is_control) {
        return None;
    }
    let email = raw.trim();
    let (local, domain) = email.split_once('@')?;
    if email.len() > 256
        || email.chars().any(char::is_whitespace)
        || local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
    {
        return None;
    }
    Some(email.to_owned())
}

fn callback_unavailable() -> SimpleProviderError {
    auth_error("Cannot start the local login callback.")
}

fn brand(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Claude => "Claude",
        ProviderKind::Codex => "Codex",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::net::Ipv4Addr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use uc_core::{HttpClient, HttpError, HttpResponse};

    struct FixtureHttp;

    #[async_trait]
    impl HttpClient for FixtureHttp {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            let body = if request.method == "POST" {
                json!({"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600})
            } else {
                json!({"account":{"uuid":"fixture-account"},"organization":{"uuid":"fixture-org"}})
            };
            Ok(HttpResponse {
                status: 200,
                headers: HashMap::new(),
                body: serde_json::to_vec(&body).unwrap(),
            })
        }
    }

    fn query(start: &LoginStart) -> HashMap<String, String> {
        Url::parse(&start.authorization_url)
            .unwrap()
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    struct EmailHttp {
        tokens: Value,
        profile: Value,
    }

    #[async_trait]
    impl HttpClient for EmailHttp {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            Ok(HttpResponse {
                status: 200,
                headers: HashMap::new(),
                body: serde_json::to_vec(if request.method == "POST" {
                    &self.tokens
                } else {
                    &self.profile
                })
                .unwrap(),
            })
        }
    }

    fn fixture_jwt(email: Value, namespaced: bool) -> String {
        let mut claims = json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "fixture-workspace",
                "chatgpt_user_id": "fixture-user"
            }
        });
        if namespaced {
            claims["https://api.openai.com/profile"] = json!({"email": email});
        } else {
            claims["email"] = email;
        }
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        )
    }

    fn email_http(email: Value) -> EmailHttp {
        EmailHttp {
            tokens: json!({
                "access_token": fixture_jwt(json!("access-token@example.com"), false),
                "id_token": fixture_jwt(email.clone(), false),
                "refresh_token": "fixture-refresh",
                "expires_in": 3600
            }),
            profile: json!({
                "account": {"uuid": "fixture-account", "email": email},
                "organization": {"uuid": "fixture-org"}
            }),
        }
    }

    async fn connect_email_fixture(
        store: Arc<AccountStore>,
        kind: ProviderKind,
        label: &str,
        http: EmailHttp,
    ) -> AccountRecord {
        let manager = OAuthManager::new(store)
            .with_http(Arc::new(http))
            .with_endpoints(OAuthEndpoints {
                codex_ports: vec![0],
                ..OAuthEndpoints::default()
            });
        let start = manager.begin_login(kind, label.into()).await.unwrap();
        let query = query(&start);
        let redirect = Url::parse(&query["redirect_uri"]).unwrap();
        let tab = browse(
            &query["redirect_uri"],
            format!("{}?code=fixture&state={}", redirect.path(), query["state"]),
        );
        let record = manager.complete_login(&start.flow_id).await.unwrap();
        assert!(tab.await.unwrap().contains("is connected"));
        record
    }

    #[tokio::test]
    async fn browser_accounts_default_to_profile_email() {
        for kind in [ProviderKind::Claude, ProviderKind::Codex] {
            let dir = tempfile::tempdir().unwrap();
            let store = Arc::new(AccountStore::new(dir.path().join("accounts")));
            let record = connect_email_fixture(
                store.clone(),
                kind,
                kind.cli(),
                email_http(json!("  account@example.com  ")),
            )
            .await;
            assert_eq!(record.label, "account@example.com");
            assert_eq!(store.list().unwrap()[0].label, "account@example.com");
        }
    }

    #[tokio::test]
    async fn browser_accounts_with_unusable_email_keep_provider_labels() {
        for kind in [ProviderKind::Claude, ProviderKind::Codex] {
            for email in [
                Value::Null,
                json!(42),
                json!(""),
                json!("  "),
                json!("not-an-email"),
                json!("local@"),
                json!("@domain"),
                json!("a b@example.com"),
                json!("a\n@example.com"),
                json!(format!("{}@example.com", "x".repeat(257))),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let store = Arc::new(AccountStore::new(dir.path().join("accounts")));
                let record =
                    connect_email_fixture(store, kind, kind.cli(), email_http(email)).await;
                assert_eq!(record.label, kind.cli());
            }
        }
    }

    #[tokio::test]
    async fn browser_codex_accepts_namespaced_id_token_email() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(AccountStore::new(dir.path().join("accounts")));
        let mut http = email_http(Value::Null);
        http.tokens["id_token"] = json!(fixture_jwt(json!("namespaced@example.com"), true));
        let record = connect_email_fixture(store, ProviderKind::Codex, "codex", http).await;
        assert_eq!(record.label, "namespaced@example.com");
    }

    #[tokio::test]
    async fn browser_codex_does_not_use_access_token_email() {
        for id_token in [
            Value::Null,
            json!("malformed"),
            json!(fixture_jwt(Value::Null, false)),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let store = Arc::new(AccountStore::new(dir.path().join("accounts")));
            let mut http = email_http(Value::Null);
            http.tokens["id_token"] = id_token;
            let record = connect_email_fixture(store, ProviderKind::Codex, "codex", http).await;
            assert_eq!(record.label, "codex");
        }
    }

    #[tokio::test]
    async fn browser_accounts_preserve_explicit_library_labels() {
        for kind in [ProviderKind::Claude, ProviderKind::Codex] {
            let dir = tempfile::tempdir().unwrap();
            let store = Arc::new(AccountStore::new(dir.path().join("accounts")));
            let record = connect_email_fixture(
                store,
                kind,
                "Work account",
                email_http(json!("account@example.com")),
            )
            .await;
            assert_eq!(record.label, "Work account");
        }
    }

    #[tokio::test]
    async fn browser_reconnection_updates_label_without_replacing_account() {
        for kind in [ProviderKind::Claude, ProviderKind::Codex] {
            let dir = tempfile::tempdir().unwrap();
            let store = Arc::new(AccountStore::new(dir.path().join("accounts")));
            let first =
                connect_email_fixture(store.clone(), kind, kind.cli(), email_http(Value::Null))
                    .await;
            for email in ["first@example.com", "second@example.com"] {
                let reconnected = connect_email_fixture(
                    store.clone(),
                    kind,
                    kind.cli(),
                    email_http(json!(email)),
                )
                .await;
                assert_eq!(reconnected.id, first.id);
                assert_eq!(reconnected.connected_at, first.connected_at);
                assert_eq!(reconnected.label, email);
                assert_eq!(store.list().unwrap(), vec![reconnected]);
            }
        }
    }

    /// Play the browser: request `target` from the callback listener and return the whole reply.
    fn browse(redirect: &str, target: String) -> tokio::task::JoinHandle<String> {
        let port = Url::parse(redirect).unwrap().port().unwrap();
        tokio::spawn(async move {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
            stream
                .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await.unwrap();
            reply
        })
    }

    fn manager(dir: &tempfile::TempDir) -> (Arc<AccountStore>, Arc<OAuthManager>) {
        let store = Arc::new(AccountStore::new(dir.path().join("accounts")));
        let manager = Arc::new(OAuthManager::new(store.clone()).with_http(Arc::new(FixtureHttp)));
        (store, manager)
    }

    #[tokio::test]
    async fn claude_returns_to_localhost_and_the_tab_shows_the_saved_account() {
        let dir = tempfile::tempdir().unwrap();
        let (store, manager) = manager(&dir);
        let start = manager
            .begin_login(ProviderKind::Claude, "Fixture".into())
            .await
            .unwrap();
        let query = query(&start);
        let redirect = Url::parse(&query["redirect_uri"]).unwrap();
        assert_eq!(redirect.host_str(), Some("localhost"));
        assert_eq!(redirect.path(), "/callback");
        assert_eq!(query["code"], "true");
        let favicon = browse(&query["redirect_uri"], "/favicon.ico".into())
            .await
            .unwrap();
        assert!(favicon.starts_with("HTTP/1.1 404"));
        let tab = browse(
            &query["redirect_uri"],
            format!("/callback?code=fixture&state={}", query["state"]),
        );
        let record = manager.complete_login(&start.flow_id).await.unwrap();
        assert_eq!(store.list().unwrap()[0].id, record.id);
        let reply = tab.await.unwrap();
        assert!(reply.starts_with("HTTP/1.1 200"));
        assert!(reply.contains("Claude is connected"));
        manager.cancel_login(&start.flow_id).await.unwrap();
        let error = manager.complete_login(&start.flow_id).await.unwrap_err();
        assert!(!is_cancelled(&error));
    }

    #[tokio::test]
    async fn a_refused_consent_ends_the_login() {
        let dir = tempfile::tempdir().unwrap();
        let (store, manager) = manager(&dir);
        let start = manager
            .begin_login_in(
                ProviderKind::Claude,
                "Fixture".into(),
                LoginLanguage::Vietnamese,
            )
            .await
            .unwrap();
        let query = query(&start);
        let tab = browse(
            &query["redirect_uri"],
            format!("/callback?error=access_denied&state={}", query["state"]),
        );
        assert!(manager.complete_login(&start.flow_id).await.is_err());
        assert!(tab.await.unwrap().contains("Chưa cấp quyền"));
        assert!(store.list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_new_login_for_the_same_provider_replaces_the_open_one() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, manager) = manager(&dir);
        let first = manager
            .begin_login(ProviderKind::Claude, "Fixture".into())
            .await
            .unwrap();
        let waiting = {
            let manager = manager.clone();
            let id = first.flow_id.clone();
            tokio::spawn(async move { manager.complete_login(&id).await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.pending.lock().await.contains_key(&first.flow_id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let second = manager
            .begin_login(ProviderKind::Claude, "Fixture".into())
            .await
            .unwrap();
        let replaced = tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .unwrap();
        assert!(replaced.is_err());
        assert!(manager.authorization_url(&first.flow_id).await.is_none());
        assert_eq!(
            manager.authorization_url(&second.flow_id).await,
            Some(second.authorization_url.clone())
        );
    }

    #[tokio::test]
    async fn codex_replacement_releases_its_only_callback_port() {
        for completing in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let port = socket.local_addr().unwrap().port();
            drop(socket);
            let manager = Arc::new(
                OAuthManager::new(Arc::new(AccountStore::new(dir.path().join("accounts"))))
                    .with_endpoints(OAuthEndpoints {
                        codex_ports: vec![port],
                        ..OAuthEndpoints::default()
                    }),
            );
            let first = manager
                .begin_login(ProviderKind::Codex, "First".into())
                .await
                .unwrap();
            let waiting = if completing {
                let manager = manager.clone();
                let id = first.flow_id.clone();
                Some(tokio::spawn(
                    async move { manager.complete_login(&id).await },
                ))
            } else {
                None
            };
            if completing {
                while manager.pending.lock().await.contains_key(&first.flow_id) {
                    tokio::task::yield_now().await;
                }
            }
            let second = manager
                .begin_login(ProviderKind::Codex, "Second".into())
                .await;
            assert!(
                second.is_ok(),
                "replacement failed: {:?}",
                second.err().map(|error| error.message)
            );
            if let Some(waiting) = waiting {
                assert!(is_cancelled(&waiting.await.unwrap().unwrap_err()));
            }
            manager
                .cancel_login(&second.unwrap().flow_id)
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn cancellation_returns_after_the_callback_port_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, manager) = manager(&dir);
        let start = manager
            .begin_login(ProviderKind::Claude, "Fixture".into())
            .await
            .unwrap();
        let query = query(&start);
        let port = Url::parse(&query["redirect_uri"]).unwrap().port().unwrap();
        let _ = browse(&query["redirect_uri"], "/favicon.ico".into())
            .await
            .unwrap();
        manager.cancel_login(&start.flow_id).await.unwrap();
        let rebound = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await;
        assert!(rebound.is_ok(), "cancelled listener still owns its port");
    }

    #[tokio::test]
    async fn cancellation_before_completion_is_reported_as_cancelled() {
        for replaced in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (_store, manager) = manager(&dir);
            let first = manager
                .begin_login(ProviderKind::Claude, "First".into())
                .await
                .unwrap();
            if replaced {
                manager
                    .begin_login(ProviderKind::Claude, "Second".into())
                    .await
                    .unwrap();
            } else {
                manager.cancel_login(&first.flow_id).await.unwrap();
            }
            let error = manager.complete_login(&first.flow_id).await.unwrap_err();
            assert!(is_cancelled(&error), "unexpected status: {}", error.message);
        }
    }

    #[tokio::test]
    async fn cancellation_history_expires_and_has_a_fixed_bound() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, manager) = manager(&dir);
        let now = Instant::now();
        for index in 0..80 {
            manager
                .remember_cancelled(index.to_string(), now + FLOW_LIFETIME)
                .await;
        }
        assert_eq!(manager.cancelled.lock().await.len(), 64);
        manager
            .cancelled
            .lock()
            .await
            .insert("expired".into(), now - Duration::from_secs(1));
        let error = manager.complete_login("expired").await.unwrap_err();
        assert!(!is_cancelled(&error));
        assert!(!manager.cancelled.lock().await.contains_key("expired"));
    }

    #[tokio::test]
    async fn cancellation_after_commit_begins_returns_the_committed_account() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("accounts");
        let store = Arc::new(AccountStore::new(root.clone()));
        store.list().unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.join("registry.lock"))
            .unwrap();
        file.lock().unwrap();
        let manager = Arc::new(OAuthManager::new(store.clone()).with_http(Arc::new(FixtureHttp)));
        let flow = manager
            .begin_login(ProviderKind::Claude, "Fixture".into())
            .await
            .unwrap();
        let query = query(&flow);
        let _tab = browse(
            &query["redirect_uri"],
            format!("/callback?code=fixture&state={}", query["state"]),
        );
        let cloned = manager.clone();
        let id = flow.flow_id.clone();
        let task = tokio::spawn(async move { cloned.complete_login(&id).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.flows.lock().await.contains_key(&flow.flow_id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        manager.cancel_login(&flow.flow_id).await.unwrap();
        file.unlock().unwrap();
        let record = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(store.list().unwrap()[0].id, record.id);
    }
}
