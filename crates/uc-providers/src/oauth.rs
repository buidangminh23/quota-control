//! Browser sign-in for Claude and Codex accounts: OAuth 2 authorization code with PKCE. Both
//! providers redirect to a listener on this computer, as their own CLIs do, so the browser hands the
//! code back by itself and shows the outcome; nobody copies a code by hand.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use uc_accounts::{AccountRecord, AccountStore, CredentialMode};
use uc_core::{
    ErrorCategory, HttpRequest, ReqwestHttpClient, SharedHttpClient, SimpleProviderError,
};
use url::Url;
use uuid::Uuid;

use crate::ProviderKind;
use crate::accounts::{
    CLAUDE_CLIENT, CLAUDE_SCOPE, CODEX_CLIENT, account_error, apply_tokens, auth_error, identity,
};
use crate::credentials::parse_credentials;

const FLOW_LIFETIME: Duration = Duration::from_secs(600);
/// How long the browser tab waits for the account to be saved before it shows an interim page.
const PAGE_WAIT: Duration = Duration::from_secs(20);
const REQUEST_LIMIT: usize = 16_384;
const LOGIN_CANCELLED: &str = "This login was cancelled.";
const LOGIN_EXPIRED: &str = "This login has expired. Start again.";

/// True for the error a login ends with when it was cancelled or replaced by a newer one.
pub fn is_cancelled(error: &SimpleProviderError) -> bool {
    error.message == LOGIN_CANCELLED
}

/// True for the error a login ends with when nobody finished it in the browser in time.
pub fn is_expired(error: &SimpleProviderError) -> bool {
    error.message == LOGIN_EXPIRED
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStart {
    pub flow_id: String,
    pub authorization_url: String,
    pub expires_in_seconds: u64,
}

/// The language of the page the browser shows once it comes back to this computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoginLanguage {
    #[default]
    English,
    Vietnamese,
}

impl LoginLanguage {
    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("vi") {
            Self::Vietnamese
        } else {
            Self::English
        }
    }
}

struct Callback {
    url: String,
    page: oneshot::Sender<LoginPage>,
}

