//! Ollama: the Ollama Cloud plan limits ollama.com's settings page shows (session, weekly and
//! monthly shares of the plan's allowance) and the spend beyond the plan over the last four weeks.
//!
//! Login: the signing key Ollama writes on its first run and `ollama signin` links to an ollama.com
//! account, an unencrypted OpenSSH ed25519 key at `%USERPROFILE%\.ollama\id_ed25519` on Windows and
//! `~/.ollama/id_ed25519` on macOS and Linux. Linux systemd installs keep the server's key under
//! `/usr/share/ollama`, which the desktop user cannot read; an API key connects those. The key never
//! leaves the computer: each request is signed as the Ollama CLI signs it, an Ed25519 signature of
//! `<METHOD>,<path>?ts=<unix seconds>` sent as `Authorization: <public key>:<signature>`. An API
//! key (`OLLAMA_API_KEY`, or one saved in Accounts) is sent as a bearer token instead.
//!
//! Endpoints: `GET https://ollama.com/api/usage` for the meters (undocumented; it backs the
//! settings page) and `POST https://ollama.com/api/me` for the plan and signup date, looked up twice
//! a day; a failed lookup keeps the meters and adds a warning.
//!
//! Signing in from Quota Control links a key of the card's own the way `ollama signin` links the
//! computer's: ollama.com's connect page asks the user to sign in (Google, GitHub or email, chosen
//! there) and to connect the key, named "Quota Control" in the account's key list, and the card asks
//! `/api/me`, signed with that key, until ollama.com knows it. Only the key's seed is saved.
//!
//! Ollama reports how much of each window is used, never when it resets, so the meters carry no
//! countdown. The one exception is a Free plan's monthly window, which ollama.com/pricing says
//! resets each month on the day the account signed up.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::{DateTime, Datelike, Duration, Months, Utc};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{
    ErrorCategory, HttpRequest, MetricKind, MetricLine, MetricValue, Provider, ProviderLink,
    SimpleProviderError, WidgetDescriptor,
};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::signin::{
    self, Converted, Look, Method, Pending, Poll, Polling, SignIn, SignedIn, StartContext,
};
use crate::support::{http, lines, oauth, value};

pub(crate) struct Ollama;

const NAME: &str = "Ollama";
/// The desktop app and the CLI share one key, so the login belongs to Ollama as a whole.
const APP: &str = "Ollama";
const HOST: &str = "https://ollama.com";
const USAGE_PATH: &str = "/api/usage";
const ACCOUNT_PATH: &str = "/api/me";
const ACCOUNT_MEMO: &str = "ollama.account";

const KEY_TYPE: &str = "ssh-ed25519";
const KEY_MAGIC: &[u8] = b"openssh-key-v1\0";
/// Far larger than the key file Ollama writes; anything bigger is not its key.
const MAX_KEY_BYTES: u64 = 64 * 1024;

/// The plan windows `/api/usage` reports under `limits`: the key doubles as the descriptor suffix
/// and the export key.
const WINDOWS: [(&str, &str); 3] = [
    ("session", "Session"),
    ("weekly", "Weekly"),
    ("monthly", "Monthly"),
];
const SPEND_SUFFIX: &str = "last4Weeks";
const SPEND: &str = "Last 4 Weeks";

const NOT_SIGNED_IN: &str = "Not signed in to Ollama Cloud. Run ollama signin to see usage.";
const KEY_REFUSED: &str =
    "Ollama refused the API key. Create a new one at ollama.com/settings/keys.";
const UNUSABLE_KEY: &str = "~/.ollama/id_ed25519 isn't a usable Ollama signing key.";
const UNREADABLE_KEY: &str = "Couldn't read ~/.ollama/id_ed25519. Check the file's permissions.";
const PLAN_WARNING: &str = "Couldn't read your Ollama plan. Usage below is still up to date.";

/// The page that links a key to the account the user signs in with.
const CONNECT: &str = "https://ollama.com/connect";
/// What the account's key list calls a key linked from Quota Control.
const DEVICE_NAME: &str = "Quota Control";

#[async_trait]
impl Service for Ollama {
    fn id(&self) -> &'static str {
        "ollama"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Usage", "https://ollama.com/settings"),
            ProviderLink::new("API Keys", "https://ollama.com/settings/keys"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::login(APP).or_api_key(ApiKeyHelp {
            env: &["OLLAMA_API_KEY"],
            url: "https://ollama.com/settings/keys",
            fields: &[],
        })
    }

    fn sign_in(&self) -> Option<&'static dyn SignIn> {
        Some(&Ollama)
    }

    /// Every Ollama install has the key, signed in to ollama.com or not, so the card waits to be
    /// turned on rather than greeting people who only run local models with a sign-in error.
    fn starts_hidden(&self) -> bool {
        true
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let path = roots.home.join(".ollama").join("id_ed25519");
        let login = match read_key(&path) {
            KeyFile::Missing => return Vec::new(),
            KeyFile::Usable(key) => Login::new(key.identity(), APP, &path, key.secret()),
            KeyFile::Unusable(fingerprint) => Login::new(
                fingerprint,
                APP,
                &path,
                Secret::new(json!({ "problem": "unusable" })),
            ),
            KeyFile::Unreadable => Login::new(
                "unreadable-key",
                APP,
                &path,
                Secret::new(json!({ "problem": "unreadable" })),
            ),
        };
        vec![login]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        // No session-start signal: telling a fresh window apart needs a reset date, and Ollama
        // publishes none.
        let mut descriptors: Vec<WidgetDescriptor> = WINDOWS
            .iter()
            .map(|(key, title)| {
                WidgetDescriptor::percent(
                    format!("{}.{key}", provider.id),
                    provider,
                    title,
                    None,
                    None,
                )
                .exporting_progress(key, "percent")
            })
            .collect();
        // Not a usage period: the row counts charges beyond the plan, so $0.00 does not mean the
        // account sat idle.
        descriptors.push(WidgetDescriptor::values(
            format!("{}.{SPEND_SUFFIX}", provider.id),
            provider,
            SPEND,
            None,
            Some(MetricKind::Dollars),
            Some("spent"),
            false,
            None,
            false,
        ));
        descriptors
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let credential = Credential::from_secret(context.secret)?;
        let request = credential.request("GET", USAGE_PATH, context.now);
        let response = http::send(context.http, request, NAME).await?;
        if matches!(response.status, 401 | 403) {
            return Err(if context.secret.is_owned() {
                SimpleProviderError::new(ErrorCategory::AuthExpired, oauth::SIGN_IN_REFUSED)
            } else {
                credential.refused()
            });
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let usage = http::parse(&response, NAME)?;
        if !usage.get("limits").is_some_and(Value::is_object) {
            return Err(http::decoding(NAME));
        }
        let account = account(context, &credential).await;
        let warning = account.is_none().then(|| PLAN_WARNING.to_string());
        let account = account.unwrap_or_default();
        let meters = meters(&usage, &account, context.now);
        Ok(Reading::new(account.plan, meters).with_warning(warning))
    }
}

