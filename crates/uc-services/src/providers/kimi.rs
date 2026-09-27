//! Kimi Code: the quota of a Kimi Code subscription (kimi.com/code) as the CLI's `/usage` shows
//! it: the rolling 5-hour window, the weekly request pool and, on plans that report it, the
//! monthly total pool.
//!
//! A login is read from the Kimi Code CLI's `~/.kimi-code/credentials/kimi-code.json`
//! (`KIMI_CODE_HOME` moves the folder) or, when that CLI has none, from the older Python CLI's
//! `~/.kimi/credentials/kimi-code.json` (`KIMI_SHARE_DIR`). Both sit under the home folder on
//! Windows (`C:\Users\<name>`), macOS and Linux alike. The CLI renews its own tokens and replaces
//! the refresh token each time, so a login is used only while its access token is fresh and is
//! never renewed here. A Kimi Code API key comes from Quota Control or from `KIMI_API_KEY` or
//! `KIMI_CODE_API_KEY`.
//!
//! Endpoint: `GET https://api.kimi.com/coding/v1/usages` with a Bearer token, as the CLI sends it.
//! An API key that host refuses is tried once on `https://api.kimi.ai` (Kimi's international
//! service), and the host that answered is remembered for 12 hours. A CLI login goes to kimi.com
//! only: that is where the CLI signs in, and its file does not say which host issued the token.

use std::path::Path;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{
    HttpRequest, HttpResponse, MetricLine, Provider, ProviderLink, SessionStartSignal,
    SimpleProviderError, WidgetDescriptor,
};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{http, jwt, lines, value};

pub(crate) struct Kimi;

const NAME: &str = "Kimi";
const APP: &str = "Kimi Code";
/// The Code API hosts: kimi.com, then kimi.ai for accounts of Kimi's international service.
const HOSTS: [&str; 2] = ["https://api.kimi.com", "https://api.kimi.ai"];
const USAGES: &str = "/coding/v1/usages";
const HOST_MEMO: &str = "kimi.host";
/// How long before its stated expiry a CLI access token counts as expired.
const LEEWAY_SECONDS: i64 = 60;
const SESSION_MS: i64 = 5 * lines::HOUR_MS;

const SESSION: &str = "Session";
const WEEKLY: &str = "Weekly";
const TOTAL: &str = "Total Usage";
/// The card's meters: (descriptor suffix, title).
const METERS: [(&str, &str); 3] = [
    ("session", SESSION),
    ("weekly", WEEKLY),
    ("totalUsage", TOTAL),
];

const LOGIN_EXPIRED: &str = "The Kimi Code login expired. Open Kimi Code once to renew it.";
const KEY_REFUSED: &str = "Kimi refused the API key. Use a Kimi Code key; Moonshot platform keys have no Kimi Code quota.";
const NO_PLAN: &str =
    "This Kimi account has no Kimi Code subscription, so there is no quota to show.";

