//! The contract every service implements, and the values it trades with the runtime.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uc_core::{
    MetricLine, PlanTerm, Provider, ProviderLink, SharedHttpClient, SimpleProviderError,
    WidgetDescriptor,
};

/// Secret material for one account: a token document, an API key, a session cookie. Only the
/// service that produced it reads it, it never crosses IPC, and `Debug` never prints it.
#[derive(Clone, PartialEq)]
pub struct Secret {
    value: Value,
    owned: bool,
}

impl Secret {
    pub fn new(value: Value) -> Self {
        Self {
            value,
            owned: false,
        }
    }

    /// The token document of an account signed in to from Quota Control: the card owns it, so it
    /// may renew a rotating refresh token and hand the new document back in its [`Reading`].
    pub fn owned(value: Value) -> Self {
        Self { value, owned: true }
    }

    /// A secret that is only an API key, as a saved key or an environment variable provides.
    pub fn api_key(key: &str) -> Self {
        Self::new(serde_json::json!({ "apiKey": key.trim() }))
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Whether the card owns the secret; another app's login is only borrowed.
    pub fn is_owned(&self) -> bool {
        self.owned
    }

    /// The API key of a key-based secret.
    pub fn key(&self) -> Option<&str> {
        self.str("/apiKey")
    }

    /// The non-empty string at a JSON pointer (`/tokens/access_token`).
    pub fn str(&self, pointer: &str) -> Option<&str> {
        self.value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(..)")
    }
}

/// A login that a service's own CLI, IDE or desktop app saved on this computer.
#[derive(Clone, Debug)]
pub struct Login {
    /// The account behind the login (a user id, an email, a token subject). It is hashed into the
    /// card id, so every login of one account shares a card and another account gets its own.
    pub identity: String,
    /// What the card shows after the service name, usually the account's email or user name.
    pub label: Option<String>,
    /// The app the login belongs to, for the Accounts screen ("Gemini CLI", "VS Code").
    pub origin: String,
    /// The file the login was read from, for diagnostics.
    pub location: PathBuf,
    pub secret: Secret,
}

impl Login {
    pub fn new(
        identity: impl Into<String>,
        origin: impl Into<String>,
        location: &Path,
        secret: Secret,
    ) -> Self {
        Self {
            identity: identity.into(),
            label: None,
            origin: origin.into(),
            location: location.to_path_buf(),
            secret,
        }
    }

    pub fn with_label(mut self, label: Option<String>) -> Self {
        self.label = label
            .map(|label| label.trim().to_string())
            .filter(|label| !label.is_empty());
        self
    }
}

/// How a service is connected.
#[derive(Clone, Copy, Debug, Default)]
pub struct Connection {
    /// The app whose login the service reads from this computer ("Gemini CLI"), when it reads one.
    pub login_from: Option<&'static str>,
    /// How to connect it with an API key, when it takes one.
    pub api_key: Option<ApiKeyHelp>,
}

impl Connection {
    pub fn login(app: &'static str) -> Self {
        Self {
            login_from: Some(app),
            api_key: None,
        }
    }

    pub fn api_key(help: ApiKeyHelp) -> Self {
        Self {
            login_from: None,
            api_key: Some(help),
        }
    }

