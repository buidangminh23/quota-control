//! Devin: the login the Devin CLI saves with `devin auth login`, else the Devin app's sign-in.
//!
//! The CLI keeps `windsurf_api_key` (and optionally an `https://` `api_server_url`) in
//! `devin/credentials.toml`, which Devin documents under `%APPDATA%` on Windows and under
//! `$XDG_DATA_HOME` (else `~/.local/share`) on macOS and Linux. `~/.local/share`,
//! `%LOCALAPPDATA%`, `~/Library/Application Support` and `~/.config` are tried after that, for
//! installs that keep the file elsewhere. The Devin app keeps `apiKey` in the `windsurfAuthStatus`
//! item of its `User/globalStorage/state.vscdb`: under `%APPDATA%\Devin` on Windows,
//! `~/Library/Application Support/Devin` on macOS and `~/.config/Devin` (or
//! `$XDG_CONFIG_HOME/Devin`) on Linux. When both exist with a different key or server, the app's
//! key stays behind the CLI's as a fallback, as OpenUsage does.
//!
//! Endpoint: the Connect RPC `exa.seat_management_pb.SeatManagementService/GetUserStatus` on the
//! login's server (`https://server.codeium.com` unless the CLI names another), with the key in the
//! JSON body. It answers the daily and weekly quota left and the extra usage balance, with the
//! account's `email` and the plan period's end (`planStatus.planEnd`). Keys are
//! never renewed here: a refused key means signing in to Devin again.
//!
//! A key pasted in Quota Control (or found in `DEVIN_API_KEY`) is read the same way when it is the
//! CLI's `windsurf_api_key`. An Enterprise admin's personal key (`apk_user_…`) instead reads the
//! organization's ACUs this month from `GET https://api.devin.ai/v2/enterprise/consumption/daily`
//! (Bearer), whose days start at 08:00 UTC; Devin allows ten such calls an hour per team, so the
//! total is kept for an hour.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, SecondsFormat, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{
    ErrorCategory, HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, MetricLine,
    PlanTerm, Provider, ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{apps, http, lines, value};

pub(crate) struct Devin;

const NAME: &str = "Devin";
/// The desktop app, and the folder its state lives in.
const APP: &str = "Devin";
const CLI: &str = "Devin CLI";
const DEFAULT_SERVER: &str = "https://server.codeium.com";
const USER_STATUS: &str = "exa.seat_management_pb.SeatManagementService/GetUserStatus";
/// The IDE and extension version the request metadata reports, as OpenUsage sends it.
const CLIENT_VERSION: &str = "1.108.2";
/// The largest credentials file read.
const MAX_CREDENTIALS_BYTES: u64 = 64 * 1024;

const DAILY: &str = "Daily quota";
const WEEKLY: &str = "Weekly quota";
const EXTRA: &str = "Extra usage balance";
const ENTERPRISE_ACUS: &str = "Enterprise ACUs";

const EXPIRED: &str = "The Devin login expired. Run devin auth login or sign in to Devin again.";
const UNAVAILABLE: &str = "Devin quota data unavailable. Try again later.";
const NO_KEY: &str = "The Devin login has no API key. Run devin auth login or sign in to Devin.";
const ENTERPRISE_REFUSED: &str = "Devin refused the key. Enterprise usage needs an enterprise admin's personal key (apk_user_…).";

/// How a `GetUserStatus` answer is named on Devin's card; Windsurf reads the same answer with its own.
const DEVIN_CARD: StatusCard = StatusCard {
    name: NAME,
    ide: "devin",
    expired: EXPIRED,
    unavailable: UNAVAILABLE,
    daily: DAILY,
    weekly: WEEKLY,
    extra: EXTRA,
};

/// The Enterprise daily consumption endpoint an admin's personal key reads.
const CONSUMPTION: &str = "https://api.devin.ai/v2/enterprise/consumption/daily";
/// Memo key: the last ACU total and the month it covers, kept for an hour.
const CONSUMPTION_MEMO: &str = "devin.consumption";
/// Devin's billing day starts at 08:00 UTC (midnight Pacific standard time).
const DAY_START_HOURS: i64 = 8;

/// Memo key: the fingerprint of the key and server that answered last.
const WORKING: &str = "devin.working";

