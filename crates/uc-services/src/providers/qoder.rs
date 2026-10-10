//! Qoder: the credits of a Qoder account.
//!
//! Logins are read on Windows, where Electron's safeStorage key is unwrapped with DPAPI for this
//! user: the Qoder desktop app's `auth.v1.dat` in `%APPDATA%\com.qoder.app.stable` (the China
//! app's in `com.qodercn.app.stable`) and the Qoder IDE's `secret://aicoding.auth.userInfo` in
//! `%APPDATA%\Qoder\User\globalStorage\state.vscdb`. On macOS and Linux that key is kept in the
//! login keychain or the Secret Service, which this reader leaves alone, so there only a login
//! stored unsealed is found (same files under `~/Library/Application Support` or `~/.config`).
//! The apps renew their own tokens: an expired one asks to open Qoder, and nothing is written
//! back. A personal access token (`QODER_PERSONAL_ACCESS_TOKEN`, created at
//! qoder.com/account/integrations) works the same on every platform.
//!
//! Endpoints on `https://openapi.qoder.sh`, or on `https://openapi.qoder.com.cn` for accounts of
//! the China site (the host that takes the credential is remembered for 12 hours):
//! - `POST /api/v1/jobToken/exchange` turns a personal access token into a job token, remembered
//!   until shortly before it expires.
//! - `GET /api/v2/quota/usage`: the plan credits (`userQuota`, reset at `expiresAt`), add-on
//!   credits (`addOnQuota`) and an organization's shared credits (`orgResourcePackage`).
//! - `GET /api/v2/user/plan`: the plan's tier and paid period, looked up twice a day.

use std::path::Path;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{
    HttpRequest, HttpResponse, MetricLine, PlanTerm, Provider, ProviderLink, SimpleProviderError,
    WidgetDescriptor,
};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{apps, http, lines, sqlite, value};

pub(crate) struct Qoder;

const NAME: &str = "Qoder";
const GLOBAL: &str = "https://openapi.qoder.sh";
const CHINA: &str = "https://openapi.qoder.com.cn";
const EXCHANGE_PATH: &str = "/api/v1/jobToken/exchange";
const USAGE_PATH: &str = "/api/v2/quota/usage";
const PLAN_PATH: &str = "/api/v2/user/plan";
/// The client headers the Qoder CLI sends with a token exchange, the call personal access tokens
/// are made for.
const CLI_HEADERS: [(&str, &str); 2] = [("Cosy-ClientType", "5"), ("Cosy-Version", "1.0.0")];
const SITE_MEMO: &str = "qoder.site";
const JOB_MEMO: &str = "qoder.job";
const PLAN_MEMO: &str = "qoder.plan";
/// The desktop app's data folders, the origin each login shows and the host it belongs to when
/// that is known.
const DESKTOP_APPS: [(&str, &str, Option<&str>); 2] = [
    ("com.qoder.app.stable", "Qoder", None),
    ("com.qodercn.app.stable", "Qoder CN", Some(CHINA)),
];
const DESKTOP_AUTH: &str = "auth.v1.dat";
const IDE_FOLDER: &str = "Qoder";
const IDE_ORIGIN: &str = "Qoder IDE";
const IDE_SECRET: &str = "secret://aicoding.auth.userInfo";
/// Electron's safeStorage marker in front of a sealed value.
const SEALED: &[u8] = b"v10";
#[cfg(test)]
const NONCE_LEN: usize = 12;
const CREDITS: &str = "Credits";
const UNIT: &str = "credits";
/// Buckets shown beside the plan credits when they hold any.
const EXTRA_BUCKETS: [(&str, &str); 2] = [
    ("addOnQuota", "Add-on Credits"),
    ("orgResourcePackage", "Shared Credits"),
];
/// The Pro plan's monthly credits: a template number, never shown.
const TEMPLATE_CREDITS: f64 = 2000.0;
/// The user type of the free plan, whose `expiresAt` is no reset.
const FREE_USER_TYPE: &str = "personal_standard";
/// Qoder dates "never" in the year 9999.
const NEVER_YEAR: i32 = 9999;
const EXPIRED: &str = "The Qoder login expired. Open Qoder once to renew it.";
const REFUSED: &str =
    "Qoder refused this personal access token. Create a new one at qoder.com/account/integrations.";

