//! Zed: the login the Zed editor saves after its GitHub sign-in, read against Zed's cloud account
//! endpoint, which reports the plan, this billing period's edit predictions and, on plans that
//! count them, hosted model requests.
//!
//! Zed keeps the login, its numeric user id and a long-lived access token it never refreshes, in
//! the operating system's credential store under its server URL `https://zed.dev`:
//! - Windows: the Credential Manager generic credential `zed:url=https://zed.dev` (user name = user
//!   id, secret = token).
//! - macOS: the login keychain internet password for server `https://zed.dev` (account = user id),
//!   read with `/usr/bin/security find-internet-password`. The item belongs to Zed, so macOS asks
//!   once whether `security` may read it; a refused or unanswered request is not repeated for
//!   30 minutes, and the card says why it has no data meanwhile.
//! - Linux: the Secret Service item `zed-github-account` with attributes `url` and `username`, read
//!   with `secret-tool` (a locked keyring may ask to be unlocked, with the same 30-minute pause).
//!
//! The store is asked only once Zed's data folder exists (`%LOCALAPPDATA%\Zed`,
//! `~/Library/Application Support/Zed`, `~/.local/share/zed`), so a computer without Zed never
//! starts a helper or meets a keychain prompt. A Zed build run from source keeps the same login as
//! JSON in `development_credentials` in Zed's config folder (`%APPDATA%\Zed`, `~/.config/zed`),
//! which is read too. A custom `server_url` in Zed's settings is not followed.
//!
//! Endpoint: `GET https://cloud.zed.dev/client/users/me` with `Authorization: <user id> <token>`.
//! A rejected token cannot be renewed here, since Zed has no refresh token: signing in to Zed again
//! replaces it.

use std::path::{Path, PathBuf};
#[cfg(any(unix, test))]
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uc_core::{
    ErrorCategory, HttpRequest, MetricLine, Provider, ProviderLink, SimpleProviderError,
    WidgetDescriptor,
};

use crate::service::{Connection, FetchContext, Login, Reading, Roots, Secret, Service};
use crate::support::{http, lines, value};

pub(crate) struct Zed;

const APP: &str = "Zed";
/// The server URL Zed files its login under (its default `server_url`).
const SERVER_URL: &str = "https://zed.dev";
const USER_URL: &str = "https://cloud.zed.dev/client/users/me";
/// Zed's Windows credential target for [`SERVER_URL`].
#[cfg(any(windows, test))]
const WINDOWS_TARGET: &str = "zed:url=https://zed.dev";

const EDIT_PREDICTIONS: &str = "Edit Predictions";
const REQUESTS: &str = "Requests";
/// The usage buckets shown, as (title, unit word, pointer under `plan`).
const BUCKETS: [(&str, &str, &str); 2] = [
    (EDIT_PREDICTIONS, "predictions", "/usage/edit_predictions"),
    (REQUESTS, "requests", "/usage/model_requests"),
];
const OVERDUE: &str =
    "This Zed account has overdue invoices. Zed may block usage until they are paid.";
const INCOMPLETE: &str = "The saved Zed login is incomplete. Sign in to Zed again.";
const UNREADABLE: &str =
    "Cannot read the Zed login from the system keychain. Allow access when the system asks again.";

