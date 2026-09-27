//! Adding a service's account by signing in through the browser, the way the service's own apps
//! sign in: a Google account (Gemini CLI, Antigravity), a GitHub account (Copilot), or the
//! service's own sign-in page. The token document the sign-in returns is saved in the key store,
//! and the card reads it the way it reads that app's login on this computer.
//!
//! Two shapes cover most services: [`start_code`], an OAuth 2 authorization code with PKCE whose
//! browser comes back to a listener on this computer, and [`start_device`], an OAuth 2 device
//! authorization the app polls while the user approves it in the browser. A service with its own
//! kind of sign-in page opens it and asks its service with [`polled`] until the page is done.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, oneshot};
use uc_accounts::{KeyRecord, KeyStore};
use uc_core::loopback::{
    self, Callback, CallbackContext, LoginLanguage, LoginPage, PageBrand, SignInAction,
};
use uc_core::{
    ErrorCategory, HttpRequest, ReqwestHttpClient, SharedHttpClient, SimpleProviderError,
};
use url::Url;

use crate::catalog::{login_card_id, service};
use crate::service::Service;

/// How long a sign-in waits for the browser before it gives up.
pub const FLOW_LIFETIME: Duration = Duration::from_secs(600);
/// How long one sign-in request may take.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Open sign-ins kept at once; each service has at most one.
const MAX_FLOWS: usize = 8;
/// The longest label a signed-in card keeps.
const MAX_LABEL_LENGTH: usize = 256;

/// The way an account is signed in to, as the Accounts screen offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Google,
    GitHub,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::GitHub => "github",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "google" => Some(Self::Google),
            "github" => Some(Self::GitHub),
            _ => None,
        }
    }

    fn action(self) -> SignInAction {
        match self {
            Self::Google => SignInAction::Google,
            Self::GitHub => SignInAction::GitHub,
        }
    }
}

/// What a finished sign-in hands back: the account and the document its card reads.
#[derive(Clone, Debug)]
pub struct SignedIn {
    /// The account behind the login (an email, a user id), hashed into the card id the same way a
    /// login found on this computer is, so the two never show as separate cards.
    pub identity: String,
    /// The account's email or user name, shown after the service's name.
    pub label: Option<String>,
    /// What the card reads, in the shape the service reads its own app's login.
    pub document: Value,
}

/// How a started sign-in ended: the account or why there is none, and the browser tab waiting to
/// say so, when the browser came back to this computer.
pub struct Finished {
    pub result: Result<SignedIn, SimpleProviderError>,
    pub page: Option<oneshot::Sender<LoginPage>>,
}

impl Finished {
    fn failed(error: SimpleProviderError) -> Self {
        Self {
            result: Err(error),
            page: None,
        }
    }
}

pub type Finish = Pin<Box<dyn Future<Output = Finished> + Send + 'static>>;

/// Turning a token answer into the account it belongs to (an email lookup, the document's shape).
pub type Converted = Pin<Box<dyn Future<Output = Result<SignedIn, SimpleProviderError>> + Send>>;
pub type Convert = fn(SharedHttpClient, Value) -> Converted;

/// A sign-in the browser is working on.
pub struct Pending {
    /// The page to open, and to open again from the Accounts screen.
    pub url: String,
    /// The code to type on that page, for a device sign-in that cannot fill it in by itself.
    pub user_code: Option<String>,
    pub finish: Finish,
}

/// What a sign-in starts with.
#[derive(Clone)]
pub struct StartContext {
    pub http: SharedHttpClient,
    pub language: LoginLanguage,
    /// The service's name, for the page the browser shows at the end.
    pub product: &'static str,
}

/// A browser sign-in a service offers.
#[async_trait]
pub trait SignIn: Send + Sync + 'static {
    /// The ways the Accounts screen offers, in order.
    fn methods(&self) -> &'static [Method];

    /// Open a sign-in: what the browser shows and how it finishes.
    async fn start(
        &self,
        method: Method,
        context: &StartContext,
    ) -> Result<Pending, SimpleProviderError>;
}

pub(crate) fn invalid(message: impl Into<String>) -> SimpleProviderError {
    SimpleProviderError::new(ErrorCategory::AuthInvalid, message)
}

pub(crate) fn network(product: &str) -> SimpleProviderError {
    SimpleProviderError::new(
        ErrorCategory::Network,
        format!("Cannot connect to {product}. Check the connection and try again."),
    )
}