#[async_trait]
impl Service for Qoder {
    fn id(&self) -> &'static str {
        "qoder"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Usage", "https://qoder.com/account/usage"),
            ProviderLink::new("Plans", "https://qoder.com/pricing"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::login(NAME).or_api_key(ApiKeyHelp {
            env: &["QODER_PERSONAL_ACCESS_TOKEN"],
            url: "https://qoder.com/account/integrations",
            fields: &[],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let mut logins = Vec::new();
        for (folder, origin, site) in DESKTOP_APPS {
            let dir = roots.app_data.join(folder);
            let path = dir.join(DESKTOP_AUTH);
            if let Some(stored) = read_small(&path)
                && let Some(document) = open(&stored, &dir.join("Local State"))
                && let Some(login) = login(&document, origin, &path, site)
            {
                logins.push(login);
            }
        }
        let database = apps::state_db(roots, IDE_FOLDER);
        let local_state = roots.app_data.join(IDE_FOLDER).join("Local State");
        if let Some(stored) = sqlite::item(&database, IDE_SECRET)
            && let Some(document) = open(stored.as_bytes(), &local_state)
            && let Some(login) = login(&document, IDE_ORIGIN, &database, None)
        {
            logins.push(login);
        }
        logins
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::bounded_count(
                format!("{}.credits", provider.id),
                provider,
                CREDITS,
                None,
                TEMPLATE_CREDITS,
                UNIT,
                Some(lines::MONTH_MS),
            )
            .exporting_progress("credits", UNIT),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let (site, bearer, body) = match context.secret.key() {
            Some(key) => personal_usage(context, &personal_token(key)).await?,
            None => login_usage(context).await?,
        };
        let meters = meters(&body, context.now)?;
        let (plan, term, checked_at) = plan(context, site, &bearer).await;
        Ok(Reading::new(plan, meters)
            .with_plan_term(term)
            .with_plan_checked_at(checked_at))
    }
}

/// How a host answered a usage request.
enum Answer {
    Usage(Value),
    /// The host does not take the token.
    Refused(HttpResponse),
}

/// The usage a saved login reads with its token as it is, first on the host the login belongs to.
async fn login_usage(
    context: &FetchContext<'_>,
) -> Result<(&'static str, String, Value), SimpleProviderError> {
    let secret = context.secret;
    let token = secret.str("/token").ok_or_else(|| http::expired(EXPIRED))?;
    if value::time(secret.value(), "/expiresAt").is_some_and(|expires| expires <= context.now) {
        return Err(http::expired(EXPIRED));
    }
    let first = (secret.str("/site") == Some(CHINA)).then_some(CHINA);
    for site in sites(context, first).await {
        if let Answer::Usage(body) = usage(context, site, token).await? {
            remember(context, site).await;
            return Ok((site, token.to_string(), body));
        }
    }
    context.memo.remove(SITE_MEMO).await;
    Err(http::expired(EXPIRED))
}

/// The usage a personal access token reads through a job token: the remembered one, else one from
/// an exchange. A remembered job token the host refuses is exchanged again once.
async fn personal_usage(
    context: &FetchContext<'_>,
    token: &str,
) -> Result<(&'static str, String, Value), SimpleProviderError> {
    let fingerprint = fingerprint(token);
    for site in sites(context, None).await {
        let mut remembered = remembered_job(context, site, &fingerprint).await;
        for _ in 0..2 {
            let (job, fresh) = match remembered.take() {
                Some(job) => (job, false),
                None => match exchange(context, site, token, &fingerprint).await? {
                    Some(job) => (job, true),
                    None => break,
                },
            };
            match usage(context, site, &job).await? {
                Answer::Usage(body) => {
                    remember(context, site).await;
                    return Ok((site, job, body));
                }
                Answer::Refused(response) => {
                    context.memo.remove(JOB_MEMO).await;
                    if fresh {
                        return Err(http::status_error(&response, NAME));
                    }
                }
            }
        }
    }
    context.memo.remove(SITE_MEMO).await;
    Err(http::invalid(REFUSED))
}

async fn usage(
    context: &FetchContext<'_>,
    site: &str,
    token: &str,
) -> Result<Answer, SimpleProviderError> {
    let response = http::send(context.http, get(site, USAGE_PATH, token), NAME).await?;
    if matches!(response.status, 401 | 403) {
        return Ok(Answer::Refused(response));
    }
    if !response.is_success() {
        return Err(http::status_error(&response, NAME));
    }
    Ok(Answer::Usage(http::parse(&response, NAME)?))
}

fn get(site: &str, path: &str, token: &str) -> HttpRequest {
    HttpRequest::get(format!("{site}{path}"))
        .bearer(token)
        .header("Accept", "application/json")
}

/// A job token from `site` for the personal access token, remembered until five minutes before it
/// expires and for a day at most; `None` when the host refuses the token.
async fn exchange(
    context: &FetchContext<'_>,
    site: &'static str,
    token: &str,
    fingerprint: &str,
) -> Result<Option<String>, SimpleProviderError> {
    let request = CLI_HEADERS.iter().fold(
        HttpRequest::post(format!("{site}{EXCHANGE_PATH}")).header("Accept", "application/json"),
        |request, (name, content)| request.header(*name, *content),
    );
    let response = http::send(
        context.http,
        request.json_body(&json!({ "personal_token": token })),
        NAME,
    )
    .await?;
    if matches!(response.status, 400 | 401 | 403) {
        return Ok(None);
    }
    if !response.is_success() {
        return Err(http::status_error(&response, NAME));
    }
    let body = http::parse(&response, NAME)?;
    let data = holder(&body, "token");
    let job = value::text(data, "/token")
        .ok_or_else(|| http::decoding(NAME))?
        .to_string();
    context
        .memo
        .put(
            JOB_MEMO,
            json!({ "for": fingerprint, "site": site, "token": job }),
            Some(job_expiry(data, context.now)),
        )
        .await;
    Ok(Some(job))
}

/// When a job token stops being reused: five minutes before the expiry the exchange states
/// (`expires_in` counts milliseconds above 10^6, seconds below), a day after issue at the latest.
fn job_expiry(data: &Value, now: DateTime<Utc>) -> DateTime<Utc> {
    let day = Duration::hours(24);
    let stated = value::time(data, "/expires_at")
        .or_else(|| value::time(data, "/expiresAt"))
        .or_else(|| {
            value::number(data, "/expires_in").map(|lifetime| {
                let seconds = if lifetime > 1e6 {
                    lifetime / 1000.0
                } else {
                    lifetime
                };
                let seconds = seconds.clamp(0.0, day.num_seconds() as f64);
                now + Duration::seconds(seconds as i64)
            })
        });
    stated.unwrap_or(now + day).min(now + day) - Duration::minutes(5)
}

async fn remembered_job(
    context: &FetchContext<'_>,
    site: &str,
    fingerprint: &str,
) -> Option<String> {
    let memo = context.memo.get(JOB_MEMO, context.now).await?;
    if memo["for"] != fingerprint || memo["site"] != site {
        return None;
    }
    memo["token"].as_str().map(str::to_string)
}

/// The hosts to try: the one remembered for this card, else `first` (the global host unless
/// given) and then the other.
async fn sites(context: &FetchContext<'_>, first: Option<&'static str>) -> Vec<&'static str> {
    let remembered = context.memo.get(SITE_MEMO, context.now).await;
    let remembered = remembered.as_ref().and_then(Value::as_str);
    if let Some(site) = [GLOBAL, CHINA]
        .into_iter()
        .find(|site| remembered == Some(*site))
    {
        return vec![site];
    }
    if first == Some(CHINA) {
        vec![CHINA, GLOBAL]
    } else {
        vec![GLOBAL, CHINA]
    }
}

async fn remember(context: &FetchContext<'_>, site: &str) {
    context
        .memo
        .put(
            SITE_MEMO,
            Value::String(site.to_string()),
            Some(context.now + Duration::hours(12)),
        )
        .await;
}

/// The plan's tier and paid period from `site`, looked up twice a day, or hourly while the lookup
/// fails; the card works without them.
async fn plan(
    context: &FetchContext<'_>,
    site: &str,
    token: &str,
) -> (Option<String>, Option<PlanTerm>, Option<DateTime<Utc>>) {
    let memo = match context
        .memo
        .get(PLAN_MEMO, context.now)
        .await
        .filter(|memo| {
            value::time(memo, "/endsAt").is_none() || value::time(memo, "/checkedAt").is_some()
        }) {
        Some(memo) => memo,
        None => {
            let answer = plan_lookup(context, site, token).await;
            let lasts = if answer.is_some() {
                Duration::hours(12)
            } else {
                Duration::hours(1)
            };
            let memo = answer
                .map(|body| plan_facts(&body, context.now))
                .unwrap_or_else(|| json!({}));
            context
                .memo
                .put(
                    PLAN_MEMO,
                    memo.clone(),
                    Some(
                        value::time(&memo, "/endsAt")
                            .filter(|end| *end > context.now)
                            .map_or(context.now + lasts, |end| end.min(context.now + lasts)),
                    ),
                )
                .await;
            memo
        }
    };
    let plan = value::text(&memo, "/plan").map(str::to_string);
    let term = value::time(&memo, "/endsAt")
        .filter(|ends| *ends > context.now)
        .map(|ends_at| PlanTerm::Stated {
            ends_at,
            checked_at: value::time(&memo, "/checkedAt"),
        });
    (plan, term, value::time(&memo, "/checkedAt"))
}

async fn plan_lookup(context: &FetchContext<'_>, site: &str, token: &str) -> Option<Value> {
    let response = http::send(context.http, get(site, PLAN_PATH, token), NAME)
        .await
        .ok()?;
    if !response.is_success() {
        return None;
    }
    http::parse(&response, NAME).ok()
}

/// What the card keeps of a plan answer: the tier's name and, for a paid plan, when its period
/// ends.
fn plan_facts(body: &Value, now: DateTime<Utc>) -> Value {
    let data = holder(body, "plan_tier_name");
    let plan = first_text(
        data,
        &[
            "/plan_tier_name",
            "/planTierName",
            "/tier_name",
            "/tierName",
        ],
    )
    .and_then(display_plan);
    let free = is_free(data)
        || plan
            .as_deref()
            .is_some_and(|plan| plan.eq_ignore_ascii_case("free"));
    let ends = first_time(data, &["/end_date", "/endDate"])
        .filter(|ends| !free && ends.year() < NEVER_YEAR);
    json!({
        "plan": plan,
        "endsAt": ends.map(|ends| ends.timestamp_millis()),
        "checkedAt": now.timestamp_millis(),
    })
}

/// A plan tier as Qoder writes it ("Pro+"), or a raw id made readable (`PRO_TRIAL` → `Pro Trial`).
fn display_plan(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let mixed_case = raw.chars().any(char::is_lowercase) && raw.chars().any(char::is_uppercase);
    if mixed_case && !raw.contains('_') {
        return Some(raw.to_string());
    }
    lines::plan_name(raw)
}

/// The plan credits, reset at `expiresAt` unless the plan is free or the date means "never", and
/// the add-on and shared credits when there are any.
fn meters(body: &Value, now: DateTime<Utc>) -> Result<Vec<MetricLine>, SimpleProviderError> {
    let data = holder(body, "userQuota");
    let resets = value::time(data, "/expiresAt")
        .filter(|resets| !is_free(data) && *resets > now && resets.year() < NEVER_YEAR);
    let mut meters = Vec::new();
    if let Some((used, total)) = bucket(data, "userQuota") {
        meters.push(lines::count(
            CREDITS,
            used,
            total,
            UNIT,
            resets,
            resets.map(|_| lines::MONTH_MS),
        ));
    }
    for (key, label) in EXTRA_BUCKETS {
        if let Some((used, total)) = bucket(data, key).filter(|(_, total)| *total > 0.0) {
            meters.push(lines::count(label, used, total, UNIT, None, None));
        }
    }
    if meters.is_empty() {
        return Err(http::decoding(NAME));
    }
    Ok(meters)
}

/// `(used, total)` of a credit bucket, working out a missing side from `remaining`.
fn bucket(data: &Value, key: &str) -> Option<(f64, f64)> {
    let bucket = data.get(key).filter(|bucket| bucket.is_object())?;
    let used = value::number(bucket, "/used");
    let remaining = value::number(bucket, "/remaining");
    let total = value::number(bucket, "/total").or_else(|| {
        used.zip(remaining)
            .map(|(used, remaining)| used + remaining)
    })?;
    let used = used.or_else(|| remaining.map(|remaining| total - remaining))?;
    Some((used.max(0.0), total.max(0.0)))
}

fn is_free(data: &Value) -> bool {
    first_text(data, &["/userType", "/user_type"])
        .is_some_and(|kind| kind.eq_ignore_ascii_case(FREE_USER_TYPE))
}

/// The object holding `key`: the answer itself, or its `data` envelope.
fn holder<'a>(body: &'a Value, key: &str) -> &'a Value {
    match body.get("data") {
        Some(data) if body.get(key).is_none() && data.get(key).is_some() => data,
        _ => body,
    }
}

