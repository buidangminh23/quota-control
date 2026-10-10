//! Z.ai (Zhipu AI) GLM Coding Plan: the quota of an API key saved in Quota Control or exported as
//! `ZAI_API_KEY`, `Z_AI_API_KEY`, `GLM_API_KEY` or `ZHIPUAI_API_KEY`. Nothing is read from disk, so
//! Windows, macOS and Linux behave the same.
//!
//! Endpoints (undocumented internal APIs of the Z.ai console, `Authorization: Bearer <key>`):
//! - `GET /api/monitor/usage/quota/limit` on `https://api.z.ai`. A key that host refuses is tried on
//!   `https://open.bigmodel.cn`, where keys issued in mainland China (BigModel) live. The host that
//!   took the key is remembered for 12 hours and asked first; when it refuses the key, the other host
//!   is asked in the same refresh. Z.ai refuses a key with HTTP 401/403 or with an HTTP 200
//!   `{"code":1000,"msg":"Authentication Failed","success":false}` envelope.
//! - `GET https://api.z.ai/api/biz/subscription/list` for the plan's product name ("GLM Coding
//!   Pro") and the day it next renews (`nextRenewTime`), looked up twice a day and never required;
//!   BigModel keys show the quota's `level`.
//!
//! The quota's `limits` carry the Session (5-hour) and Weekly windows as `CREDIT_LIMIT` percentages
//! (`TOKENS_LIMIT` on older plans) and the monthly web-search, web-reader and Zread calls as a
//! `TIME_LIMIT` count.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use uc_core::{
    HttpRequest, HttpResponse, MetricLine, PlanTerm, Provider, ProviderLink, SessionStartSignal,
    SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Zai;

const NAME: &str = "Z.ai";
/// The global host, then the mainland-China (BigModel) host a key may belong to instead.
const HOSTS: [&str; 2] = ["https://api.z.ai", "https://open.bigmodel.cn"];
const QUOTA_PATH: &str = "/api/monitor/usage/quota/limit";
const SUBSCRIPTIONS: &str = "https://api.z.ai/api/biz/subscription/list";
const HOST_MEMO: &str = "zai.host";
const PLAN_MEMO: &str = "zai.plan";
const REFUSED: &str = "Z.ai refused this API key. Check the key or create a new one.";
const NO_PLAN: &str =
    "This key's account has no active GLM Coding Plan, so there is no quota to show.";