#[async_trait]
impl Service for Zed {
    fn id(&self) -> &'static str {
        "zed"
    }

    fn name(&self) -> &'static str {
        APP
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Usage", "https://zed.dev/account"),
            ProviderLink::new("Plans", "https://zed.dev/pricing"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::login(APP)
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let mut logins = Vec::new();
        if data_dir(roots).is_dir() {
            logins.extend(store::logins().into_iter().filter_map(|stored| {
                login(&stored.user, stored.token.as_deref(), &stored.location)
            }));
        }
        logins.extend(development_login(
            &config_dir(roots).join("development_credentials"),
        ));
        logins
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::bounded_count(
                format!("{}.editPredictions", provider.id),
                provider,
                EDIT_PREDICTIONS,
                None,
                2000.0,
                "predictions",
                Some(lines::MONTH_MS),
            )
            .exporting_progress("editPredictions", "predictions"),
            WidgetDescriptor::bounded_count(
                format!("{}.requests", provider.id),
                provider,
                REQUESTS,
                None,
                500.0,
                "requests",
                Some(lines::MONTH_MS),
            )
            .exporting_progress("requests", "requests"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let secret = context.secret;
        let Some(user_id) = secret.str("/userId") else {
            return Err(http::invalid(INCOMPLETE));
        };
        let Some(token) = secret.str("/accessToken") else {
            return Err(
                if value::flag(secret.value(), "/unreadable") == Some(true) {
                    SimpleProviderError::new(ErrorCategory::CredentialAccess, UNREADABLE)
                } else {
                    http::invalid(INCOMPLETE)
                },
            );
        };
        let response = http::send(
            context.http,
            HttpRequest::get(USER_URL)
                .header("Authorization", format!("{user_id} {token}"))
                .header("Accept", "application/json"),
            APP,
        )
        .await?;
        match response.status {
            401 => {
                return Err(http::expired(
                    "The Zed login expired. Sign in to Zed again.",
                ));
            }
            403 => {
                return Err(http::expired(
                    "Zed refused the saved login. Sign in to Zed again.",
                ));
            }
            _ if !response.is_success() => return Err(http::status_error(&response, APP)),
            _ => {}
        }
        let body = http::parse(&response, APP)?;
        reading(&body).ok_or_else(|| http::decoding(APP))
    }
}

/// The card's reading from `/client/users/me`, or `None` when the answer carries no plan.
fn reading(body: &Value) -> Option<Reading> {
    let plan = body.get("plan").filter(|plan| plan.is_object())?;
    let resets_at = value::time(plan, "/subscription_period/ended_at");
    let period = resets_at.map(|ended| {
        value::time(plan, "/subscription_period/started_at")
            .filter(|started| *started < ended)
            .map_or(lines::MONTH_MS, |started| {
                (ended - started).num_milliseconds()
            })
    });
    let meters = BUCKETS
        .into_iter()
        .filter_map(|(title, unit, pointer)| {
            usage_line(title, unit, plan.pointer(pointer)?, resets_at, period)
        })
        .collect();
    let overdue = value::flag(plan, "/has_overdue_invoices") == Some(true);
    Some(Reading::new(plan_label(plan), meters).with_warning(overdue.then(|| OVERDUE.to_string())))
}