    pub fn or_api_key(mut self, help: ApiKeyHelp) -> Self {
        self.api_key = Some(help);
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ApiKeyHelp {
    /// Environment variables that may hold a key; a key found in one becomes a card by itself.
    pub env: &'static [&'static str],
    /// The page where the user creates or copies a key.
    pub url: &'static str,
    /// Other values the Accounts screen asks for beside the key, as `(field, English label)`;
    /// they reach the service in the secret next to `apiKey` (`/teamId`).
    pub fields: &'static [(&'static str, &'static str)],
}

/// The longest key the Accounts screen keeps.
pub const MAX_KEY_LENGTH: usize = 8192;

/// What a service's key is, so the Accounts screen takes it the way it is copied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyFormat {
    /// One token without spaces: an API key or an access token.
    #[default]
    Token,
    /// One cookie's value from a signed-in browser session; the key label names the cookie.
    Cookie,
    /// A browser's whole `Cookie` request header, `name=value; other=value`, for a service whose
    /// web session needs every cookie of its site.
    CookieHeader,
}

impl KeyFormat {
    /// The key `raw` stands for, or `None` when it cannot be one. A token must have no spaces
    /// or line breaks inside, which would mean it was cut or joined. A Cookie header, pasted with
    /// or without its `Cookie:` name, becomes its `name=value` pairs joined by `; `.
    pub fn normalize(self, raw: &str) -> Option<String> {
        let text = raw.trim();
        let key = match self {
            KeyFormat::Token | KeyFormat::Cookie => (!text
                .chars()
                .any(|character| character.is_whitespace() || character.is_control()))
            .then(|| text.to_string())?,
            KeyFormat::CookieHeader => cookie_header(text)?,
        };
        (!key.is_empty() && key.len() <= MAX_KEY_LENGTH).then_some(key)
    }
}

fn cookie_header(text: &str) -> Option<String> {
    let body = match text.get(..7) {
        Some(name) if name.eq_ignore_ascii_case("cookie:") => &text[7..],
        _ => text,
    };
    let pairs: Vec<&str> = body
        .split(';')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .collect();
    let valid = !pairs.is_empty()
        && pairs.iter().all(|pair| {
            pair.split_once('=')
                .is_some_and(|(name, _)| !name.is_empty())
                && !pair
                    .chars()
                    .any(|character| character.is_whitespace() || character.is_control())
        });
    valid.then(|| pairs.join("; "))
}

/// The folders logins are read from. Tests point them at a temporary directory.
#[derive(Clone, Debug)]
pub struct Roots {
    /// The user's home (`~`).
    pub home: PathBuf,
    /// Roaming application data: `%APPDATA%` on Windows, `~/Library/Application Support` on
    /// macOS, `$XDG_CONFIG_HOME` (else `~/.config`) on Linux. VS Code-style apps keep their
    /// `User/globalStorage` here.
    pub app_data: PathBuf,
    /// Local application data: `%LOCALAPPDATA%` on Windows, `~/Library/Application Support` on
    /// macOS, `$XDG_DATA_HOME` (else `~/.local/share`) on Linux.
    pub local_data: PathBuf,
    /// Environment variables, or `None` for the process environment.
    vars: Option<HashMap<String, String>>,
}

impl Roots {
    /// This computer's folders and environment.
    pub fn system() -> Self {
        let home = uc_core::paths::home_dir();
        let (app_data, local_data) = if cfg!(windows) {
            (
                dirs::config_dir().unwrap_or_else(|| home.join("AppData").join("Roaming")),
                dirs::data_local_dir().unwrap_or_else(|| home.join("AppData").join("Local")),
            )
        } else if cfg!(target_os = "macos") {
            let support = home.join("Library").join("Application Support");
            (support.clone(), support)
        } else {
            (
                dirs::config_dir().unwrap_or_else(|| home.join(".config")),
                dirs::data_dir().unwrap_or_else(|| home.join(".local").join("share")),
            )
        };
        Self {
            home,
            app_data,
            local_data,
            vars: None,
        }
    }

    /// Folders under `dir` laid out like this platform's, with an empty environment.
    pub fn under(dir: &Path) -> Self {
        let (app_data, local_data) = if cfg!(windows) {
            (
                dir.join("AppData").join("Roaming"),
                dir.join("AppData").join("Local"),
            )
        } else if cfg!(target_os = "macos") {
            let support = dir.join("Library").join("Application Support");
            (support.clone(), support)
        } else {
            (dir.join(".config"), dir.join(".local").join("share"))
        };
        Self {
            home: dir.to_path_buf(),
            app_data,
            local_data,
            vars: Some(HashMap::new()),
        }
    }

    pub fn with_var(mut self, name: &str, value: &str) -> Self {
        self.vars
            .get_or_insert_with(HashMap::new)
            .insert(name.into(), value.into());
        self
    }

    /// A non-empty environment variable.
    pub fn var(&self, name: &str) -> Option<String> {
        let value = match &self.vars {
            Some(vars) => vars.get(name).cloned(),
            None => std::env::var(name).ok(),
        }?;
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    }