/// `bytes` random bytes, URL-safe base64 without padding.
pub fn random_token(bytes: usize) -> Result<String, SimpleProviderError> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes(bytes)?))
}

/// `bytes` random bytes from the operating system.
pub fn random_bytes(bytes: usize) -> Result<Vec<u8>, SimpleProviderError> {
    let mut buffer = vec![0u8; bytes];
    SystemRandom::new()
        .fill(&mut buffer)
        .map_err(|_| invalid("Cannot start a sign-in on this computer."))?;
    Ok(buffer)
}

/// `raw` when it is an `https` page on one of `hosts` or their subdomains: the only pages a sign-in
/// opens from a service's answer.
pub fn page_on(raw: &str, hosts: &[&str]) -> Option<String> {
    let url = Url::parse(raw.trim()).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let known = hosts.iter().any(|allowed| {
        host == *allowed
            || host
                .strip_suffix(allowed)
                .is_some_and(|prefix| prefix.ends_with('.'))
    });
    (url.scheme() == "https" && known).then(|| url.to_string())
}

/// One look at whether a sign-in the service's page is working on has finished.
pub enum Poll {
    /// Not yet.
    Waiting,
    /// Not yet, and the service asks to be asked less often.
    SlowDown,
    /// Finished: the service's answer, for the service to turn into the account.
    Approved(Value),
    /// Refused, expired, or no longer valid.
    Failed(SimpleProviderError),
}

/// Asking the service once; each call sends a new request.
pub type Look = Pin<Box<dyn Future<Output = Poll> + Send>>;

/// How often a polled sign-in asks its service, and for how long.
#[derive(Clone, Copy, Debug)]
pub struct Polling {
    pub interval: Duration,
    /// What a request to slow down adds to the interval.
    pub slow_down: Duration,
    /// Capped at [`FLOW_LIFETIME`].
    pub lifetime: Duration,
}

/// A sign-in that finishes on the service's side: after each interval `look` asks the service
/// whether the user approved it, until the service answers, refuses, or `timing.lifetime` passes.
pub fn polled<F>(
    url: String,
    user_code: Option<String>,
    timing: Polling,
    http: SharedHttpClient,
    mut look: F,
    convert: Convert,
) -> Pending
where
    F: FnMut() -> Look + Send + 'static,
{
    let finish: Finish = Box::pin(async move {
        let deadline = tokio::time::Instant::now() + timing.lifetime.min(FLOW_LIFETIME);
        let mut interval = timing.interval;
        loop {
            tokio::time::sleep(interval).await;
            if tokio::time::Instant::now() >= deadline {
                return Finished::failed(loopback::expired());
            }
            match look().await {
                Poll::Waiting => {}
                Poll::SlowDown => interval += timing.slow_down,
                Poll::Approved(answer) => {
                    return Finished {
                        result: convert(http, answer).await,
                        page: None,
                    };
                }
                Poll::Failed(error) => return Finished::failed(error),
            }
        }
    });
    Pending {
        url,
        user_code,
        finish,
    }
}

/// The PKCE challenge of `verifier` (S256).
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Where the browser comes back to this computer.
#[derive(Clone, Copy, Debug)]
pub enum Redirect {
    /// `http://localhost:<port><path>`; port 0 takes any free port.
    Localhost { port: u16, path: &'static str },
    /// `http://127.0.0.1:<port><path>` on the first free port of a registered list.
    Ipv4 {
        ports: &'static [u16],
        path: &'static str,
    },
}

impl Redirect {
    async fn bind(self) -> Result<(Vec<tokio::net::TcpListener>, String), SimpleProviderError> {
        match self {
            Self::Localhost { port, path } => {
                let listeners = loopback::bind_localhost(port).await?;
                let port = loopback::port(&listeners[0])?;
                Ok((listeners, format!("http://localhost:{port}{path}")))
            }
            Self::Ipv4 { ports, path } => {
                let listener = loopback::bind_ipv4(ports).await?;
                let port = loopback::port(&listener)?;
                Ok((vec![listener], format!("http://127.0.0.1:{port}{path}")))
            }
        }
    }
}

/// How the token endpoint takes the authorization code.
#[derive(Clone, Copy, Debug)]
pub enum TokenBody {
    Form,
    Json,
}

/// An OAuth 2 client that signs in with an authorization code and PKCE, the browser coming back
/// to this computer, as the app it belongs to does.
#[derive(Clone, Copy, Debug)]
pub struct CodeClient {
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    pub client_id: &'static str,
    /// Empty for clients without one.
    pub client_secret: &'static str,
    pub scopes: &'static [&'static str],
    pub redirect: Redirect,
    /// More authorization parameters, such as `access_type=offline`.
    pub params: &'static [(&'static str, &'static str)],
    pub body: TokenBody,
}