#[async_trait]
impl Service for Zai {
    fn id(&self) -> &'static str {
        "zai"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new(
                "Dashboard",
                "https://z.ai/manage-apikey/coding-plan/personal/my-plan",
            ),
            ProviderLink::new("API Keys", "https://z.ai/manage-apikey/apikey-list"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[
                "ZAI_API_KEY",
                "Z_AI_API_KEY",
                "GLM_API_KEY",
                "ZHIPUAI_API_KEY",
            ],
            url: "https://z.ai/manage-apikey/apikey-list",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            // Z.ai states no reset for a 5-hour window that has not started yet.
            WidgetDescriptor::percent(
                format!("{}.session", provider.id),
                provider,
                "Session",
                None,
                Some(SessionStartSignal::MissingResetDate),
            )
            .exporting_progress("session", "percent"),
            WidgetDescriptor::percent(
                format!("{}.weekly", provider.id),
                provider,
                "Weekly",
                None,
                None,
            )
            .exporting_progress("weekly", "percent"),
            WidgetDescriptor::bounded_count(
                format!("{}.webSearches", provider.id),
                provider,
                "Web Searches",
                None,
                1000.0,
                "searches",
                Some(lines::MONTH_MS),
            )
            .exporting_progress("webSearches", "searches"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context.secret.key().ok_or_else(|| {
            http::invalid("The saved Z.ai key is empty. Add the key again in Accounts.")
        })?;
        let (host, body) = quota(context, key).await?;
        let data = container(&body)?;
        let meters = read_limits(data, context.now)?;
        let level = value::text(data, "/level").and_then(lines::plan_name);
        let subscription = if host == HOSTS[0] {
            subscription(context, key).await
        } else {
            Subscription::default()
        };
        let term = subscription.renews_at.map(|ends_at| PlanTerm::Stated {
            ends_at,
            checked_at: subscription.checked_at,
        });
        Ok(Reading::new(subscription.name.or(level), meters)
            .with_plan_checked_at(subscription.checked_at)
            .with_plan_term(term))
    }
}

/// How a host answered a quota request.
enum Answer {
    Quota(Value),
    /// The host does not take the key; it may belong to the other host.
    Refused,
    /// The key works, but its account has no GLM Coding Plan.
    NoPlan,
}

/// The quota and the host that answered it. The remembered host is asked first, else the global
/// host, and a host that refuses the key hands over to the other one. Only a refusal from every
/// host, or from one while the other cannot be reached, rejects the key.
async fn quota(
    context: &FetchContext<'_>,
    key: &str,
) -> Result<(&'static str, Value), SimpleProviderError> {
    let remembered = context
        .memo
        .get(HOST_MEMO, context.now)
        .await
        .and_then(|host| {
            HOSTS
                .into_iter()
                .find(|known| host.as_str() == Some(*known))
        });
    let mut refused = false;
    for host in order(remembered) {
        let request = get(&format!("{host}{QUOTA_PATH}"), key);
        let response = match http::send(context.http, request, NAME).await {
            Ok(response) => response,
            // A host that cannot be reached says nothing about a key the other host refused.
            Err(_) if refused => break,
            Err(error) => return Err(error),
        };
        match answer(&response)? {
            Answer::Refused => refused = true,
            Answer::NoPlan => {
                remember(context, host).await;
                return Err(http::not_available(NO_PLAN));
            }
            Answer::Quota(body) => {
                remember(context, host).await;
                return Ok((host, body));
            }
        }
    }
    context.memo.remove(HOST_MEMO).await;
    Err(http::invalid(REFUSED))
}

/// The hosts in the order a refresh asks them: the remembered one first, else the global one.
fn order(remembered: Option<&'static str>) -> [&'static str; 2] {
    let [global, china] = HOSTS;
    if remembered == Some(china) {
        [china, global]
    } else {
        [global, china]
    }
}

async fn remember(context: &FetchContext<'_>, host: &str) {
    context
        .memo
        .put(
            HOST_MEMO,
            Value::String(host.to_string()),
            Some(context.now + Duration::hours(12)),
        )
        .await;
}

/// A quota response classified. Z.ai reports failures either as an HTTP status or as an HTTP 200
/// envelope whose `code` carries the status or one of its API error codes.
fn answer(response: &HttpResponse) -> Result<Answer, SimpleProviderError> {
    if matches!(response.status, 401 | 403) {
        return Ok(Answer::Refused);
    }
    if !response.is_success() {
        return Err(http::status_error(response, NAME));
    }
    let body = http::parse(response, NAME)?;
    if !is_failure(&body) {
        return Ok(Answer::Quota(body));
    }
    // Z.ai says "当前用户不存在coding plan" (this user has no coding plan), or that the GLM Coding
    // Plan package expired; the phrase is ASCII either way.
    let message = value::text(&body, "/msg")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if message.contains("coding plan") {
        return Ok(Answer::NoPlan);
    }
    match value::number(&body, "/code").map(|code| code as i64) {
        // 1000-1005: authentication failed, missing, invalid or expired, or two-factor
        // authentication required (all HTTP 401 in Z.ai's error code table).
        Some(401 | 403 | 1000..=1005) => Ok(Answer::Refused),
        // 1302: rate limit reached; 1305: temporarily overloaded.
        Some(429 | 1302 | 1305) => Err(http::status_error(
            &HttpResponse {
                status: 429,
                ..response.clone()
            },
            NAME,
        )),
        _ => Err(http::decoding(NAME)),
    }
}

/// Whether a 2xx body is Z.ai's failure envelope rather than a quota. A `"data": null` beside an
/// error `code` carries no quota.
fn is_failure(body: &Value) -> bool {
    if value::flag(body, "/success") == Some(false) {
        return true;
    }
    let present = |field: &str| body.get(field).is_some_and(|found| !found.is_null());
    let carries_quota = present("data") || present("limits");
    !carries_quota
        && value::number(body, "/code").is_some_and(|code| !matches!(code as i64, 0 | 200))
}

/// The object holding `limits` and `level`: `data`, or the body itself in older answers.
fn container(body: &Value) -> Result<&Value, SimpleProviderError> {
    match body.get("data") {
        Some(data) if data.is_object() => Ok(data),
        Some(_) => Err(http::decoding(NAME)),
        None => Ok(body),
    }
}

/// A percentage window: used percent, stated reset and length in milliseconds.
struct Window {
    used: f64,
    resets_at: Option<DateTime<Utc>>,
    period: i64,
}

/// Session and Weekly from the percentage windows (the most-used one of each, since it runs out
/// first) and Web Searches from the first `TIME_LIMIT`. A missing value is a decoding error, never
/// zero usage; an empty list, or one with only unknown entries, leaves the "no usage data" status.
fn read_limits(data: &Value, now: DateTime<Utc>) -> Result<Vec<MetricLine>, SimpleProviderError> {
    let limits = data
        .get("limits")
        .and_then(Value::as_array)
        .ok_or_else(|| http::decoding(NAME))?;
    let mut session = None;
    let mut weekly = None;
    let mut searches = None;
    for entry in limits {
        match value::text(entry, "/type").or_else(|| value::text(entry, "/name")) {
            Some("CREDIT_LIMIT" | "TOKENS_LIMIT") => {
                let Some(period) = window_ms(entry)? else {
                    continue;
                };
                let used = used_percent(entry)?;
                let resets_at = value::time(entry, "/nextResetTime");
                let (slot, resets_at) = if period < lines::DAY_MS {
                    (&mut session, plausible_reset(resets_at, period, now))
                } else {
                    (&mut weekly, resets_at)
                };
                keep_tightest(
                    slot,
                    Window {
                        used,
                        resets_at,
                        period,
                    },
                );
            }
            Some("TIME_LIMIT") if searches.is_none() => searches = Some(web_searches(entry)?),
            _ => {}
        }
    }
    let mut meters: Vec<MetricLine> = [("Session", session), ("Weekly", weekly)]
        .into_iter()
        .filter_map(|(label, window)| {
            window.map(|window| {
                lines::percent(label, window.used, window.resets_at, Some(window.period))
            })
        })
        .collect();
    meters.extend(searches);
    let mut resources = std::collections::BTreeMap::<String, f64>::new();
    for entry in limits {
        for detail in entry
            .get("usageDetails")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(model), Some(used)) = (
                value::text(detail, "/modelCode"),
                value::number(detail, "/usage").filter(|used| *used >= 0.0),
            ) {
                let label = match model {
                    "search-prime" => "Search Prime",
                    "web-reader" => "Web Reader",
                    "zread" => "Zread",
                    other => other,
                };
                *resources.entry(label.to_string()).or_default() += used;
            }
        }
    }
    meters.extend(
        resources
            .into_iter()
            .map(|(label, used)| lines::count_value(&label, used, "calls")),
    );
    MetricLine::append_no_data_if_needed(&mut meters);
    Ok(meters)
}

