//! OpenCode: the OpenCode Go subscription's usage windows (the rolling 5-hour Session, Weekly and
//! Monthly percents the OpenCode console shows), read with one of the account's OpenCode API keys.
//!
//! Login: OpenCode keeps its keys in `auth.json` in its data folder, which follows the XDG layout on
//! every OS: `$XDG_DATA_HOME/opencode`, else `~/.local/share/opencode` (on Windows that is
//! `%USERPROFILE%\.local\share\opencode`, on macOS and Linux `~/.local/share/opencode`). The
//! `$OPENCODE_DATA_DIR` override upstream OpenUsage honours and the platform data folder
//! (`%LOCALAPPDATA%\opencode`, `~/Library/Application Support/opencode`, `~/.local/share/opencode`)
//! are read too; a leading `~` in either variable is the home folder. Only the `opencode-go` (Go)
//! and `opencode` (Zen) entries of type `api` are taken; the file's other credentials never reach a
//! card. An API key comes from `OPENCODE_API_KEY` or from Accounts.
//!
//! Endpoint: `GET https://opencode.ai/zen/go/v1/usage` with `Authorization: Bearer <key>`, which
//! answers `{"usage": {"rolling" | "weekly" | "monthly": {"status", "percent", "resetsAt"}}}`. 401
//! means the key was rejected; 403 `EntitlementError` means the key's user has no Go subscription
//! in its workspace. A login tries its Go key, then its Zen key; the first key's answer explains a
//! failure. Zen pay-as-you-go keeps its balance in the web console only: no key-authenticated Zen
//! usage endpoint exists (`/zen/v1/usage` answers 404), so a Zen-only key reports that it has no
//! Go subscription.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{
    HttpRequest, HttpResponse, MetricLine, Provider, ProviderLink, SessionStartSignal,
    SimpleProviderError, WidgetDescriptor,
};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{http, lines, value};

pub(crate) struct OpenCode;

const NAME: &str = "OpenCode";
/// The app whose login is read, as the Accounts screen names it.
const APP: &str = "OpenCode";
const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";
/// Only a Go subscription has usage windows, so every answered request belongs to it.
const PLAN: &str = "Go";
/// The largest `auth.json` read; OpenCode keeps one small entry per provider in it.
const MAX_AUTH_BYTES: u64 = 1024 * 1024;
/// The longest `resetInSec` taken as a real countdown (a year).
const MAX_RESET_SECONDS: f64 = 366.0 * 86_400.0;
const NO_SUBSCRIPTION: &str =
    "No OpenCode Go subscription on this key. Zen balance is only shown on opencode.ai.";

/// One Go usage window: its descriptor suffix, the answer's names for it, its title and length.
struct Window {
    suffix: &'static str,
    /// The API says `rolling`; the `…Usage` names are the console's own field names, read in case a
    /// migrated workspace answers with them.
    keys: [&'static str; 2],
    title: &'static str,
    period_ms: i64,
}

const WINDOWS: [Window; 3] = [
    Window {
        suffix: "session",
        keys: ["rolling", "rollingUsage"],
        title: "Session",
        period_ms: 5 * lines::HOUR_MS,
    },
    Window {
        suffix: "weekly",
        keys: ["weekly", "weeklyUsage"],
        title: "Weekly",
        period_ms: lines::WEEK_MS,
    },
    Window {
        suffix: "monthly",
        keys: ["monthly", "monthlyUsage"],
        title: "Monthly",
        period_ms: lines::MONTH_MS,
    },
];

#[async_trait]
impl Service for OpenCode {
    fn id(&self) -> &'static str {
        "opencode"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![ProviderLink::new("Dashboard", "https://opencode.ai/auth")]
    }

