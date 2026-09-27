//! The contract every service implements, and the values it trades with the runtime.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uc_core::{
    MetricLine, PlanTerm, Provider, ProviderLink, SharedHttpClient, SimpleProviderError,
    WidgetDescriptor,
};

/// Secret material for one account: a token document, an API key, a session cookie. Only the
/// service that produced it reads it, it never crosses IPC, and `Debug` never prints it.
#[derive(Clone, PartialEq)]
pub struct Secret(Value);

impl Secret {
    pub fn new(value: Value) -> Self {
        Self(value)
    }

    /// A secret that is only an API key, as a saved key or an environment variable provides.
    pub fn api_key(key: &str) -> Self {
        Self(serde_json::json!({ "apiKey": key.trim() }))
    }

    pub fn value(&self) -> &Value {
        &self.0
    }

    /// The API key of a key-based secret.
    pub fn key(&self) -> Option<&str> {
        self.str("/apiKey")
    }

    /// The non-empty string at a JSON pointer (`/tokens/access_token`).
    pub fn str(&self, pointer: &str) -> Option<&str> {
        self.0
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
#[derive(Default)]
pub struct Memo(tokio::sync::Mutex<HashMap<&'static str, MemoEntry>>);

impl Memo {
    /// The value under `key`, unless it expired by `now`.
    pub async fn get(&self, key: &'static str, now: DateTime<Utc>) -> Option<Value> {
        let mut entries = self.0.lock().await;
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
        self.0.lock().await.insert(key, (value, expires));
    }

    pub async fn remove(&self, key: &'static str) {
        self.0.lock().await.remove(key);
    }
}

/// Everything one fetch may use.
pub struct FetchContext<'a> {
    pub secret: &'a Secret,
    pub http: &'a SharedHttpClient,
    pub now: DateTime<Utc>,
    pub memo: &'a Memo,
}

/// One refresh's answer: the plan's name and the metric lines the service's widgets read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub plan: Option<String>,
    pub lines: Vec<MetricLine>,
    pub plan_term: Option<PlanTerm>,
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
            warning: None,
        }
    }

    pub fn with_plan_term(mut self, term: Option<PlanTerm>) -> Self {
        self.plan_term = term;
        self
    }

    pub fn with_warning(mut self, warning: Option<String>) -> Self {
        self.warning = warning;
        self
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