#[async_trait]
impl Service for Devin {
    fn id(&self) -> &'static str {
        "devin"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![ProviderLink::new(
            "Plans",
            "https://app.devin.ai/settings/plans",
        )]
    }

    fn connection(&self) -> Connection {
        Connection::login(APP).or_api_key(ApiKeyHelp {
            env: &["DEVIN_API_KEY"],
            url: "https://app.devin.ai/settings/api-keys",
            fields: &[],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let (primary, fallback) = match (cli_login(roots), app_login(roots)) {
            (Some(cli), Some(app)) => {
                let differs = cli.key != app.key || cli.server() != app.server();
                (cli, differs.then_some(app))
            }
            (Some(found), None) | (None, Some(found)) => (found, None),
            (None, None) => return Vec::new(),
        };
        let mut secret = primary.secret();
        if let Some(fallback) = &fallback {
            secret["fallback"] = fallback.secret();
        }
        vec![Login::new(
            digest(&primary.key),
            primary.origin,
            &primary.location,
            Secret::new(secret),
        )]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::percent(
                format!("{}.daily", provider.id),
                provider,
                "Daily",
                Some(DAILY),
                None,
            )
            .exporting_progress("daily", "percent"),
            WidgetDescriptor::percent(
                format!("{}.weekly", provider.id),
                provider,
                "Weekly",
                Some(WEEKLY),
                None,
            )
            .exporting_progress("weekly", "percent"),
            WidgetDescriptor::dollar_balance(
                format!("{}.extra", provider.id),
                provider,
                "Extra Usage",
                Some(EXTRA),
                "left",
            )
            .exporting_limit(
                "extraUsageBalance",
                LimitResourceKind::Balance,
                "usd",
                LimitResourceSource::Value {
                    kind: MetricKind::Dollars,
                    label: None,
                },
                false,
            ),
            WidgetDescriptor::values(
                format!("{}.enterprise", provider.id),
                provider,
                ENTERPRISE_ACUS,
                None,
                Some(MetricKind::Count),
                None,
                true,
                None,
                false,
            )
            .exporting_limit(
                "enterpriseAcus",
                LimitResourceKind::Consumption,
                "count",
                LimitResourceSource::Value {
                    kind: MetricKind::Count,
                    label: None,
                },
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        if let Some(key) = context.secret.key().filter(|key| is_enterprise_key(key)) {
            return enterprise_usage(context, key).await;
        }
        let mut attempts = attempts(context.secret);
        if attempts.is_empty() {
            return Err(http::invalid(NO_KEY));
        }
        if let Some(working) = context.memo.get(WORKING, context.now).await
            && let Some(index) = attempts
                .iter()
                .position(|attempt| working.as_str() == Some(attempt.fingerprint().as_str()))
        {
            let remembered = attempts.remove(index);
            attempts.insert(0, remembered);
        }
        let mut first_failure = None;
        let mut refused = false;
        for attempt in &attempts {
            match user_status(context, attempt).await {
                Ok(reading) => {
                    context
                        .memo
                        .put(
                            WORKING,
                            Value::String(attempt.fingerprint()),
                            Some(context.now + Duration::hours(12)),
                        )
                        .await;
                    return Ok(reading);
                }
                Err(error) if error.category == ErrorCategory::RateLimited => return Err(error),
                Err(error) => {
                    refused |= error.category == ErrorCategory::AuthExpired;
                    first_failure.get_or_insert(error);
                }
            }
        }
        if refused {
            return Err(http::expired(EXPIRED));
        }
        Err(first_failure.unwrap_or_else(|| http::invalid(NO_KEY)))
    }
}

/// A key found on this computer and where it came from.
struct Found {
    key: String,
    server: Option<String>,
    origin: &'static str,
    location: PathBuf,
}

impl Found {
    fn server(&self) -> &str {
        self.server.as_deref().unwrap_or(DEFAULT_SERVER)
    }

    fn secret(&self) -> Value {
        let mut secret = json!({ "apiKey": self.key });
        if let Some(server) = &self.server {
            secret["apiServerUrl"] = Value::String(server.clone());
        }
        secret
    }
}

/// The first CLI credentials file that holds a key.
fn cli_login(roots: &Roots) -> Option<Found> {
    credential_files(roots).into_iter().find_map(|path| {
        let text = read_text(&path, MAX_CREDENTIALS_BYTES)?;
        let (key, server) = parse_credentials(&text)?;
        Some(Found {
            key,
            server,
            origin: CLI,
            location: path,
        })
    })
}

/// Where the CLI may keep `credentials.toml`, without repeats: the folder Devin documents first
/// (`%APPDATA%` on Windows, `$XDG_DATA_HOME` else `~/.local/share` elsewhere), so a stale copy in
/// another folder never wins over it.
fn credential_files(roots: &Roots) -> Vec<PathBuf> {
    let share = roots.home.join(".local").join("share");
    let xdg = roots.dir_from("XDG_DATA_HOME", share.clone());
    let folders = if cfg!(windows) {
        [roots.app_data.clone(), xdg, share, roots.local_data.clone()]
    } else {
        [xdg, share, roots.local_data.clone(), roots.app_data.clone()]
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for folder in folders {
        let file = folder.join("devin").join("credentials.toml");
        if !files.contains(&file) {
            files.push(file);
        }
    }
    files
}

fn read_text(path: &Path, max_bytes: u64) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The key and the `https://` server of a CLI credentials file. The key may sit at the top level or
/// in a table; the server is read beside it.
fn parse_credentials(text: &str) -> Option<(String, Option<String>)> {
    let document: toml::Table = toml::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let holder = holder(&document, "windsurf_api_key", 2)?;
    let key = table_text(holder, "windsurf_api_key")?.to_string();
    let server = table_text(holder, "api_server_url")
        .or_else(|| table_text(&document, "api_server_url"))
        .and_then(clean_server)
        .map(str::to_string);
    Some((key, server))
}

fn table_text<'a>(table: &'a toml::Table, key: &str) -> Option<&'a str> {
    table
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// The table holding a non-empty `key`: the document itself, else one nested `depth` levels deep.
fn holder<'a>(table: &'a toml::Table, key: &str, depth: usize) -> Option<&'a toml::Table> {
    if table_text(table, key).is_some() {
        return Some(table);
    }
    if depth == 0 {
        return None;
    }
    table
        .values()
        .filter_map(toml::Value::as_table)
        .find_map(|nested| holder(nested, key, depth - 1))
}