    fn connection(&self) -> Connection {
        Connection::login(APP).or_api_key(ApiKeyHelp {
            env: &["OPENCODE_API_KEY"],
            url: "https://opencode.ai/auth",
            fields: &[],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        data_dirs(roots)
            .into_iter()
            .filter_map(|dir| login(&dir.join("auth.json")))
            .collect()
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        WINDOWS
            .iter()
            .map(|window| {
                let signal = (window.suffix == "session").then_some(SessionStartSignal::ZeroUsage);
                WidgetDescriptor::percent(
                    format!("{}.{}", provider.id, window.suffix),
                    provider,
                    window.title,
                    None,
                    signal,
                )
                .exporting_progress(window.suffix, "percent")
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let keys = keys(context.secret);
        if keys.is_empty() {
            return Err(http::invalid(
                "No OpenCode API key is saved for this account.",
            ));
        }
        // The first key's answer explains a failure: a stand-in key only helps when it succeeds, so
        // a rejected Go key is not reported as the Zen key's missing Go subscription.
        let mut first_failure = None;
        for key in keys {
            let response = http::send(
                context.http,
                HttpRequest::get(USAGE_URL)
                    .bearer(key)
                    .header("Accept", "application/json"),
                NAME,
            )
            .await?;
            if response.is_success() {
                let lines = meters(&http::parse(&response, NAME)?, context.now);
                if lines.is_empty() {
                    return Err(http::decoding(NAME));
                }
                return Ok(Reading::new(Some(PLAN.to_string()), lines));
            }
            let failure = match response.status {
                401 => rejected(context.secret),
                403 if error_type(&response).as_deref() == Some("EntitlementError") => {
                    http::not_available(NO_SUBSCRIPTION)
                }
                403 => http::status_error(&response, NAME),
                _ => return Err(http::status_error(&response, NAME)),
            };
            first_failure.get_or_insert(failure);
        }
        Err(first_failure.unwrap_or_else(|| rejected(context.secret)))
    }
}

/// Where OpenCode may keep its data, each folder once: the override upstream OpenUsage honours,
/// the folder OpenCode itself uses (`$XDG_DATA_HOME/opencode`, else `~/.local/share/opencode`, on
/// every OS), and the platform data folder.
fn data_dirs(roots: &Roots) -> Vec<PathBuf> {
    let candidates = [
        roots
            .var("OPENCODE_DATA_DIR")
            .map(|dir| expand_home(roots, &dir)),
        roots
            .var("XDG_DATA_HOME")
            .map(|dir| expand_home(roots, &dir).join("opencode")),
        Some(roots.home.join(".local").join("share").join("opencode")),
        Some(roots.local_data.join("opencode")),
    ];
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in candidates.into_iter().flatten() {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// A folder named in an environment variable, a leading `~` read as the home folder (as upstream
/// OpenUsage reads `OPENCODE_DATA_DIR` and `XDG_DATA_HOME`).
fn expand_home(roots: &Roots, raw: &str) -> PathBuf {
    if raw == "~" {
        return roots.home.clone();
    }
    let rest = raw
        .strip_prefix("~/")
        .or_else(|| cfg!(windows).then(|| raw.strip_prefix("~\\")).flatten());
    match rest {
        Some(rest) => roots.home.join(rest),
        None => PathBuf::from(raw),
    }
}

/// The login one `auth.json` holds: its Go and Zen API keys and nothing else from the file. It is
/// identified by its Go key, or its Zen key when there is no Go key.
fn login(path: &Path) -> Option<Login> {
    let document = value::read_json(path, MAX_AUTH_BYTES)?;
    let go = api_key(&document, "opencode-go");
    let zen = api_key(&document, "opencode");
    let identity = sha256_hex(go.or(zen)?);
    let secret = Secret::new(json!({ "goKey": go, "zenKey": zen }));
    Some(Login::new(identity, APP, path, secret))
}

/// The key of an entry OpenCode saved as `{"type": "api", "key": …}`.
fn api_key<'a>(document: &'a Value, entry: &str) -> Option<&'a str> {
    let entry = document.get(entry)?;
    if value::text(entry, "/type") != Some("api") {
        return None;
    }
    value::text(entry, "/key")
}

/// Lowercase hex SHA-256: a stable identity for a key that does not reveal it.
fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The keys a secret offers, in the order they are tried. A login offers its Go key, then its Zen
/// key: OpenCode keys Go usage by the key's user and workspace, not by the key, so the Zen key
/// stands in when the Go key was revoked. A saved or environment key is tried alone.
fn keys(secret: &Secret) -> Vec<&str> {
    let mut keys = Vec::new();
    for key in [secret.key(), secret.str("/goKey"), secret.str("/zenKey")]
        .into_iter()
        .flatten()
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

/// The error for a key OpenCode refused (HTTP 401).
fn rejected(secret: &Secret) -> SimpleProviderError {
    http::expired(if secret.key().is_some() {
        "OpenCode rejected this API key. Create a new one at opencode.ai/auth."
    } else {
        "OpenCode rejected its saved key. Run opencode auth login to sign in again."
    })
}

/// The error kind a JSON error answer names (`AuthError`, `EntitlementError`).
fn error_type(response: &HttpResponse) -> Option<String> {
    let body: Value = response.json().ok()?;
    value::text(&body, "/error/type").map(str::to_string)
}

/// One percent meter per Go window in a usage answer; a window without a percent is left out.
fn meters(body: &Value, now: DateTime<Utc>) -> Vec<MetricLine> {
    let usage = body
        .get("usage")
        .filter(|usage| usage.is_object())
        .unwrap_or(body);
    WINDOWS
        .iter()
        .filter_map(|window| {
            let data = window
                .keys
                .iter()
                .find_map(|key| usage.get(*key).filter(|data| data.is_object()))?;
            let used = value::number(data, "/percent")
                .or_else(|| value::number(data, "/usagePercent"))
                .or_else(|| {
                    (value::text(data, "/status") == Some("rate-limited")).then_some(100.0)
                })?;
            Some(lines::percent(
                window.title,
                used,
                reset_time(data, now),
                Some(window.period_ms),
            ))
        })
        .collect()
}

/// When a window resets: its `resetsAt` time, else `resetInSec` seconds after `now`.
fn reset_time(data: &Value, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    value::time(data, "/resetsAt").or_else(|| {
        let seconds = value::number(data, "/resetInSec")
            .filter(|seconds| *seconds > 0.0 && *seconds <= MAX_RESET_SECONDS)?;
        now.checked_add_signed(TimeDelta::try_seconds(seconds.round() as i64)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use uc_core::{
        ErrorCategory, LimitResourceKind, LimitResourceSource, MetricKind, ProgressFormat,
        ProgressLine,
    };

    const GO_KEY: &str = "sk-go-test";
    const ZEN_KEY: &str = "sk-zen-test";
    const GO_HASH: &str = "c51400c36697f537858736476497e71c4ea1df9e483f4296f3c35ad09dee08c3";
    const ZEN_HASH: &str = "8da3b5dba3ab016124729788ec8731d6d4a8434bfe6930c4288239f1f4ee8be9";

    /// Shaped like the answer of OpenCode's `zen/go/v1/usage` route (percent used, ISO reset).
    const USAGE: &str = r#"{"usage":{
        "rolling":{"status":"ok","percent":12,"resetsAt":"2026-09-27T13:24:10.000Z"},
        "weekly":{"status":"ok","percent":8,"resetsAt":"2026-09-28T00:00:00.000Z"},
        "monthly":{"status":"ok","percent":35,"resetsAt":"2026-10-04T11:18:32.000Z"}
    }}"#;
    const UNAUTHORIZED: &str =
        r#"{"type":"error","error":{"type":"AuthError","message":"Unauthorized"}}"#;
    const ENTITLEMENT: &str = r#"{"type":"error","error":{"type":"EntitlementError","message":"OpenCode Go subscription required."}}"#;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn login_secret() -> Value {
        json!({"goKey": GO_KEY, "zenKey": ZEN_KEY})
    }

    fn meter(label: &str, used: f64, resets_at: DateTime<Utc>, period_ms: i64) -> MetricLine {
        MetricLine::Progress(ProgressLine {
            label: label.into(),
            used,
            limit: 100.0,
            format: ProgressFormat::Percent,
            resets_at: Some(resets_at),
            period_duration_ms: Some(period_ms),
            color_hex: None,
        })
    }

    fn bearers(http: &Scripted) -> Vec<String> {
        http.requests()
            .iter()
            .map(|request| header(request, "Authorization").unwrap_or("").to_string())
            .collect()
    }

    #[tokio::test]
    async fn reads_the_go_windows_as_percent_meters() {
        let http = Scripted::new().on("GET", USAGE_URL, 200, USAGE);
        let scope = context_at(&http, login_secret(), now());
        let reading = OpenCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Go"));
        assert_eq!(
            reading.lines,
            vec![
                meter(
                    "Session",
                    12.0,
                    Utc.with_ymd_and_hms(2026, 9, 27, 13, 24, 10).unwrap(),
                    5 * 3_600_000
                ),
                meter(
                    "Weekly",
                    8.0,
                    Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap(),
                    7 * 86_400_000
                ),
                meter(
                    "Monthly",
                    35.0,
                    Utc.with_ymd_and_hms(2026, 10, 4, 11, 18, 32).unwrap(),
                    30 * 86_400_000
                ),
            ]
        );
        assert_eq!(reading.plan_term, None);
        assert_eq!(reading.warning, None);
    }

    #[tokio::test]
    async fn sends_the_go_key_as_a_bearer_token() {
        let http = Scripted::new().on("GET", USAGE_URL, 200, USAGE);
        let scope = context_at(&http, login_secret(), now());
        OpenCode.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].url, "https://opencode.ai/zen/go/v1/usage");
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some("Bearer sk-go-test")
        );
        assert_eq!(header(&requests[0], "Accept"), Some("application/json"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn a_zen_only_login_and_an_api_key_send_their_own_key() {
        let http = Scripted::new().on("GET", USAGE_URL, 200, USAGE);
        let scope = context_at(&http, json!({"goKey": null, "zenKey": ZEN_KEY}), now());
        OpenCode.fetch(&scope.context()).await.unwrap();
        let scope = context_at(&http, json!({"apiKey": "oc_sk_saved"}), now());
        OpenCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(bearers(&http), ["Bearer sk-zen-test", "Bearer oc_sk_saved"]);
    }

    #[tokio::test]
    async fn a_rejected_go_key_falls_back_to_the_zen_key() {
        let http = Scripted::new()
            .on("GET", USAGE_URL, 401, UNAUTHORIZED)
            .on("GET", USAGE_URL, 200, USAGE);
        let scope = context_at(&http, login_secret(), now());
        let reading = OpenCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Go"));
        assert_eq!(reading.lines.len(), 3);
        assert_eq!(bearers(&http), ["Bearer sk-go-test", "Bearer sk-zen-test"]);
    }

    #[tokio::test]
    async fn rejected_keys_are_an_expired_login_without_the_key_in_the_message() {
        let http = Scripted::new().on("GET", USAGE_URL, 401, UNAUTHORIZED);
        let scope = context_at(&http, login_secret(), now());
        let error = OpenCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "OpenCode rejected its saved key. Run opencode auth login to sign in again."
        );
        assert!(!error.message.contains(GO_KEY) && !error.message.contains(ZEN_KEY));
        assert_eq!(http.requests().len(), 2);

        let http = Scripted::new().on("GET", USAGE_URL, 401, UNAUTHORIZED);
        let scope = context_at(&http, json!({"apiKey": "oc_sk_revoked"}), now());
        let error = OpenCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "OpenCode rejected this API key. Create a new one at opencode.ai/auth."
        );
    }

    #[tokio::test]
    async fn a_key_without_go_reports_that_zen_has_no_usage_to_read() {
        let http = Scripted::new().on("GET", USAGE_URL, 403, ENTITLEMENT);
        let scope = context_at(&http, json!({"zenKey": ZEN_KEY}), now());
        let error = OpenCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(
            error.message,
            "No OpenCode Go subscription on this key. Zen balance is only shown on opencode.ai."
        );
        assert_eq!(http.requests().len(), 1);

        let http = Scripted::new().on("GET", USAGE_URL, 403, ENTITLEMENT).on(
            "GET",
            USAGE_URL,
            401,
            UNAUTHORIZED,
        );
        let scope = context_at(&http, login_secret(), now());
        let error = OpenCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(http.requests().len(), 2);
    }

    #[tokio::test]
    async fn a_rejected_go_key_wins_over_the_zen_keys_missing_subscription() {
        let http = Scripted::new().on("GET", USAGE_URL, 401, UNAUTHORIZED).on(
            "GET",
            USAGE_URL,
            403,
            ENTITLEMENT,
        );
        let scope = context_at(&http, login_secret(), now());
        let error = OpenCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "OpenCode rejected its saved key. Run opencode auth login to sign in again."
        );
        assert_eq!(bearers(&http), ["Bearer sk-go-test", "Bearer sk-zen-test"]);
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_stop_without_trying_another_key() {
        let http = Scripted::new().on("GET", USAGE_URL, 429, "{}");
        let scope = context_at(&http, login_secret(), now());
        let error = OpenCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(http.requests().len(), 1);

        let http = Scripted::new().on(
            "GET",
            USAGE_URL,
            503,
            r#"{"error":{"type":"api_error","message":"Inference routing is unavailable. Please retry later."}}"#,
        );
        let scope = context_at(&http, login_secret(), now());
        let error = OpenCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "OpenCode answered with HTTP 503.");
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn another_403_is_a_refused_key() {
        let http = Scripted::new().on("GET", USAGE_URL, 403, "<html>denied</html>");
        let scope = context_at(&http, json!({"apiKey": "oc_sk_saved"}), now());
        let error = OpenCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
    }

    #[tokio::test]
    async fn reads_console_field_names_and_skips_missing_windows() {
        let http = Scripted::new().on(
            "GET",
            USAGE_URL,
            200,
            r#"{"usage":{
                "rollingUsage":{"status":"rate-limited","resetInSec":3600},
                "weeklyUsage":{"status":"ok","usagePercent":"42.5","resetInSec":86400}
            }}"#,
        );
        let scope = context_at(&http, login_secret(), now());
        let reading = OpenCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                meter(
                    "Session",
                    100.0,
                    Utc.with_ymd_and_hms(2026, 9, 27, 11, 0, 0).unwrap(),
                    5 * 3_600_000
                ),
                meter(
                    "Weekly",
                    42.5,
                    Utc.with_ymd_and_hms(2026, 9, 28, 10, 0, 0).unwrap(),
                    7 * 86_400_000
                ),
            ]
        );
    }

    #[tokio::test]
    async fn an_answer_without_windows_cannot_be_read() {
        for body in [r#"{"usage":{}}"#, "<html>maintenance</html>"] {
            let http = Scripted::new().on("GET", USAGE_URL, 200, body);
            let scope = context_at(&http, login_secret(), now());
            let error = OpenCode.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
        }
    }

    #[tokio::test]
    async fn a_secret_without_a_key_is_invalid() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({"goKey": null, "zenKey": " "}), now());
        let error = OpenCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert!(http.requests().is_empty());
    }

    #[test]
    fn describes_one_percent_meter_per_go_window() {
        let provider = Provider::new(format!("opencode@{GO_HASH}"), "OpenCode");
        let descriptors = OpenCode.descriptors(&provider);
        let ids: Vec<_> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                format!("opencode@{GO_HASH}.session"),
                format!("opencode@{GO_HASH}.weekly"),
                format!("opencode@{GO_HASH}.monthly"),
            ]
        );
        let labels: Vec<_> = descriptors
            .iter()
            .map(|d| d.metric_label.as_str())
            .collect();
        assert_eq!(labels, ["Session", "Weekly", "Monthly"]);
        assert!(
            descriptors
                .iter()
                .all(|d| d.template.kind == MetricKind::Percent
                    && d.template.limit == Some(100.0)
                    && d.provider_id == provider.id)
        );
        assert_eq!(
            descriptors[0].template.session_start_signal,
            Some(SessionStartSignal::ZeroUsage)
        );
        assert_eq!(descriptors[1].template.session_start_signal, None);
        let exports: Vec<_> = descriptors
            .iter()
            .map(|d| {
                let export = &d.limit_resources[0];
                assert_eq!(export.kind, LimitResourceKind::Consumption);
                assert_eq!(export.source, LimitResourceSource::Progress);
                (export.key.as_str(), export.unit.as_str())
            })
            .collect();
        assert_eq!(
            exports,
            [
                ("session", "percent"),
                ("weekly", "percent"),
                ("monthly", "percent")
            ]
        );
    }

    #[test]
    fn connects_with_the_opencode_login_or_an_api_key() {
        let connection = OpenCode.connection();
        assert_eq!(connection.login_from, Some("OpenCode"));
        let help = connection.api_key.unwrap();
        assert_eq!(help.env, ["OPENCODE_API_KEY"]);
        assert_eq!(help.url, "https://opencode.ai/auth");
        assert!(help.fields.is_empty());
        assert_eq!(OpenCode.id(), "opencode");
        assert_eq!(OpenCode.name(), "OpenCode");
    }

    fn write_auth(dir: &Path, document: &Value) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("auth.json");
        std::fs::write(&path, document.to_string()).unwrap();
        path
    }

    #[test]
    fn discovers_the_go_and_zen_keys_in_opencodes_auth_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_auth(
            &dir.path().join(".local").join("share").join("opencode"),
            &json!({
                "anthropic": {"type": "oauth", "refresh": "r", "access": "a", "expires": 1},
                "opencode-go": {"type": "api", "key": GO_KEY},
                "opencode": {"type": "api", "key": ZEN_KEY}
            }),
        );
        let logins = OpenCode.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, GO_HASH);
        assert_eq!(logins[0].label, None);
        assert_eq!(logins[0].origin, "OpenCode");
        assert_eq!(logins[0].location, path);
        assert_eq!(
            logins[0].secret.value(),
            &json!({"goKey": GO_KEY, "zenKey": ZEN_KEY})
        );
    }

    #[test]
    fn a_zen_only_login_is_identified_by_its_zen_key() {
        let dir = tempfile::tempdir().unwrap();
        write_auth(
            &dir.path().join(".local").join("share").join("opencode"),
            &json!({"opencode": {"type": "api", "key": ZEN_KEY}}),
        );
        let logins = OpenCode.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, ZEN_HASH);
        assert_eq!(logins[0].secret.str("/goKey"), None);
        assert_eq!(logins[0].secret.str("/zenKey"), Some(ZEN_KEY));
    }

    #[test]
    fn finds_the_auth_file_under_xdg_data_home_and_the_platform_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let xdg = dir.path().join("xdg");
        write_auth(
            &xdg.join("opencode"),
            &json!({"opencode-go": {"type": "api", "key": GO_KEY}}),
        );
        let roots = Roots::under(dir.path()).with_var("XDG_DATA_HOME", xdg.to_str().unwrap());
        let logins = OpenCode.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].location, xdg.join("opencode").join("auth.json"));

        let custom = dir.path().join("custom");
        write_auth(
            &custom,
            &json!({"opencode": {"type": "api", "key": ZEN_KEY}}),
        );
        let roots =
            Roots::under(dir.path()).with_var("OPENCODE_DATA_DIR", custom.to_str().unwrap());
        let logins = OpenCode.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].location, custom.join("auth.json"));
        assert_eq!(logins[0].identity, ZEN_HASH);

        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        let path = write_auth(
            &roots.local_data.join("opencode"),
            &json!({"opencode-go": {"type": "api", "key": GO_KEY}}),
        );
        let logins = OpenCode.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].location, path);
        assert_eq!(logins[0].identity, GO_HASH);
    }

    #[test]
    fn a_leading_tilde_in_a_data_folder_variable_is_the_home_folder() {
        let dir = tempfile::tempdir().unwrap();
        let custom = dir.path().join("custom").join("oc");
        let path = write_auth(
            &custom,
            &json!({"opencode-go": {"type": "api", "key": GO_KEY}}),
        );
        let roots = Roots::under(dir.path()).with_var("OPENCODE_DATA_DIR", "~/custom/oc");
        let logins = OpenCode.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].location, path);

        let xdg = dir.path().join("xdg");
        let path = write_auth(
            &xdg.join("opencode"),
            &json!({"opencode": {"type": "api", "key": ZEN_KEY}}),
        );
        let roots = Roots::under(dir.path()).with_var("XDG_DATA_HOME", "~/xdg");
        let logins = OpenCode.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].location, path);
        assert_eq!(logins[0].identity, ZEN_HASH);

        let roots = Roots::under(dir.path());
        assert_eq!(expand_home(&roots, "~"), dir.path());
        assert_eq!(
            expand_home(&roots, "/srv/opencode"),
            PathBuf::from("/srv/opencode")
        );
        assert_eq!(expand_home(&roots, "~other"), PathBuf::from("~other"));
    }

    #[test]
    fn a_folder_reached_two_ways_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let share = dir.path().join(".local").join("share");
        write_auth(
            &share.join("opencode"),
            &json!({"opencode-go": {"type": "api", "key": GO_KEY}}),
        );
        let roots = Roots::under(dir.path()).with_var("XDG_DATA_HOME", share.to_str().unwrap());
        assert_eq!(OpenCode.discover(&roots).len(), 1);
    }

    #[test]
    fn ignores_folders_without_an_opencode_api_key() {
        let dir = tempfile::tempdir().unwrap();
        assert!(OpenCode.discover(&Roots::under(dir.path())).is_empty());

        let opencode = dir.path().join(".local").join("share").join("opencode");
        write_auth(
            &opencode,
            &json!({
                "opencode": {"type": "oauth", "refresh": "r", "access": "a", "expires": 1},
                "opencode-go": {"type": "api", "key": "  "},
                "openai": {"type": "api", "key": "sk-openai"}
            }),
        );
        assert!(OpenCode.discover(&Roots::under(dir.path())).is_empty());

        std::fs::write(opencode.join("auth.json"), "{not json").unwrap();
        assert!(OpenCode.discover(&Roots::under(dir.path())).is_empty());
    }
}
