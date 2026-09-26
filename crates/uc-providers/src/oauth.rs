use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, oneshot};
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
const CLAUDE_REDIRECT: &str = "https://platform.claude.com/oauth/code/callback";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStart {
    pub flow_id: String,
    pub authorization_url: String,
    pub callback_mode: String,
    pub expires_in_seconds: u64,
}

struct PendingLogin {
    kind: ProviderKind,
    label: String,
    state: String,
    verifier: String,
    redirect: String,
    expires: Instant,
    callback: Option<oneshot::Receiver<Result<String, SimpleProviderError>>>,
    listener: Option<tokio::task::AbortHandle>,
}

struct PreparedAccount {
    kind: ProviderKind,
    label: String,
    identity: String,
    document: Value,
}

impl Drop for PendingLogin {
    fn drop(&mut self) {
        if let Some(listener) = &self.listener {
            listener.abort();
        }
    }
}

#[derive(Clone)]
pub struct OAuthEndpoints {
    pub claude_authorize: String,
    pub claude_token: String,
    pub claude_profile: String,
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
    cancellations: Mutex<HashMap<String, (Instant, Arc<tokio::sync::Notify>)>>,
}

impl OAuthManager {
    pub fn new(store: Arc<AccountStore>) -> Self {
        Self {
            store,
            http: ReqwestHttpClient::shared(),
            endpoints: OAuthEndpoints::default(),
            pending: Mutex::new(HashMap::new()),
            cancellations: Mutex::new(HashMap::new()),
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
        let mut pending = self.pending.lock().await;
        pending.retain(|_, flow| flow.expires > Instant::now());
        self.cancellations
            .lock()
            .await
            .retain(|_, (expires, _)| *expires > Instant::now());
        if pending.len() >= 8 {
            return Err(auth_error(
                "Too many pending logins. Cancel an existing login first.",
            ));
        }
        let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let flow_id = Uuid::new_v4().to_string();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let (redirect, callback, listener) = match kind {
            ProviderKind::Claude => (CLAUDE_REDIRECT.to_owned(), None, None),
            ProviderKind::Codex => {
                let mut bound = None;
                for port in &self.endpoints.codex_ports {
                    if let Ok(listener) =
                        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, *port)).await
                    {
                        bound = Some(listener);
                        break;
                    }
                }
                let listener = bound.ok_or_else(|| auth_error("The login callback ports are in use. Finish the other login and try again."))?;
                let port = listener
                    .local_addr()
                    .map_err(|_| auth_error("Cannot start the local login callback."))?
                    .port();
                let redirect = format!("http://127.0.0.1:{port}/auth/callback");
                let (sender, receiver) = oneshot::channel();
                let expected_state = state.clone();
                let expected_redirect = redirect.clone();
                let task = tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        FLOW_LIFETIME,
                        listen_for_callback(listener, expected_redirect, expected_state),
                    )
                    .await
                    .unwrap_or_else(|_| Err(auth_error("This login has expired. Start again.")));
                    let _ = sender.send(result);
                });
                (redirect, Some(receiver), Some(task.abort_handle()))
            }
        };
        let endpoint = match kind {
            ProviderKind::Claude => &self.endpoints.claude_authorize,
            ProviderKind::Codex => &self.endpoints.codex_authorize,
        };
        let mut url = Url::parse(endpoint)
            .map_err(|_| auth_error("The authorization endpoint is invalid."))?;
        let client = match kind {
            ProviderKind::Claude => CLAUDE_CLIENT,
            ProviderKind::Codex => CODEX_CLIENT,
        };
        let scope = match kind {
            ProviderKind::Claude => CLAUDE_SCOPE,
            ProviderKind::Codex => "openid profile email offline_access",
        };
        url.query_pairs_mut()
            .append_pair("client_id", client)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect)
            .append_pair("scope", scope)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
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
        pending.insert(
            flow_id.clone(),
            PendingLogin {
                kind,
                label,
                state,
                verifier,
                redirect,
                expires: Instant::now() + FLOW_LIFETIME,
                callback,
                listener,
            },
        );
        self.cancellations.lock().await.insert(
            flow_id.clone(),
            (
                Instant::now() + FLOW_LIFETIME,
                Arc::new(tokio::sync::Notify::new()),
            ),
        );
        Ok(LoginStart {
            flow_id,
            authorization_url: url.to_string(),
            callback_mode: match kind {
                ProviderKind::Claude => "manual",
                ProviderKind::Codex => "loopback",
            }
            .into(),
            expires_in_seconds: FLOW_LIFETIME.as_secs(),
        })
    }

    pub async fn cancel_login(&self, flow_id: &str) -> Result<(), SimpleProviderError> {
        self.pending.lock().await.remove(flow_id);
        if let Some((_, cancellation)) = self.cancellations.lock().await.remove(flow_id) {
            cancellation.notify_one();
        }
        Ok(())
    }

    pub async fn complete_login(
        &self,
        flow_id: &str,
        callback: Option<String>,
    ) -> Result<AccountRecord, SimpleProviderError> {
        let mut flow = self
            .pending
            .lock()
            .await
            .remove(flow_id)
            .ok_or_else(|| auth_error("This login is no longer active. Start again."))?;
        if flow.expires <= Instant::now() {
            return Err(auth_error("This login has expired. Start again."));
        }
        let cancellation = self
            .cancellations
            .lock()
            .await
            .get(flow_id)
            .map(|(_, cancellation)| cancellation.clone())
            .ok_or_else(|| auth_error("This login was cancelled."))?;
        let result = tokio::select! {
            _ = cancellation.notified() => Err(auth_error("This login was cancelled.")),
            result = tokio::time::timeout(flow.expires.saturating_duration_since(Instant::now()), self.complete_flow(&mut flow, callback)) => result.unwrap_or_else(|_| Err(auth_error("This login has expired. Start again."))),
        };
        self.cancellations.lock().await.remove(flow_id);
        let prepared = result?;
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

    async fn complete_flow(
        &self,
        flow: &mut PendingLogin,
        callback: Option<String>,
    ) -> Result<PreparedAccount, SimpleProviderError> {
        let code = match flow.kind {
            ProviderKind::Claude => validate_callback(
                callback.as_deref().ok_or_else(|| {
                    auth_error(
                        "Paste the complete code from the browser, including its state suffix.",
                    )
                })?,
                &flow.redirect,
                &flow.state,
                true,
            )?,
            ProviderKind::Codex => {
                if callback.is_some() {
                    return Err(auth_error(
                        "Complete the browser login using its local callback.",
                    ));
                }
                let receiver = flow
                    .callback
                    .take()
                    .ok_or_else(|| auth_error("The login callback is unavailable."))?;
                let received = tokio::time::timeout(
                    flow.expires.saturating_duration_since(Instant::now()),
                    receiver,
                )
                .await
                .map_err(|_| auth_error("This login has expired. Start again."))?
                .map_err(|_| auth_error("The login was cancelled."))??;
                validate_callback(&received, &flow.redirect, &flow.state, false)?
            }
        };
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
}

fn validate_callback(
    raw: &str,
    redirect: &str,
    expected_state: &str,
    allow_manual: bool,
) -> Result<String, SimpleProviderError> {
    let raw = raw.trim();
    let (code, state) = if raw.starts_with("http://") || raw.starts_with("https://") {
        let url = Url::parse(raw).map_err(|_| auth_error("The callback URL is invalid."))?;
        let expected =
            Url::parse(redirect).map_err(|_| auth_error("The callback URL is invalid."))?;
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
        (codes[0].1.to_string(), states[0].1.to_string())
    } else if allow_manual {
        let (code, state) = raw
            .split_once('#')
            .ok_or_else(|| auth_error("Paste the complete code, including the state suffix."))?;
        (code.to_owned(), state.to_owned())
    } else {
        return Err(auth_error("A complete callback URL is required."));
    };
    if state != expected_state {
        return Err(auth_error("The callback state does not match this login."));
    }
    if code.is_empty() || code.len() > 8192 || code.chars().any(char::is_control) {
        return Err(auth_error("The authorization code is invalid."));
    }
    Ok(code)
}

async fn listen_for_callback(
    listener: TcpListener,
    redirect: String,
    state: String,
) -> Result<String, SimpleProviderError> {
    loop {
        let (mut socket, address) = listener
            .accept()
            .await
            .map_err(|_| auth_error("The local login callback stopped."))?;
        if !address.ip().is_loopback() {
            continue;
        }
        let mut buffer = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(5), async {
            while buffer.len() < 16_384 && !buffer.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
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
            continue;
        }
        let header = String::from_utf8_lossy(&buffer);
        let request = header.lines().next().unwrap_or_default();
        let parts: Vec<_> = request.split_whitespace().collect();
        let raw =
            if parts.len() == 3 && parts[0] == "GET" && parts[1].starts_with("/auth/callback?") {
                Some(format!(
                    "{}{}",
                    redirect.trim_end_matches("/auth/callback"),
                    parts[1]
                ))
            } else {
                None
            };
        let valid = raw
            .as_ref()
            .is_some_and(|raw| validate_callback(raw, &redirect, &state, false).is_ok());
        let (status, body) = if valid {
            (
                "200 OK",
                "Login received. Return to Quota Control to finish connecting your account.",
            )
        } else {
            (
                "400 Bad Request",
                "This callback does not match an active Quota Control login.",
            )
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            socket.write_all(response.as_bytes()),
        )
        .await;
        if valid {
            return Ok(raw.unwrap_or_default());
        }
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

    #[test]
    fn callbacks_reject_conflicting_state_and_wrong_origins() {
        for callback in [
            "https://attacker.invalid/oauth/code/callback?code=fixture&state=expected",
            "https://platform.claude.com/oauth/code/callback?code=fixture&state=expected&state=other",
            "https://platform.claude.com/oauth/code/callback?code=fixture&state=other",
            "fixture#other",
            "fixture",
        ] {
            assert!(validate_callback(callback, CLAUDE_REDIRECT, "expected", true).is_err());
        }
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
        let state = Url::parse(&flow.authorization_url)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        let cloned = manager.clone();
        let id = flow.flow_id.clone();
        let task = tokio::spawn(async move {
            cloned
                .complete_login(&id, Some(format!("fixture#{state}")))
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager
                .cancellations
                .lock()
                .await
                .contains_key(&flow.flow_id)
            {
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