    /// A directory named by `variable` when set, else `fallback`.
    pub fn dir_from(&self, variable: &str, fallback: PathBuf) -> PathBuf {
        self.var(variable).map(PathBuf::from).unwrap_or(fallback)
    }
}

/// A remembered value and when it stops counting.
type MemoEntry = (Value, Option<DateTime<Utc>>);

/// Per-card memory kept between refreshes: a renewed access token, a looked-up project id.
#[derive(Clone, Default)]
pub struct Memo {
    entries: Arc<tokio::sync::Mutex<HashMap<&'static str, MemoEntry>>>,
    renewal: Option<Arc<tokio::sync::Mutex<Renewal>>>,
}

struct Renewal {
    snapshot: uc_accounts::LoginSnapshot,
    lock: Option<uc_accounts::RenewalLock>,
}

impl Memo {
    pub(crate) fn with_renewal(
        &self,
        snapshot: uc_accounts::LoginSnapshot,
        lock: uc_accounts::RenewalLock,
    ) -> Self {
        Self {
            entries: self.entries.clone(),
            renewal: Some(Arc::new(tokio::sync::Mutex::new(Renewal {
                snapshot,
                lock: Some(lock),
            }))),
        }
    }

    /// The value under `key`, unless it expired by `now`.
    pub async fn get(&self, key: &'static str, now: DateTime<Utc>) -> Option<Value> {
        let mut entries = self.entries.lock().await;
        match entries.get(key) {
            Some((_, Some(expires))) if *expires <= now => {
                entries.remove(key);
                None
            }
            Some((value, _)) => Some(value.clone()),
            None => None,
        }
    }

    pub async fn put(&self, key: &'static str, value: Value, expires: Option<DateTime<Utc>>) {
        self.entries.lock().await.insert(key, (value, expires));
    }

    pub async fn remove(&self, key: &'static str) {
        self.entries.lock().await.remove(key);
    }
}

/// Everything one fetch may use.
pub struct FetchContext<'a> {
    pub secret: &'a Secret,
    pub http: &'a SharedHttpClient,
    pub now: DateTime<Utc>,
    pub memo: &'a Memo,
}

/// Where a fetch leaves an owned sign-in's renewed token document for its card to save.
pub(crate) const RENEWED: &str = "signin.renewed";

impl FetchContext<'_> {
    pub async fn keep_renewed(&self, document: Value) -> Result<(), SimpleProviderError> {
        if self.secret.is_owned() {
            if let Some(renewal) = &self.memo.renewal {
                let mut renewal = renewal.lock().await;
                let mut snapshot = renewal.snapshot.clone();
                renewal.snapshot = uc_core::load_blocking(move || {
                    snapshot.replace(&document)?;
                    Ok::<_, uc_accounts::AccountError>(snapshot)
                })
                .await
                .map_err(|_| {
                    SimpleProviderError::new(
                        uc_core::ErrorCategory::CredentialAccess,
                        "The renewed sign-in could not be saved. Reconnect it in Accounts.",
                    )
                })?;
                renewal.lock.take();
            } else {
                self.memo.put(RENEWED, document, None).await;
            }
        }
        Ok(())
    }
}

/// One refresh's answer: the plan's name and the metric lines the service's widgets read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub plan: Option<String>,
    pub lines: Vec<MetricLine>,
    pub plan_term: Option<PlanTerm>,
    /// The account's email when the service's API names it; the card shows it under the title.
    pub account: Option<String>,
    /// A soft notice shown on a card that did refresh.
    pub warning: Option<String>,
}

impl Reading {
    pub fn new(plan: Option<String>, lines: Vec<MetricLine>) -> Self {
        Self {
            plan: plan
                .map(|plan| plan.trim().to_string())
                .filter(|plan| !plan.is_empty()),
            lines,
            plan_term: None,
            account: None,
            warning: None,
        }
    }

    pub fn with_plan_term(mut self, term: Option<PlanTerm>) -> Self {
        self.plan_term = term;
        self
    }

    /// Keep only a value that reads as an email address, so a user id never shows as one.
    pub fn with_account(mut self, account: Option<impl Into<String>>) -> Self {
        self.account = account
            .map(|account| account.into().trim().to_string())
            .filter(|account| is_email(account));
        self
    }

    pub fn with_warning(mut self, warning: Option<String>) -> Self {
        self.warning = warning;
        self
    }
}