/// A percentage window's length from Z.ai's (`unit`, `number`) pair, with the unit codes OpenUsage
/// inferred from Z.ai's answers: 3 hours, 4 days, 6 weeks, 5 months of 30 days. `None` for a unit
/// this version does not know, so a new kind of window cannot hide the known ones.
fn window_ms(entry: &Value) -> Result<Option<i64>, SimpleProviderError> {
    let unit = value::number(entry, "/unit");
    let number = value::number(entry, "/number").filter(|number| *number > 0.0);
    let (Some(unit), Some(number)) = (unit, number) else {
        return Err(http::decoding(NAME));
    };
    let unit_ms = match (unit.fract() == 0.0).then_some(unit as i64) {
        Some(3) => lines::HOUR_MS,
        Some(4) => lines::DAY_MS,
        Some(6) => lines::WEEK_MS,
        Some(5) => lines::MONTH_MS,
        _ => return Ok(None),
    };
    let length = unit_ms as f64 * number;
    if !(1.0..i64::MAX as f64).contains(&length) {
        return Err(http::decoding(NAME));
    }
    Ok(Some(length as i64))
}

/// An entry's used percent: Z.ai's own `percentage`, else `currentValue` of `usage`.
fn used_percent(entry: &Value) -> Result<f64, SimpleProviderError> {
    if let Some(percentage) = value::number(entry, "/percentage") {
        return Ok(lines::clamp_percent(percentage));
    }
    let used = value::number(entry, "/currentValue");
    let limit = value::number(entry, "/usage").filter(|limit| *limit > 0.0);
    match (used, limit) {
        (Some(used), Some(limit)) => Ok(lines::clamp_percent(used / limit * 100.0)),
        _ => Err(http::decoding(NAME)),
    }
}

/// A sub-daily window's reset, unless it lies more than one window (plus a minute of clock skew)
/// ahead. Z.ai has been seen stating 5-hour resets hours too late (CodexBar drops them too); such
/// a reset is dropped, never corrected.
fn plausible_reset(
    resets_at: Option<DateTime<Utc>>,
    period_ms: i64,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    resets_at.filter(|reset| *reset <= now + Duration::milliseconds(period_ms + 60_000))
}

/// Keeps the more-used of two windows that feed one meter, the first one on a tie.
fn keep_tightest(slot: &mut Option<Window>, window: Window) {
    if slot
        .as_ref()
        .is_none_or(|current| window.used > current.used)
    {
        *slot = Some(window);
    }
}

/// The monthly web-search, web-reader and Zread calls: `currentValue` of `usage`.
fn web_searches(entry: &Value) -> Result<MetricLine, SimpleProviderError> {
    let used = value::number(entry, "/currentValue").filter(|used| *used >= 0.0);
    let limit = value::number(entry, "/usage").filter(|limit| *limit >= 0.0);
    let (Some(used), Some(limit)) = (used, limit) else {
        return Err(http::decoding(NAME));
    };
    Ok(lines::count(
        "Web Searches",
        used,
        limit,
        "searches",
        value::time(entry, "/nextResetTime"),
        Some(lines::MONTH_MS),
    ))
}