type CallbackResult = Result<Callback, SimpleProviderError>;

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
            kind,
            language,
        };
        let url = self.authorization_url_for(kind, &redirect, &challenge, &state)?;
        let (sender, receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(FLOW_LIFETIME, serve_callback(listeners, context))
                .await
                .unwrap_or_else(|_| Err(auth_error(LOGIN_EXPIRED)));
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
                return Err(auth_error(if cancelled.remove(flow_id).is_some() {
                    LOGIN_CANCELLED
                } else {
                    "This login is no longer active. Start again."
                }));
            }
        };
        if flow.expires <= Instant::now() {
            return Err(auth_error(LOGIN_EXPIRED));
        }
        let cancellation = self
            .flows
            .lock()
            .await
            .get(flow_id)
            .map(|flow| flow.cancel.clone())
            .ok_or_else(|| auth_error(LOGIN_CANCELLED))?;
        let remaining = flow.expires.saturating_duration_since(Instant::now());
        let stop = tokio::select! {
            _ = cancellation.notified() => Stop::Cancelled,
            result = tokio::time::timeout(remaining, self.complete_flow(&mut flow)) => Stop::Finished(
                result.unwrap_or_else(|_| Err(auth_error(LOGIN_EXPIRED))),
            ),
        };
        self.flows.lock().await.remove(flow_id);
        let (outcome, page) = match stop {
            Stop::Cancelled => (Err(auth_error(LOGIN_CANCELLED)), LoginPage::Cancelled),
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
                let listeners = bind_loopback_pair(self.endpoints.claude_port).await?;
                let port = listeners[0]
                    .local_addr()
                    .map_err(|_| callback_unavailable())?
                    .port();
                Ok((listeners, format!("http://localhost:{port}/callback")))
            }
            ProviderKind::Codex => {
                for port in &self.endpoints.codex_ports {
                    if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, *port)).await {
                        let port = listener
                            .local_addr()
                            .map_err(|_| callback_unavailable())?
                            .port();
                        return Ok((
                            vec![listener],
                            format!("http://127.0.0.1:{port}/auth/callback"),
                        ));
                    }
                }
                Err(auth_error(
                    "The login callback ports are in use. Finish the other login and try again.",
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
        let callback = receiver.await.map_err(|_| auth_error(LOGIN_CANCELLED))??;
        flow.page = Some(callback.page);
        let code = validate_callback(&callback.url, &flow.redirect, &flow.state)?;
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
        if flow.kind == ProviderKind::Claude {
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
        }
        let key = identity(flow.kind, &document)?;
        Ok(PreparedAccount {
            kind: flow.kind,
            label: flow.label.clone(),
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

fn callback_unavailable() -> SimpleProviderError {
    auth_error("Cannot start the local login callback.")
}

/// `localhost` resolves to both loopback addresses, so the listener takes the same port on each
/// when this computer has IPv6.
async fn bind_loopback_pair(port: u16) -> Result<Vec<TcpListener>, SimpleProviderError> {
    for _ in 0..8 {
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| callback_unavailable())?;
        let bound = v4.local_addr().map_err(|_| callback_unavailable())?.port();
        match TcpListener::bind((Ipv6Addr::LOCALHOST, bound)).await {
            Ok(v6) => return Ok(vec![v4, v6]),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse && port == 0 => continue,
            Err(_) => return Ok(vec![v4]),
        }
    }
    Err(callback_unavailable())
}

fn validate_callback(
    raw: &str,
    redirect: &str,
    expected_state: &str,
) -> Result<String, SimpleProviderError> {
    let url = Url::parse(raw.trim()).map_err(|_| auth_error("The callback URL is invalid."))?;
    let expected = Url::parse(redirect).map_err(|_| auth_error("The callback URL is invalid."))?;
    if url.origin() != expected.origin()
        || url.path() != expected.path()
        || url.fragment().is_some()
    {
        return Err(auth_error(
            "The callback belongs to a different login endpoint.",
        ));
    }
    let params: Vec<_> = url.query_pairs().collect();
    if params.iter().any(|(key, _)| key == "error") {
        return Err(auth_error("The browser login was not authorized."));
    }
    let codes: Vec<_> = params.iter().filter(|(key, _)| key == "code").collect();
    let states: Vec<_> = params.iter().filter(|(key, _)| key == "state").collect();
    if codes.len() != 1 || states.len() != 1 {
        return Err(auth_error("The callback is missing its code or state."));
    }
    let code = codes[0].1.to_string();
    if states[0].1 != expected_state {
        return Err(auth_error("The callback state does not match this login."));
    }
    if code.is_empty() || code.len() > 8192 || code.chars().any(char::is_control) {
        return Err(auth_error("The authorization code is invalid."));
    }
    Ok(code)
}

struct CallbackContext {
    redirect: Url,
    state: String,
    kind: ProviderKind,
    language: LoginLanguage,
}

#[derive(Debug, PartialEq, Eq)]
enum Arrival {
    Callback,
    Denied,
    Foreign,
    Elsewhere,
}

impl CallbackContext {
    /// What a request target means for this sign-in, judged on its path and state alone; the code
    /// itself is checked again before it is exchanged.
    fn classify(&self, target: &str) -> Arrival {
        let Some(query) = target
            .strip_prefix(self.redirect.path())
            .and_then(|rest| rest.strip_prefix('?'))
        else {
            return Arrival::Elsewhere;
        };
        let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let states: Vec<_> = params.iter().filter(|(key, _)| key == "state").collect();
        if states.len() != 1 || states[0].1 != self.state {
            return Arrival::Foreign;
        }
        if params.iter().any(|(key, _)| key == "error") {
            return Arrival::Denied;
        }
        Arrival::Callback
    }
}

/// Accept connections until one carries this sign-in's callback, then stop listening. Each
/// connection is answered on its own task, so a browser that opens a spare connection first cannot
/// hold up the real one.
async fn serve_callback(listeners: Vec<TcpListener>, context: CallbackContext) -> CallbackResult {
    let context = Arc::new(context);
    let (sender, mut receiver) = mpsc::channel(1);
    loop {
        let accepted = tokio::select! {
            result = receiver.recv() => return result.unwrap_or_else(|| Err(callback_unavailable())),
            accepted = listeners[0].accept() => accepted,
            accepted = async {
                match listeners.get(1) {
                    Some(listener) => listener.accept().await,
                    None => std::future::pending().await,
                }
            } => accepted,
        };
        let (socket, address) = accepted.map_err(|_| callback_unavailable())?;
        if address.ip().is_loopback() {
            tokio::spawn(answer(socket, context.clone(), sender.clone()));
        }
    }
}

async fn answer(
    mut socket: TcpStream,
    context: Arc<CallbackContext>,
    sender: mpsc::Sender<CallbackResult>,
) {
    let Some(target) = read_target(&mut socket).await else {
        return;
    };
    let page = match context.classify(&target) {
        Arrival::Elsewhere => {
            respond(&mut socket, "404 Not Found", "").await;
            return;
        }
        Arrival::Foreign => LoginPage::Inactive,
        Arrival::Denied => {
            let _ = sender.try_send(Err(auth_error("The browser login was not authorized.")));
            LoginPage::Denied
        }
        Arrival::Callback => {
            let (page, outcome) = oneshot::channel();
            let url = format!(
                "{}{target}",
                context.redirect.origin().ascii_serialization()
            );
            if sender.try_send(Ok(Callback { url, page })).is_err() {
                LoginPage::Inactive
            } else {
                match tokio::time::timeout(PAGE_WAIT, outcome).await {
                    Ok(Ok(page)) => page,
                    Ok(Err(_)) => LoginPage::Cancelled,
                    Err(_) => LoginPage::Pending,
                }
            }
        }
    };
    let status = if page == LoginPage::Inactive {
        "400 Bad Request"
    } else {
        "200 OK"
    };
    respond(
        &mut socket,
        status,
        &page.html(context.kind, context.language),
    )
    .await;
}

/// The target of a `GET` request, once its head has arrived.
async fn read_target(socket: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), async {
        while buffer.len() < REQUEST_LIMIT && !buffer.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut chunk = [0; 1024];
            let size = socket.read(&mut chunk).await?;
            if size == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..size]);
        }
        Ok::<_, std::io::Error>(())
    })
    .await;
    if !matches!(read, Ok(Ok(()))) {
        return None;
    }
    let head = String::from_utf8_lossy(&buffer);
    let mut parts = head.lines().next()?.split_whitespace();
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("GET"), Some(target), Some(version), None)
            if version.starts_with("HTTP/1.") && target.starts_with('/') =>
        {
            Some(target.to_owned())
        }
        _ => None,
    }
}