#[async_trait]
impl SignIn for Ollama {
    /// ollama.com's page offers both, and the choice is made there.
    fn methods(&self) -> &'static [Method] {
        &[Method::Google, Method::GitHub]
    }

    async fn start(
        &self,
        _method: Method,
        context: &StartContext,
    ) -> Result<Pending, SimpleProviderError> {
        let seed: [u8; 32] = signin::random_bytes(32)?
            .try_into()
            .map_err(|_| signin::invalid("Cannot start a sign-in on this computer."))?;
        let key = SigningKey::from_seed(seed)
            .ok_or_else(|| signin::invalid("Cannot start a sign-in on this computer."))?;
        let line = format!(
            "{KEY_TYPE} {}",
            STANDARD.encode(wire_public_key(key.public()))
        );
        let mut page = url::Url::parse(CONNECT).map_err(|_| http::decoding(NAME))?;
        page.query_pairs_mut()
            .append_pair("name", DEVICE_NAME)
            .append_pair("key", &URL_SAFE_NO_PAD.encode(line));
        let key = Arc::new(key);
        let requests = context.http.clone();
        let look = move || -> Look {
            let http = requests.clone();
            let key = key.clone();
            Box::pin(async move { look_account(&http, &key).await })
        };
        Ok(signin::polled(
            page.to_string(),
            None,
            Polling {
                interval: std::time::Duration::from_secs(2),
                slow_down: std::time::Duration::from_secs(2),
                lifetime: signin::FLOW_LIFETIME,
            },
            context.http.clone(),
            look,
            signed_in,
        ))
    }
}

/// One look at a connect page: ollama.com refuses the key's signed `/api/me` until the user has
/// connected it, then names the account.
async fn look_account(http: &uc_core::SharedHttpClient, key: &SigningKey) -> Poll {
    let uri = format!("{ACCOUNT_PATH}?ts={}", Utc::now().timestamp());
    let request = HttpRequest::new("POST", format!("{HOST}{uri}"))
        .header("Authorization", key.authorization("POST", &uri))
        .header("Accept", "application/json")
        .header("Content-Length", "0")
        .timeout(signin::REQUEST_TIMEOUT);
    let Ok(response) = http.send(request).await else {
        return Poll::Waiting;
    };
    match response.status {
        200 => Poll::Approved(json!({
            "seed": STANDARD.encode(key.seed),
            "account": response.json::<Value>().unwrap_or(Value::Null),
        })),
        401 | 403 => Poll::Waiting,
        429 => Poll::SlowDown,
        status if status >= 500 => Poll::Waiting,
        status => Poll::Failed(signin::unfinished(status)),
    }
}

/// A connected key, saved as the card saves the computer's key (its seed) and named by the account
/// ollama.com reports, whose answer uses Go's field names or lowercase ones.
fn signed_in(_: uc_core::SharedHttpClient, answer: Value) -> Converted {
    Box::pin(async move {
        let seed = value::text(&answer, "/seed")
            .ok_or_else(|| signin::invalid("Cannot start a sign-in on this computer."))?
            .to_string();
        let account = &answer["account"];
        let text = |names: &[&str]| {
            names
                .iter()
                .find_map(|name| value::text(account, &format!("/{name}")))
                .map(str::to_string)
        };
        let email = text(&["email", "Email"]);
        let identity = text(&["id", "ID", "Id"])
            .or_else(|| email.clone())
            .ok_or_else(|| signin::invalid("Ollama did not say which account signed in."))?;
        let label = email.or_else(|| text(&["name", "Name"]));
        Ok(SignedIn {
            identity,
            label,
            document: json!({ "seed": seed }),
        })
    })
}

/// How a card signs its requests: with the key found on this computer, or with an API key.
enum Credential {
    Signed(SigningKey),
    ApiKey(String),
}

impl Credential {
    fn from_secret(secret: &Secret) -> Result<Self, SimpleProviderError> {
        if let Some(key) = secret.key() {
            return Ok(Self::ApiKey(key.to_string()));
        }
        if secret.str("/problem") == Some("unreadable") {
            return Err(SimpleProviderError::new(
                ErrorCategory::CredentialAccess,
                UNREADABLE_KEY,
            ));
        }
        secret
            .str("/seed")
            .and_then(|seed| STANDARD.decode(seed).ok())
            .and_then(|seed| <[u8; 32]>::try_from(seed).ok())
            .and_then(SigningKey::from_seed)
            .map(Self::Signed)
            .ok_or_else(|| http::invalid(UNUSABLE_KEY))
    }