#[async_trait]
impl Service for Kimi {
    fn id(&self) -> &'static str {
        "kimi"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![ProviderLink::new(
            "Usage",
            "https://www.kimi.com/code/console",
        )]
    }

    fn connection(&self) -> Connection {
        Connection::login(APP).or_api_key(ApiKeyHelp {
            env: &["KIMI_API_KEY", "KIMI_CODE_API_KEY"],
            url: "https://www.kimi.com/code",
            fields: &[],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        [
            roots.dir_from("KIMI_CODE_HOME", roots.home.join(".kimi-code")),
            roots.dir_from("KIMI_SHARE_DIR", roots.home.join(".kimi")),
        ]
        .into_iter()
        .find_map(|home| read_login(&home.join("credentials").join("kimi-code.json")))
        .into_iter()
        .collect()
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        METERS
            .iter()
            .map(|(suffix, title)| {
                let signal = (*suffix == "session").then_some(SessionStartSignal::ZeroUsage);
                WidgetDescriptor::percent(
                    format!("{}.{suffix}", provider.id),
                    provider,
                    title,
                    None,
                    signal,
                )
                .exporting_progress(suffix, "percent")
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context.secret.key();
        let token = match key {
            Some(key) => key,
            None => fresh_access_token(context.secret, context.now)?,
        };
        let body = usages(context, token, key.is_none()).await?;
        let meters = meters(&body, context.now);
        if meters.is_empty() {
            return Err(http::decoding(NAME));
        }
        Ok(Reading::new(plan(&body), meters))
    }
}

/// The CLI login saved in `path`: only its access token and when that expires. The refresh token
/// stays in the file, since the CLI rotates it and alone may spend it; it is only read for the
/// account id its claims may carry, which, unlike its hash, survives that rotation.
fn read_login(path: &Path) -> Option<Login> {
    let document = value::read_json(path, 64 * 1024)?;
    let access = value::text(&document, "/access_token");
    let refresh = value::text(&document, "/refresh_token");
    let fallback = refresh.or(access)?;
    let expires =
        value::time(&document, "/expires_at").or_else(|| access.and_then(jwt::expires_at));
    let claim = |names: &[&str]| {
        [access, refresh]
            .into_iter()
            .flatten()
            .find_map(|token| jwt::claim(token, names))
    };
    let identity = claim(&["sub", "user_id"]).unwrap_or_else(|| {
        Sha256::digest(fallback.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    });
    let label = claim(&["email"]);
    let secret = json!({
        "access_token": access,
        "expires_at": expires.map(|time| time.timestamp()),
    });
    Some(Login::new(identity, APP, path, Secret::new(secret)).with_label(label))
}

/// The login's access token while it is fresh. An expired one is left for the CLI to renew: it
/// rotates the refresh token, so renewing here would sign the CLI out.
fn fresh_access_token(secret: &Secret, now: DateTime<Utc>) -> Result<&str, SimpleProviderError> {
    let fresh = value::time(secret.value(), "/expires_at")
        .is_none_or(|expires| expires > now + Duration::seconds(LEEWAY_SECONDS));
    secret
        .str("/access_token")
        .filter(|_| fresh)
        .ok_or_else(|| http::expired(LOGIN_EXPIRED))
}

/// The usage document. A CLI login asks kimi.com only. An API key asks the host that last answered
/// this card (kimi.com at first); a 401 or 403 moves on to the other host once, and the host that
/// answers is remembered for 12 hours.
async fn usages(
    context: &FetchContext<'_>,
    token: &str,
    login: bool,
) -> Result<Value, SimpleProviderError> {
    let mut hosts = HOSTS;
    if !login
        && context
            .memo
            .get(HOST_MEMO, context.now)
            .await
            .is_some_and(|host| host.as_str() == Some(HOSTS[1]))
    {
        hosts.reverse();
    }
    let mut host = hosts[0];
    let mut response = get(context, host, token).await?;
    if matches!(response.status, 401 | 403) {
        if login {
            return Err(refusal(&response, None, true));
        }
        match get(context, hosts[1], token).await {
            Ok(other) if other.is_success() => (host, response) = (hosts[1], other),
            other => return Err(refusal(&response, other.ok().as_ref(), false)),
        }
    }
    if !response.is_success() {
        return Err(http::status_error(&response, NAME));
    }
    if !login {
        context
            .memo
            .put(
                HOST_MEMO,
                json!(host),
                Some(context.now + Duration::hours(12)),
            )
            .await;
    }
    http::parse(&response, NAME)
}

async fn get(
    context: &FetchContext<'_>,
    host: &str,
    token: &str,
) -> Result<HttpResponse, SimpleProviderError> {
    http::send(
        context.http,
        HttpRequest::get(format!("{host}{USAGES}"))
            .bearer(token)
            .header("Accept", "application/json"),
        NAME,
    )
    .await
}

/// The error for a credential the first host refused, after the second host's answer (`second` is
/// `None` for a CLI login, which is not offered to it, or when it could not be reached). A 429 or
/// 5xx there is reported as it is, since the key may belong to that host; otherwise a 403 saying
/// the account has no Kimi Code plan wins over a plain refusal.
fn refusal(
    first: &HttpResponse,
    second: Option<&HttpResponse>,
    login: bool,
) -> SimpleProviderError {
    if let Some(second) = second.filter(|second| second.status == 429 || second.status >= 500) {
        return http::status_error(second, NAME);
    }
    if lacks_plan(first) || second.is_some_and(lacks_plan) {
        return http::not_available(NO_PLAN);
    }
    if login {
        http::expired(LOGIN_EXPIRED)
    } else {
        http::invalid(KEY_REFUSED)
    }
}

/// Whether a 403 says the credential works but the account has no Kimi Code plan: Kimi then
/// answers `permission_denied` with reason `REASON_FEATURE_NO_PERMISSION`.
fn lacks_plan(response: &HttpResponse) -> bool {
    if response.status != 403 {
        return false;
    }
    let text = response.text().to_ascii_lowercase();
    [
        "reason_feature_no_permission",
        "permission_denied",
        "do not have permission",
        "subscribe",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

/// A window's absolute request counts, its length (when known) and when it resets.
#[derive(Clone, Copy)]
struct Counts {
    used: f64,
    limit: f64,
    period_ms: Option<i64>,
    resets_at: Option<DateTime<Utc>>,
}

/// A ratio pool of the newer `usages` map: the used percent, its length and when it resets.
#[derive(Clone, Copy)]
struct Pool {
    percent: f64,
    period_ms: i64,
    resets_at: Option<DateTime<Utc>>,
}

/// The card's lines: the 5-hour window from `limits`, the weekly pool from `usage`, and the
/// monthly total pool, which only the `usages` ratio map reports.
fn meters(body: &Value, now: DateTime<Utc>) -> Vec<MetricLine> {
    let weekly = body
        .get("usage")
        .and_then(|usage| counts(usage, Some(lines::WEEK_MS)));
    [
        window(
            SESSION,
            session_counts(body),
            pool(body, "limit_5h", SESSION_MS),
            now,
        ),
        window(WEEKLY, weekly, pool(body, "limit_7d", lines::WEEK_MS), now),
        window(
            TOTAL,
            None,
            pool(body, "limit_month_total", lines::MONTH_MS),
            now,
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// One window's meter. Counts win over the ratio pool, since Kimi has sent `used_ratio: 0` beside
/// a used-up count (MoonshotAI/kimi-code#3951), unless the counts' window already reset while
/// the pool reports the next one.
fn window(
    title: &str,
    counts: Option<Counts>,
    pool: Option<Pool>,
    now: DateTime<Utc>,
) -> Option<MetricLine> {
    let stale = counts.is_some_and(|counts| counts.resets_at.is_some_and(|reset| reset <= now))
        && pool.is_some_and(|pool| pool.resets_at.is_some_and(|reset| reset > now));
    match (counts.filter(|_| !stale), pool) {
        (Some(counts), _) => Some(lines::count(
            title,
            counts.used,
            counts.limit,
            "requests",
            counts.resets_at,
            counts.period_ms,
        )),
        (None, Some(pool)) => Some(lines::percent(
            title,
            pool.percent,
            pool.resets_at,
            Some(pool.period_ms),
        )),
        (None, None) => None,
    }
}

/// A `usage` or `limits[].detail` object as counts. Kimi sends them as decimal strings; `used` may
/// pass the limit during overage, and `remaining` stands in for it only when it fits the limit.
fn counts(detail: &Value, period_ms: Option<i64>) -> Option<Counts> {
    let limit = value::number(detail, "/limit").filter(|limit| *limit > 0.0)?;
    let used = value::number(detail, "/used")
        .filter(|used| *used >= 0.0)
        .or_else(|| {
            value::number(detail, "/remaining")
                .filter(|remaining| (0.0..=limit).contains(remaining))
                .map(|remaining| limit - remaining)
        })?;
    Some(Counts {
        used,
        limit,
        period_ms,
        resets_at: reset(detail),
    })
}

/// The 5-hour window among `limits`, or the first usable one when none lasts 5 hours, sized by its
/// own `window`: an item without one is taken as 5 hours, and one whose window this version cannot
/// read gets no length rather than a guessed one.
fn session_counts(body: &Value) -> Option<Counts> {
    let windows: Vec<Counts> = body
        .get("limits")?
        .as_array()?
        .iter()
        .filter_map(|item| {
            let period = match item.get("window").filter(|window| !window.is_null()) {
                Some(window) => window_ms(window),
                None => Some(SESSION_MS),
            };
            counts(item.get("detail")?, period)
        })
        .collect();
    windows
        .iter()
        .find(|counts| counts.period_ms == Some(SESSION_MS))
        .or(windows.first())
        .copied()
}

/// A `limits[].window` (`{"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"}`) in milliseconds.
fn window_ms(window: &Value) -> Option<i64> {
    let unit_ms = match value::text(window, "/timeUnit")? {
        "TIME_UNIT_SECOND" => 1_000,
        "TIME_UNIT_MINUTE" => 60_000,
        "TIME_UNIT_HOUR" => lines::HOUR_MS,
        "TIME_UNIT_DAY" => lines::DAY_MS,
        _ => return None,
    };
    let duration = value::number(window, "/duration")?;
    let milliseconds = duration * unit_ms as f64;
    (duration > 0.0 && milliseconds <= 366.0 * lines::DAY_MS as f64)
        .then(|| milliseconds.round() as i64)
}

/// A ratio pool of the `usages` map (`{"used_ratio": 0.25, "reset_time": "…"}`).
fn pool(body: &Value, key: &str, period_ms: i64) -> Option<Pool> {
    let pool = body.get("usages")?.get(key)?;
    let ratio = value::number(pool, "/used_ratio").filter(|ratio| *ratio >= 0.0)?;
    Some(Pool {
        percent: ratio * 100.0,
        period_ms,
        resets_at: reset(pool),
    })
}

/// When a window resets, under any of the names Kimi has used.
fn reset(window: &Value) -> Option<DateTime<Utc>> {
    ["/resetTime", "/reset_time", "/resetAt", "/reset_at"]
        .into_iter()
        .find_map(|pointer| value::time(window, pointer))
}

/// The membership tier's name. The V1 goods catalog names tiers after tempos; a level outside it,
/// or from another catalog version, shows as itself (`LEVEL_ULTRA` → `Ultra`).
fn plan(body: &Value) -> Option<String> {
    let level = value::text(body, "/user/membership/level")
        .filter(|level| *level != "LEVEL_UNSPECIFIED")?;
    let catalog_v1 = body
        .get("version")
        .filter(|version| !version.is_null())
        .is_none_or(|version| version.as_str() == Some("GOODS_VERSION_V1"));
    let tempo = match level {
        "LEVEL_FREE" => Some("Adagio"),
        "LEVEL_TRIAL" => Some("Andante"),
        "LEVEL_BASIC" => Some("Moderato"),
        "LEVEL_INTERMEDIATE" => Some("Allegretto"),
        "LEVEL_ADVANCED" => Some("Allegro"),
        "LEVEL_STANDARD" => Some("Vivace"),
        _ => None,
    };
    match tempo.filter(|_| catalog_v1) {
        Some(tempo) => Some(tempo.to_string()),
        None => lines::plan_name(level.strip_prefix("LEVEL_").unwrap_or(level)),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::service::Memo;
    use crate::testing::{Scripted, context_at, header};
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use chrono::TimeZone;
    use uc_core::{ErrorCategory, HttpClient, HttpError, SharedHttpClient};

    const COM: &str = "https://api.kimi.com/coding/v1/usages";
    const AI: &str = "https://api.kimi.ai/coding/v1/usages";

    /// A Kimi Code answer with counts and the newer ratio pools, whose zero 5-hour and weekly
    /// ratios contradict the counts, as reported in MoonshotAI/kimi-code#3951.
    const USAGES_BODY: &str = r#"{
        "user": {"membership": {"level": "LEVEL_INTERMEDIATE"}},
        "usage": {"limit": "7168", "used": "1792", "remaining": "5376", "resetTime": "2026-10-01T15:23:13.716839300Z"},
        "limits": [{
            "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
            "detail": {"limit": "200", "used": "139", "remaining": "61", "resetTime": "2026-09-27T13:33:02.717479433Z"}
        }],
        "usages": {
            "limit_5h": {"used_ratio": 0, "reset_time": "2026-09-27T13:33:01Z"},
            "limit_7d": {"used_ratio": 0, "reset_time": "2026-10-01T15:23:12Z"},
            "limit_month_total": {"used_ratio": 0.25, "reset_time": "2026-10-17T00:00:00Z"}
        }
    }"#;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn key() -> Value {
        json!({"apiKey": "sk-kimi-test"})
    }

    fn login(expires: DateTime<Utc>) -> Value {
        json!({"access_token": "cli-access", "expires_at": expires.timestamp()})
    }

    fn jwt(payload: &str) -> String {
        format!("h.{}.s", URL_SAFE_NO_PAD.encode(payload))
    }

    fn expected_lines() -> Vec<MetricLine> {
        vec![
            lines::count(
                "Session",
                139.0,
                200.0,
                "requests",
                Some(at("2026-09-27T13:33:02.717479433Z")),
                Some(5 * lines::HOUR_MS),
            ),
            lines::count(
                "Weekly",
                1792.0,
                7168.0,
                "requests",
                Some(at("2026-10-01T15:23:13.716839300Z")),
                Some(lines::WEEK_MS),
            ),
            lines::percent(
                "Total Usage",
                25.0,
                Some(at("2026-10-17T00:00:00Z")),
                Some(lines::MONTH_MS),
            ),
        ]
    }

    #[tokio::test]
    async fn reads_counts_before_ratio_pools_and_names_the_plan() {
        let http = Scripted::new().on("GET", COM, 200, USAGES_BODY);
        let scope = context_at(&http, key(), now());
        let reading = Kimi.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Allegretto"));
        assert_eq!(reading.lines, expected_lines());
    }

    #[tokio::test]
    async fn sends_the_key_as_a_bearer_token_to_the_kimi_com_host() {
        let http = Scripted::new().on("GET", COM, 200, USAGES_BODY);
        let scope = context_at(&http, key(), now());
        Kimi.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].url, COM);
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some("Bearer sk-kimi-test")
        );
        assert_eq!(header(&requests[0], "Accept"), Some("application/json"));
        assert_eq!(header(&requests[0], "x-api-key"), None);
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn a_key_kimi_com_refuses_is_tried_on_kimi_ai_and_that_host_is_remembered() {
        let http = Scripted::new()
            .on(
                "GET",
                COM,
                401,
                r#"{"error":{"type":"invalid_authentication_error"}}"#,
            )
            .on("GET", AI, 200, USAGES_BODY);
        let scope = context_at(&http, key(), now());
        for _ in 0..2 {
            let reading = Kimi.fetch(&scope.context()).await.unwrap();
            assert_eq!(reading.lines, expected_lines());
        }
        let urls: Vec<String> = http
            .requests()
            .into_iter()
            .map(|request| request.url)
            .collect();
        assert_eq!(urls, [COM, AI, AI]);
    }

    #[tokio::test]
    async fn a_key_both_hosts_refuse_is_rejected() {
        let http = Scripted::new()
            .on("GET", COM, 401, "{}")
            .on("GET", AI, 401, "{}");
        let scope = context_at(&http, key(), now());
        let error = Kimi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(error.message, KEY_REFUSED);
        assert_eq!(http.requests().len(), 2);
    }

    #[tokio::test]
    async fn a_missing_endpoint_on_kimi_ai_keeps_the_kimi_com_refusal() {
        let http = Scripted::new()
            .on("GET", COM, 401, "{}")
            .on("GET", AI, 404, "not found");
        let scope = context_at(&http, key(), now());
        let error = Kimi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
    }

    /// Answers kimi.com from a script and cannot reach kimi.ai at all.
    struct KimiAiUnreachable(Scripted);

    #[async_trait]
    impl HttpClient for KimiAiUnreachable {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            if request.url.starts_with(HOSTS[1]) {
                return Err(HttpError::Timeout);
            }
            self.0.send(request).await
        }
    }

    #[tokio::test]
    async fn an_unreachable_kimi_ai_keeps_the_kimi_com_refusal() {
        let scripted = Scripted::new().on("GET", COM, 401, "{}");
        let http: SharedHttpClient = Arc::new(KimiAiUnreachable(scripted.clone()));
        let secret = Secret::new(key());
        let memo = Memo::default();
        let context = FetchContext {
            secret: &secret,
            http: &http,
            now: now(),
            memo: &memo,
        };
        let error = Kimi.fetch(&context).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(error.message, KEY_REFUSED);
        assert_eq!(scripted.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_refused_login_asks_to_open_the_cli_and_is_never_sent_to_kimi_ai() {
        let http = Scripted::new()
            .on("GET", COM, 401, "{}")
            .on("GET", AI, 200, USAGES_BODY);
        let scope = context_at(&http, login(now() + Duration::hours(1)), now());
        let error = Kimi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "The Kimi Code login expired. Open Kimi Code once to renew it."
        );
        let urls: Vec<String> = http
            .requests()
            .into_iter()
            .map(|request| request.url)
            .collect();
        assert_eq!(urls, [COM]);
    }

    #[tokio::test]
    async fn a_login_without_kimi_code_is_unavailable() {
        let http = Scripted::new().on(
            "GET",
            COM,
            403,
            r#"{"code":"permission_denied","message":"You do not have permission to access this feature"}"#,
        );
        let scope = context_at(&http, login(now() + Duration::hours(1)), now());
        let error = Kimi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(error.message, NO_PLAN);
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_server_error_is_reported_without_trying_the_other_host() {
        let http =
            Scripted::new()
                .on("GET", COM, 503, "unavailable")
                .on("GET", AI, 200, USAGES_BODY);
        let scope = context_at(&http, key(), now());
        let error = Kimi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "Kimi answered with HTTP 503.");
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_body_that_is_not_json_is_a_decoding_error() {
        let http = Scripted::new().on("GET", COM, 200, "<html>maintenance</html>");
        let scope = context_at(&http, key(), now());
        let error = Kimi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);
        assert_eq!(
            error.message,
            "Kimi returned usage data this version cannot read."
        );
    }

    #[tokio::test]
    async fn a_fresh_login_reads_usage_with_its_access_token() {
        let http = Scripted::new().on("GET", COM, 200, USAGES_BODY);
        let scope = context_at(&http, login(now() + Duration::minutes(30)), now());
        let reading = Kimi.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines, expected_lines());
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer cli-access")
        );
    }

    #[tokio::test]
    async fn an_expiring_login_is_left_for_the_cli_to_renew() {
        let http = Scripted::new().on("GET", COM, 200, USAGES_BODY);
        let scope = context_at(&http, login(now() + Duration::seconds(30)), now());
        let error = Kimi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(error.message, LOGIN_EXPIRED);
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn rate_limiting_stops_without_trying_the_other_host() {
        let http = Scripted::new()
            .on(
                "GET",
                COM,
                429,
                r#"{"error":{"type":"rate_limit_reached_error"}}"#,
            )
            .on("GET", AI, 200, USAGES_BODY);
        let scope = context_at(&http, key(), now());
        let error = Kimi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn an_account_without_kimi_code_is_unavailable_rather_than_refused() {
        let http = Scripted::new()
            .on(
                "GET",
                COM,
                403,
                r#"{"code":"permission_denied","message":"You do not have permission to access this feature","details":[{"type":"google.rpc.ErrorInfo","debug":{"reason":"REASON_FEATURE_NO_PERMISSION"}}]}"#,
            )
            .on("GET", AI, 401, "{}");
        let scope = context_at(&http, key(), now());
        let error = Kimi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(error.message, NO_PLAN);
    }

    #[tokio::test]
    async fn an_answer_without_a_quota_window_is_not_read_as_unused() {
        let http = Scripted::new().on(
            "GET",
            COM,
            200,
            r#"{"usages":{"limit_5h":{}},"usage":{"limit":"0"}}"#,
        );
        let scope = context_at(&http, key(), now());
        let error = Kimi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);
    }

    #[test]
    fn ratio_pools_stand_in_when_counts_are_missing() {
        let body = json!({"usages": {
            "limit_5h": {"used_ratio": 0.625, "reset_time": "2026-09-27T12:00:00Z"},
            "limit_7d": {"used_ratio": 0.125, "reset_time": "2026-10-02T00:00:00Z"}
        }});
        assert_eq!(
            meters(&body, now()),
            vec![
                lines::percent(
                    "Session",
                    62.5,
                    Some(at("2026-09-27T12:00:00Z")),
                    Some(5 * lines::HOUR_MS)
                ),
                lines::percent(
                    "Weekly",
                    12.5,
                    Some(at("2026-10-02T00:00:00Z")),
                    Some(lines::WEEK_MS)
                ),
            ]
        );
        assert_eq!(plan(&body), None);
    }

    #[test]
    fn counts_whose_window_already_reset_give_way_to_the_next_window_pool() {
        let body = json!({
            "usage": {"limit": "100", "used": "19", "resetTime": "2026-09-26T16:45:59Z"},
            "usages": {"limit_7d": {"used_ratio": 0.0625, "reset_time": "2026-10-03T16:45:59Z"}}
        });
        assert_eq!(
            meters(&body, now()),
            vec![lines::percent(
                "Weekly",
                6.25,
                Some(at("2026-10-03T16:45:59Z")),
                Some(lines::WEEK_MS)
            )]
        );
    }

    #[test]
    fn the_five_hour_limit_is_picked_and_remaining_stands_in_for_used() {
        let body = json!({"limits": [
            {"window": {"duration": 1, "timeUnit": "TIME_UNIT_DAY"}, "detail": {"limit": 1000, "used": 10}},
            {"window": {"duration": 5, "timeUnit": "TIME_UNIT_HOUR"}, "detail": {"limit": "100", "remaining": "99", "reset_at": "2026-09-27T14:45:59Z"}}
        ]});
        assert_eq!(
            meters(&body, now()),
            vec![lines::count(
                "Session",
                1.0,
                100.0,
                "requests",
                Some(at("2026-09-27T14:45:59Z")),
                Some(5 * lines::HOUR_MS)
            )]
        );
    }

    #[test]
    fn membership_levels_become_tier_names() {
        let plan_of = |level: &str, version: Value| {
            plan(&json!({"user": {"membership": {"level": level}}, "version": version}))
        };
        assert_eq!(
            plan_of("LEVEL_BASIC", Value::Null).as_deref(),
            Some("Moderato")
        );
        assert_eq!(
            plan_of("LEVEL_TRIAL", json!("GOODS_VERSION_V1")).as_deref(),
            Some("Andante")
        );
        assert_eq!(
            plan_of("LEVEL_BASIC", json!("GOODS_VERSION_V2")).as_deref(),
            Some("Basic")
        );
        assert_eq!(
            plan_of("LEVEL_ULTRA_PLUS", Value::Null).as_deref(),
            Some("Ultra Plus")
        );
        assert_eq!(plan_of("LEVEL_UNSPECIFIED", Value::Null), None);
    }

    #[test]
    fn every_line_feeds_a_descriptor() {
        let provider = Provider::new("kimi@abc", "Kimi");
        let descriptors = Kimi.descriptors(&provider);
        let ids: Vec<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            ["kimi@abc.session", "kimi@abc.weekly", "kimi@abc.totalUsage"]
        );
        assert_eq!(
            descriptors[0].template.session_start_signal,
            Some(SessionStartSignal::ZeroUsage)
        );
        let body: Value = serde_json::from_str(USAGES_BODY).unwrap();
        for line in meters(&body, now()) {
            assert!(
                descriptors
                    .iter()
                    .any(|descriptor| descriptor.metric_label == line.label()),
                "{}",
                line.label()
            );
        }
    }

    #[test]
    fn discovers_the_cli_login_without_its_refresh_token() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = dir.path().join(".kimi-code").join("credentials");
        std::fs::create_dir_all(&credentials).unwrap();
        let access = jwt(r#"{"sub":"user-123","exp":1790503200}"#);
        std::fs::write(
            credentials.join("kimi-code.json"),
            json!({
                "access_token": access,
                "refresh_token": "refresh-1",
                "expires_at": 1790503200.5,
                "token_type": "Bearer",
                "expires_in": 900.0
            })
            .to_string(),
        )
        .unwrap();
        let logins = Kimi.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, "user-123");
        assert_eq!(logins[0].label, None);
        assert_eq!(logins[0].origin, APP);
        assert_eq!(logins[0].location, credentials.join("kimi-code.json"));
        assert_eq!(
            logins[0].secret.value(),
            &json!({"access_token": access, "expires_at": 1790503200})
        );
        assert!(
            Kimi.discover(&Roots::under(&dir.path().join("empty")))
                .is_empty()
        );
    }

    #[test]
    fn falls_back_to_the_older_cli_and_follows_folder_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(".kimi").join("credentials");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join("kimi-code.json"),
            r#"{"access_token":"opaque","refresh_token":"refresh-2","expires_at":1790503200}"#,
        )
        .unwrap();
        let logins = Kimi.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(
            logins[0].identity,
            "f865a88f563b9fbc4c224f6fbabd2ec9f810efefc61b212bda20ee64a6fb2840"
        );
        assert_eq!(logins[0].location, legacy.join("kimi-code.json"));

        let moved = dir.path().join("elsewhere").join("credentials");
        std::fs::create_dir_all(&moved).unwrap();
        std::fs::write(
            moved.join("kimi-code.json"),
            json!({"access_token": jwt(r#"{"sub":"user-9","email":"me@example.com"}"#)})
                .to_string(),
        )
        .unwrap();
        let roots = Roots::under(dir.path()).with_var(
            "KIMI_CODE_HOME",
            dir.path().join("elsewhere").to_str().unwrap(),
        );
        let logins = Kimi.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, "user-9");
        assert_eq!(logins[0].label.as_deref(), Some("me@example.com"));
    }

    #[test]
    fn an_opaque_access_token_takes_the_account_from_the_refresh_token() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = dir.path().join(".kimi-code").join("credentials");
        std::fs::create_dir_all(&credentials).unwrap();
        let refresh = jwt(r#"{"sub":"user-42","exp":1792000000}"#);
        std::fs::write(
            credentials.join("kimi-code.json"),
            json!({"access_token": "opaque", "refresh_token": refresh, "expires_at": 1790503200})
                .to_string(),
        )
        .unwrap();
        let logins = Kimi.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, "user-42");
        assert_eq!(
            logins[0].secret.value(),
            &json!({"access_token": "opaque", "expires_at": 1790503200})
        );
    }
}