/// The personal access token as the exchange takes it, `pt-` prefix included.
fn personal_token(key: &str) -> String {
    let key = key.trim();
    if key.starts_with("pt-") {
        key.to_string()
    } else {
        format!("pt-{key}")
    }
}

fn fingerprint(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes())[..8])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The login a document describes, when it holds a token. The secret keeps only the token, its
/// expiry and the host the login belongs to.
fn login(document: &Value, origin: &str, location: &Path, site: Option<&str>) -> Option<Login> {
    let token = first_text(
        document,
        &[
            "/token",
            "/securityOauthToken",
            "/accessToken",
            "/access_token",
        ],
    )?;
    let email = first_text(document, &["/user/email", "/email"]);
    let name = first_text(document, &["/user/name", "/name"]);
    let identity = first_id(
        document,
        &["/user/id", "/id", "/uid", "/userId", "/user_id"],
    )
    .or_else(|| email.map(str::to_lowercase))
    .unwrap_or_else(|| {
        let seed = first_text(document, &["/refreshToken", "/refresh_token"]).unwrap_or(token);
        hex(&Sha256::digest(seed.as_bytes()))
    });
    let mut secret = json!({ "token": token });
    if let Some(expires) = first_time(
        document,
        &["/expiresAt", "/expireTime", "/expires_at", "/expire_time"],
    ) {
        secret["expiresAt"] = json!(expires.timestamp_millis());
    }
    if let Some(site) = site {
        secret["site"] = json!(site);
    }
    Some(
        Login::new(identity, origin, location, Secret::new(secret))
            .with_label(email.or(name).map(str::to_string)),
    )
}

