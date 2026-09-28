//! Shared HTTP plumbing. Port of upstream `HTTPClient.swift` and `ProxyConfig.swift`.
//!
//! Providers depend on the `HttpClient` trait, never on `reqwest` directly, so tests can swap in a
//! scripted fake. Headers and successful bodies are never logged; error bodies are logged only as a
//! redacted, truncated preview.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::redact;

#[derive(Clone, Debug, PartialEq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub max_response_bytes: Option<usize>,
}

impl HttpRequest {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

    pub fn new(method: &str, url: impl Into<String>) -> Self {
        Self {
            method: method.to_string(),
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout: Self::DEFAULT_TIMEOUT,
            max_response_bytes: None,
        }
    }

    pub fn get(url: impl Into<String>) -> Self {
        Self::new("GET", url)
    }

    pub fn post(url: impl Into<String>) -> Self {
        Self::new("POST", url)
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn bearer(self, token: &str) -> Self {
        self.header("Authorization", format!("Bearer {token}"))
    }

    pub fn json_body(mut self, value: &serde_json::Value) -> Self {
        self.body = Some(serde_json::to_vec(value).unwrap_or_default());
        self.header("Content-Type", "application/json")
    }

    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    /// An `application/x-www-form-urlencoded` body (OAuth token requests).
    pub fn form_body(mut self, fields: &[(&str, &str)]) -> Self {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        self.body = Some(encoded.into_bytes());
        self.header("Content-Type", "application/x-www-form-urlencoded")
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn max_response_bytes(mut self, max_response_bytes: usize) -> Self {
        self.max_response_bytes = Some(max_response_bytes);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    /// Header names are lower-cased.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("The request could not be completed: {0}")]
    Transport(String),
    #[error("The request timed out.")]
    Timeout,
    #[error("Invalid HTTP request: {0}")]
    InvalidRequest(String),
}

#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError>;
}

pub type SharedHttpClient = Arc<dyn HttpClient>;

/// Optional proxy routing: `~/.usage-control/config.json` containing
/// `{"proxy": {"enabled": true, "url": "socks5://127.0.0.1:10808"}}`. Loopback always bypasses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyConfig {
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyScheme {
    Socks5,
    Http,
    Https,
}

impl ProxyScheme {
    fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "socks5" => Some(Self::Socks5),
            "http" => Some(Self::Http),
            "https" => Some(Self::Https),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Socks5 => "socks5",
            Self::Http => "http",
            Self::Https => "https",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Socks5 => 1080,
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

#[derive(Deserialize)]
struct ConfigFile {
    proxy: Option<ProxySection>,
}

#[derive(Deserialize)]
struct ProxySection {
    enabled: Option<bool>,
    url: Option<String>,
}

impl ProxyConfig {
    pub fn config_path() -> PathBuf {
        crate::paths::user_config_file()
    }

    /// Parses config-file text. `None` unless `proxy.enabled == true` with a valid socks5/http/https URL.
    pub fn parse(text: &str) -> Option<Self> {
        let config: ConfigFile = serde_json::from_str(text).ok()?;
        let proxy = config.proxy?;
        if proxy.enabled != Some(true) {
            return None;
        }
        let url = url::Url::parse(proxy.url?.trim()).ok()?;
        let scheme = ProxyScheme::parse(url.scheme())?;
        let host = url.host_str().filter(|host| !host.is_empty())?.to_string();
        let username = (!url.username().is_empty()).then(|| percent_decode(url.username()));
        let password = url.password().map(percent_decode);
        Some(Self {
            port: url.port().unwrap_or(scheme.default_port()),
            scheme,
            host,
            username,
            password,
        })
    }

    pub fn load_from(path: &Path) -> Option<Self> {
        std::fs::read_to_string(path)
            .ok()
            .as_deref()
            .and_then(Self::parse)
    }

    /// The process-wide proxy, read from disk exactly once.
    pub fn current() -> Option<&'static ProxyConfig> {
        static CURRENT: OnceLock<Option<ProxyConfig>> = OnceLock::new();
        CURRENT
            .get_or_init(|| Self::load_from(&Self::config_path()))
            .as_ref()
    }

    fn proxy_url(&self) -> String {
        format!("{}://{}:{}", self.scheme.as_str(), self.host, self.port)
    }

    /// The `reqwest` rule for this proxy: every scheme routed through it, credentials attached,
    /// loopback left direct. Shared by the provider client and the app updater.
    pub fn reqwest_proxy(&self) -> reqwest::Result<reqwest::Proxy> {
        let mut rule = reqwest::Proxy::all(self.proxy_url())?;
        if let (Some(user), Some(password)) = (&self.username, &self.password) {
            rule = rule.basic_auth(user, password);
        }
        Ok(rule.no_proxy(reqwest::NoProxy::from_string("localhost,127.0.0.1,::1")))
    }
}

fn percent_decode(raw: &str) -> String {
    fn hex(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            out.push(high << 4 | low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The production client: one pooled `reqwest` client with the optional proxy applied.
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

impl ReqwestHttpClient {
    pub const USER_AGENT: &'static str = concat!("QuotaControl/", env!("CARGO_PKG_VERSION"));

    pub fn new(proxy: Option<&ProxyConfig>) -> Result<Self, HttpError> {
        let mut builder = reqwest::Client::builder()
            .user_agent(Self::USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(90));
        if let Some(proxy) = proxy {
            let rule = proxy
                .reqwest_proxy()
                .map_err(|e| HttpError::InvalidRequest(e.to_string()))?;
            builder = builder.proxy(rule);
            tracing::info!(target: "config", "proxy enabled {}://{}:{}", proxy.scheme.as_str(), proxy.host, proxy.port);
        } else {
            builder = builder.no_proxy();
        }
        let client = builder
            .build()
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        Ok(Self { client })
    }

    /// A loopback-only client that accepts self-signed certificates, for local language servers.
    pub fn insecure_loopback() -> Result<Self, HttpError> {
        let client = reqwest::Client::builder()
            .user_agent(Self::USER_AGENT)
            .no_proxy()
            .tls_danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        Ok(Self { client })
    }

    /// The shared process-wide client configured from `ProxyConfig::current()`.
    pub fn shared() -> SharedHttpClient {
        static SHARED: OnceLock<SharedHttpClient> = OnceLock::new();
        SHARED
            .get_or_init(|| {
                let client = Self::new(ProxyConfig::current())
                    .or_else(|_| Self::new(None))
                    .expect("building the HTTP client without a proxy cannot fail");
                Arc::new(client)
            })
            .clone()
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|e| HttpError::InvalidRequest(e.to_string()))?;
        let mut builder = self
            .client
            .request(method, &request.url)
            .timeout(request.timeout);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body.clone() {
            builder = builder.body(body);
        }
        let mut response = builder.send().await.map_err(|error| {
            if error.is_timeout() {
                HttpError::Timeout
            } else {
                HttpError::Transport(error.to_string())
            }
        })?;
        let too_large = || HttpError::Transport("The response exceeded its size limit.".into());
        if let Some(limit) = request.max_response_bytes
            && response
                .content_length()
                .is_some_and(|length| length > limit as u64)
        {
            return Err(too_large());
        }
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            if error.is_timeout() {
                HttpError::Timeout
            } else {
                HttpError::Transport(error.to_string())
            }
        })? {
            if let Some(limit) = request.max_response_bytes
                && chunk.len() > limit.saturating_sub(body.len())
            {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        let line = format!(
            "{} {} -> {status}",
            request.method,
            redact::url(&request.url)
        );
        if status >= 400 {
            tracing::debug!(target: "http", "{line} body: {}", redact::body_preview(&String::from_utf8_lossy(&body)));
        } else {
            tracing::debug!(target: "http", "{line}");
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn serve(response: Vec<u8>, limit: Option<usize>) -> Result<HttpResponse, HttpError> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
                assert!(request.len() < 8192);
            }
            stream.write_all(&response).await.unwrap();
        });
        let mut request =
            HttpRequest::get(format!("http://{address}/")).timeout(Duration::from_secs(2));
        if let Some(limit) = limit {
            request = request.max_response_bytes(limit);
        }
        let result = ReqwestHttpClient::new(None).unwrap().send(request).await;
        server.await.unwrap();
        result
    }

    fn assert_size_error(result: Result<HttpResponse, HttpError>) {
        assert!(matches!(
            result,
            Err(HttpError::Transport(message)) if message == "The response exceeded its size limit."
        ));
    }

    #[tokio::test]
    async fn rejects_oversized_content_length_before_reading_the_body() {
        assert_size_error(
            serve(
                b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\n".to_vec(),
                Some(8),
            )
            .await,
        );
    }

    #[tokio::test]
    async fn rejects_chunked_body_before_accepting_an_oversized_chunk() {
        assert_size_error(serve(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\n1234\r\n5\r\n56789\r\n".to_vec(),
            Some(8),
        ).await);
    }

    #[tokio::test]
    async fn accepts_fixed_and_chunked_bodies_exactly_at_the_budget() {
        for response in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n12345678".to_vec(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\n1234\r\n4\r\n5678\r\n0\r\n\r\n".to_vec(),
        ] {
            let response = serve(response, Some(8)).await.unwrap();
            assert_eq!(response.status, 200);
            assert_eq!(response.body, b"12345678");
        }
    }

    #[tokio::test]
    async fn rejects_oversized_error_bodies() {
        assert_size_error(
            serve(
                b"HTTP/1.1 500 Internal Server Error\r\nConnection: close\r\n\r\n123456789"
                    .to_vec(),
                Some(8),
            )
            .await,
        );
    }

    #[tokio::test]
    async fn limits_decoded_gzip_bytes_and_preserves_exact_budget_content() {
        let compressed = [
            31, 139, 8, 0, 0, 0, 0, 0, 0, 10, 75, 76, 28, 5, 163, 96, 20, 36, 142, 80, 0, 0, 185,
            151, 85, 124, 0, 4, 0, 0,
        ];
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            compressed.len()
        ).into_bytes();
        response.extend_from_slice(&compressed);
        assert_size_error(serve(response.clone(), Some(64)).await);
        assert_eq!(
            serve(response, Some(1024)).await.unwrap().body,
            vec![b'a'; 1024]
        );
    }