/// A server URL when it is `https://` with a host and nothing after its path, without trailing
/// slashes. A plain `http://` server is dropped, so the key goes to the default server instead.
fn clean_server(raw: &str) -> Option<&str> {
    let trimmed = raw.trim().trim_end_matches('/');
    let url = url::Url::parse(trimmed).ok()?;
    let clean = url.scheme() == "https"
        && url.host_str().is_some_and(|host| !host.is_empty())
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    clean.then_some(trimmed)
}

/// The key the Devin app saved in its state database.
fn app_login(roots: &Roots) -> Option<Found> {
    let stored = apps::item(roots, APP, "windsurfAuthStatus")?;
    let status: Value = serde_json::from_str(stored.trim()).ok()?;
    let key = value::text(&status, "/apiKey")?.to_string();
    Some(Found {
        key,
        server: None,
        origin: APP,
        location: apps::state_db(roots, APP),
    })
}

/// A lowercase hex SHA-256, so a key can name an account without being kept.
fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A key to try and the server it belongs to.
struct Attempt<'a> {
    key: &'a str,
    server: &'a str,
}

impl Attempt<'_> {
    fn fingerprint(&self) -> String {
        digest(&format!("{}\n{}", self.server, self.key))
    }
}

/// The saved key, then the fallback key when it differs in key or server.
fn attempts(secret: &Secret) -> Vec<Attempt<'_>> {
    let mut attempts: Vec<Attempt<'_>> = Vec::new();
    for prefix in ["", "/fallback"] {
        let Some(key) = secret.str(&format!("{prefix}/apiKey")) else {
            continue;
        };
        let server = secret
            .str(&format!("{prefix}/apiServerUrl"))
            .and_then(clean_server)
            .unwrap_or(DEFAULT_SERVER);
        if !attempts
            .iter()
            .any(|attempt| attempt.key == key && attempt.server == server)
        {
            attempts.push(Attempt { key, server });
        }
    }
    attempts
}

async fn user_status(
    context: &FetchContext<'_>,
    attempt: &Attempt<'_>,
) -> Result<Reading, SimpleProviderError> {
    read_user_status(context, attempt.key, attempt.server, &DEVIN_CARD).await
}

/// How a `GetUserStatus` answer is named on a card of the Codeium-backed services.
pub(crate) struct StatusCard {
    pub name: &'static str,
    /// The IDE the request metadata names.
    pub ide: &'static str,
    pub expired: &'static str,
    pub unavailable: &'static str,
    pub daily: &'static str,
    pub weekly: &'static str,
    pub extra: &'static str,
}

/// The Codeium server a key without a named server belongs to.
pub(crate) const USER_STATUS_SERVER: &str = DEFAULT_SERVER;

/// The daily and weekly quota and the extra usage balance of the account behind `key`.
pub(crate) async fn read_user_status(
    context: &FetchContext<'_>,
    key: &str,
    server: &str,
    card: &StatusCard,
) -> Result<Reading, SimpleProviderError> {
    let request = HttpRequest::post(format!("{server}/{USER_STATUS}"))
        .header("Connect-Protocol-Version", "1")
        .json_body(&json!({
            "metadata": {
                "apiKey": key,
                "ideName": card.ide,
                "ideVersion": CLIENT_VERSION,
                "extensionName": card.ide,
                "extensionVersion": CLIENT_VERSION,
                "locale": "en"
            }
        }));
    let response = http::send(context.http, request, card.name).await?;
    match response.status {
        401 | 403 => Err(http::expired(card.expired)),
        _ if !response.is_success() => Err(http::status_error(&response, card.name)),
        _ => usage(&http::parse(&response, card.name)?, card),
    }
}