/// One usage bucket (`{used, limit}`): a count meter against a positive limit, an "Unlimited"
/// badge, or nothing for a zero limit (Zed then bills hosted models per token instead).
fn usage_line(
    title: &str,
    unit: &str,
    bucket: &Value,
    resets_at: Option<DateTime<Utc>>,
    period: Option<i64>,
) -> Option<MetricLine> {
    match usage_limit(bucket.get("limit")?)? {
        Limit::Unlimited => Some(lines::badge(title, "Unlimited")),
        Limit::Limited(limit) if limit > 0.0 => {
            let used = value::number(bucket, "/used")?;
            Some(lines::count(title, used, limit, unit, resets_at, period))
        }
        Limit::Limited(_) => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Limit {
    Unlimited,
    Limited(f64),
}

/// Zed's `UsageLimit`: `"unlimited"` or `{"limited": N}`; older answers send a bare number.
fn usage_limit(limit: &Value) -> Option<Limit> {
    let count = match limit {
        Value::String(text) if text.trim().eq_ignore_ascii_case("unlimited") => {
            return Some(Limit::Unlimited);
        }
        Value::Object(fields) => fields
            .get("limited")
            .or_else(|| fields.get("Limited"))
            .and_then(value::as_number),
        other => value::as_number(other),
    };
    count.filter(|count| *count >= 0.0).map(Limit::Limited)
}

/// The plan's name without Zed's `zed_` prefix, since the card already says Zed: `zed_pro_trial`
/// becomes `Pro Trial`.
fn plan_label(plan: &Value) -> Option<String> {
    let raw = ["/plan_v3", "/plan_v2", "/plan"]
        .into_iter()
        .find_map(|pointer| value::text(plan, pointer))?;
    let bare = match raw.get(..4) {
        Some(prefix) if prefix.eq_ignore_ascii_case("zed_") => &raw[4..],
        _ => raw,
    };
    lines::plan_name(bare).or_else(|| lines::plan_name(raw))
}

/// A login found in the credential store. `token` is `None` when the store would not hand the
/// secret over (access refused, a prompt left unanswered, or the pause after either).
struct Stored {
    user: String,
    token: Option<String>,
    location: PathBuf,
}

/// A login from a user id and, when it could be read, the access token. Zed itself only accepts a
/// numeric user id. A login whose token the store kept back still becomes a card, which explains
/// why it has no data.
fn login(user: &str, token: Option<&str>, location: &Path) -> Option<Login> {
    let user = user.trim();
    if user.parse::<u64>().is_err() {
        return None;
    }
    let secret = match token.map(str::trim) {
        Some("") => return None,
        Some(token) => json!({ "userId": user, "accessToken": token }),
        None => json!({ "userId": user, "unreadable": true }),
    };
    Some(Login::new(user, APP, location, Secret::new(secret)))
}

/// The login a Zed build run from source keeps in `development_credentials`: a JSON map from
/// server URL to `[user id, token bytes]`. Only the production server's entry counts.
fn development_login(path: &Path) -> Option<Login> {
    let document = value::read_json(path, 64 * 1024)?;
    let entry = document.get(SERVER_URL)?.as_array()?;
    let user = entry.first()?.as_str()?;
    let bytes = entry
        .get(1)?
        .as_array()?
        .iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect::<Option<Vec<u8>>>()?;
    login(user, Some(&String::from_utf8(bytes).ok()?), path)
}

/// Zed's data folder: its databases and extensions live there once it has run.
fn data_dir(roots: &Roots) -> PathBuf {
    if cfg!(any(windows, target_os = "macos")) {
        roots.local_data.join("Zed")
    } else {
        roots.local_data.join("zed")
    }
}

/// Zed's config folder, home of `settings.json`.
fn config_dir(roots: &Roots) -> PathBuf {
    if cfg!(windows) {
        roots.app_data.join("Zed")
    } else if cfg!(target_os = "macos") {
        roots.home.join(".config").join("zed")
    } else {
        roots.app_data.join("zed")
    }
}

/// Whether a Windows credential target is Zed's own for the production server.
#[cfg(any(windows, test))]
fn is_zed_target(target: &str) -> bool {
    target
        .trim_end_matches('/')
        .eq_ignore_ascii_case(WINDOWS_TARGET)
}

/// How long a listing may take: it reads no secret, so it never waits on the user.
#[cfg(unix)]
const LISTING_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a secret read may take: the system may be asking the user to allow access.
#[cfg(unix)]
const SECRET_TIMEOUT: Duration = Duration::from_secs(60);
/// After the store keeps a secret back, it is left alone this long, so a refusal costs one prompt
/// per window instead of one on every refresh.
#[cfg(any(unix, test))]
const REFUSAL_BACKOFF: Duration = Duration::from_secs(30 * 60);

#[cfg(unix)]
static REFUSED_AT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

#[cfg(any(unix, test))]
fn backing_off(refused_at: Option<Instant>, now: Instant) -> bool {
    refused_at.is_some_and(|at| now.saturating_duration_since(at) < REFUSAL_BACKOFF)
}

/// The secret `read` returns, unless the store kept one back within [`REFUSAL_BACKOFF`]; a secret
/// kept back starts the pause. Reads wait on each other, so two refreshes never prompt twice.
#[cfg(unix)]
fn unless_refused(read: impl FnOnce() -> Option<String>) -> Option<String> {
    let mut refused = REFUSED_AT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if backing_off(*refused, Instant::now()) {
        return None;
    }
    let secret = read()
        .map(|secret| secret.trim().to_string())
        .filter(|secret| !secret.is_empty());
    *refused = secret.is_none().then(Instant::now);
    secret
}

/// What a credential-store helper printed before it exited.
#[cfg(unix)]
struct Finished {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Runs a credential-store helper with no input; one still running after `timeout` is stopped
/// (which also dismisses a prompt it was waiting on) and gives `None`. Its output is never waited
/// on past that point, since a process it started may still hold the pipes open.
#[cfg(unix)]
fn run(program: &str, args: &[&str], timeout: Duration) -> Option<Finished> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let collect = |output: std::sync::mpsc::Receiver<String>| {
        output
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or_default()
    };
    Some(Finished {
        success: status.success(),
        stdout: collect(stdout),
        stderr: collect(stderr),
    })
}

/// Reads a helper's pipe to its end on its own thread, so a full pipe never stalls the helper, and
/// hands the text over once the pipe closes.
#[cfg(unix)]
fn drain(pipe: Option<impl std::io::Read + Send + 'static>) -> std::sync::mpsc::Receiver<String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(pipe) = pipe {
            let _ =
                std::io::Read::read_to_end(&mut std::io::Read::take(pipe, 1024 * 1024), &mut bytes);
        }
        let _ = sender.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    receiver
}

/// The value on the line of `security`'s output that starts with `key`: `"acct"<blob>=` in the
/// attributes, `password:` in the `-g` dump.
#[cfg(any(target_os = "macos", test))]
fn security_field(output: &str, key: &str) -> Option<String> {
    let raw = output
        .lines()
        .find_map(|line| line.trim_start().strip_prefix(key))?;
    let decoded = security_value(raw.trim())?;
    let decoded = decoded.trim();
    (!decoded.is_empty()).then(|| decoded.to_string())
}

/// A value as `security` prints it: `"text"` with `\\` and `\ooo` escapes, `0x<hex>` (then the
/// text again) when the bytes are not all printable, or `<NULL>`.
#[cfg(any(target_os = "macos", test))]
fn security_value(raw: &str) -> Option<String> {
    if let Some(hex) = raw.strip_prefix("0x") {
        let digits: String = hex.chars().take_while(char::is_ascii_hexdigit).collect();
        let bytes = (0..digits.len())
            .step_by(2)
            .map(|index| {
                digits
                    .get(index..index + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            })
            .collect::<Option<Vec<u8>>>()?;
        return String::from_utf8(bytes).ok();
    }
    let quoted = raw.strip_prefix('"')?.strip_suffix('"')?.as_bytes();
    let mut bytes = Vec::with_capacity(quoted.len());
    let mut index = 0;
    while let Some(&byte) = quoted.get(index) {
        let octal = if byte == b'\\' {
            quoted
                .get(index + 1..index + 4)
                .filter(|digits| digits.iter().all(|digit| (b'0'..=b'7').contains(digit)))
                .and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok())
        } else {
            None
        };
        if let Some(escaped) = octal {
            bytes.push(escaped);
            index += 4;
        } else if byte == b'\\' && quoted.get(index + 1) == Some(&b'\\') {
            bytes.push(b'\\');
            index += 2;
        } else {
            bytes.push(byte);
            index += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

/// The numeric `username` attributes in `secret-tool search` output, each once, in order.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn secret_tool_users(output: &str) -> Vec<String> {
    let mut users: Vec<String> = Vec::new();
    for line in output.lines() {
        let Some((key, user)) = line.split_once('=') else {
            continue;
        };
        let user = user.trim();
        if key.trim() == "attribute.username"
            && user.parse::<u64>().is_ok()
            && !users.iter().any(|known| known == user)
        {
            users.push(user.to_string());
        }
    }
    users
}

/// Zed's login in the Windows Credential Manager, which never prompts.
#[cfg(windows)]
mod store {
    use std::path::PathBuf;

    use super::Stored;
    use crate::support::keyring;

    pub(super) fn logins() -> Vec<Stored> {
        keyring::windows_prefixed(super::WINDOWS_TARGET)
            .into_iter()
            .filter(|(target, _, _)| super::is_zed_target(target))
            .map(|(target, user, token)| Stored {
                user,
                token: Some(token),
                location: PathBuf::from(target),
            })
            .collect()
    }
}

/// Zed's login in the login keychain. The attribute listing carries no secret, so it never
/// prompts; only a numeric account's password is then asked for.
#[cfg(target_os = "macos")]
mod store {
    use std::path::PathBuf;

    use super::{
        LISTING_TIMEOUT, SECRET_TIMEOUT, SERVER_URL, Stored, run, security_field, unless_refused,
    };

    const SECURITY: &str = "/usr/bin/security";

    pub(super) fn logins() -> Vec<Stored> {
        let Some(listing) = run(
            SECURITY,
            &["find-internet-password", "-s", SERVER_URL],
            LISTING_TIMEOUT,
        ) else {
            return Vec::new();
        };
        if !listing.success {
            return Vec::new();
        }
        let Some(user) = security_field(&listing.stdout, "\"acct\"<blob>=")
            .filter(|user| user.parse::<u64>().is_ok())
        else {
            return Vec::new();
        };
        let token = unless_refused(|| {
            let dump = run(
                SECURITY,
                &[
                    "find-internet-password",
                    "-s",
                    SERVER_URL,
                    "-a",
                    user.as_str(),
                    "-g",
                ],
                SECRET_TIMEOUT,
            )?;
            if !dump.success {
                return None;
            }
            security_field(&dump.stderr, "password:")
                .or_else(|| security_field(&dump.stdout, "password:"))
        });
        vec![Stored {
            user,
            token,
            location: PathBuf::from(SERVER_URL),
        }]
    }
}

/// Zed's logins with the Secret Service. `search` lists the attributes (on stderr) without
/// unlocking anything; each user id found is then looked up, which may ask to unlock the keyring.
#[cfg(all(unix, not(target_os = "macos")))]
mod store {
    use std::path::PathBuf;

    use super::{
        LISTING_TIMEOUT, SECRET_TIMEOUT, SERVER_URL, Stored, run, secret_tool_users, unless_refused,
    };

    pub(super) fn logins() -> Vec<Stored> {
        let Some(search) = run(
            "secret-tool",
            &["search", "--all", "url", SERVER_URL],
            LISTING_TIMEOUT,
        ) else {
            return Vec::new();
        };
        let listing = format!("{}\n{}", search.stdout, search.stderr);
        secret_tool_users(&listing)
            .into_iter()
            .map(|user| {
                let token = unless_refused(|| {
                    let lookup = run(
                        "secret-tool",
                        &["lookup", "url", SERVER_URL, "username", user.as_str()],
                        SECRET_TIMEOUT,
                    )?;
                    lookup.success.then_some(lookup.stdout)
                });
                Stored {
                    user,
                    token,
                    location: PathBuf::from(format!("url={SERVER_URL}")),
                }
            })
            .collect()
    }
}

#[cfg(not(any(windows, unix)))]
mod store {
    pub(super) fn logins() -> Vec<super::Stored> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use uc_core::{BadgeLine, ProgressFormat, ProgressLine};

    /// The shape of the token Zed saves: its access-token JSON.
    const TOKEN: &str = r#"{"version":2,"id":17,"token":"fixture-token"}"#;

    const FREE: &str = r#"{
        "user": {
            "id": 4242,
            "metrics_id": "0b6d0f0e-4b3c-4a57-9d3c-6f1f2d9e7a10",
            "avatar_url": "https://avatars.example.com/u/4242",
            "github_login": "octocat",
            "name": "The Octocat",
            "is_staff": false,
            "accepted_tos_at": "2026-01-04T09:00:00Z"
        },
        "feature_flags": [],
        "organizations": [],
        "plan": {
            "plan_v3": "zed_free",
            "subscription_period": {
                "started_at": "2026-09-13T00:00:00.000Z",
                "ended_at": "2026-10-13T00:00:00.000Z"
            },
            "usage": {
                "model_requests": {"used": 12, "limit": {"limited": 50}},
                "edit_predictions": {"used": 1234, "limit": {"limited": 2000}}
            },
            "trial_started_at": null,
            "is_usage_based_billing_enabled": false,
            "is_account_too_young": false,
            "has_overdue_invoices": false
        }
    }"#;

    const PRO: &str = r#"{
        "user": {"id": 4242, "github_login": "octocat", "name": null},
        "feature_flags": ["agent-sharing"],
        "plan": {
            "plan": "zed_pro",
            "plan_v2": "zed_pro",
            "plan_v3": "zed_pro",
            "subscription_period": {
                "started_at": "2026-09-13T08:30:00Z",
                "ended_at": "2026-10-13T08:30:00Z"
            },
            "usage": {
                "model_requests": {"used": 0, "limit": {"limited": 0}},
                "edit_predictions": {"used": 5321, "limit": "unlimited"}
            },
            "trial_started_at": "2026-05-01T00:00:00Z",
            "is_usage_based_billing_enabled": true,
            "is_account_too_young": false,
            "has_overdue_invoices": true
        }
    }"#;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn secret() -> Value {
        json!({"userId": "4242", "accessToken": TOKEN})
    }

    fn meter(
        title: &str,
        used: f64,
        limit: f64,
        unit: &str,
        resets_at: Option<DateTime<Utc>>,
        period: Option<i64>,
    ) -> MetricLine {
        MetricLine::Progress(ProgressLine {
            label: title.into(),
            used,
            limit,
            format: ProgressFormat::Count {
                suffix: unit.into(),
            },
            resets_at,
            period_duration_ms: period,
            color_hex: None,
        })
    }

    async fn fetch(status: u16, body: &str, secret: Value) -> Result<Reading, SimpleProviderError> {
        let http = Scripted::new().on("GET", USER_URL, status, body);
        let scope = context_at(&http, secret, now());
        Zed.fetch(&scope.context()).await
    }

    #[tokio::test]
    async fn reads_the_free_plan_quotas_until_the_billing_period_ends() {
        let reading = fetch(200, FREE, secret()).await.unwrap();
        let ends = Utc.with_ymd_and_hms(2026, 10, 13, 0, 0, 0).unwrap();
        let month = Some(30 * lines::DAY_MS);
        assert_eq!(reading.plan.as_deref(), Some("Free"));
        assert_eq!(
            reading.lines,
            vec![
                meter(
                    "Edit Predictions",
                    1234.0,
                    2000.0,
                    "predictions",
                    Some(ends),
                    month
                ),
                meter("Requests", 12.0, 50.0, "requests", Some(ends), month),
            ]
        );
        assert_eq!(reading.warning, None);
        assert_eq!(reading.plan_term, None);
    }

    #[tokio::test]
    async fn unlimited_predictions_read_unlimited_and_per_token_requests_are_left_out() {
        let reading = fetch(200, PRO, secret()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(
            reading.lines,
            vec![MetricLine::Badge(BadgeLine {
                label: "Edit Predictions".into(),
                text: "Unlimited".into(),
                color_hex: None,
                subtitle: None,
            })]
        );
        assert_eq!(
            reading.warning.as_deref(),
            Some("This Zed account has overdue invoices. Zed may block usage until they are paid.")
        );
    }

    #[tokio::test]
    async fn older_answers_without_a_billing_period_still_count() {
        let body = r#"{"user": {"id": 4242, "github_login": "octocat"}, "plan": {
            "plan_v2": "zed_pro_trial",
            "subscription_period": null,
            "usage": {
                "model_requests": {"used": 3, "limit": 150},
                "edit_predictions": {"used": 10, "limit": {"Limited": 20}}
            },
            "has_overdue_invoices": false
        }}"#;
        let reading = fetch(200, body, secret()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro Trial"));
        assert_eq!(
            reading.lines,
            vec![
                meter("Edit Predictions", 10.0, 20.0, "predictions", None, None),
                meter("Requests", 3.0, 150.0, "requests", None, None),
            ]
        );
    }

    #[tokio::test]
    async fn sends_the_user_id_and_token_the_way_zed_signs_its_requests() {
        let http = Scripted::new().on("GET", USER_URL, 200, FREE);
        let scope = context_at(&http, secret(), now());
        Zed.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].url, "https://cloud.zed.dev/client/users/me");
        let authorization = format!("4242 {TOKEN}");
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some(authorization.as_str())
        );
        assert_eq!(header(&requests[0], "Accept"), Some("application/json"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn a_rejected_login_asks_to_sign_in_to_zed_again() {
        for (status, message) in [
            (401, "The Zed login expired. Sign in to Zed again."),
            (403, "Zed refused the saved login. Sign in to Zed again."),
        ] {
            let error = fetch(status, "{}", secret()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthExpired, "{status}");
            assert_eq!(error.message, message);
        }
    }

    #[tokio::test]
    async fn rate_limits_outages_and_unreadable_answers_are_reported() {
        for (status, body, category) in [
            (429, "{}", ErrorCategory::RateLimited),
            (503, "", ErrorCategory::Http5xx),
            (200, "<html>sign in</html>", ErrorCategory::Decoding),
            (200, r#"{"user": {"id": 4242}}"#, ErrorCategory::Decoding),
        ] {
            let error = fetch(status, body, secret()).await.unwrap_err();
            assert_eq!(error.category, category, "{status} {body}");
        }
        let error = fetch(429, "{}", secret()).await.unwrap_err();
        assert_eq!(
            error.message,
            "Zed is rate limiting usage requests. Waiting before retrying."
        );
    }

    #[tokio::test]
    async fn a_login_without_its_token_is_never_sent() {
        for (secret, category, message) in [
            (
                json!({"userId": "4242"}),
                ErrorCategory::AuthInvalid,
                "The saved Zed login is incomplete. Sign in to Zed again.",
            ),
            (
                json!({"userId": "4242", "unreadable": true}),
                ErrorCategory::CredentialAccess,
                "Cannot read the Zed login from the system keychain. Allow access when the system asks again.",
            ),
        ] {
            let http = Scripted::new().on("GET", USER_URL, 200, FREE);
            let scope = context_at(&http, secret, now());
            let error = Zed.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert_eq!(error.message, message);
            assert!(http.requests().is_empty());
        }
    }

    fn development_file(dir: &Path, document: &Value) -> PathBuf {
        let config = config_dir(&Roots::under(dir));
        std::fs::create_dir_all(&config).unwrap();
        let path = config.join("development_credentials");
        std::fs::write(&path, document.to_string()).unwrap();
        path
    }

    #[test]
    fn discovers_the_login_a_zed_build_from_source_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = development_file(
            dir.path(),
            &json!({
                "https://zed.dev": ["4242", TOKEN.as_bytes()],
                "http://localhost:3000": ["1", b"dev-server-token"]
            }),
        );
        let logins = Zed.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, "4242");
        assert_eq!(logins[0].label, None);
        assert_eq!(logins[0].origin, "Zed");
        assert_eq!(logins[0].location, path);
        assert_eq!(logins[0].secret.value(), &secret());
        assert!(
            Zed.discover(&Roots::under(&dir.path().join("empty")))
                .is_empty()
        );
    }

    #[test]
    fn skips_development_logins_zed_itself_could_not_use() {
        for document in [
            json!({"https://zed.dev": ["octocat", TOKEN.as_bytes()]}),
            json!({"https://zed.dev": ["4242", []]}),
            json!({"https://zed.dev": ["4242", [300]]}),
            json!({"https://staging.zed.dev": ["4242", TOKEN.as_bytes()]}),
        ] {
            let dir = tempfile::tempdir().unwrap();
            development_file(dir.path(), &document);
            assert!(
                Zed.discover(&Roots::under(dir.path())).is_empty(),
                "{document}"
            );
        }
    }

    #[test]
    fn a_token_the_store_kept_back_still_makes_a_card() {
        let place = Path::new("zed:url=https://zed.dev");
        let kept = login(" 4242 ", None, place).unwrap();
        assert_eq!(kept.identity, "4242");
        assert_eq!(
            kept.secret.value(),
            &json!({"userId": "4242", "unreadable": true})
        );
        assert!(login("4242", Some("  "), place).is_none());
        assert!(login("octocat", Some(TOKEN), place).is_none());
    }

    #[test]
    fn a_refused_secret_is_left_alone_for_the_backoff_window() {
        let now = Instant::now();
        assert!(!backing_off(None, now));
        assert!(backing_off(Some(now), now + Duration::from_secs(60)));
        assert!(!backing_off(Some(now), now + REFUSAL_BACKOFF));
    }

    #[cfg(unix)]
    #[test]
    fn helpers_report_their_output_and_stop_at_the_timeout() {
        let finished = run(
            "sh",
            &["-c", "printf out; printf err >&2; exit 3"],
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(!finished.success);
        assert_eq!(finished.stdout, "out");
        assert_eq!(finished.stderr, "err");
        for script in ["exec sleep 30", "sleep 30; true"] {
            let started = Instant::now();
            assert!(
                run("sh", &["-c", script], Duration::from_millis(200)).is_none(),
                "{script}"
            );
            assert!(started.elapsed() < Duration::from_secs(10), "{script}");
        }
        assert!(run("/nonexistent/zed-helper", &[], Duration::from_secs(1)).is_none());
    }

    #[test]
    fn only_zeds_own_windows_credential_counts() {
        assert!(is_zed_target("zed:url=https://zed.dev"));
        assert!(is_zed_target("ZED:url=https://zed.dev/"));
        assert!(!is_zed_target("zed:url=https://zed.dev.example.com"));
        assert!(!is_zed_target("zed:url=https://staging.zed.dev"));
    }

    #[test]
    fn keychain_listings_yield_the_account_and_the_token() {
        let attributes = "keychain: \"/Users/me/Library/Keychains/login.keychain-db\"\n\
            version: 512\nclass: \"inet\"\nattributes:\n    \
            0x00000007 <blob>=\"https://zed.dev\"\n    \"acct\"<blob>=\"4242\"\n    \
            \"desc\"<blob>=<NULL>\n    \"srvr\"<blob>=\"https://zed.dev\"\n";
        assert_eq!(
            security_field(attributes, "\"acct\"<blob>=").as_deref(),
            Some("4242")
        );
        assert_eq!(security_field(attributes, "\"desc\"<blob>="), None);
        assert_eq!(security_field(attributes, "password:"), None);
        let dump = format!("password: \"{TOKEN}\"\n");
        assert_eq!(security_field(&dump, "password:").as_deref(), Some(TOKEN));
        assert_eq!(
            security_field("password: 0x34323432  \"4242\"\n", "password:").as_deref(),
            Some("4242")
        );
        assert_eq!(
            security_field("password: \"x\\\\y\\101z\"", "password:").as_deref(),
            Some("x\\yAz")
        );
        assert_eq!(security_field("password: \n", "password:"), None);
        assert_eq!(security_field("password: 0x343", "password:"), None);
    }

    #[test]
    fn secret_tool_listings_yield_each_numeric_user_once() {
        let listing = "[/12]\nlabel = zed-github-account\nsecret = {\"version\":2}\n\
            created = 2026-09-01 10:00:00\nmodified = 2026-09-01 10:00:00\n\
            attribute.url = https://zed.dev\nattribute.username = 4242\n\
            attribute.username = 4242\nattribute.username = octocat\n";
        assert_eq!(secret_tool_users(listing), vec!["4242".to_string()]);
        assert!(secret_tool_users("").is_empty());
    }

    #[test]
    fn usage_limits_accept_each_shape_zed_sends() {
        assert_eq!(usage_limit(&json!("unlimited")), Some(Limit::Unlimited));
        assert_eq!(
            usage_limit(&json!({"limited": 2000})),
            Some(Limit::Limited(2000.0))
        );
        assert_eq!(
            usage_limit(&json!({"Limited": 20})),
            Some(Limit::Limited(20.0))
        );
        assert_eq!(usage_limit(&json!(150)), Some(Limit::Limited(150.0)));
        assert_eq!(usage_limit(&json!({"limited": -1})), None);
        assert_eq!(usage_limit(&json!(null)), None);
        assert_eq!(usage_limit(&json!("soon")), None);
    }

    #[test]
    fn plan_names_drop_the_zed_prefix() {
        let label = |plan: Value| plan_label(&plan);
        assert_eq!(
            label(json!({"plan_v3": "zed_pro_trial", "plan": "zed_free"})).as_deref(),
            Some("Pro Trial")
        );
        assert_eq!(
            label(json!({"plan_v2": "zed_student"})).as_deref(),
            Some("Student")
        );
        assert_eq!(
            label(json!({"plan": "ZED_BUSINESS"})).as_deref(),
            Some("Business")
        );
        assert_eq!(label(json!({"plan_v3": "zed"})).as_deref(), Some("Zed"));
        assert_eq!(label(json!({})), None);
    }
}