fn is_email(value: &str) -> bool {
    match value.split_once('@') {
        Some((user, domain)) => {
            !user.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !value.chars().any(char::is_whitespace)
                && !domain.contains('@')
        }
        None => false,
    }
}

/// One AI service Quota Control can read.
#[async_trait]
pub trait Service: Send + Sync + 'static {
    /// The family id: cards are `<id>@<hash>`, and the popup draws the brand mark keyed by it.
    fn id(&self) -> &'static str;

    /// The service's name on its cards ("Copilot").
    fn name(&self) -> &'static str;

    /// Links on the card: a status page, the usage dashboard.
    fn links(&self) -> Vec<ProviderLink> {
        Vec::new()
    }

    fn connection(&self) -> Connection;

    /// Whether a card of this service starts hidden until the user turns it on in Customize: for
    /// services whose local files exist on computers that never signed in (Ollama's SSH key).
    fn starts_hidden(&self) -> bool {
        false
    }

    /// What the Accounts screen asks the user to paste as this service's key, in English: "API
    /// key", or for a service reached with a browser session, the session cookie it needs
    /// ("Session cookie (session_token)").
    fn key_label(&self) -> &'static str {
        "API key"
    }

    /// What the key field takes: one token, or for a web session that needs every cookie of its
    /// site, the whole Cookie header.
    fn key_format(&self) -> KeyFormat {
        KeyFormat::Token
    }

    /// How an account is added by signing in through the browser, when the service's apps sign in
    /// that way and the reader can use what the sign-in returns.
    fn sign_in(&self) -> Option<&'static dyn crate::signin::SignIn> {
        None
    }

    /// The logins the service's own apps saved under `roots`. Reads files only, never the network,
    /// and never writes: renewing a login stays the app's job.
    fn discover(&self, _roots: &Roots) -> Vec<Login> {
        Vec::new()
    }

    /// The widgets a card of this service feeds, in their default order.
    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor>;

    /// The account's current usage.
    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_keeps_only_an_email_as_its_account() {
        let account = |value: &str| Reading::default().with_account(Some(value)).account;
        assert_eq!(
            account(" dev@example.com ").as_deref(),
            Some("dev@example.com")
        );
        assert_eq!(account("user_2abc"), None);
        assert_eq!(account("dev@localhost"), None);
        assert_eq!(account("dev@example.com extra"), None);
        assert_eq!(account("@example.com"), None);
        assert_eq!(
            Reading::default().with_account(None::<String>).account,
            None
        );
    }

    #[test]
    fn a_token_keeps_no_inner_spaces() {
        let token = KeyFormat::Token;
        assert_eq!(
            token.normalize("  sk-abc123\n").as_deref(),
            Some("sk-abc123")
        );
        assert_eq!(token.normalize(""), None);
        assert_eq!(token.normalize("sk-abc 123"), None);
        assert_eq!(token.normalize("sk-abc\n123"), None);
        assert_eq!(token.normalize(&"k".repeat(MAX_KEY_LENGTH + 1)), None);
        assert_eq!(
            KeyFormat::Cookie.normalize(" eyJhbGciOi.x ").as_deref(),
            Some("eyJhbGciOi.x")
        );
        assert_eq!(KeyFormat::Cookie.normalize("a=1; b=2"), None);
    }

    #[test]
    fn a_cookie_header_is_kept_as_its_pairs() {
        let header = KeyFormat::CookieHeader;
        assert_eq!(
            header
                .normalize(" Cookie: session=a1;  theme=dark ; ")
                .as_deref(),
            Some("session=a1; theme=dark")
        );
        assert_eq!(header.normalize("cookie:sid=x").as_deref(), Some("sid=x"));
        assert_eq!(
            header.normalize("session=a=b==").as_deref(),
            Some("session=a=b==")
        );
        assert_eq!(header.normalize(""), None);
        assert_eq!(header.normalize("Cookie:"), None);
        assert_eq!(header.normalize("just-a-token"), None);
        assert_eq!(header.normalize("=value"), None);
        assert_eq!(header.normalize("a=1; b 2=3"), None);
        assert_eq!(header.normalize("a=1\nb=2"), None);
    }
}