/// The plan and meters of a `GetUserStatus` answer.
fn usage(body: &Value, card: &StatusCard) -> Result<Reading, SimpleProviderError> {
    let status = body
        .get("userStatus")
        .filter(|status| status.is_object())
        .ok_or_else(|| http::decoding(card.name))?;
    let field = |name: &str| status.pointer(&format!("/planStatus/{name}"));
    let plan = value::text(status, "/planStatus/planInfo/planName").map(str::to_string);
    let hide_daily = truthy(field("planInfo/hideDailyQuota"));
    let daily = field("dailyQuotaRemainingPercent").and_then(value::as_number);
    let weekly_field = field("weeklyQuotaRemainingPercent");
    let weekly = weekly_field.and_then(value::as_number);
    if weekly_field.is_some() && weekly.is_none() {
        return Err(http::decoding(card.name));
    }
    let daily_reset = field("dailyQuotaResetAtUnix").and_then(value::as_time);
    let weekly_reset = field("weeklyQuotaResetAtUnix").and_then(value::as_time);
    let mut meters = Vec::new();
    if let Some(remaining) = daily.filter(|_| !hide_daily) {
        meters.push(quota(card.daily, remaining, daily_reset, lines::DAY_MS));
    }
    // Proto3 JSON leaves zeros out, so a weekly reset without a percentage is a week used up. With
    // neither and the daily quota hidden, the daily figure stands in for the week.
    let weekly = weekly
        .or(weekly_reset.map(|_| 0.0))
        .or(daily.filter(|_| hide_daily));
    if let Some(remaining) = weekly {
        meters.push(quota(card.weekly, remaining, weekly_reset, lines::WEEK_MS));
    }
    if let Some(micros) = field("overageBalanceMicros").and_then(value::as_number) {
        meters.push(lines::dollar_value(
            card.extra,
            micros.max(0.0) / 1_000_000.0,
        ));
    }
    if meters.is_empty() {
        return Err(http::not_available(card.unavailable));
    }
    let ends_at = field("planEnd").and_then(value::as_time);
    Ok(Reading::new(plan, meters)
        .with_account(value::text(status, "/email"))
        .with_plan_term(ends_at.map(|ends_at| PlanTerm::Stated {
            ends_at,
            checked_at: None,
        })))
}

/// An Enterprise admin's personal key, which reads the organization's consumption instead of an
/// account's quota.
fn is_enterprise_key(key: &str) -> bool {
    key.starts_with("apk_")
}

/// When Devin's current billing month began: the first of the month at 08:00 UTC.
fn month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let shifted = (now - Duration::hours(DAY_START_HOURS)).date_naive();
    let first = shifted.with_day(1).unwrap_or(shifted);
    first.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc() + Duration::hours(DAY_START_HOURS)
}