/// Open `client`'s sign-in page with `extra` parameters (an identity provider to preselect); the
/// sign-in finishes when the browser comes back, the code is exchanged and `convert` names the
/// account.
pub async fn start_code(
    client: &'static CodeClient,
    method: Method,
    extra: &[(&str, &str)],
    context: &StartContext,
    convert: Convert,
) -> Result<Pending, SimpleProviderError> {
    let (listeners, redirect) = client.redirect.bind().await?;
    let verifier = random_token(32)?;
    let state = random_token(32)?;
    let mut url = Url::parse(client.authorize_url)
        .map_err(|_| invalid("The sign-in page address is invalid."))?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", client.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect);
        if !client.scopes.is_empty() {
            query.append_pair("scope", &client.scopes.join(" "));
        }
        query
            .append_pair("code_challenge", &challenge(&verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        for (name, value) in client.params.iter().chain(extra) {
            query.append_pair(name, value);
        }
    }
    let callback = CallbackContext {
        redirect: Url::parse(&redirect).map_err(|_| invalid("The sign-in callback is invalid."))?,
        state: state.clone(),
        brand: PageBrand::new(context.product, method.action()),
        language: context.language,
    };
    let http = context.http.clone();
    let product = context.product;
    let finish: Finish = Box::pin(async move {
        let Callback { url: back, page } = match loopback::serve(listeners, callback).await {
            Ok(callback) => callback,
            Err(error) => return Finished::failed(error),
        };
        let result = async {
            let code = loopback::validate(&back, &redirect, &state)?;
            let tokens = exchange(&http, client, &code, &redirect, &verifier, product).await?;
            convert(http.clone(), tokens).await
        }
        .await;
        Finished {
            result,
            page: Some(page),
        }
    });
    Ok(Pending {
        url: url.to_string(),
        user_code: None,
        finish,
    })
}

async fn exchange(
    http: &SharedHttpClient,
    client: &CodeClient,
    code: &str,
    redirect: &str,
    verifier: &str,
    product: &str,
) -> Result<Value, SimpleProviderError> {
    let mut fields = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect),
        ("client_id", client.client_id),
        ("code_verifier", verifier),
    ];
    if !client.client_secret.is_empty() {
        fields.push(("client_secret", client.client_secret));
    }
    let request = match client.body {
        TokenBody::Form => HttpRequest::post(client.token_url).form_body(&fields),
        TokenBody::Json => {
            let body: Map<String, Value> = fields
                .iter()
                .map(|(name, value)| ((*name).to_string(), Value::String((*value).to_string())))
                .collect();
            HttpRequest::post(client.token_url).json_body(&Value::Object(body))
        }
    };
    let response = http
        .send(
            request
                .header("Accept", "application/json")
                .timeout(REQUEST_TIMEOUT),
        )
        .await
        .map_err(|_| network(product))?;
    if !response.is_success() {
        return Err(SimpleProviderError::new(
            ErrorCategory::http(response.status),
            "The sign-in service rejected this code. Start a new sign-in.",
        ));
    }
    response
        .json::<Value>()
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| invalid("The sign-in service returned invalid credentials."))
}

/// An OAuth 2 device authorization client (RFC 8628), as the app it belongs to signs in.
#[derive(Clone, Copy, Debug)]
pub struct DeviceClient {
    pub device_url: &'static str,
    pub token_url: &'static str,
    pub client_id: &'static str,
    pub scope: &'static str,
    /// Headers every request carries beside `Accept: application/json`.
    pub headers: &'static [(&'static str, &'static str)],
}

impl DeviceClient {
    fn request(&self, url: &str, fields: &[(&str, &str)]) -> HttpRequest {
        let mut request = HttpRequest::post(url)
            .form_body(fields)
            .header("Accept", "application/json")
            .timeout(REQUEST_TIMEOUT);
        for (name, value) in self.headers {
            request = request.header(*name, *value);
        }
        request
    }
}