    /// `method` on `path`: signed over a fresh `ts` parameter, or carrying the API key.
    fn request(&self, method: &str, path: &str, now: DateTime<Utc>) -> HttpRequest {
        let request = match self {
            Self::Signed(key) => {
                let uri = format!("{path}?ts={}", now.timestamp());
                HttpRequest::new(method, format!("{HOST}{uri}"))
                    .header("Authorization", key.authorization(method, &uri))
            }
            Self::ApiKey(key) => HttpRequest::new(method, format!("{HOST}{path}")).bearer(key),
        };
        request.header("Accept", "application/json")
    }

    /// What a 401 or 403 means: ollama.com links the key to no account (Ollama is installed but
    /// `ollama signin` never ran, or the account signed out), or the API key was revoked (Ollama's
    /// API keys do not expire). The first is "not logged in", as upstream reports it: most people
    /// who see it never had an Ollama Cloud login that could have expired.
    fn refused(&self) -> SimpleProviderError {
        match self {
            Self::Signed(_) => SimpleProviderError::new(ErrorCategory::NotLoggedIn, NOT_SIGNED_IN),
            Self::ApiKey(_) => http::invalid(KEY_REFUSED),
        }
    }
}

/// What `/api/me` says about the account.
#[derive(Default)]
struct Account {
    plan: Option<String>,
    signed_up: Option<DateTime<Utc>>,
}

/// The plan and signup date, looked up twice a day. `None` when the lookup failed, which keeps the
/// meters and asks for a warning; a failure is not remembered, so the next refresh tries again.
async fn account(context: &FetchContext<'_>, credential: &Credential) -> Option<Account> {
    if let Some(memo) = context.memo.get(ACCOUNT_MEMO, context.now).await {
        return Some(Account {
            plan: value::text(&memo, "/plan").map(str::to_string),
            signed_up: value::time(&memo, "/signedUp"),
        });
    }
    // Like the Ollama CLI's Go client, state the empty body: some front ends refuse a POST that
    // carries no length.
    let request = credential
        .request("POST", ACCOUNT_PATH, context.now)
        .header("Content-Length", "0");
    // Logged as upstream logs it, so a plan that never shows can be told apart from a missing one.
    let Ok(response) = http::send(context.http, request, NAME).await else {
        tracing::warn!(target: "ollama", "plan lookup could not connect; meters unaffected");
        return None;
    };
    if !response.is_success() {
        tracing::warn!(target: "ollama", "plan lookup answered HTTP {}; meters unaffected", response.status);
        return None;
    }
    let Ok(body) = http::parse(&response, NAME) else {
        tracing::warn!(target: "ollama", "plan lookup returned unreadable data; meters unaffected");
        return None;
    };
    // ollama.com answers with Go's default field names (`Plan`, `CreatedAt`), and Go's zero time
    // (0001-01-01) stands for an unset date: no account predates 2020.
    let account = Account {
        plan: ["/Plan", "/plan"]
            .iter()
            .find_map(|pointer| value::text(&body, pointer))
            .and_then(lines::plan_name),
        signed_up: ["/CreatedAt", "/createdAt", "/created_at"]
            .iter()
            .find_map(|pointer| value::time(&body, pointer))
            .filter(|time| time.year() >= 2020),
    };
    context
        .memo
        .put(
            ACCOUNT_MEMO,
            json!({
                "plan": account.plan,
                "signedUp": account.signed_up.map(|time| time.to_rfc3339()),
            }),
            Some(context.now + Duration::hours(12)),
        )
        .await;
    Some(account)
}

/// The window meters and the spend row. `usage` is a fraction of the plan's allowance (0.349 is
/// 34.9%); a window the answer leaves out is left out rather than shown as unused.
fn meters(usage: &Value, account: &Account, now: DateTime<Utc>) -> Vec<MetricLine> {
    let mut meters: Vec<MetricLine> = WINDOWS
        .iter()
        .filter_map(|(key, title)| {
            let used = value::number(usage, &format!("/limits/{key}/usage"))?;
            let reset = if *key == "monthly" {
                free_plan_reset(account, now)
            } else {
                None
            };
            Some(lines::percent(
                title,
                used * 100.0,
                reset,
                reset.map(|_| lines::MONTH_MS),
            ))
        })
        .collect();
    // `cost` is a decimal string over a rolling four weeks: $0.00 on a subscription, real amounts
    // for usage bought beyond the plan and for API keys.
    if let Some(cost) = value::number(usage, "/activity/cost").filter(|cost| *cost >= 0.0) {
        meters.push(lines::dollar_value(SPEND, cost));
    }
    let mut models: BTreeMap<String, Vec<MetricValue>> = BTreeMap::new();
    for (key, title) in WINDOWS {
        for model in usage
            .pointer(&format!("/limits/{key}/models"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(name), Some(count)) = (
                value::text(model, "/name"),
                value::number(model, "/request_count").filter(|count| *count >= 0.0),
            ) {
                models
                    .entry(name.to_string())
                    .or_default()
                    .push(MetricValue::count(count, "requests").with_label(title));
            }
        }
    }
    meters.extend(
        models
            .into_iter()
            .map(|(name, values)| lines::values(&name, values)),
    );
    MetricLine::append_no_data_if_needed(&mut meters);
    meters
}

/// When a Free plan's monthly window resets: every month on the day the account signed up.
fn free_plan_reset(account: &Account, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let free = account
        .plan
        .as_deref()
        .is_some_and(|plan| plan.eq_ignore_ascii_case("free"));
    if !free {
        return None;
    }
    next_anniversary(account.signed_up?, now)
}

/// The first monthly anniversary of `anchor` after `now`, a day the month lacks falling back to its
/// last day (a signup on the 31st renews on February 28). `None` for an anchor in the future.
fn next_anniversary(anchor: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if anchor > now {
        return None;
    }
    let months = (now.year() - anchor.year()) * 12 + now.month() as i32 - anchor.month() as i32;
    let months = u32::try_from(months).ok()?;
    (months..=months + 1)
        .filter_map(|offset| anchor.checked_add_months(Months::new(offset)))
        .find(|candidate| *candidate > now)
}