/// The subscription's product name ("GLM Coding Pro"), best effort: an answer stands for 12 hours,
/// a failed lookup is retried after an hour, and neither fails the card.
async fn subscription(context: &FetchContext<'_>, key: &str) -> Subscription {
    // An older memo held only the name as a string; it is looked up again.
    if let Some(remembered) = context
        .memo
        .get(PLAN_MEMO, context.now)
        .await
        .filter(|memo| {
            memo.is_object()
                && (value::time(memo, "/renewsAt").is_none()
                    || value::time(memo, "/checkedAt").is_some())
        })
    {
        return Subscription {
            name: value::text(&remembered, "/name").map(str::to_string),
            renews_at: value::time(&remembered, "/renewsAt"),
            checked_at: value::time(&remembered, "/checkedAt"),
        };
    }
    let list = match http::send(context.http, get(SUBSCRIPTIONS, key), NAME).await {
        Ok(response) if response.is_success() => http::parse(&response, NAME)
            .ok()
            .and_then(|body| body.get("data").and_then(Value::as_array).cloned()),
        _ => None,
    };
    let entry = list.as_deref().and_then(current_subscription);
    let found = Subscription {
        name: entry
            .and_then(|entry| value::text(entry, "/productName"))
            .map(str::to_string),
        renews_at: entry
            .filter(|entry| current_period(entry))
            .and_then(|entry| value::time(entry, "/nextRenewTime")),
        checked_at: list.as_ref().map(|_| context.now),
    };
    let recheck = if list.is_some() {
        Duration::hours(12)
    } else {
        Duration::hours(1)
    };
    context
        .memo
        .put(
            PLAN_MEMO,
            serde_json::json!({
                "name": found.name,
                "renewsAt": found.renews_at.map(|time| time.to_rfc3339()),
                "checkedAt": found.checked_at.map(|time| time.to_rfc3339()),
            }),
            Some(
                found
                    .renews_at
                    .filter(|end| *end > context.now)
                    .map_or(context.now + recheck, |end| end.min(context.now + recheck)),
            ),
        )
        .await;
    found
}

/// The product name of a Z.ai subscription and the day it next renews (`nextRenewTime`, a date).
#[derive(Default)]
struct Subscription {
    name: Option<String>,
    renews_at: Option<DateTime<Utc>>,
    checked_at: Option<DateTime<Utc>>,
}

fn current_period(entry: &Value) -> bool {
    if value::flag(entry, "/inCurrentPeriod") == Some(false) {
        return false;
    }
    match value::text(entry, "/status")
        .map(str::to_ascii_uppercase)
        .as_deref()
    {
        Some("VALID") => true,
        Some("EXPIRED" | "CANCELED" | "CANCELLED" | "INVALID" | "INACTIVE") => false,
        _ => value::flag(entry, "/inCurrentPeriod") == Some(true),
    }
}

/// The named subscription in its current period, else the first named one listed.
fn current_subscription(list: &[Value]) -> Option<&Value> {
    let named = || {
        list.iter()
            .filter(|entry| value::text(entry, "/productName").is_some())
    };
    let current = |entry: &&Value| current_period(entry);
    named().find(current).or_else(|| named().next())
}