/// The verification page to open, when it is an `https` address, and whether it carries the
/// user code (`verification_uri_complete`), so the user does not type it.
fn verification_page(body: &Value) -> Option<(String, bool)> {
    let page = |name: &str| {
        body.get(name)
            .and_then(Value::as_str)
            .and_then(|raw| Url::parse(raw.trim()).ok())
            .filter(|url| url.scheme() == "https" && url.host_str().is_some())
            .map(String::from)
    };
    page("verification_uri_complete")
        .map(|url| (url, true))
        .or_else(|| {
            page("verification_uri")
                .or_else(|| page("verification_url"))
                .map(|url| (url, false))
        })
}

/// Start a device sign-in: the page to open, the code to type there (unless the page carries it),
/// and polling until the user approves.
pub async fn start_device(
    client: &'static DeviceClient,
    context: &StartContext,
    convert: Convert,
) -> Result<Pending, SimpleProviderError> {
    let product = context.product;
    let response = context
        .http
        .send(client.request(
            client.device_url,
            &[("client_id", client.client_id), ("scope", client.scope)],
        ))
        .await
        .map_err(|_| network(product))?;
    let body: Value = response.json().unwrap_or(Value::Null);
    let device_code = body.get("device_code").and_then(Value::as_str);
    let user_code = body.get("user_code").and_then(Value::as_str);
    let (Some(device_code), Some(user_code), Some((url, carries_code)), true) = (
        device_code,
        user_code,
        verification_page(&body),
        response.is_success(),
    ) else {
        return Err(SimpleProviderError::new(
            ErrorCategory::http(response.status),
            format!("{product} did not start a sign-in. Try again later."),
        ));
    };
    let interval = body
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .clamp(1, 60);
    let lifetime = body
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(FLOW_LIFETIME.as_secs())
        .min(FLOW_LIFETIME.as_secs());
    let device_code = device_code.to_string();
    let requests = context.http.clone();
    let look = move || -> Look {
        let http = requests.clone();
        let device_code = device_code.clone();
        Box::pin(async move { look_device(client, &http, &device_code).await })
    };
    Ok(polled(
        url,
        (!carries_code).then(|| user_code.to_string()),
        Polling {
            interval: Duration::from_secs(interval),
            slow_down: Duration::from_secs(5),
            lifetime: Duration::from_secs(lifetime),
        },
        context.http.clone(),
        look,
        convert,
    ))
}

async fn look_device(client: &DeviceClient, http: &SharedHttpClient, device_code: &str) -> Poll {
    let request = client.request(
        client.token_url,
        &[
            ("client_id", client.client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ],
    );
    let Ok(response) = http.send(request).await else {
        return Poll::Waiting;
    };
    let body: Value = response.json().unwrap_or(Value::Null);
    if body
        .get("access_token")
        .and_then(Value::as_str)
        .is_some_and(|token| !token.trim().is_empty())
    {
        return Poll::Approved(body);
    }
    match body.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => Poll::Waiting,
        Some("slow_down") => Poll::SlowDown,
        Some("expired_token") => Poll::Failed(loopback::expired()),
        Some("access_denied") => Poll::Failed(denied()),
        _ if response.status >= 500 => Poll::Waiting,
        _ => Poll::Failed(unfinished(response.status)),
    }
}

/// The user turned the sign-in down on the service's page.
pub(crate) fn denied() -> SimpleProviderError {
    invalid("The browser login was not authorized.")
}

/// A sign-in the service answered with an error it gave no reason for.
pub(crate) fn unfinished(status: u16) -> SimpleProviderError {
    SimpleProviderError::new(
        ErrorCategory::http(status.max(400)),
        "The sign-in could not be completed. Start a new sign-in.",
    )
}

/// A sign-in the Accounts screen can follow.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    pub flow_id: String,
    pub authorization_url: String,
    /// The code to type on the page, for a device sign-in that cannot fill it in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    pub expires_in_seconds: u64,
}

struct Flow {
    service: &'static str,
    url: String,
    expires: Instant,
    task: Option<tokio::task::JoinHandle<Result<KeyRecord, SimpleProviderError>>>,
    abort: tokio::task::AbortHandle,
}

/// The open browser sign-ins of services, each saved in the key store once the browser finished.
pub struct SignInManager {
    keys: KeyStore,
    http: SharedHttpClient,
    flows: Mutex<HashMap<String, Flow>>,
    cancelled: Mutex<HashMap<String, Instant>>,
}