async fn respond(socket: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        socket.write_all(response.as_bytes()).await?;
        socket.shutdown().await
    })
    .await;
}

/// The page the browser tab shows once the sign-in reaches this computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoginPage {
    Connected,
    Failed,
    Denied,
    Pending,
    Cancelled,
    Inactive,
}

impl LoginPage {
    fn text(self, brand: &str, language: LoginLanguage) -> (String, &'static str) {
        use LoginLanguage::{English, Vietnamese};
        match (self, language) {
            (Self::Connected, Vietnamese) => (
                format!("Đã kết nối {brand}"),
                "Quota Control đã lưu tài khoản này và sẽ hiện hạn mức sau vài giây. Có thể đóng tab này.",
            ),
            (Self::Connected, English) => (
                format!("{brand} is connected"),
                "Quota Control saved this account and will show its limits in a few seconds. You can close this tab.",
            ),
            (Self::Failed, Vietnamese) => (
                format!("Chưa kết nối được {brand}"),
                "Quota Control không hoàn tất được lần đăng nhập này. Mở Quota Control để xem lý do rồi thử lại.",
            ),
            (Self::Failed, English) => (
                format!("{brand} was not connected"),
                "Quota Control could not finish this sign-in. Open Quota Control to see why, then try again.",
            ),
            (Self::Denied, Vietnamese) => (
                "Chưa cấp quyền cho Quota Control".into(),
                "Lần này chưa được cho phép. Muốn thử lại, bấm Đăng nhập bằng Google trong Quota Control.",
            ),
            (Self::Denied, English) => (
                "Access was not granted".into(),
                "To try again, choose Sign in with Google in Quota Control.",
            ),
            (Self::Pending, Vietnamese) => (
                format!("Đang hoàn tất đăng nhập {brand}"),
                "Quay lại Quota Control để xem kết quả. Có thể đóng tab này.",
            ),
            (Self::Pending, English) => (
                format!("Finishing the {brand} sign-in"),
                "Return to Quota Control to see the result. You can close this tab.",
            ),
            (Self::Cancelled, Vietnamese) => (
                "Lần đăng nhập này đã bị hủy".into(),
                "Quota Control đã dừng lần đăng nhập này. Bấm Đăng nhập bằng Google để bắt đầu lại.",
            ),
            (Self::Cancelled, English) => (
                "This sign-in was cancelled".into(),
                "Quota Control stopped this sign-in. Choose Sign in with Google to start again.",
            ),
            (Self::Inactive, Vietnamese) => (
                "Lần đăng nhập này không còn hiệu lực".into(),
                "Bấm Đăng nhập bằng Google trong Quota Control để bắt đầu lại.",
            ),
            (Self::Inactive, English) => (
                "This sign-in is no longer active".into(),
                "Choose Sign in with Google in Quota Control to start again.",
            ),
        }
    }