fn first_text<'a>(document: &'a Value, pointers: &[&str]) -> Option<&'a str> {
    pointers
        .iter()
        .find_map(|pointer| value::text(document, pointer))
}

fn first_time(document: &Value, pointers: &[&str]) -> Option<DateTime<Utc>> {
    pointers
        .iter()
        .find_map(|pointer| value::time(document, pointer))
}

/// An account id, which Qoder writes as a string or a number.
fn first_id(document: &Value, pointers: &[&str]) -> Option<String> {
    pointers
        .iter()
        .find_map(|pointer| match document.pointer(pointer)? {
            Value::String(text) => Some(text.trim())
                .filter(|text| !text.is_empty())
                .map(str::to_string),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
}

/// The bytes of a small file, or `None` when it is missing or larger than 64 KiB.
fn read_small(path: &Path) -> Option<Vec<u8>> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return None;
    }
    std::fs::read(path).ok()
}

/// A saved login document: JSON (an object, or a string holding one), a Node `Buffer` of the
/// stored bytes, or safeStorage bytes sealed with the key in `local_state`.
fn open(stored: &[u8], local_state: &Path) -> Option<Value> {
    if stored.starts_with(SEALED) {
        return document(&apps::unseal(local_state, stored)?);
    }
    let parsed = parse(stored)?;
    let buffer = (parsed["type"] == "Buffer")
        .then(|| parsed.get("data").and_then(Value::as_array))
        .flatten();
    let Some(buffer) = buffer else {
        return as_document(parsed);
    };
    let bytes: Vec<u8> = buffer
        .iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect::<Option<_>>()?;
    if bytes.starts_with(SEALED) {
        document(&apps::unseal(local_state, &bytes)?)
    } else {
        document(&bytes)
    }
}

fn parse(bytes: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}').trim()).ok()
}

fn document(bytes: &[u8]) -> Option<Value> {
    as_document(parse(bytes)?)
}