    #[tokio::test]
    async fn an_unset_budget_preserves_existing_requests_and_zero_accepts_empty_bodies() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n12345678";
        assert_eq!(
            serve(response.to_vec(), None).await.unwrap().body,
            b"12345678"
        );
        assert_size_error(serve(response.to_vec(), Some(0)).await);
        assert!(
            serve(
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                Some(0),
            )
            .await
            .unwrap()
            .body
            .is_empty()
        );
    }

    #[test]
    fn parses_enabled_socks_proxy_with_default_port() {
        let config =
            ProxyConfig::parse(r#"{"proxy":{"enabled":true,"url":"socks5://127.0.0.1"}}"#).unwrap();
        assert_eq!(config.scheme, ProxyScheme::Socks5);
        assert_eq!(config.port, 1080);
    }

    #[test]
    fn parses_credentials_embedded_in_url() {
        let config = ProxyConfig::parse(
            r#"{"proxy":{"enabled":true,"url":"http://us%40r:p%3Ass@proxy.example.com:8080"}}"#,
        )
        .unwrap();
        assert_eq!(config.username.as_deref(), Some("us@r"));
        assert_eq!(config.password.as_deref(), Some("p:ss"));
        assert_eq!(config.port, 8080);
    }

    #[test]
    fn disabled_or_invalid_config_turns_proxy_off() {
        assert!(
            ProxyConfig::parse(r#"{"proxy":{"enabled":false,"url":"socks5://h:1"}}"#).is_none()
        );
        assert!(ProxyConfig::parse(r#"{"proxy":{"enabled":true,"url":"ftp://h:1"}}"#).is_none());
        assert!(ProxyConfig::parse("not json").is_none());
        assert!(ProxyConfig::parse(r#"{"proxy":{"enabled":true}}"#).is_none());
    }
}