/// The organization's ACUs since the month began, asked at most once an hour.
async fn enterprise_usage(
    context: &FetchContext<'_>,
    key: &str,
) -> Result<Reading, SimpleProviderError> {
    let stamp = month_start(context.now).to_rfc3339_opts(SecondsFormat::Secs, true);
    if let Some(saved) = context.memo.get(CONSUMPTION_MEMO, context.now).await
        && value::text(&saved, "/since") == Some(stamp.as_str())
        && let Some(total) = value::number(&saved, "/acus")
    {
        return Ok(enterprise_reading(total));
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("start_date", &stamp)
        .append_pair(
            "end_date",
            &context.now.to_rfc3339_opts(SecondsFormat::Secs, true),
        )
        .finish();
    let request = HttpRequest::get(format!("{CONSUMPTION}?{query}"))
        .bearer(key)
        .header("Accept", "application/json");
    let response = http::send(context.http, request, NAME).await?;
    match response.status {
        401 | 403 => return Err(http::invalid(ENTERPRISE_REFUSED)),
        _ if !response.is_success() => return Err(http::status_error(&response, NAME)),
        _ => {}
    }
    let body = http::parse(&response, NAME)?;
    let total = value::number(&body, "/total_acus").ok_or_else(|| http::decoding(NAME))?;
    context
        .memo
        .put(
            CONSUMPTION_MEMO,
            json!({ "since": stamp, "acus": total }),
            Some(context.now + Duration::hours(1)),
        )
        .await;
    Ok(enterprise_reading(total))
}

fn enterprise_reading(total: f64) -> Reading {
    Reading::new(
        Some("Enterprise".to_string()),
        vec![lines::count_value(ENTERPRISE_ACUS, total.max(0.0), "ACUs")],
    )
}

/// Devin reports the share left; the meter shows the share used.
fn quota(
    label: &str,
    remaining: f64,
    resets_at: Option<DateTime<Utc>>,
    period_ms: i64,
) -> MetricLine {
    lines::percent(label, 100.0 - remaining, resets_at, Some(period_ms))
}

/// A flag given as `true`, a non-zero number, or `"true"`/`"1"`.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number.abs() > 0.0),
        Some(Value::String(text)) => {
            matches!(text.trim().to_ascii_lowercase().as_str(), "true" | "1")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;

    const CUSTOM: &str = "https://server.codeium.test";
    const CLI_KEY: &str = "devin-session-token$cli";
    const APP_KEY: &str = "devin-session-token$app";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn at(month: u32, day: u32) -> Option<DateTime<Utc>> {
        Some(Utc.with_ymd_and_hms(2026, month, day, 0, 0, 0).unwrap())
    }

    fn url(server: &str) -> String {
        format!("{server}/{USER_STATUS}")
    }

    fn status(plan_status: Value) -> Value {
        json!({ "userStatus": { "planStatus": plan_status } })
    }

    /// A `GetUserStatus` answer as the server sends it: proto3 JSON, 64-bit numbers as strings.
    fn answer(plan: &str) -> String {
        status(json!({
            "planInfo": {"planName": plan, "billingStrategy": "BILLING_STRATEGY_QUOTA"},
            "dailyQuotaRemainingPercent": 100,
            "weeklyQuotaRemainingPercent": 40,
            "overageBalanceMicros": "964220000",
            "dailyQuotaResetAtUnix": "1790553600",
            "weeklyQuotaResetAtUnix": "1790812800"
        }))
        .to_string()
    }

    fn cli_secret() -> Value {
        json!({ "apiKey": CLI_KEY, "apiServerUrl": CUSTOM })
    }

    fn both_secret() -> Value {
        let mut secret = cli_secret();
        secret["fallback"] = json!({ "apiKey": APP_KEY });
        secret
    }

    type Meter = (String, f64, Option<DateTime<Utc>>, Option<i64>);

    fn meters(reading: &Reading) -> Vec<Meter> {
        reading
            .lines
            .iter()
            .filter_map(|line| match line {
                MetricLine::Progress(line) => Some((
                    line.label.clone(),
                    line.used,
                    line.resets_at,
                    line.period_duration_ms,
                )),
                _ => None,
            })
            .collect()
    }

    fn balance(reading: &Reading) -> Option<f64> {
        reading.lines.iter().find_map(|line| match line {
            MetricLine::Values(line) if line.label == EXTRA => {
                line.values.first().map(|value| value.number)
            }
            _ => None,
        })
    }

    fn body(request: &uc_core::HttpRequest) -> Value {
        serde_json::from_slice(request.body.as_ref().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn names_the_account_and_the_end_of_the_plan_period() {
        let body = json!({ "userStatus": {
            "name": "Minh",
            "email": "me@example.com",
            "planStatus": {
                "planInfo": {"planName": "Pro"},
                "planStart": "2026-09-01T00:00:00Z",
                "planEnd": "2026-10-01T00:00:00Z",
                "weeklyQuotaRemainingPercent": 40
            }
        }})
        .to_string();
        let http = Scripted::new().on("POST", &url(CUSTOM), 200, &body);
        let scope = context_at(&http, cli_secret(), now());
        let reading = Devin.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.account.as_deref(), Some("me@example.com"));
        assert_eq!(
            reading.plan_term,
            at(10, 1).map(|ends_at| PlanTerm::Stated {
                ends_at,
                checked_at: None,
            })
        );
    }

    #[tokio::test]
    async fn reads_the_daily_and_weekly_quota_and_the_extra_usage_balance() {
        let http = Scripted::new().on("POST", &url(CUSTOM), 200, &answer("Max"));
        let scope = context_at(&http, cli_secret(), now());
        let reading = Devin.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Max".into()),
                vec![
                    lines::percent(DAILY, 0.0, at(9, 28), Some(lines::DAY_MS)),
                    lines::percent(WEEKLY, 60.0, at(10, 1), Some(lines::WEEK_MS)),
                    lines::dollar_value(EXTRA, 964.22),
                ],
            )
        );
    }

    #[tokio::test]
    async fn sends_the_key_in_the_body_of_a_connect_request() {
        let http = Scripted::new().on("POST", &url(CUSTOM), 200, &answer("Pro"));
        let scope = context_at(&http, cli_secret(), now());
        Devin.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(
            requests[0].url,
            "https://server.codeium.test/exa.seat_management_pb.SeatManagementService/GetUserStatus"
        );
        assert_eq!(requests[0].timeout, std::time::Duration::from_secs(15));
        assert_eq!(
            header(&requests[0], "Content-Type"),
            Some("application/json")
        );
        assert_eq!(header(&requests[0], "Connect-Protocol-Version"), Some("1"));
        assert_eq!(header(&requests[0], "Authorization"), None);
        assert_eq!(
            body(&requests[0]),
            json!({"metadata": {
                "apiKey": CLI_KEY,
                "ideName": "devin",
                "ideVersion": "1.108.2",
                "extensionName": "devin",
                "extensionVersion": "1.108.2",
                "locale": "en"
            }})
        );
    }

    #[tokio::test]
    async fn a_refused_key_asks_to_sign_in_again() {
        for status in [401, 403] {
            let http = Scripted::new().on(
                "POST",
                &url(DEFAULT_SERVER),
                status,
                r#"{"code":"unauthenticated","message":"invalid api key"}"#,
            );
            let scope = context_at(&http, json!({ "apiKey": APP_KEY }), now());
            let error = Devin.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthExpired);
            assert_eq!(error.message, EXPIRED);
            assert_eq!(http.requests()[0].url, url(DEFAULT_SERVER));
        }
    }

    #[tokio::test]
    async fn an_answer_that_is_not_json_is_unreadable() {
        let http = Scripted::new().on(
            "POST",
            &url(DEFAULT_SERVER),
            200,
            "<html>maintenance</html>",
        );
        let scope = context_at(&http, json!({ "apiKey": APP_KEY }), now());
        let error = Devin.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);
        assert_eq!(
            error.message,
            "Devin returned usage data this version cannot read."
        );
    }

    #[tokio::test]
    async fn a_refused_cli_key_falls_back_to_the_app_login_and_remembers_it() {
        let http = Scripted::new().on("POST", &url(CUSTOM), 401, "{}").on(
            "POST",
            &url(DEFAULT_SERVER),
            200,
            &answer("Teams"),
        );
        let scope = context_at(&http, both_secret(), now());
        let reading = Devin.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Teams"));
        let requests = http.requests();
        let urls: Vec<&str> = requests
            .iter()
            .map(|request| request.url.as_str())
            .collect();
        assert_eq!(urls, vec![url(CUSTOM), url(DEFAULT_SERVER)]);
        assert_eq!(body(&requests[0])["metadata"]["apiKey"], CLI_KEY);
        assert_eq!(body(&requests[1])["metadata"]["apiKey"], APP_KEY);

        Devin.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].url, url(DEFAULT_SERVER));
    }

    #[tokio::test]
    async fn rate_limits_stop_without_trying_the_other_login() {
        let http = Scripted::new().on("POST", &url(CUSTOM), 429, "{}").on(
            "POST",
            &url(DEFAULT_SERVER),
            200,
            &answer("Max"),
        );
        let scope = context_at(&http, both_secret(), now());
        let error = Devin.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn when_every_login_fails_a_refusal_wins_over_the_first_failure() {
        let http = Scripted::new().on("POST", &url(CUSTOM), 503, "{}").on(
            "POST",
            &url(DEFAULT_SERVER),
            500,
            "{}",
        );
        let scope = context_at(&http, both_secret(), now());
        let error = Devin.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "Devin answered with HTTP 503.");
        assert_eq!(http.requests().len(), 2);

        let http = Scripted::new().on("POST", &url(CUSTOM), 503, "{}").on(
            "POST",
            &url(DEFAULT_SERVER),
            401,
            "{}",
        );
        let scope = context_at(&http, both_secret(), now());
        let error = Devin.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(error.message, EXPIRED);
    }

    #[test]
    fn a_hidden_daily_quota_stands_in_for_a_missing_weekly_one() {
        let reading = usage(
            &status(json!({
                "planInfo": {"planName": "Max", "hideDailyQuota": true},
                "dailyQuotaRemainingPercent": 30,
                "dailyQuotaResetAtUnix": "1790553600"
            })),
            &DEVIN_CARD,
        )
        .unwrap();
        assert_eq!(
            meters(&reading),
            vec![(WEEKLY.to_string(), 70.0, None, Some(lines::WEEK_MS))]
        );
    }

    #[test]
    fn a_weekly_reset_without_a_percentage_means_the_week_is_used_up() {
        for hide_daily in [true, false] {
            let reading = usage(
                &status(json!({
                    "planInfo": {"planName": "Max", "hideDailyQuota": hide_daily},
                    "dailyQuotaRemainingPercent": 100,
                    "dailyQuotaResetAtUnix": "1790553600",
                    "weeklyQuotaResetAtUnix": "1790812800",
                    "overageBalanceMicros": "-44347"
                })),
                &DEVIN_CARD,
            )
            .unwrap();
            let meters = meters(&reading);
            assert_eq!(
                meters.last(),
                Some(&(WEEKLY.to_string(), 100.0, at(10, 1), Some(lines::WEEK_MS)))
            );
            assert_eq!(meters.iter().any(|meter| meter.0 == DAILY), !hide_daily);
            assert_eq!(balance(&reading), Some(0.0));
        }
    }

    #[test]
    fn a_malformed_weekly_percentage_is_unreadable_rather_than_used_up() {
        for malformed in [json!("abc"), json!(true), Value::Null] {
            let error = usage(
                &status(json!({
                    "planInfo": {"planName": "Max"},
                    "weeklyQuotaRemainingPercent": malformed,
                    "weeklyQuotaResetAtUnix": "1790812800"
                })),
                &DEVIN_CARD,
            )
            .unwrap_err();
            assert_eq!(error.category, ErrorCategory::Decoding);
        }
    }

    #[test]
    fn answers_without_quota_are_unavailable_and_other_shapes_unreadable() {
        let error = usage(
            &status(json!({"planInfo": {"planName": "Max"}})),
            &DEVIN_CARD,
        )
        .unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(error.message, UNAVAILABLE);
        let error = usage(&json!({"code": "internal"}), &DEVIN_CARD).unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);
    }

    #[test]
    fn a_zero_balance_stays_a_real_zero_and_no_plan_name_means_no_plan() {
        let reading = usage(&status(json!({"overageBalanceMicros": "0"})), &DEVIN_CARD).unwrap();
        assert_eq!(balance(&reading), Some(0.0));
        assert_eq!(reading.plan, None);
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn save_app_login(roots: &Roots, auth_status: &str) {
        let path = apps::state_db(roots, APP);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ItemTable VALUES ('windsurfAuthStatus', ?1)",
                [auth_status],
            )
            .unwrap();
    }

    #[test]
    fn discovers_the_cli_login_with_its_https_server() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        let file = roots
            .home
            .join(".local")
            .join("share")
            .join("devin")
            .join("credentials.toml");
        write(
            &file,
            "windsurf_api_key = \"devin-session-token$cli\"\napi_server_url = \"https://server.codeium.test/\"\n",
        );
        let logins = Devin.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(
            logins[0].identity,
            "605b14ff4d51211a0ba2c597dc1a733ad7cf3c30e040991260d8310d5aa5f7c1"
        );
        assert_eq!(logins[0].label, None);
        assert_eq!(logins[0].origin, CLI);
        assert_eq!(logins[0].location, file);
        assert_eq!(logins[0].secret.key(), Some(CLI_KEY));
        assert_eq!(logins[0].secret.str("/apiServerUrl"), Some(CUSTOM));
        assert_eq!(logins[0].secret.str("/fallback/apiKey"), None);

        write(
            &file,
            "windsurf_api_key = 'devin-session-token$cli' # devin auth login\napi_server_url = \"http://server.codeium.test\"\n",
        );
        let logins = Devin.discover(&roots);
        assert_eq!(logins[0].secret.key(), Some(CLI_KEY));
        assert_eq!(logins[0].secret.str("/apiServerUrl"), None);

        let xdg = dir.path().join("xdg");
        write(
            &xdg.join("devin").join("credentials.toml"),
            "windsurf_api_key = \"devin-session-token$xdg\"\n",
        );
        let logins = Devin.discover(
            &roots
                .clone()
                .with_var("XDG_DATA_HOME", &xdg.to_string_lossy()),
        );
        assert_eq!(logins[0].secret.key(), Some("devin-session-token$xdg"));

        assert!(
            Devin
                .discover(&Roots::under(&dir.path().join("empty")))
                .is_empty()
        );
    }

    #[test]
    fn the_documented_credentials_file_wins_over_a_copy_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        let share = roots.home.join(".local").join("share");
        let (documented, elsewhere) = if cfg!(windows) {
            (roots.app_data.join("devin"), share.join("devin"))
        } else {
            (share.join("devin"), roots.app_data.join("devin"))
        };
        write(
            &elsewhere.join("credentials.toml"),
            "windsurf_api_key = \"devin-session-token$stale\"\n",
        );
        write(
            &documented.join("credentials.toml"),
            "windsurf_api_key = \"devin-session-token$cli\"\n",
        );
        let logins = Devin.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].secret.key(), Some(CLI_KEY));
        assert_eq!(logins[0].location, documented.join("credentials.toml"));
    }

    #[test]
    fn falls_back_to_the_app_login_and_keeps_a_different_one_behind_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        save_app_login(
            &roots,
            r#"{"apiKey":"devin-session-token$app","name":"Someone"}"#,
        );
        let logins = Devin.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].origin, APP);
        assert_eq!(logins[0].location, apps::state_db(&roots, APP));
        assert_eq!(
            logins[0].identity,
            "565c36c9077b1f43ee414b77e22bfedcf970129a51474c3d566d0c5f0d5738b7"
        );
        assert_eq!(logins[0].secret.key(), Some(APP_KEY));

        let file = roots.local_data.join("devin").join("credentials.toml");
        write(
            &file,
            "[auth]\nwindsurf_api_key = \"devin-session-token$cli\"\n",
        );
        let logins = Devin.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].origin, CLI);
        assert_eq!(logins[0].secret.key(), Some(CLI_KEY));
        assert_eq!(logins[0].secret.str("/fallback/apiKey"), Some(APP_KEY));

        write(&file, "windsurf_api_key = \"devin-session-token$app\"\n");
        let logins = Devin.discover(&roots);
        assert_eq!(logins[0].secret.key(), Some(APP_KEY));
        assert_eq!(logins[0].secret.str("/fallback/apiKey"), None);

        write(
            &file,
            "windsurf_api_key = \"devin-session-token$app\"\napi_server_url = \"https://server.codeium.test\"\n",
        );
        let logins = Devin.discover(&roots);
        let attempts = attempts(&logins[0].secret);
        let tried: Vec<(&str, &str)> = attempts
            .iter()
            .map(|attempt| (attempt.key, attempt.server))
            .collect();
        assert_eq!(tried, vec![(APP_KEY, CUSTOM), (APP_KEY, DEFAULT_SERVER)]);
    }

    #[tokio::test]
    async fn a_pasted_cli_key_reads_the_quota_on_the_default_server() {
        let http = Scripted::new().on("POST", &url(DEFAULT_SERVER), 200, &answer("Pro"));
        let scope = context_at(&http, json!({ "apiKey": "cli-key" }), now());
        let reading = Devin.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(body(&http.requests()[0])["metadata"]["apiKey"], "cli-key");
    }

    #[tokio::test]
    async fn an_enterprise_admin_key_reads_this_months_acus_once_an_hour() {
        let consumption = "https://api.devin.ai/v2/enterprise/consumption/daily?start_date=2026-09-01T08%3A00%3A00Z&end_date=2026-09-27T10%3A00%3A00Z";
        let http = Scripted::new().on("GET", consumption, 200, r#"{"total_acus": 412.5}"#);
        let scope = context_at(&http, json!({ "apiKey": "apk_user_abc" }), now());
        for _ in 0..2 {
            let reading = Devin.fetch(&scope.context()).await.unwrap();
            assert_eq!(reading.plan.as_deref(), Some("Enterprise"));
            assert_eq!(
                reading.lines,
                vec![lines::count_value(ENTERPRISE_ACUS, 412.5, "ACUs")]
            );
        }
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some("Bearer apk_user_abc")
        );
    }

    #[tokio::test]
    async fn a_refused_enterprise_key_says_which_key_is_needed() {
        let http = Scripted::new().on(
            "GET",
            "https://api.devin.ai/v2/enterprise/consumption/daily?start_date=2026-09-01T08%3A00%3A00Z&end_date=2026-09-27T10%3A00%3A00Z",
            403,
            "{}",
        );
        let scope = context_at(&http, json!({ "apiKey": "apk_user_abc" }), now());
        let error = Devin.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, ENTERPRISE_REFUSED);
    }

    #[test]
    fn the_billing_month_starts_on_the_first_at_eight_utc() {
        let at = |month, day, hour| Utc.with_ymd_and_hms(2026, month, day, hour, 0, 0).unwrap();
        assert_eq!(month_start(at(9, 27, 10)), at(9, 1, 8));
        assert_eq!(month_start(at(10, 1, 7)), at(9, 1, 8));
        assert_eq!(month_start(at(10, 1, 8)), at(10, 1, 8));
    }

    #[test]
    fn descriptors_read_the_lines_the_reader_writes() {
        let provider = Provider::new("devin@abc", NAME);
        let descriptors = Devin.descriptors(&provider);
        let summary: Vec<(&str, &str, &str)> = descriptors
            .iter()
            .map(|descriptor| {
                (
                    descriptor.id.as_str(),
                    descriptor.title(),
                    descriptor.metric_label.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("devin@abc.daily", "Daily", DAILY),
                ("devin@abc.weekly", "Weekly", WEEKLY),
                ("devin@abc.extra", "Extra Usage", EXTRA),
                ("devin@abc.enterprise", ENTERPRISE_ACUS, ENTERPRISE_ACUS),
            ]
        );
        let exports: Vec<(&str, LimitResourceKind, &str)> = descriptors
            .iter()
            .flat_map(|descriptor| &descriptor.limit_resources)
            .map(|resource| (resource.key.as_str(), resource.kind, resource.unit.as_str()))
            .collect();
        assert_eq!(
            exports,
            vec![
                ("daily", LimitResourceKind::Consumption, "percent"),
                ("weekly", LimitResourceKind::Consumption, "percent"),
                ("extraUsageBalance", LimitResourceKind::Balance, "usd"),
                ("enterpriseAcus", LimitResourceKind::Consumption, "count"),
            ]
        );
        assert_eq!(
            descriptors[2].template.unbounded_value_word.as_deref(),
            Some("left")
        );
    }
}