/// What `~/.ollama/id_ed25519` holds.
enum KeyFile {
    /// No file (Ollama never ran here) or an empty one.
    Missing,
    Usable(SigningKey),
    /// Not an unencrypted OpenSSH ed25519 key; carries a hash of the file as the card's identity.
    Unusable(String),
    /// A file that exists but cannot be read.
    Unreadable,
}

fn read_key(path: &Path) -> KeyFile {
    let mut bytes = Vec::new();
    let read = std::fs::File::open(path)
        .and_then(|file| file.take(MAX_KEY_BYTES + 1).read_to_end(&mut bytes));
    match read {
        Err(error) if error.kind() == ErrorKind::NotFound => return KeyFile::Missing,
        Err(_) => return KeyFile::Unreadable,
        Ok(_) => {}
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return KeyFile::Missing;
    }
    let key = if bytes.len() as u64 <= MAX_KEY_BYTES {
        std::str::from_utf8(&bytes).ok().and_then(parse_openssh)
    } else {
        None
    };
    match key {
        Some(key) => KeyFile::Usable(key),
        None => KeyFile::Unusable(
            Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        ),
    }
}

/// The key in an unencrypted OpenSSH private key file holding one ed25519 key, the only kind Ollama
/// writes. The container (OpenSSH `PROTOCOL.key`) is the magic, then the cipher, kdf and kdf
/// options, the key count, the public key and the private section: two equal check numbers, the
/// key type, the public key again, the 64-byte private key (seed then public key), a comment and
/// padding. Both public keys the file declares must be the one the seed derives: a damaged file
/// would otherwise sign requests that ollama.com attributes to no account, a dead end no sign-in
/// can fix.
fn parse_openssh(text: &str) -> Option<SigningKey> {
    let body: String = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("-----"))
        .collect();
    let blob = STANDARD.decode(body).ok()?;
    let mut outer = Wire(blob.strip_prefix(KEY_MAGIC)?);
    let cipher = outer.string()?;
    let kdf = outer.string()?;
    outer.string()?;
    // An encrypted key would need the user's passphrase; Ollama never writes one.
    if cipher != b"none" || kdf != b"none" || outer.u32()? != 1 {
        return None;
    }
    let mut public = Wire(outer.string()?);
    let mut private = Wire(outer.string()?);
    let check = private.u32()?;
    if private.u32()? != check || private.string()? != KEY_TYPE.as_bytes() {
        return None;
    }
    private.string()?;
    let pair = private.string()?;
    if pair.len() != 64 {
        return None;
    }
    let (seed, stated) = pair.split_first_chunk::<32>()?;
    let key = SigningKey::from_seed(*seed)?;
    let consistent = stated == key.public()
        && public.string()? == KEY_TYPE.as_bytes()
        && public.string()? == key.public();
    consistent.then_some(key)
}

/// A cursor over the SSH wire format: big-endian `u32` lengths followed by that many bytes. Every
/// read is bounds-checked, so a truncated file ends in `None` rather than a read past its end.
struct Wire<'a>(&'a [u8]);

impl<'a> Wire<'a> {
    fn u32(&mut self) -> Option<u32> {
        let data: &'a [u8] = self.0;
        let (head, rest) = data.split_first_chunk::<4>()?;
        self.0 = rest;
        Some(u32::from_be_bytes(*head))
    }

    fn string(&mut self) -> Option<&'a [u8]> {
        let length = usize::try_from(self.u32()?).ok()?;
        let data: &'a [u8] = self.0;
        let (value, rest) = data.split_at_checked(length)?;
        self.0 = rest;
        Some(value)
    }
}

/// The SSH wire form of an ed25519 public key, the base64 body of `id_ed25519.pub`: the key type
/// and the key, each length-prefixed.
fn wire_public_key(key: &[u8]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(4 + KEY_TYPE.len() + 4 + key.len());
    for part in [KEY_TYPE.as_bytes(), key] {
        blob.extend_from_slice(&(part.len() as u32).to_be_bytes());
        blob.extend_from_slice(part);
    }
    blob
}

/// An Ollama signing key: the ed25519 seed and the key pair it derives. The seed stays in the
/// card's memory; only signatures made with it leave the computer.
struct SigningKey {
    seed: [u8; 32],
    pair: Ed25519KeyPair,
}

impl SigningKey {
    fn from_seed(seed: [u8; 32]) -> Option<Self> {
        let pair = Ed25519KeyPair::from_seed_unchecked(&seed).ok()?;
        Some(Self { seed, pair })
    }

    fn public(&self) -> &[u8] {
        self.pair.public_key().as_ref()
    }

    /// The account the key stands for, as ollama.com knows it: its public key, which is not secret.
    fn identity(&self) -> String {
        STANDARD.encode(self.public())
    }

    fn secret(&self) -> Secret {
        Secret::new(json!({ "seed": STANDARD.encode(self.seed) }))
    }

    /// The `Authorization` value for `method` on `uri` (path and query), as the Ollama CLI signs a
    /// request: the SSH public key, a colon, and the Ed25519 signature of `<METHOD>,<uri>`.
    fn authorization(&self, method: &str, uri: &str) -> String {
        let signature = self.pair.sign(format!("{method},{uri}").as_bytes());
        format!(
            "{}:{}",
            STANDARD.encode(wire_public_key(self.public())),
            STANDARD.encode(signature.as_ref())
        )
    }
}

#[cfg(test)]
mod sign_in_tests {
    use super::*;
    use crate::testing::{Scripted, header, owned_context_at};
    use ring::signature::{ED25519, UnparsedPublicKey};