    fn html(self, kind: ProviderKind, language: LoginLanguage) -> String {
        let brand = match kind {
            ProviderKind::Claude => "Claude",
            ProviderKind::Codex => "Codex",
        };
        let (title, message) = self.text(brand, language);
        let lang = match language {
            LoginLanguage::Vietnamese => "vi",
            LoginLanguage::English => "en",
        };
        let accent = match self {
            Self::Connected => "#1f9d55",
            Self::Pending => "#c98a00",
            _ => "#d14343",
        };
        format!(
            r#"<!doctype html><html lang="{lang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title} · Quota Control</title><style>:root{{color-scheme:light dark;--bg:#f4f5f7;--card:#fff;--text:#1d1f23;--muted:#5b626d}}@media (prefers-color-scheme:dark){{:root{{--bg:#16181c;--card:#212429;--text:#eceef1;--muted:#a5abb4}}}}body{{margin:0;min-height:100vh;display:grid;place-items:center;background:var(--bg);color:var(--text);font:16px/1.5 system-ui,"Segoe UI",Roboto,sans-serif}}main{{box-sizing:border-box;width:min(440px,calc(100vw - 32px));padding:32px;background:var(--card);border-radius:16px;box-shadow:0 1px 3px rgba(0,0,0,.14)}}.app{{margin:0 0 8px;font-size:13px;font-weight:600;color:var(--muted)}}h1{{margin:0 0 12px;font-size:22px;line-height:1.3}}h1::before{{content:"";display:inline-block;width:10px;height:10px;margin-right:12px;border-radius:50%;background:{accent};vertical-align:middle}}p{{margin:0;color:var(--muted)}}</style></head><body><main><p class="app">Quota Control</p><h1>{title}</h1><p>{message}</p></main></body></html>"#
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
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

    #[test]
    fn callbacks_reject_conflicting_state_and_wrong_origins() {
        let redirect = "http://localhost:4545/callback";
        for callback in [
            "http://attacker.invalid/callback?code=fixture&state=expected",
            "http://localhost:4546/callback?code=fixture&state=expected",
            "http://localhost:4545/other?code=fixture&state=expected",
            "http://localhost:4545/callback?code=fixture&state=expected&state=other",
            "http://localhost:4545/callback?code=fixture&state=other",
            "http://localhost:4545/callback?error=access_denied&state=expected",
            "fixture#expected",
        ] {
            assert!(validate_callback(callback, redirect, "expected").is_err());
        }
        assert_eq!(
            validate_callback(
                "http://localhost:4545/callback?code=fixture&state=expected",
                redirect,
                "expected"
            )
            .unwrap(),
            "fixture"
        );
    }

    #[test]
    fn listener_answers_only_its_own_path_and_state() {
        let context = CallbackContext {
            redirect: Url::parse("http://localhost:4545/callback").unwrap(),
            state: "expected".into(),
            kind: ProviderKind::Claude,
            language: LoginLanguage::English,
        };
        assert_eq!(context.classify("/favicon.ico"), Arrival::Elsewhere);
        assert_eq!(
            context.classify("/callbacks?code=a&state=expected"),
            Arrival::Elsewhere
        );
        assert_eq!(
            context.classify("/callback?code=a&state=other"),
            Arrival::Foreign
        );
        assert_eq!(
            context.classify("/callback?error=access_denied&state=expected"),
            Arrival::Denied
        );
        assert_eq!(
            context.classify("/callback?code=a&state=expected"),
            Arrival::Callback
        );
    }

    #[test]
    fn result_pages_follow_the_language() {
        let english = LoginPage::Connected.html(ProviderKind::Claude, LoginLanguage::English);
        assert!(english.contains("<html lang=\"en\">") && english.contains("Claude is connected"));
        let vietnamese = LoginPage::Denied.html(ProviderKind::Codex, LoginLanguage::parse("vi"));
        assert!(vietnamese.contains("<html lang=\"vi\">") && vietnamese.contains("Chưa cấp quyền"));
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