fn as_document(value: Value) -> Option<Value> {
    match value {
        Value::Object(_) => Some(value),
        Value::String(text) => serde_json::from_str::<Value>(text.trim())
            .ok()
            .filter(Value::is_object),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use uc_core::ErrorCategory;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn day(month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, month, day, 0, 0, 0).unwrap()
    }

    fn url(site: &str, path: &str) -> String {
        format!("{site}{path}")
    }

    fn key(key: &str) -> Value {
        json!({ "apiKey": key })
    }

    fn signed_in(expires: DateTime<Utc>) -> Value {
        json!({ "token": "dt-login-token", "expiresAt": expires.timestamp_millis() })
    }

    fn credits(used: f64, total: f64, resets: Option<DateTime<Utc>>) -> MetricLine {
        lines::count(
            "Credits",
            used,
            total,
            "credits",
            resets,
            resets.map(|_| lines::MONTH_MS),
        )
    }

    fn urls(http: &Scripted) -> Vec<String> {
        http.requests()
            .into_iter()
            .map(|request| request.url)
            .collect()
    }

    const EXCHANGED: &str =
        r#"{"token":"jt-job-token","refresh_token":"jrt-job","expires_at":"2026-09-28T10:00:00Z"}"#;
    const QUOTA: &str = r#"{"userId":"u-1001","userType":"personal_pro","usageType":"credits",
        "totalUsagePercentage":37.5,"isQuotaExceeded":false,"expiresAt":1792022400000,
        "isPlanQuotaProrated":false,
        "userQuota":{"total":2000,"used":750.5,"remaining":1249.5,"percentage":37.5,
            "unit":"credits"},
        "addOnQuota":{"total":1500,"used":16,"remaining":1484,"unit":"credits"},
        "orgResourcePackage":{"total":0,"used":0,"remaining":0,"unit":"credits"}}"#;
    const PLAN: &str = r#"{"user_type":"personal_pro","plan_tier_name":"Pro",
        "is_personal_version":true,"is_highest_tier":false,
        "start_date":1789430400000,"end_date":1792022400000}"#;

    #[tokio::test]
    async fn a_personal_access_token_reads_credits_plan_and_paid_period() {
        let http = Scripted::new()
            .on("POST", &url(GLOBAL, EXCHANGE_PATH), 200, EXCHANGED)
            .on("GET", &url(GLOBAL, USAGE_PATH), 200, QUOTA)
            .on("GET", &url(GLOBAL, PLAN_PATH), 200, PLAN);
        let scope = context_at(&http, key("pt-personal"), now());
        let reading = Qoder.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.plan_checked_at, Some(now()));
        assert_eq!(
            reading.lines,
            vec![
                credits(750.5, 2000.0, Some(day(10, 15))),
                lines::count("Add-on Credits", 16.0, 1500.0, "credits", None, None),
            ]
        );
        assert_eq!(
            reading.plan_term,
            Some(PlanTerm::Stated {
                ends_at: day(10, 15),
                checked_at: Some(now()),
            })
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].url, url(GLOBAL, EXCHANGE_PATH));
        assert_eq!(header(&requests[0], "Authorization"), None);
        assert_eq!(header(&requests[0], "Cosy-ClientType"), Some("5"));
        assert_eq!(
            header(&requests[0], "Content-Type"),
            Some("application/json")
        );
        let body: Value = serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body, json!({ "personal_token": "pt-personal" }));
        assert_eq!(requests[1].method, "GET");
        assert_eq!(requests[1].url, url(GLOBAL, USAGE_PATH));
        assert_eq!(
            header(&requests[1], "Authorization"),
            Some("Bearer jt-job-token")
        );
        assert_eq!(requests[2].url, url(GLOBAL, PLAN_PATH));
        assert_eq!(
            header(&requests[2], "Authorization"),
            Some("Bearer jt-job-token")
        );
    }

    #[tokio::test]
    async fn the_job_token_host_and_plan_are_remembered_between_refreshes() {
        let http = Scripted::new()
            .on("POST", &url(GLOBAL, EXCHANGE_PATH), 200, EXCHANGED)
            .on("GET", &url(GLOBAL, USAGE_PATH), 200, QUOTA)
            .on("GET", &url(GLOBAL, PLAN_PATH), 200, PLAN);
        let scope = context_at(&http, key("personal"), now());
        Qoder.fetch(&scope.context()).await.unwrap();
        let mut later = scope.context();
        later.now += Duration::hours(1);
        let reading = Qoder.fetch(&later).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.plan_checked_at, Some(now()));
        assert_eq!(
            urls(&http),
            vec![
                url(GLOBAL, EXCHANGE_PATH),
                url(GLOBAL, USAGE_PATH),
                url(GLOBAL, PLAN_PATH),
                url(GLOBAL, USAGE_PATH),
            ]
        );
        let body: Value =
            serde_json::from_slice(http.requests()[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body, json!({ "personal_token": "pt-personal" }));
    }

    #[tokio::test]
    async fn a_remembered_job_token_that_stops_working_is_exchanged_again() {
        let http = Scripted::new()
            .on("POST", &url(GLOBAL, EXCHANGE_PATH), 200, EXCHANGED)
            .on("GET", &url(GLOBAL, USAGE_PATH), 200, QUOTA)
            .on(
                "GET",
                &url(GLOBAL, USAGE_PATH),
                401,
                r#"{"message":"Login expired"}"#,
            )
            .on("GET", &url(GLOBAL, USAGE_PATH), 200, QUOTA)
            .on("GET", &url(GLOBAL, PLAN_PATH), 200, PLAN);
        let scope = context_at(&http, key("pt-personal"), now());
        Qoder.fetch(&scope.context()).await.unwrap();
        let reading = Qoder.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines.len(), 2);
        assert_eq!(
            urls(&http),
            vec![
                url(GLOBAL, EXCHANGE_PATH),
                url(GLOBAL, USAGE_PATH),
                url(GLOBAL, PLAN_PATH),
                url(GLOBAL, USAGE_PATH),
                url(GLOBAL, EXCHANGE_PATH),
                url(GLOBAL, USAGE_PATH),
            ]
        );
    }

    #[tokio::test]
    async fn a_china_site_token_is_found_on_the_china_host_and_remembered() {
        let http = Scripted::new()
            .on(
                "POST",
                &url(GLOBAL, EXCHANGE_PATH),
                401,
                r#"{"message":"invalid token"}"#,
            )
            .on("POST", &url(CHINA, EXCHANGE_PATH), 200, EXCHANGED)
            .on("GET", &url(CHINA, USAGE_PATH), 200, QUOTA)
            .on("GET", &url(CHINA, PLAN_PATH), 200, PLAN);
        let scope = context_at(&http, key("pt-china"), now());
        let reading = Qoder.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        Qoder.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            urls(&http),
            vec![
                url(GLOBAL, EXCHANGE_PATH),
                url(CHINA, EXCHANGE_PATH),
                url(CHINA, USAGE_PATH),
                url(CHINA, PLAN_PATH),
                url(CHINA, USAGE_PATH),
            ]
        );
    }

    #[tokio::test]
    async fn a_token_both_hosts_refuse_is_rejected() {
        let http = Scripted::new()
            .on("POST", &url(GLOBAL, EXCHANGE_PATH), 401, "{}")
            .on("POST", &url(CHINA, EXCHANGE_PATH), 403, "{}");
        let scope = context_at(&http, key("pt-revoked"), now());
        let error = Qoder.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(error.message, REFUSED);
        assert_eq!(http.requests().len(), 2);
    }

    #[tokio::test]
    async fn a_refused_login_asks_to_open_qoder_again() {
        let http = Scripted::new()
            .on("GET", &url(GLOBAL, USAGE_PATH), 401, "{}")
            .on("GET", &url(CHINA, USAGE_PATH), 401, "{}");
        let scope = context_at(&http, signed_in(now() + Duration::days(1)), now());
        let error = Qoder.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(error.message, EXPIRED);
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, url(GLOBAL, USAGE_PATH));
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some("Bearer dt-login-token")
        );
        assert_eq!(requests[1].url, url(CHINA, USAGE_PATH));
    }

    #[tokio::test]
    async fn an_expired_login_is_reported_without_a_request() {
        let http = Scripted::new().on("GET", &url(GLOBAL, USAGE_PATH), 200, QUOTA);
        let scope = context_at(&http, signed_in(now() - Duration::minutes(1)), now());
        let error = Qoder.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(error.message, EXPIRED);
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn a_china_login_asks_the_china_host_first() {
        let http = Scripted::new()
            .on("GET", &url(CHINA, USAGE_PATH), 200, QUOTA)
            .on("GET", &url(CHINA, PLAN_PATH), 200, PLAN);
        let mut secret = signed_in(now() + Duration::days(1));
        secret["site"] = json!(CHINA);
        let scope = context_at(&http, secret, now());
        let reading = Qoder.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines[0], credits(750.5, 2000.0, Some(day(10, 15))));
        assert_eq!(
            urls(&http),
            vec![url(CHINA, USAGE_PATH), url(CHINA, PLAN_PATH)]
        );
    }

    #[tokio::test]
    async fn rate_limiting_is_reported() {
        let http = Scripted::new().on("GET", &url(GLOBAL, USAGE_PATH), 429, "{}");
        let scope = context_at(&http, signed_in(now() + Duration::days(1)), now());
        let error = Qoder.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
    }

    #[tokio::test]
    async fn a_free_account_shows_no_reset_and_leaves_empty_buckets_out() {
        let http = Scripted::new()
            .on(
                "GET",
                &url(GLOBAL, USAGE_PATH),
                200,
                r#"{"userType":"personal_standard","expiresAt":1792022400000,
                    "userQuota":{"total":0,"used":0,"remaining":0,"unit":"credits"},
                    "addOnQuota":{"total":300,"used":20.25,"remaining":279.75},
                    "orgResourcePackage":{"total":0,"used":0}}"#,
            )
            .on(
                "GET",
                &url(GLOBAL, PLAN_PATH),
                200,
                r#"{"user_type":"personal_standard","plan_tier_name":"FREE",
                    "start_date":0,"end_date":253402214400000}"#,
            );
        let scope = context_at(&http, signed_in(now() + Duration::days(1)), now());
        let reading = Qoder.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Free"));
        assert_eq!(reading.plan_term, None);
        assert_eq!(
            reading.lines,
            vec![
                credits(0.0, 0.0, None),
                lines::count("Add-on Credits", 20.25, 300.0, "credits", None, None),
            ]
        );
    }

    #[test]
    fn never_dates_and_past_dates_are_no_reset_and_shared_credits_show() {
        let body = json!({
            "userType": "enterprise_member",
            "expiresAt": 253402214400000_i64,
            "userQuota": {"total": 3000, "remaining": 2500},
            "orgResourcePackage": {"used": 40, "remaining": 960}
        });
        assert_eq!(
            meters(&body, now()).unwrap(),
            vec![
                credits(500.0, 3000.0, None),
                lines::count("Shared Credits", 40.0, 1000.0, "credits", None, None),
            ]
        );
        let past = json!({"expiresAt": "2026-09-01T00:00:00Z", "data": null,
            "userQuota": {"total": 10, "used": 1}});
        assert_eq!(
            meters(&past, now()).unwrap(),
            vec![credits(1.0, 10.0, None)]
        );
        let wrapped = json!({"data": {"expiresAt": 1792022400000_i64,
            "userQuota": {"total": 10, "used": 1}}});
        assert_eq!(
            meters(&wrapped, now()).unwrap(),
            vec![credits(1.0, 10.0, Some(day(10, 15)))]
        );
    }

    #[tokio::test]
    async fn usage_without_credits_cannot_be_read() {
        let http = Scripted::new().on("GET", &url(GLOBAL, USAGE_PATH), 200, r#"{"success":true}"#);
        let scope = context_at(&http, signed_in(now() + Duration::days(1)), now());
        let error = Qoder.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);
        assert_eq!(urls(&http), vec![url(GLOBAL, USAGE_PATH)]);
    }

    #[tokio::test]
    async fn a_failed_plan_lookup_leaves_the_plan_out_and_is_not_repeated_at_once() {
        let http = Scripted::new()
            .on("GET", &url(GLOBAL, USAGE_PATH), 200, QUOTA)
            .on("GET", &url(GLOBAL, PLAN_PATH), 500, "{}");
        let scope = context_at(&http, signed_in(now() + Duration::days(1)), now());
        let reading = Qoder.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(reading.plan_term, None);
        assert_eq!(reading.lines.len(), 2);
        Qoder.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            urls(&http),
            vec![
                url(GLOBAL, USAGE_PATH),
                url(GLOBAL, PLAN_PATH),
                url(GLOBAL, USAGE_PATH),
            ]
        );
    }

    #[test]
    fn job_tokens_are_reused_until_shortly_before_they_expire() {
        assert_eq!(
            job_expiry(&json!({"expires_at": "2026-09-27T12:00:00Z"}), now()),
            now() + Duration::minutes(115)
        );
        assert_eq!(
            job_expiry(&json!({"expires_in": 3600}), now()),
            now() + Duration::minutes(55)
        );
        assert_eq!(
            job_expiry(&json!({"expires_in": 3_600_000}), now()),
            now() + Duration::minutes(55)
        );
        assert_eq!(
            job_expiry(&json!({"expires_at": "2027-01-01T00:00:00Z"}), now()),
            now() + Duration::hours(24) - Duration::minutes(5)
        );
        assert_eq!(
            job_expiry(&json!({}), now()),
            now() + Duration::hours(24) - Duration::minutes(5)
        );
    }

    #[test]
    fn the_card_declares_the_plan_credits_meter_and_both_ways_to_connect() {
        let provider = Provider::new("qoder@abc", "Qoder");
        let descriptors = Qoder.descriptors(&provider);
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id, "qoder@abc.credits");
        assert_eq!(descriptors[0].metric_label, "Credits");
        assert_eq!(
            descriptors[0].template.count_suffix.as_deref(),
            Some("credits")
        );
        assert_eq!(descriptors[0].limit_resources[0].key, "credits");
        let connection = Qoder.connection();
        assert_eq!(connection.login_from, Some("Qoder"));
        let help = connection.api_key.unwrap();
        assert_eq!(help.env, ["QODER_PERSONAL_ACCESS_TOKEN"]);
        assert_eq!(help.url, "https://qoder.com/account/integrations");
    }

    #[test]
    fn plan_names_read_as_qoder_writes_them() {
        assert_eq!(display_plan("Pro+").as_deref(), Some("Pro+"));
        assert_eq!(display_plan("PRO_TRIAL").as_deref(), Some("Pro Trial"));
        assert_eq!(display_plan("ultra").as_deref(), Some("Ultra"));
        assert_eq!(display_plan(" "), None);
    }

    fn user_info() -> Value {
        json!({
            "id": "u-42",
            "token": "dt-ide-token",
            "refreshToken": "ide-refresh",
            "expireTime": "1792022400000",
            "name": "Minh",
            "email": "me@example.com",
            "userType": "personal_pro"
        })
    }

    fn write_item(database: &Path, key: &str, stored: &str) {
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = rusqlite::Connection::open(database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS ItemTable
                 (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
            )
            .unwrap();
        connection
            .execute("INSERT INTO ItemTable VALUES (?1, ?2)", [key, stored])
            .unwrap();
    }

    #[test]
    fn discovers_the_ide_login_and_nothing_in_an_empty_folder() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        let database = apps::state_db(&roots, IDE_FOLDER);
        write_item(&database, IDE_SECRET, &user_info().to_string());
        let logins = Qoder.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, "u-42");
        assert_eq!(logins[0].label.as_deref(), Some("me@example.com"));
        assert_eq!(logins[0].origin, IDE_ORIGIN);
        assert_eq!(logins[0].location, database);
        assert_eq!(
            logins[0].secret.value(),
            &json!({ "token": "dt-ide-token", "expiresAt": 1792022400000_i64 })
        );
        assert!(
            Qoder
                .discover(&Roots::under(&dir.path().join("empty")))
                .is_empty()
        );
    }

    #[test]
    fn a_login_without_an_account_id_is_told_apart_by_its_refresh_token() {
        let location = Path::new("state.vscdb");
        let anonymous = login(
            &json!({"token": "t-1", "refreshToken": "r-1"}),
            IDE_ORIGIN,
            location,
            None,
        )
        .unwrap();
        assert_eq!(anonymous.identity, hex(&Sha256::digest(b"r-1")));
        assert_eq!(anonymous.label, None);
        let numbered = login(
            &json!({"token": "t-2", "user": {"id": 1001, "name": "Minh"}}),
            IDE_ORIGIN,
            location,
            None,
        )
        .unwrap();
        assert_eq!(numbered.identity, "1001");
        assert_eq!(numbered.label.as_deref(), Some("Minh"));
        assert!(login(&json!({"id": "u-1"}), IDE_ORIGIN, location, None).is_none());
    }

    fn seal(master: &[u8], plain: &[u8]) -> Vec<u8> {
        use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
        let nonce = [9u8; NONCE_LEN];
        let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, master).unwrap());
        let mut ciphertext = plain.to_vec();
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut ciphertext,
        )
        .unwrap();
        let mut blob = SEALED.to_vec();
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        blob
    }

    #[test]
    fn sealed_values_open_only_with_their_key() {
        let master = [5u8; 32];
        let blob = seal(&master, b"{\"token\":\"t\"}");
        assert_eq!(
            apps::open_sealed(&master, &blob).as_deref(),
            Some(&b"{\"token\":\"t\"}"[..])
        );
        assert_eq!(apps::open_sealed(&[6u8; 32], &blob), None);
        assert_eq!(apps::open_sealed(&master, b"v10short"), None);
    }

    #[test]
    fn stored_logins_open_from_json_strings_and_plain_buffers() {
        let nowhere = Path::new("missing-local-state");
        let inner = user_info().to_string();
        let wrapped = Value::String(inner.clone()).to_string();
        assert_eq!(open(wrapped.as_bytes(), nowhere), Some(user_info()));
        let buffer = json!({ "type": "Buffer", "data": inner.as_bytes() }).to_string();
        assert_eq!(open(buffer.as_bytes(), nowhere), Some(user_info()));
        assert_eq!(open(b"[1,2]", nowhere), None);
        assert_eq!(open(&seal(&[1u8; 32], inner.as_bytes()), nowhere), None);
    }

    #[cfg(windows)]
    fn protect(bytes: &[u8]) -> Vec<u8> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
        };
        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let success = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        assert_ne!(success, 0);
        let blob =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        unsafe { LocalFree(output.pbData.cast()) };
        blob
    }

    #[cfg(windows)]
    fn write_local_state(dir: &Path, master: &[u8]) {
        use base64::Engine;
        let mut wrapped = b"DPAPI".to_vec();
        wrapped.extend_from_slice(&protect(master));
        let encoded = base64::engine::general_purpose::STANDARD.encode(wrapped);
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("Local State"),
            json!({ "os_crypt": { "encrypted_key": encoded } }).to_string(),
        )
        .unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn discovers_sealed_desktop_and_ide_logins() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        let desktop = roots.app_data.join("com.qodercn.app.stable");
        let desktop_key = [3u8; 32];
        write_local_state(&desktop, &desktop_key);
        let auth = json!({
            "schemaVersion": 1,
            "token": "qd-desktop-token",
            "refreshToken": "desktop-refresh",
            "expiresAt": 1792022400000_i64,
            "refreshTokenExpiresAt": 1820000000000_i64,
            "user": {"id": "cn-7", "name": "Li", "email": "li@example.cn", "phone": ""}
        });
        std::fs::write(
            desktop.join(DESKTOP_AUTH),
            seal(&desktop_key, auth.to_string().as_bytes()),
        )
        .unwrap();
        let ide_key = [4u8; 32];
        write_local_state(&roots.app_data.join(IDE_FOLDER), &ide_key);
        let sealed = seal(&ide_key, user_info().to_string().as_bytes());
        write_item(
            &apps::state_db(&roots, IDE_FOLDER),
            IDE_SECRET,
            &json!({ "type": "Buffer", "data": sealed }).to_string(),
        );
        let logins = Qoder.discover(&roots);
        assert_eq!(logins.len(), 2);
        assert_eq!(logins[0].identity, "cn-7");
        assert_eq!(logins[0].origin, "Qoder CN");
        assert_eq!(logins[0].label.as_deref(), Some("li@example.cn"));
        assert_eq!(logins[0].location, desktop.join(DESKTOP_AUTH));
        assert_eq!(
            logins[0].secret.value(),
            &json!({
                "token": "qd-desktop-token",
                "expiresAt": 1792022400000_i64,
                "site": CHINA
            })
        );
        assert_eq!(logins[1].identity, "u-42");
        assert_eq!(logins[1].origin, IDE_ORIGIN);
        assert_eq!(logins[1].secret.str("/token"), Some("dt-ide-token"));
    }
}