    fn start_context(http: &Scripted) -> StartContext {
        StartContext {
            http: http.shared(),
            language: uc_core::loopback::LoginLanguage::English,
            product: NAME,
        }
    }

    /// The raw public key an `ssh-ed25519 <base64>` line carries.
    fn public_key(line: &str) -> Vec<u8> {
        let blob = STANDARD
            .decode(line.strip_prefix("ssh-ed25519 ").unwrap())
            .unwrap();
        blob[blob.len() - 32..].to_vec()
    }

    #[tokio::test(start_paused = true)]
    async fn a_connected_key_is_saved_by_its_seed_and_named_by_the_account() {
        let http = Scripted::new()
            .on(
                "POST",
                &format!("{HOST}{ACCOUNT_PATH}"),
                401,
                "unauthorized",
            )
            .on(
                "POST",
                &format!("{HOST}{ACCOUNT_PATH}"),
                200,
                r#"{"id":"u-1","email":"me@example.com","name":"Minh","plan":"pro"}"#,
            );
        let pending = Ollama
            .start(Method::Google, &start_context(&http))
            .await
            .unwrap();
        let page = url::Url::parse(&pending.url).unwrap();
        assert_eq!(page.host_str(), Some("ollama.com"));
        assert_eq!(page.path(), "/connect");
        let query: std::collections::HashMap<_, _> = page.query_pairs().into_owned().collect();
        assert_eq!(query["name"], "Quota Control");
        let line = String::from_utf8(URL_SAFE_NO_PAD.decode(&query["key"]).unwrap()).unwrap();
        let public = public_key(&line);
        let account = pending.finish.await.result.unwrap();
        assert_eq!(account.identity, "u-1");
        assert_eq!(account.label.as_deref(), Some("me@example.com"));
        let seed: [u8; 32] = STANDARD
            .decode(account.document["seed"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            SigningKey::from_seed(seed).unwrap().public(),
            public.as_slice()
        );
        let requests = http.requests();
        let request = &requests[1];
        let (key, signature) = header(request, "Authorization")
            .unwrap()
            .split_once(':')
            .unwrap();
        assert_eq!(STANDARD.decode(key).unwrap(), wire_public_key(&public));
        let uri = request.url.strip_prefix(HOST).unwrap();
        assert!(uri.starts_with("/api/me?ts="));
        UnparsedPublicKey::new(&ED25519, &public)
            .verify(
                format!("POST,{uri}").as_bytes(),
                &STANDARD.decode(signature).unwrap(),
            )
            .unwrap();
    }

    #[tokio::test]
    async fn a_key_ollama_no_longer_knows_asks_to_sign_in_again_in_accounts() {
        let http = Scripted::new().on("GET", &format!("{HOST}{USAGE_PATH}"), 401, "{}");
        let scope = owned_context_at(
            &http,
            json!({"seed": STANDARD.encode([7u8; 32])}),
            Utc::now(),
        );
        let error = Ollama.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, oauth::SIGN_IN_REFUSED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Memo;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use ring::signature::{ED25519, UnparsedPublicKey};

    /// RFC 8032's first Ed25519 test key: published test data, not a secret.
    const SEED_HEX: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const PUBLIC_HEX: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    /// The test key's `id_ed25519.pub` body.
    const PUBLIC_BLOB: &str =
        "AAAAC3NzaC1lZDI1NTE5AAAAINdamAGCsQq31Uv+08lkBzoO4XLz2qYjJa8CGmj3B1Ea";
    /// Ed25519 is deterministic: these signatures of the two requests at `now()` were made with
    /// the test key by an independent implementation (Python `cryptography`).
    const USAGE_SIGNATURE: &str =
        "cVGoQ5dNqvBBEjgkBtqUXWTQt+2hDOVZ58kFwdLCGvQUh9Eu2svZvns1/QHscQhIS0WBTyZqzdozsr+ndgv+Bw==";
    const ACCOUNT_SIGNATURE: &str =
        "kt0u9/pBqtCSiplCehoD6WK09mNreaOwdHeGww5ETA/dYduRvGQL1t7TupbjFwW330IKuL1k+D05x9GEEKYZCA==";
    const CHECK: u32 = 0x1234_5678;

    /// A live `/api/usage` answer, trimmed (upstream's test fixture).
    const USAGE: &str = r#"{"activity":{"cost":"1.25000","period":{"type":"last_4_weeks","starting_at":"2026-08-03T00:00:00Z","ending_at":"2026-08-24T19:39:56Z"},"models":[]},
        "limits":{"session":{"usage":0.349,"models":[{"name":"minimax-m3","request_count":139}]},
                  "weekly":{"usage":0.316,"models":[{"name":"minimax-m3","request_count":646}]}}}"#;
    const ACCOUNT: &str =
        r#"{"ID":"0f3c2a1b","Email":"someone@example.com","Name":"someone","Plan":"pro"}"#;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn usage_url() -> String {
        format!("{HOST}{USAGE_PATH}")
    }

    fn account_url() -> String {
        format!("{HOST}{ACCOUNT_PATH}")
    }

    fn bytes32(hex: &str) -> [u8; 32] {
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect();
        bytes.try_into().unwrap()
    }

    fn seed() -> [u8; 32] {
        bytes32(SEED_HEX)
    }

    fn signed_secret() -> Value {
        json!({ "seed": STANDARD.encode(seed()) })
    }

    fn wire(out: &mut Vec<u8>, part: &[u8]) {
        out.extend_from_slice(&(part.len() as u32).to_be_bytes());
        out.extend_from_slice(part);
    }

    /// How a test key file differs from the one Ollama writes.
    struct Damage {
        cipher: &'static str,
        keys: u32,
        second_check: u32,
        key_type: &'static str,
        outer_public: Option<[u8; 32]>,
    }