impl SignInManager {
    pub fn new(keys: KeyStore) -> Self {
        Self {
            keys,
            http: ReqwestHttpClient::shared(),
            flows: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_http(mut self, http: SharedHttpClient) -> Self {
        self.http = http;
        self
    }

    /// Start signing in to `service_id` with `method`. An open sign-in of the same service is
    /// cancelled first, which releases its callback port.
    pub async fn begin(
        &self,
        service_id: &str,
        method: Method,
        language: LoginLanguage,
    ) -> Result<Started, SimpleProviderError> {
        let known = service(service_id).ok_or_else(|| invalid("Unsupported service"))?;
        self.begin_with(known, method, language).await
    }

    async fn begin_with(
        &self,
        known: &'static dyn Service,
        method: Method,
        language: LoginLanguage,
    ) -> Result<Started, SimpleProviderError> {
        let sign_in = known
            .sign_in()
            .filter(|sign_in| sign_in.methods().contains(&method))
            .ok_or_else(|| invalid("This service does not offer that sign-in."))?;
        let stale: Vec<String> = {
            let flows = self.flows.lock().await;
            flows
                .iter()
                .filter(|(_, flow)| flow.service == known.id() || flow.expires <= Instant::now())
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in stale {
            self.cancel(&id).await;
        }
        if self.flows.lock().await.len() >= MAX_FLOWS {
            return Err(invalid(
                "Too many pending logins. Cancel an existing login first.",
            ));
        }
        let context = StartContext {
            http: self.http.clone(),
            language,
            product: known.name(),
        };
        let Pending {
            url,
            user_code,
            finish,
        } = sign_in.start(method, &context).await?;
        let flow_id = random_token(16)?;
        let keys = self.keys.clone();
        let task = tokio::spawn(async move {
            let Ok(Finished { result, page }) = tokio::time::timeout(FLOW_LIFETIME, finish).await
            else {
                return Err(loopback::expired());
            };
            let outcome = match result {
                Ok(signed_in) => save(keys, known, method, signed_in).await,
                Err(error) => Err(error),
            };
            if let Some(page) = page {
                let _ = page.send(if outcome.is_ok() {
                    LoginPage::Connected
                } else {
                    LoginPage::Failed
                });
            }
            outcome
        });
        self.flows.lock().await.insert(
            flow_id.clone(),
            Flow {
                service: known.id(),
                url: url.clone(),
                expires: Instant::now() + FLOW_LIFETIME,
                abort: task.abort_handle(),
                task: Some(task),
            },
        );
        Ok(Started {
            flow_id,
            authorization_url: url,
            user_code,
            expires_in_seconds: FLOW_LIFETIME.as_secs(),
        })
    }

    /// Whether `flow_id` is one of this manager's sign-ins, open or just cancelled.
    pub async fn knows(&self, flow_id: &str) -> bool {
        self.flows.lock().await.contains_key(flow_id)
            || self.cancelled.lock().await.contains_key(flow_id)
    }

    /// The service a sign-in belongs to.
    pub async fn service_of(&self, flow_id: &str) -> Option<&'static str> {
        self.flows
            .lock()
            .await
            .get(flow_id)
            .map(|flow| flow.service)
    }

    /// The page of a sign-in that is still open, to show it again.
    pub async fn authorization_url(&self, flow_id: &str) -> Option<String> {
        self.flows
            .lock()
            .await
            .get(flow_id)
            .filter(|flow| flow.expires > Instant::now())
            .map(|flow| flow.url.clone())
    }

    /// Wait until the browser finished the sign-in and the account is saved.
    pub async fn complete(&self, flow_id: &str) -> Result<KeyRecord, SimpleProviderError> {
        let task = self
            .flows
            .lock()
            .await
            .get_mut(flow_id)
            .and_then(|flow| flow.task.take());
        let Some(task) = task else {
            let mut cancelled = self.cancelled.lock().await;
            cancelled.retain(|_, expires| *expires > Instant::now());
            return Err(if cancelled.remove(flow_id).is_some() {
                loopback::cancelled()
            } else {
                invalid("This login is no longer active. Start again.")
            });
        };
        let joined = task.await;
        self.flows.lock().await.remove(flow_id);
        match joined {
            Ok(outcome) => outcome,
            Err(error) if error.is_cancelled() => Err(loopback::cancelled()),
            Err(_) => Err(invalid(
                "The sign-in could not be completed. Start a new sign-in.",
            )),
        }
    }

    /// Stop a sign-in; the browser tab it opened then shows that it was cancelled.
    pub async fn cancel(&self, flow_id: &str) {
        let Some(flow) = self.flows.lock().await.remove(flow_id) else {
            return;
        };
        flow.abort.abort();
        let mut cancelled = self.cancelled.lock().await;
        cancelled.retain(|_, expires| *expires > Instant::now());
        cancelled.insert(flow_id.to_string(), flow.expires);
        while cancelled.len() > 64 {
            let oldest = cancelled
                .iter()
                .min_by_key(|(_, expires)| **expires)
                .map(|(id, _)| id.clone());
            if let Some(oldest) = oldest {
                cancelled.remove(&oldest);
            }
        }
    }
}

/// A label the card can show: the sign-in's, trimmed, else the service's name.
fn card_label(label: Option<&str>, name: &str) -> String {
    label
        .map(str::trim)
        .filter(|label| {
            !label.is_empty()
                && label.len() <= MAX_LABEL_LENGTH
                && !label.chars().any(char::is_control)
        })
        .unwrap_or(name)
        .to_string()
}

/// Saved on its own task, so a cancellation that arrives while the store is being written still
/// leaves a whole account.
async fn save(
    keys: KeyStore,
    known: &'static dyn Service,
    method: Method,
    signed_in: SignedIn,
) -> Result<KeyRecord, SimpleProviderError> {
    let identity = signed_in.identity.trim().to_string();
    if identity.is_empty() {
        return Err(invalid("The signed-in account could not be verified."));
    }
    let id = login_card_id(known.id(), &identity);
    let label = card_label(signed_in.label.as_deref(), known.name());
    let service_id = known.id();
    let saving = tokio::task::spawn_blocking(move || {
        keys.add_login(
            &id,
            service_id,
            &label,
            method.as_str(),
            &signed_in.document,
        )
    });
    match saving.await {
        Ok(Ok(record)) => Ok(record),
        _ => Err(SimpleProviderError::new(
            ErrorCategory::CredentialAccess,
            "The account could not be saved. Try signing in again.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Connection, FetchContext, Reading};
    use crate::testing::Scripted;
    use std::net::Ipv4Addr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use uc_core::{Provider, WidgetDescriptor};

    const CODE: CodeClient = CodeClient {
        authorize_url: "https://auth.example.com/authorize",
        token_url: "https://auth.example.com/token",
        client_id: "client-1",
        client_secret: "secret-1",
        scopes: &["email", "usage"],
        redirect: Redirect::Localhost {
            port: 0,
            path: "/oauth2callback",
        },
        params: &[("access_type", "offline")],
        body: TokenBody::Form,
    };

    const DEVICE: DeviceClient = DeviceClient {
        device_url: "https://auth.example.com/device",
        token_url: "https://auth.example.com/device-token",
        client_id: "device-client",
        scope: "read:user",
        headers: &[("User-Agent", "Fixture")],
    };

    fn account(_: SharedHttpClient, tokens: Value) -> Converted {
        Box::pin(async move {
            let token = tokens["access_token"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            Ok(SignedIn {
                identity: format!("user-of-{token}"),
                label: Some(" Me@Example.com ".into()),
                document: serde_json::json!({ "access_token": token }),
            })
        })
    }

    struct Fixture;

    #[async_trait]
    impl SignIn for Fixture {
        fn methods(&self) -> &'static [Method] {
            &[Method::Google, Method::GitHub]
        }

        async fn start(
            &self,
            method: Method,
            context: &StartContext,
        ) -> Result<Pending, SimpleProviderError> {
            match method {
                Method::Google => {
                    start_code(&CODE, method, &[("idp", "Google")], context, account).await
                }
                Method::GitHub => start_device(&DEVICE, context, account).await,
            }
        }
    }

    #[async_trait]
    impl Service for Fixture {
        fn id(&self) -> &'static str {
            "fixture"
        }

        fn name(&self) -> &'static str {
            "Fixture"
        }

        fn connection(&self) -> Connection {
            Connection::default()
        }

        fn sign_in(&self) -> Option<&'static dyn SignIn> {
            Some(&Fixture)
        }

        fn descriptors(&self, _: &Provider) -> Vec<WidgetDescriptor> {
            Vec::new()
        }

        async fn fetch(&self, _: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
            Ok(Reading::default())
        }
    }

    static FIXTURE: Fixture = Fixture;

    fn query(url: &str) -> HashMap<String, String> {
        Url::parse(url)
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

    fn manager(http: &Scripted) -> (SignInManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let keys = KeyStore::new(dir.path().join("api-keys"));
        (SignInManager::new(keys).with_http(http.shared()), dir)
    }

    #[tokio::test]
    async fn a_code_sign_in_comes_back_saves_the_account_and_tells_the_tab() {
        let http = Scripted::new().on(
            "POST",
            "https://auth.example.com/token",
            200,
            r#"{"access_token":"at-1","refresh_token":"rt-1","expires_in":3599}"#,
        );
        let (manager, _dir) = manager(&http);
        let started = manager
            .begin_with(&FIXTURE, Method::Google, LoginLanguage::Vietnamese)
            .await
            .unwrap();
        assert!(started.user_code.is_none());
        let params = query(&started.authorization_url);
        assert_eq!(params["client_id"], "client-1");
        assert_eq!(params["response_type"], "code");
        assert_eq!(params["scope"], "email usage");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["access_type"], "offline");
        assert_eq!(params["idp"], "Google");
        let redirect = Url::parse(&params["redirect_uri"]).unwrap();
        assert_eq!(redirect.host_str(), Some("localhost"));
        assert_eq!(redirect.path(), "/oauth2callback");
        let tab = browse(
            &params["redirect_uri"],
            format!("/oauth2callback?code=code-1&state={}", params["state"]),
        );
        let record = manager.complete(&started.flow_id).await.unwrap();
        assert!(tab.await.unwrap().contains("Đã kết nối Fixture"));
        assert_eq!(record.id, login_card_id("fixture", "user-of-at-1"));
        assert_eq!(record.label, "Me@Example.com");
        assert_eq!(record.sign_in.as_deref(), Some("google"));
        let token = &http.requests()[0];
        let body = String::from_utf8(token.body.clone().unwrap()).unwrap();
        assert!(body.contains("grant_type=authorization_code"));
        assert!(body.contains("code=code-1"));
        assert!(body.contains("client_secret=secret-1"));
        let verifier = body
            .split('&')
            .find_map(|pair| pair.strip_prefix("code_verifier="))
            .unwrap();
        assert_eq!(challenge(verifier), params["code_challenge"]);
        assert!(!manager.knows(&started.flow_id).await);
    }

    #[tokio::test]
    async fn a_refused_code_fails_the_sign_in_and_saves_nothing() {
        let http = Scripted::new().on(
            "POST",
            "https://auth.example.com/token",
            400,
            r#"{"error":"invalid_grant"}"#,
        );
        let (manager, dir) = manager(&http);
        let started = manager
            .begin_with(&FIXTURE, Method::Google, LoginLanguage::English)
            .await
            .unwrap();
        let params = query(&started.authorization_url);
        let tab = browse(
            &params["redirect_uri"],
            format!("/oauth2callback?code=bad&state={}", params["state"]),
        );
        assert!(manager.complete(&started.flow_id).await.is_err());
        assert!(tab.await.unwrap().contains("Fixture was not connected"));
        let keys = KeyStore::new(dir.path().join("api-keys"));
        assert!(keys.list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_cancelled_sign_in_releases_its_port_and_reports_cancelled() {
        let http = Scripted::new();
        let (manager, _dir) = manager(&http);
        let started = manager
            .begin_with(&FIXTURE, Method::Google, LoginLanguage::English)
            .await
            .unwrap();
        let params = query(&started.authorization_url);
        let port = Url::parse(&params["redirect_uri"]).unwrap().port().unwrap();
        assert_eq!(
            manager.authorization_url(&started.flow_id).await.as_deref(),
            Some(started.authorization_url.as_str())
        );
        manager.cancel(&started.flow_id).await;
        let error = manager.complete(&started.flow_id).await.unwrap_err();
        assert!(loopback::is_cancelled(&error));
        tokio::task::yield_now().await;
        let mut rebound = None;
        for _ in 0..50 {
            rebound = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
                .await
                .ok();
            if rebound.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            rebound.is_some(),
            "the cancelled listener still owns its port"
        );
        assert!(manager.authorization_url(&started.flow_id).await.is_none());
    }

    #[tokio::test]
    async fn a_new_sign_in_of_the_same_service_replaces_the_open_one() {
        let http = Scripted::new();
        let (manager, _dir) = manager(&http);
        let first = manager
            .begin_with(&FIXTURE, Method::Google, LoginLanguage::English)
            .await
            .unwrap();
        let second = manager
            .begin_with(&FIXTURE, Method::Google, LoginLanguage::English)
            .await
            .unwrap();
        assert!(loopback::is_cancelled(
            &manager.complete(&first.flow_id).await.unwrap_err()
        ));
        assert_eq!(manager.service_of(&second.flow_id).await, Some("fixture"));
        manager.cancel(&second.flow_id).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_device_sign_in_polls_until_approved() {
        let http = Scripted::new()
            .on(
                "POST",
                "https://auth.example.com/device",
                200,
                r#"{"device_code":"dc-1","user_code":"ABCD-1234","verification_uri":"https://auth.example.com/login/device","interval":5,"expires_in":900}"#,
            )
            .on(
                "POST",
                "https://auth.example.com/device-token",
                200,
                r#"{"error":"authorization_pending"}"#,
            )
            .on(
                "POST",
                "https://auth.example.com/device-token",
                200,
                r#"{"error":"slow_down","interval":10}"#,
            )
            .on(
                "POST",
                "https://auth.example.com/device-token",
                200,
                r#"{"access_token":"gho_1","token_type":"bearer"}"#,
            );
        let (manager, _dir) = manager(&http);
        let started = manager
            .begin_with(&FIXTURE, Method::GitHub, LoginLanguage::English)
            .await
            .unwrap();
        assert_eq!(started.user_code.as_deref(), Some("ABCD-1234"));
        assert_eq!(
            started.authorization_url,
            "https://auth.example.com/login/device"
        );
        let record = manager.complete(&started.flow_id).await.unwrap();
        assert_eq!(record.sign_in.as_deref(), Some("github"));
        let requests = http.requests();
        assert_eq!(requests.len(), 4);
        let body = String::from_utf8(requests[3].body.clone().unwrap()).unwrap();
        assert!(body.contains("device_code=dc-1"));
        assert!(body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"));
        assert_eq!(
            crate::testing::header(&requests[0], "User-Agent"),
            Some("Fixture")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_device_sign_in_ends_without_an_account() {
        for (answer, expired) in [
            (r#"{"error":"access_denied"}"#, false),
            (r#"{"error":"expired_token"}"#, true),
        ] {
            let http = Scripted::new()
                .on(
                    "POST",
                    "https://auth.example.com/device",
                    200,
                    r#"{"device_code":"dc","user_code":"U","verification_uri_complete":"https://auth.example.com/device?code=U"}"#,
                )
                .on("POST", "https://auth.example.com/device-token", 400, answer);
            let (manager, _dir) = manager(&http);
            let started = manager
                .begin_with(&FIXTURE, Method::GitHub, LoginLanguage::English)
                .await
                .unwrap();
            assert!(started.user_code.is_none(), "the page carries the code");
            let error = manager.complete(&started.flow_id).await.unwrap_err();
            assert_eq!(loopback::is_expired(&error), expired);
        }
    }

    #[tokio::test]
    async fn a_device_start_without_an_https_page_is_refused() {
        let http = Scripted::new().on(
            "POST",
            "https://auth.example.com/device",
            200,
            r#"{"device_code":"dc","user_code":"U","verification_uri":"javascript:alert(1)"}"#,
        );
        let (manager, _dir) = manager(&http);
        assert!(
            manager
                .begin_with(&FIXTURE, Method::GitHub, LoginLanguage::English)
                .await
                .is_err()
        );
    }

    #[test]
    fn labels_fall_back_to_the_service_name() {
        assert_eq!(card_label(Some(" a@b.c "), "Gemini"), "a@b.c");
        assert_eq!(card_label(Some("  "), "Gemini"), "Gemini");
        assert_eq!(card_label(Some("a\nb"), "Gemini"), "Gemini");
        assert_eq!(card_label(None, "Gemini"), "Gemini");
        assert_eq!(Method::parse("github"), Some(Method::GitHub));
        assert_eq!(Method::parse("GitHub"), None);
        assert_eq!(serde_json::to_value(Method::GitHub).unwrap(), "github");
    }
}