fn get(url: &str, key: &str) -> HttpRequest {
    HttpRequest::get(url)
        .bearer(key)
        .header("Accept", "application/json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Memo, Secret};
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use serde_json::json;
    use std::sync::Arc;
    use uc_core::{ErrorCategory, HttpClient, HttpError, SharedHttpClient};

    const GLOBAL_QUOTA: &str = "https://api.z.ai/api/monitor/usage/quota/limit";
    const CHINA_QUOTA: &str = "https://open.bigmodel.cn/api/monitor/usage/quota/limit";

    /// A GLM Coding Pro plan captured on 2026-06-29 (upstream OpenUsage's anonymized fixture).
    const PRO_QUOTA: &str = r#"{"code":200,"msg":"Operation successful","data":{"limits":[
        {"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":17,"nextResetTime":1782724971179},
        {"type":"TOKENS_LIMIT","unit":6,"number":1,"percentage":3,"nextResetTime":1783305486997},
        {"type":"TIME_LIMIT","unit":5,"number":1,"usage":1000,"currentValue":0,"remaining":1000,"percentage":0,"nextResetTime":1785292686976,"usageDetails":[{"modelCode":"search-prime","usage":0},{"modelCode":"web-reader","usage":0},{"modelCode":"zread","usage":0}]}
    ],"level":"pro"},"success":true}"#;
    const PRO_SUBSCRIPTIONS: &str = r#"{"code":200,"msg":"Operation successful","data":[{"productName":"GLM Coding Pro","status":"VALID","nextRenewTime":"2026-07-29","billingCycle":"monthly","inCurrentPeriod":true}],"success":true}"#;
    /// A GLM Coding Lite plan captured on 2026-08-13: `CREDIT_LIMIT` windows, and no reset for the
    /// 5-hour window that has not started.
    const LITE_QUOTA: &str = r#"{"code":200,"msg":"Operation successful","data":{"limits":[
        {"type":"CREDIT_LIMIT","unit":3,"number":5,"usage":2000,"currentValue":0,"remaining":2000,"percentage":0},
        {"type":"CREDIT_LIMIT","unit":6,"number":1,"usage":10000,"currentValue":9855,"remaining":145,"percentage":98,"nextResetTime":1786685679998}
    ],"level":"lite"},"success":true}"#;
    /// Z.ai's answer to a key it does not know: HTTP 200 with a failure envelope.
    const REFUSED_ENVELOPE: &str = r#"{"code":1000,"msg":"Authentication Failed","success":false}"#;

    fn secret() -> Value {
        json!({"apiKey": "zai-test-key"})
    }

    fn at(millis: i64) -> Option<DateTime<Utc>> {
        Utc.timestamp_millis_opt(millis).single()
    }

    fn june() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 29, 7, 0, 0).unwrap()
    }

    fn august() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 13, 12, 0, 0).unwrap()
    }

    fn urls(http: &Scripted) -> Vec<String> {
        http.requests()
            .into_iter()
            .map(|request| request.url)
            .collect()
    }

    /// A client that cannot reach `host` and answers every other request from `rest`.
    struct Unreachable {
        host: &'static str,
        rest: Scripted,
    }

    #[async_trait]
    impl HttpClient for Unreachable {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            if request.url.starts_with(self.host) {
                return Err(HttpError::Timeout);
            }
            self.rest.send(request).await
        }
    }

    async fn fetch_without(
        host: &'static str,
        rest: &Scripted,
    ) -> Result<Reading, SimpleProviderError> {
        let http: SharedHttpClient = Arc::new(Unreachable {
            host,
            rest: rest.clone(),
        });
        let saved = Secret::new(secret());
        let memo = Memo::default();
        let context = FetchContext {
            secret: &saved,
            http: &http,
            now: june(),
            memo: &memo,
        };
        Zai.fetch(&context).await
    }

    #[tokio::test]
    async fn reads_session_weekly_web_searches_and_the_product_name() {
        let http = Scripted::new().on("GET", GLOBAL_QUOTA, 200, PRO_QUOTA).on(
            "GET",
            SUBSCRIPTIONS,
            200,
            PRO_SUBSCRIPTIONS,
        );
        let scope = context_at(&http, secret(), june());
        let reading = Zai.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("GLM Coding Pro"));
        assert_eq!(
            reading.plan_term,
            Some(PlanTerm::Stated {
                ends_at: Utc.with_ymd_and_hms(2026, 7, 29, 0, 0, 0).unwrap(),
                checked_at: Some(june()),
            })
        );
        assert_eq!(
            reading.lines,
            vec![
                lines::percent("Session", 17.0, at(1782724971179), Some(5 * lines::HOUR_MS)),
                lines::percent("Weekly", 3.0, at(1783305486997), Some(lines::WEEK_MS)),
                lines::count(
                    "Web Searches",
                    0.0,
                    1000.0,
                    "searches",
                    at(1785292686976),
                    Some(lines::MONTH_MS)
                ),
                lines::count_value("Search Prime", 0.0, "calls"),
                lines::count_value("Web Reader", 0.0, "calls"),
                lines::count_value("Zread", 0.0, "calls"),
            ]
        );
        let requests = http.requests();
        assert_eq!(urls(&http), [GLOBAL_QUOTA, SUBSCRIPTIONS]);
        for request in &requests {
            assert_eq!(request.method, "GET");
            assert_eq!(
                header(request, "Authorization"),
                Some("Bearer zai-test-key")
            );
            assert_eq!(header(request, "Accept"), Some("application/json"));
            assert_eq!(request.body, None);
        }
    }

    #[tokio::test]
    async fn remembered_periods_keep_the_original_confirmation_and_recheck_at_the_boundary() {
        let first_end = june() + Duration::minutes(30);
        let renewed_end = first_end + Duration::days(30);
        let answer = |end: DateTime<Utc>| {
            json!({"data": [{
                "productName": "GLM Coding Pro", "status": "VALID", "inCurrentPeriod": true,
                "nextRenewTime": end.to_rfc3339()
            }]})
            .to_string()
        };
        let http = Scripted::new()
            .on("GET", GLOBAL_QUOTA, 200, PRO_QUOTA)
            .on("GET", SUBSCRIPTIONS, 200, &answer(first_end))
            .on("GET", SUBSCRIPTIONS, 200, &answer(renewed_end));
        let scope = context_at(&http, secret(), june());
        let first = Zai.fetch(&scope.context()).await.unwrap();
        let mut later = scope.context();
        later.now += Duration::minutes(15);
        let cached = Zai.fetch(&later).await.unwrap();
        assert_eq!(cached.plan_term, first.plan_term);
        assert_eq!(cached.plan_checked_at, Some(june()));
        assert_eq!(http.requests().len(), 3);

        later.now = first_end;
        let renewed = Zai.fetch(&later).await.unwrap();
        assert_eq!(renewed.plan_checked_at, Some(first_end));
        assert_eq!(
            renewed.plan_term,
            Some(PlanTerm::Stated {
                ends_at: renewed_end,
                checked_at: Some(first_end)
            })
        );
        assert_eq!(http.requests().len(), 5);
    }

    #[tokio::test]
    async fn inactive_or_unconfirmed_periods_keep_usage_without_a_paid_date() {
        for (status, in_current_period) in [
            (Some("EXPIRED"), Some(true)),
            (Some("CANCELED"), Some(true)),
            (Some("INVALID"), Some(true)),
            (Some("VALID"), Some(false)),
            (None, None),
        ] {
            let answer = json!({"data": [{
                "productName": "GLM Coding Pro", "status": status,
                "inCurrentPeriod": in_current_period,
                "nextRenewTime": "2026-07-29"
            }]})
            .to_string();
            let http = Scripted::new().on("GET", GLOBAL_QUOTA, 200, PRO_QUOTA).on(
                "GET",
                SUBSCRIPTIONS,
                200,
                &answer,
            );
            let scope = context_at(&http, secret(), june());
            let reading = Zai.fetch(&scope.context()).await.unwrap();
            assert_eq!(reading.plan_term, None, "{status:?} {in_current_period:?}");
            assert_eq!(reading.plan_checked_at, Some(june()));
            assert_eq!(reading.lines.len(), 6);
        }
    }

    #[tokio::test]
    async fn a_remembered_date_without_a_confirmation_is_looked_up_again() {
        let http = Scripted::new().on("GET", GLOBAL_QUOTA, 200, PRO_QUOTA).on(
            "GET",
            SUBSCRIPTIONS,
            200,
            PRO_SUBSCRIPTIONS,
        );
        let scope = context_at(&http, secret(), june());
        scope
            .context()
            .memo
            .put(
                PLAN_MEMO,
                json!({
                    "name": "GLM Coding Lite", "renewsAt": "2026-07-10"
                }),
                Some(june() + Duration::hours(12)),
            )
            .await;
        let reading = Zai.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("GLM Coding Pro"));
        assert_eq!(reading.plan_checked_at, Some(june()));
        assert_eq!(
            reading.plan_term,
            Some(PlanTerm::Stated {
                ends_at: Utc.with_ymd_and_hms(2026, 7, 29, 0, 0, 0).unwrap(),
                checked_at: Some(june())
            })
        );
        assert_eq!(urls(&http), [GLOBAL_QUOTA, SUBSCRIPTIONS]);
    }

    #[tokio::test]
    async fn credit_limits_map_and_a_failed_plan_lookup_falls_back_to_the_level() {
        let http = Scripted::new().on("GET", GLOBAL_QUOTA, 200, LITE_QUOTA).on(
            "GET",
            SUBSCRIPTIONS,
            500,
            "{}",
        );
        let scope = context_at(&http, secret(), august());
        let reading = Zai.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Lite"));
        assert_eq!(
            reading.lines,
            vec![
                lines::percent("Session", 0.0, None, Some(5 * lines::HOUR_MS)),
                lines::percent("Weekly", 98.0, at(1786685679998), Some(lines::WEEK_MS)),
            ]
        );
        let again = Zai.fetch(&scope.context()).await.unwrap();
        assert_eq!(again.plan.as_deref(), Some("Lite"));
        assert_eq!(urls(&http), [GLOBAL_QUOTA, SUBSCRIPTIONS, GLOBAL_QUOTA]);
    }

    #[tokio::test]
    async fn a_key_refused_by_api_z_ai_is_read_from_open_bigmodel_cn_and_remembered() {
        let http = Scripted::new()
            .on("GET", GLOBAL_QUOTA, 200, REFUSED_ENVELOPE)
            .on("GET", CHINA_QUOTA, 200, LITE_QUOTA);
        let scope = context_at(&http, secret(), august());
        let reading = Zai.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Lite"));
        assert_eq!(
            reading.lines[1],
            lines::percent("Weekly", 98.0, at(1786685679998), Some(lines::WEEK_MS))
        );
        Zai.fetch(&scope.context()).await.unwrap();
        assert_eq!(urls(&http), [GLOBAL_QUOTA, CHINA_QUOTA, CHINA_QUOTA]);
        assert_eq!(
            header(&http.requests()[1], "Authorization"),
            Some("Bearer zai-test-key")
        );
    }

    #[tokio::test]
    async fn a_key_both_hosts_refuse_is_rejected() {
        for (status, body) in [
            (401, r#"{"error":"token expired or incorrect"}"#),
            (403, "{}"),
            (200, REFUSED_ENVELOPE),
            // A refusal code with `data: null` and no `success` flag is an envelope, not a quota.
            (
                200,
                r#"{"code":1005,"msg":"Need Two-Factor Authentication","data":null}"#,
            ),
        ] {
            let http = Scripted::new().on("GET", GLOBAL_QUOTA, status, body).on(
                "GET",
                CHINA_QUOTA,
                200,
                REFUSED_ENVELOPE,
            );
            let scope = context_at(&http, secret(), june());
            let error = Zai.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(
                error.category,
                ErrorCategory::AuthInvalid,
                "{status} {body}"
            );
            assert_eq!(error.message, REFUSED, "{status} {body}");
            assert_eq!(urls(&http), [GLOBAL_QUOTA, CHINA_QUOTA], "{status} {body}");
        }
    }

    #[tokio::test]
    async fn a_remembered_host_that_refuses_the_key_hands_over_to_the_other_host() {
        let http = Scripted::new()
            .on("GET", GLOBAL_QUOTA, 200, REFUSED_ENVELOPE)
            .on("GET", GLOBAL_QUOTA, 200, PRO_QUOTA)
            .on("GET", CHINA_QUOTA, 200, LITE_QUOTA)
            .on("GET", CHINA_QUOTA, 403, "{}")
            .on("GET", SUBSCRIPTIONS, 200, PRO_SUBSCRIPTIONS);
        let scope = context_at(&http, secret(), june());
        let china = Zai.fetch(&scope.context()).await.unwrap();
        assert_eq!(china.plan.as_deref(), Some("Lite"));
        let global = Zai.fetch(&scope.context()).await.unwrap();
        assert_eq!(global.plan.as_deref(), Some("GLM Coding Pro"));
        Zai.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            urls(&http),
            [
                GLOBAL_QUOTA,
                CHINA_QUOTA,
                CHINA_QUOTA,
                GLOBAL_QUOTA,
                SUBSCRIPTIONS,
                GLOBAL_QUOTA
            ]
        );
    }

    #[tokio::test]
    async fn a_key_every_host_refuses_is_rejected_and_its_host_forgotten() {
        let http = Scripted::new()
            .on("GET", GLOBAL_QUOTA, 200, REFUSED_ENVELOPE)
            .on("GET", CHINA_QUOTA, 200, LITE_QUOTA)
            .on("GET", CHINA_QUOTA, 401, "{}");
        let scope = context_at(&http, secret(), august());
        Zai.fetch(&scope.context()).await.unwrap();
        for _ in 0..2 {
            let error = Zai.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthInvalid);
            assert_eq!(error.message, REFUSED);
        }
        // The China host goes first while it is remembered, the global host once it is forgotten.
        assert_eq!(
            urls(&http),
            [
                GLOBAL_QUOTA,
                CHINA_QUOTA,
                CHINA_QUOTA,
                GLOBAL_QUOTA,
                GLOBAL_QUOTA,
                CHINA_QUOTA
            ]
        );
    }

    #[tokio::test]
    async fn an_unreachable_china_host_leaves_the_global_refusal_standing() {
        let rest = Scripted::new().on("GET", GLOBAL_QUOTA, 401, "{}");
        let error = fetch_without("https://open.bigmodel.cn", &rest)
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(error.message, REFUSED);
        assert_eq!(urls(&rest), [GLOBAL_QUOTA]);
    }

    #[tokio::test]
    async fn an_unreachable_global_host_is_a_network_error_and_china_is_not_tried() {
        let rest = Scripted::new().on("GET", CHINA_QUOTA, 200, LITE_QUOTA);
        let error = fetch_without("https://api.z.ai", &rest).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Network);
        assert!(rest.requests().is_empty());
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_are_reported_without_trying_the_china_host() {
        for (status, body, category) in [
            (429, "{}", ErrorCategory::RateLimited),
            (
                200,
                r#"{"code":1302,"msg":"Rate limit reached for requests","success":false}"#,
                ErrorCategory::RateLimited,
            ),
            (
                200,
                r#"{"code":1305,"msg":"The service may be temporarily overloaded, please try again later","success":false}"#,
                ErrorCategory::RateLimited,
            ),
            (502, "Bad Gateway", ErrorCategory::Http5xx),
        ] {
            let http = Scripted::new().on("GET", GLOBAL_QUOTA, status, body);
            let scope = context_at(&http, secret(), june());
            let error = Zai.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category, "{status} {body}");
            assert_eq!(urls(&http), [GLOBAL_QUOTA], "{status} {body}");
        }
    }

    #[tokio::test]
    async fn an_account_without_a_coding_plan_is_not_available() {
        for body in [
            r#"{"code":500,"msg":"当前用户不存在coding plan","success":false}"#,
            r#"{"code":1309,"msg":"Your GLM Coding Plan package has expired","success":false}"#,
        ] {
            let http = Scripted::new().on("GET", GLOBAL_QUOTA, 200, body);
            let scope = context_at(&http, secret(), june());
            let error = Zai.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::NotAvailable, "{body}");
            assert_eq!(error.message, NO_PLAN, "{body}");
            assert_eq!(urls(&http), [GLOBAL_QUOTA], "{body}");
        }
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({"apiKey": "  "}), june());
        let error = Zai.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn missing_values_and_broken_envelopes_are_decoding_errors_never_zero_usage() {
        for body in [
            r#"{"data":{"limits":[{"type":"TOKENS_LIMIT","unit":3,"number":5}]}}"#,
            r#"{"data":{"limits":[{"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":true}]}}"#,
            r#"{"data":{"limits":[{"type":"CREDIT_LIMIT","percentage":40}]}}"#,
            r#"{"data":{"limits":[{"type":"CREDIT_LIMIT","unit":3,"number":0,"percentage":40}]}}"#,
            r#"{"data":{"limits":[{"type":"TIME_LIMIT","usage":1000}]}}"#,
            r#"{"data":{"limits":[{"type":"TIME_LIMIT","currentValue":10}]}}"#,
            r#"{"data":{"limits":[{"type":"TIME_LIMIT","currentValue":-1,"usage":1000}]}}"#,
            r#"{"code":500,"msg":"internal error","success":false}"#,
            "not-json",
            r#"{"data":[]}"#,
            r#"{"data":{}}"#,
            r#"{"data":{"limits":{}}}"#,
        ] {
            let http = Scripted::new().on("GET", GLOBAL_QUOTA, 200, body);
            let scope = context_at(&http, secret(), june());
            let error = Zai.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
        }
    }

    #[test]
    fn windows_follow_the_payload_and_unknown_entries_do_not_hide_known_ones() {
        let data = json!({"limits": [
            {"type": "FUTURE_LIMIT"},
            {"type": "TOKENS_LIMIT", "unit": 99, "number": 1, "percentage": 70},
            {"type": "TOKENS_LIMIT", "unit": 3, "number": 3, "percentage": 10},
            {"type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 25},
            {"name": "CREDIT_LIMIT", "unit": 4, "number": 3, "percentage": 150}
        ]});
        assert_eq!(
            read_limits(&data, june()).unwrap(),
            vec![
                lines::percent("Session", 25.0, None, Some(5 * lines::HOUR_MS)),
                lines::percent("Weekly", 100.0, None, Some(3 * lines::DAY_MS)),
            ]
        );
    }

    #[test]
    fn an_empty_or_unrecognised_list_shows_no_usage_data() {
        for data in [
            json!({"limits": []}),
            json!({"limits": [
                {"type": "FUTURE_LIMIT"},
                {"type": "TOKENS_LIMIT", "unit": 99, "number": 1, "percentage": 70}
            ]}),
        ] {
            assert_eq!(
                read_limits(&data, june()).unwrap(),
                vec![MetricLine::no_usage_data()],
                "{data}"
            );
        }
        let legacy =
            json!({"limits": [{"type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 15}]});
        assert_eq!(container(&legacy).unwrap(), &legacy);
    }

    #[test]
    fn a_five_hour_reset_more_than_one_window_ahead_is_dropped() {
        let now = june();
        let later = |seconds: i64| (now + Duration::seconds(seconds)).timestamp_millis();
        for (offset, kept) in [
            (-60, true),
            (3_600, true),
            (18_060, true),
            (18_061, false),
            (36_000, false),
        ] {
            let data = json!({"limits": [
                {"type": "CREDIT_LIMIT", "unit": 3, "number": 5, "percentage": 25, "nextResetTime": later(offset)},
                {"type": "CREDIT_LIMIT", "unit": 6, "number": 1, "percentage": 9, "nextResetTime": later(6 * 86_400)}
            ]});
            let reset = kept.then(|| now + Duration::seconds(offset));
            assert_eq!(
                read_limits(&data, now).unwrap(),
                vec![
                    lines::percent("Session", 25.0, reset, Some(5 * lines::HOUR_MS)),
                    lines::percent(
                        "Weekly",
                        9.0,
                        Some(now + Duration::days(6)),
                        Some(lines::WEEK_MS)
                    ),
                ],
                "{offset}"
            );
        }
    }

    fn subscription_name(list: &[Value]) -> Option<&str> {
        current_subscription(list).and_then(|entry| value::text(entry, "/productName"))
    }

    #[test]
    fn the_current_subscription_names_the_plan() {
        let list = json!([
            {"productName": "GLM Coding Lite", "status": "EXPIRED"},
            {"productName": "GLM Coding Max", "status": "VALID"}
        ]);
        assert_eq!(
            subscription_name(list.as_array().unwrap()),
            Some("GLM Coding Max")
        );
        assert_eq!(
            subscription_name(&[json!({"productName": "GLM Coding Pro"})]),
            Some("GLM Coding Pro")
        );
        assert_eq!(subscription_name(&[json!({"status": "VALID"})]), None);
    }

    #[test]
    fn declares_the_key_connection_and_three_widgets() {
        // The popup draws Z.ai's mark and brand tint by this family id.
        assert_eq!(Zai.id(), "zai");
        assert_eq!(Zai.name(), "Z.ai");
        let links: Vec<_> = Zai
            .links()
            .into_iter()
            .map(|link| (link.label, link.url))
            .collect();
        assert_eq!(
            links,
            [
                (
                    "Dashboard".to_string(),
                    "https://z.ai/manage-apikey/coding-plan/personal/my-plan".to_string()
                ),
                (
                    "API Keys".to_string(),
                    "https://z.ai/manage-apikey/apikey-list".to_string()
                ),
            ]
        );
        let help = Zai.connection().api_key.unwrap();
        assert_eq!(
            help.env,
            [
                "ZAI_API_KEY",
                "Z_AI_API_KEY",
                "GLM_API_KEY",
                "ZHIPUAI_API_KEY"
            ]
        );
        assert_eq!(help.url, "https://z.ai/manage-apikey/apikey-list");
        assert!(help.fields.is_empty());
        assert!(Zai.connection().login_from.is_none());
        let provider = Provider::new("zai", NAME);
        let descriptors = Zai.descriptors(&provider);
        let ids: Vec<_> = descriptors
            .iter()
            .map(|widget| widget.id.as_str())
            .collect();
        assert_eq!(ids, ["zai.session", "zai.weekly", "zai.webSearches"]);
        let labels: Vec<_> = descriptors
            .iter()
            .map(|widget| widget.metric_label.as_str())
            .collect();
        assert_eq!(labels, ["Session", "Weekly", "Web Searches"]);
        assert_eq!(
            descriptors[0].template.session_start_signal,
            Some(SessionStartSignal::MissingResetDate)
        );
        let exports: Vec<_> = descriptors
            .iter()
            .map(|widget| widget.limit_resources[0].key.as_str())
            .collect();
        assert_eq!(exports, ["session", "weekly", "webSearches"]);
    }
}