    impl Default for Damage {
        fn default() -> Self {
            Self {
                cipher: "none",
                keys: 1,
                second_check: CHECK,
                key_type: KEY_TYPE,
                outer_public: None,
            }
        }
    }

    /// An OpenSSH private key file for `seed`, built at test time so no key file is committed.
    fn key_file(seed: [u8; 32], damage: Damage, newline: &str) -> String {
        let public = Ed25519KeyPair::from_seed_unchecked(&seed)
            .unwrap()
            .public_key()
            .as_ref()
            .to_vec();
        let mut outer_public = Vec::new();
        wire(&mut outer_public, damage.key_type.as_bytes());
        wire(
            &mut outer_public,
            damage
                .outer_public
                .as_ref()
                .map_or(&public[..], |key| &key[..]),
        );
        let mut private = Vec::new();
        private.extend_from_slice(&CHECK.to_be_bytes());
        private.extend_from_slice(&damage.second_check.to_be_bytes());
        wire(&mut private, damage.key_type.as_bytes());
        wire(&mut private, &public);
        wire(&mut private, &[&seed[..], &public[..]].concat());
        wire(&mut private, b"someone@computer");
        let mut padding = 1u8;
        while !private.len().is_multiple_of(8) {
            private.push(padding);
            padding += 1;
        }
        let mut blob = KEY_MAGIC.to_vec();
        wire(&mut blob, damage.cipher.as_bytes());
        wire(&mut blob, b"none");
        wire(&mut blob, b"");
        blob.extend_from_slice(&damage.keys.to_be_bytes());
        wire(&mut blob, &outer_public);
        wire(&mut blob, &private);
        let body = STANDARD.encode(blob);
        let lines: Vec<&str> = body
            .as_bytes()
            .chunks(70)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect();
        format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----{newline}{}{newline}-----END OPENSSH PRIVATE KEY-----{newline}",
            lines.join(newline)
        )
    }

    fn write_key(dir: &Path, contents: &str) {
        let folder = dir.join(".ollama");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("id_ed25519"), contents).unwrap();
    }

    #[tokio::test]
    async fn reads_the_meters_the_spend_and_the_plan() {
        let http = Scripted::new().on("GET", &usage_url(), 200, USAGE).on(
            "POST",
            &account_url(),
            200,
            ACCOUNT,
        );
        let scope = context_at(&http, signed_secret(), now());
        let reading = Ollama.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.warning, None);
        assert_eq!(
            reading.lines,
            vec![
                lines::percent("Session", 34.9, None, None),
                lines::percent("Weekly", 31.6, None, None),
                lines::dollar_value("Last 4 Weeks", 1.25),
                lines::values(
                    "minimax-m3",
                    vec![
                        MetricValue::count(139.0, "requests").with_label("Session"),
                        MetricValue::count(646.0, "requests").with_label("Weekly")
                    ]
                ),
            ]
        );
    }

    #[tokio::test]
    async fn requests_are_signed_exactly_as_the_ollama_cli_signs_them() {
        let http = Scripted::new().on("GET", &usage_url(), 200, USAGE).on(
            "POST",
            &account_url(),
            200,
            ACCOUNT,
        );
        let scope = context_at(&http, signed_secret(), now());
        Ollama.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(
            requests[0].url,
            "https://ollama.com/api/usage?ts=1790503200"
        );
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some(format!("{PUBLIC_BLOB}:{USAGE_SIGNATURE}").as_str())
        );
        assert_eq!(header(&requests[0], "Accept"), Some("application/json"));
        assert_eq!(requests[0].body, None);
        assert_eq!(requests[1].method, "POST");
        assert_eq!(requests[1].url, "https://ollama.com/api/me?ts=1790503200");
        assert_eq!(
            header(&requests[1], "Authorization"),
            Some(format!("{PUBLIC_BLOB}:{ACCOUNT_SIGNATURE}").as_str())
        );
        assert_eq!(header(&requests[1], "Content-Length"), Some("0"));
        assert_eq!(requests[1].body, None);
        let public = bytes32(PUBLIC_HEX);
        for (request, challenge) in [
            (&requests[0], "GET,/api/usage?ts=1790503200"),
            (&requests[1], "POST,/api/me?ts=1790503200"),
        ] {
            let (_, signature) = header(request, "Authorization")
                .unwrap()
                .split_once(':')
                .unwrap();
            UnparsedPublicKey::new(&ED25519, public)
                .verify(challenge.as_bytes(), &STANDARD.decode(signature).unwrap())
                .expect("the signature verifies with the RFC 8032 public key");
        }
    }

    #[tokio::test]
    async fn an_api_key_is_sent_as_a_bearer_token() {
        let http = Scripted::new().on("GET", &usage_url(), 200, USAGE).on(
            "POST",
            &account_url(),
            200,
            ACCOUNT,
        );
        let scope = context_at(&http, json!({ "apiKey": "ollama-test-key" }), now());
        let reading = Ollama.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        let requests = http.requests();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].url, "https://ollama.com/api/usage");
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some("Bearer ollama-test-key")
        );
        assert_eq!(header(&requests[0], "Accept"), Some("application/json"));
        assert_eq!(requests[1].method, "POST");
        assert_eq!(requests[1].url, "https://ollama.com/api/me");
        assert_eq!(
            header(&requests[1], "Authorization"),
            Some("Bearer ollama-test-key")
        );
        assert_eq!(header(&requests[1], "Content-Length"), Some("0"));
    }

    #[tokio::test]
    async fn a_free_plan_monthly_window_resets_on_the_signup_day() {
        let http = Scripted::new()
            .on(
                "GET",
                &usage_url(),
                200,
                r#"{"activity":{"cost":"0.00000","period":{"type":"last_4_weeks"},"models":[]},"limits":{"monthly":{"usage":0.053,"models":[]}}}"#,
            )
            .on(
                "POST",
                &account_url(),
                200,
                r#"{"ID":"0f3c2a1b","Email":"someone@example.com","Plan":"free","CreatedAt":"2025-01-31T08:30:00Z"}"#,
            );
        let scope = context_at(&http, signed_secret(), now());
        let reading = Ollama.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Free"));
        assert_eq!(
            reading.lines,
            vec![
                lines::percent(
                    "Monthly",
                    5.3,
                    Some(Utc.with_ymd_and_hms(2026, 9, 30, 8, 30, 0).unwrap()),
                    Some(lines::MONTH_MS),
                ),
                lines::dollar_value("Last 4 Weeks", 0.0),
            ]
        );
    }

    #[tokio::test]
    async fn an_unlinked_key_asks_to_sign_in_without_reading_the_plan() {
        for status in [401, 403] {
            let http = Scripted::new().on("GET", &usage_url(), status, "{}");
            let scope = context_at(&http, signed_secret(), now());
            let error = Ollama.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::NotLoggedIn, "HTTP {status}");
            assert_eq!(error.message, NOT_SIGNED_IN);
            assert_eq!(http.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn a_refused_api_key_is_reported_as_invalid() {
        for status in [401, 403] {
            let http = Scripted::new().on("GET", &usage_url(), status, "{}");
            let scope = context_at(&http, json!({ "apiKey": "revoked" }), now());
            let error = Ollama.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthInvalid, "HTTP {status}");
            assert_eq!(error.message, KEY_REFUSED);
            assert_eq!(http.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_are_reported() {
        for (status, category) in [
            (429, ErrorCategory::RateLimited),
            (503, ErrorCategory::Http5xx),
        ] {
            let http = Scripted::new().on("GET", &usage_url(), status, "{}");
            let scope = context_at(&http, signed_secret(), now());
            let error = Ollama.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category, "HTTP {status}");
        }
    }

    #[tokio::test]
    async fn an_answer_without_limits_cannot_be_read() {
        for body in [r#"{"activity":{"cost":"0"}}"#, "not json"] {
            let http = Scripted::new().on("GET", &usage_url(), 200, body);
            let scope = context_at(&http, signed_secret(), now());
            let error = Ollama.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
            assert_eq!(http.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn absent_windows_are_left_out_and_empty_limits_read_as_no_data() {
        let http = Scripted::new()
            .on(
                "GET",
                &usage_url(),
                200,
                r#"{"limits":{"weekly":{"usage":0.5}}}"#,
            )
            .on("GET", &usage_url(), 200, r#"{"limits":{}}"#)
            .on("POST", &account_url(), 200, ACCOUNT);
        let scope = context_at(&http, signed_secret(), now());
        let reading = Ollama.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![lines::percent("Weekly", 50.0, None, None)]
        );
        let reading = Ollama.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines, vec![MetricLine::no_usage_data()]);
    }

    #[tokio::test]
    async fn a_failed_plan_lookup_warns_keeps_the_meters_and_retries() {
        let http = Scripted::new().on("GET", &usage_url(), 200, USAGE).on(
            "POST",
            &account_url(),
            500,
            "{}",
        );
        let scope = context_at(&http, signed_secret(), now());
        for _ in 0..2 {
            let reading = Ollama.fetch(&scope.context()).await.unwrap();
            assert_eq!(reading.plan, None);
            assert_eq!(reading.warning.as_deref(), Some(PLAN_WARNING));
            assert_eq!(reading.lines.len(), 4);
        }
        assert_eq!(http.requests().len(), 4);
    }

    #[tokio::test]
    async fn the_plan_is_looked_up_twice_a_day() {
        let http = Scripted::new().on("GET", &usage_url(), 200, USAGE).on(
            "POST",
            &account_url(),
            200,
            r#"{"plan":"max"}"#,
        );
        let secret = Secret::new(signed_secret());
        let shared = http.shared();
        let memo = Memo::default();
        for hours in [0, 11, 13] {
            let context = FetchContext {
                secret: &secret,
                http: &shared,
                now: now() + Duration::hours(hours),
                memo: &memo,
            };
            let reading = Ollama.fetch(&context).await.unwrap();
            assert_eq!(reading.plan.as_deref(), Some("Max"));
        }
        let methods: Vec<_> = http
            .requests()
            .into_iter()
            .map(|request| request.method)
            .collect();
        assert_eq!(methods, ["GET", "POST", "GET", "GET", "POST"]);
    }

    #[tokio::test]
    async fn go_zero_signup_times_give_no_reset() {
        let http = Scripted::new()
            .on(
                "GET",
                &usage_url(),
                200,
                r#"{"limits":{"monthly":{"usage":0.25}}}"#,
            )
            .on(
                "POST",
                &account_url(),
                200,
                r#"{"Plan":"free","CreatedAt":"0001-01-01T00:00:00Z"}"#,
            );
        let scope = context_at(&http, signed_secret(), now());
        let reading = Ollama.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Free"));
        assert_eq!(
            reading.lines,
            vec![lines::percent("Monthly", 25.0, None, None)]
        );
    }

    #[tokio::test]
    async fn discovers_the_key_and_signs_with_it() {
        let dir = tempfile::tempdir().unwrap();
        write_key(dir.path(), &key_file(seed(), Damage::default(), "\n"));
        let logins = Ollama.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(
            logins[0].identity,
            "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo="
        );
        assert_eq!(logins[0].label, None);
        assert_eq!(logins[0].origin, "Ollama");
        assert_eq!(
            logins[0].location,
            dir.path().join(".ollama").join("id_ed25519")
        );
        let http = Scripted::new().on("GET", &usage_url(), 200, USAGE).on(
            "POST",
            &account_url(),
            200,
            ACCOUNT,
        );
        let scope = context_at(&http, logins[0].secret.value().clone(), now());
        Ollama.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some(format!("{PUBLIC_BLOB}:{USAGE_SIGNATURE}").as_str())
        );
        assert!(
            Ollama
                .discover(&Roots::under(&dir.path().join("empty")))
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_damaged_key_file_becomes_a_card_that_explains_itself() {
        let dir = tempfile::tempdir().unwrap();
        write_key(dir.path(), "-----BEGIN OPENSSH PRIVATE KEY-----\ngarbage\n");
        let logins = Ollama.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity.len(), 64);
        assert!(
            logins[0]
                .identity
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
        let http = Scripted::new();
        let scope = context_at(&http, logins[0].secret.value().clone(), now());
        let error = Ollama.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(error.message, UNUSABLE_KEY);
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn an_unreadable_key_file_is_reported_as_such() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ollama").join("id_ed25519")).unwrap();
        let logins = Ollama.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, "unreadable-key");
        let http = Scripted::new();
        let scope = context_at(&http, logins[0].secret.value().clone(), now());
        let error = Ollama.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::CredentialAccess);
        assert_eq!(error.message, UNREADABLE_KEY);
        assert!(http.requests().is_empty());
    }

    #[test]
    fn an_empty_key_file_reads_as_no_login() {
        let dir = tempfile::tempdir().unwrap();
        write_key(dir.path(), " \n");
        assert!(Ollama.discover(&Roots::under(dir.path())).is_empty());
    }

    #[test]
    fn parses_the_key_ollama_writes() {
        for (newline, bom) in [("\n", ""), ("\r\n", "\u{feff}")] {
            let text = format!("{bom}{}", key_file(seed(), Damage::default(), newline));
            let key = parse_openssh(&text).expect("a well-formed key parses");
            assert_eq!(key.public(), bytes32(PUBLIC_HEX));
            assert_eq!(key.seed, seed());
            assert_eq!(STANDARD.encode(wire_public_key(key.public())), PUBLIC_BLOB);
        }
    }

    #[test]
    fn rejects_keys_ollama_could_not_sign_with() {
        let foreign = bytes32("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c");
        let damaged = [
            Damage {
                cipher: "aes256-ctr",
                ..Damage::default()
            },
            Damage {
                keys: 2,
                ..Damage::default()
            },
            Damage {
                second_check: CHECK + 1,
                ..Damage::default()
            },
            Damage {
                key_type: "ssh-rsa",
                ..Damage::default()
            },
            Damage {
                outer_public: Some(foreign),
                ..Damage::default()
            },
        ];
        for damage in damaged {
            assert!(parse_openssh(&key_file(seed(), damage, "\n")).is_none());
        }
        let good = key_file(seed(), Damage::default(), "\n");
        let body: String = good
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        let truncated = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
            &body[..body.len() / 2]
        );
        for text in [
            "",
            "not a key at all",
            "-----BEGIN OPENSSH PRIVATE KEY-----\nZm9v\n-----END OPENSSH PRIVATE KEY-----",
            truncated.as_str(),
        ] {
            assert!(parse_openssh(text).is_none(), "{text}");
        }
    }

    #[test]
    fn monthly_anniversaries_fall_back_to_the_last_day_of_short_months() {
        let at = |year, month, day, hour, minute| {
            Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
                .unwrap()
        };
        let anchor = at(2025, 1, 31, 8, 30);
        assert_eq!(
            next_anniversary(anchor, at(2026, 9, 27, 10, 0)),
            Some(at(2026, 9, 30, 8, 30))
        );
        assert_eq!(
            next_anniversary(anchor, at(2026, 9, 30, 9, 0)),
            Some(at(2026, 10, 31, 8, 30))
        );
        assert_eq!(
            next_anniversary(anchor, at(2026, 2, 15, 0, 0)),
            Some(at(2026, 2, 28, 8, 30))
        );
        assert_eq!(
            next_anniversary(anchor, anchor),
            Some(at(2025, 2, 28, 8, 30))
        );
        assert_eq!(next_anniversary(at(2027, 1, 1, 0, 0), anchor), None);
        let paid = Account {
            plan: Some("Pro".into()),
            signed_up: Some(anchor),
        };
        assert_eq!(free_plan_reset(&paid, at(2026, 9, 27, 10, 0)), None);
    }

    #[test]
    fn descriptors_match_the_lines_and_export_the_windows() {
        let provider = Provider::new("ollama@abc", "Ollama");
        let descriptors = Ollama.descriptors(&provider);
        let ids: Vec<_> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "ollama@abc.session",
                "ollama@abc.weekly",
                "ollama@abc.monthly",
                "ollama@abc.last4Weeks"
            ]
        );
        let labels: Vec<_> = descriptors
            .iter()
            .map(|d| d.metric_label.as_str())
            .collect();
        assert_eq!(labels, ["Session", "Weekly", "Monthly", "Last 4 Weeks"]);
        let exports: Vec<_> = descriptors
            .iter()
            .flat_map(|d| d.limit_resources.iter())
            .map(|resource| (resource.key.as_str(), resource.unit.as_str()))
            .collect();
        assert_eq!(
            exports,
            [
                ("session", "percent"),
                ("weekly", "percent"),
                ("monthly", "percent")
            ]
        );
        let spend = &descriptors[3].template;
        assert_eq!(spend.selection_kind, Some(MetricKind::Dollars));
        assert_eq!(spend.unbounded_value_word.as_deref(), Some("spent"));
        assert!(!spend.is_usage_period);
    }

    #[test]
    fn connects_by_key_file_or_api_key_and_starts_hidden() {
        let connection = Ollama.connection();
        assert_eq!(connection.login_from, Some("Ollama"));
        let help = connection.api_key.unwrap();
        assert_eq!(help.env, ["OLLAMA_API_KEY"]);
        assert_eq!(help.url, "https://ollama.com/settings/keys");
        assert!(Ollama.starts_hidden());
    }
}
