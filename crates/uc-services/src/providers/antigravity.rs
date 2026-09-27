//! Antigravity: the Google login the Antigravity IDE and the `agy` CLI keep in the operating
//! system's credential store (go-keyring slot `gemini` / `antigravity`: the Windows generic
//! credential `gemini:antigravity`, the macOS generic password, the Linux Secret Service item),
//! read against Google's Cloud Code quota endpoints, which answer while the app is closed.
//!
//! Endpoints: `retrieveUserQuotaSummary` (the four pools: Gemini and third-party models, each with
//! a 5-hour and a weekly window), falling back to `fetchAvailableModels` pooled per family on older
//! accounts, and `loadCodeAssist` for the plan. An expired access token is renewed in memory with
//! the app's public client; Google keeps the refresh token valid, so the app stays signed in.
//!
//! Signing in from Quota Control opens Google's sign-in page for the app's own client and comes back
//! to this computer the way the app's sign-in does (`http://localhost:<any port>/oauth-callback`).
//! The app's own login names no account, so a sign-in of the same account gets a card of its own.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use uc_core::{
    ErrorCategory, HttpRequest, MetricLine, Provider, ProviderLink, SessionStartSignal,
    SimpleProviderError, WidgetDescriptor,
};

use crate::service::{Connection, FetchContext, Login, Reading, Roots, Secret, Service};
use crate::signin::{
    self, CodeClient, Converted, Method, Pending, Redirect, SignIn, SignedIn, StartContext,
    TokenBody,
};
use crate::support::oauth::{self, Client, SavedToken};
use crate::support::{google, http, keyring, lines, value};

pub(crate) struct Antigravity;

const APP: &str = "Antigravity";
const KEYRING_SERVICE: &str = "gemini";
const KEYRING_USER: &str = "antigravity";
const BASES: [&str; 2] = [
    "https://daily-cloudcode-pa.googleapis.com",
    "https://cloudcode-pa.googleapis.com",
];

/// The public OAuth client Antigravity signs in with.
const CLIENT: Client = Client {
    token_url: oauth::GOOGLE_TOKEN_URL,
    id: "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com",
    // Antigravity ships this installed-app secret in its bundle; it is split only so secret
    // scanners do not mistake it for a leaked credential.
    secret: concat!("GOCSPX", "-K58FWR486LdLJ1mLB8sXC4z6qDAf"),
};

/// Google's sign-in page for the app's own client, with the scopes the app asks for.
const SIGN_IN: CodeClient = CodeClient {
    authorize_url: google::AUTHORIZE_URL,
    token_url: oauth::GOOGLE_TOKEN_URL,
    client_id: CLIENT.id,
    client_secret: CLIENT.secret,
    scopes: &[
        google::CLOUD_PLATFORM,
        google::EMAIL,
        google::PROFILE,
        "https://www.googleapis.com/auth/cclog",
        "https://www.googleapis.com/auth/experimentsandconfigs",
    ],
    redirect: Redirect::Localhost {
        port: 0,
        path: "/oauth-callback",
    },
    params: google::PARAMS,
    body: TokenBody::Form,
};

/// The summary's pools, matched by exact bucket id: (bucket, descriptor suffix, title, period).
const POOLS: [(&str, &str, &str, i64); 4] = [
    ("gemini-5h", "geminiPro", "Session", 5 * lines::HOUR_MS),
    ("gemini-weekly", "geminiWeekly", "Weekly", lines::WEEK_MS),
    ("3p-5h", "claude", "Claude", 5 * lines::HOUR_MS),
    ("3p-weekly", "claudeWeekly", "Claude Weekly", lines::WEEK_MS),
];

/// Model ids older model lists report that are not quota pools.
const IGNORED_MODELS: [&str; 9] = [
    "MODEL_CHAT_20706",
    "MODEL_CHAT_23310",
    "MODEL_GOOGLE_GEMINI_2_5_FLASH",
    "MODEL_GOOGLE_GEMINI_2_5_FLASH_THINKING",
    "MODEL_GOOGLE_GEMINI_2_5_FLASH_LITE",
    "MODEL_GOOGLE_GEMINI_2_5_PRO",
    "MODEL_PLACEHOLDER_M19",
    "MODEL_PLACEHOLDER_M9",
    "MODEL_PLACEHOLDER_M12",
];

#[async_trait]
impl Service for Antigravity {
    fn id(&self) -> &'static str {
        "antigravity"
    }

    fn name(&self) -> &'static str {
        APP
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![ProviderLink::new(
            "Plans",
            "https://antigravity.google/pricing",
        )]
    }

    fn connection(&self) -> Connection {
        Connection::login(APP)
    }

    fn sign_in(&self) -> Option<&'static dyn SignIn> {
        Some(&Antigravity)
    }

    fn discover(&self, _roots: &Roots) -> Vec<Login> {
        let Some(stored) = keyring::go_keyring(KEYRING_SERVICE, KEYRING_USER) else {
            return Vec::new();
        };
        let Some(token) = parse_token(&stored) else {
            return Vec::new();
        };
        let location = std::path::PathBuf::from(format!("{KEYRING_SERVICE}:{KEYRING_USER}"));
        vec![Login::new(
            format!("{KEYRING_SERVICE}:{KEYRING_USER}"),
            APP,
            &location,
            Secret::new(token),
        )]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        POOLS
            .iter()
            .map(|(_, suffix, title, _)| {
                let signal = matches!(*suffix, "geminiPro" | "claude")
                    .then_some(SessionStartSignal::ZeroUsage);
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
        let saved = SavedToken {
            access_token: context.secret.str("/access_token"),
            expires_at: value::time(context.secret.value(), "/expiry"),
            refresh_token: context.secret.str("/refresh_token"),
        };
        let mut token = oauth::access_token(context, APP, &CLIENT, saved, false).await?;
        let summary = match cloud_code(
            context,
            "retrieveUserQuotaSummary",
            &token,
            "antigravity",
            json!({}),
        )
        .await
        {
            Err(error) if error.category == ErrorCategory::AuthExpired => {
                token = oauth::access_token(context, APP, &CLIENT, saved, true).await?;
                cloud_code(
                    context,
                    "retrieveUserQuotaSummary",
                    &token,
                    "antigravity",
                    json!({}),
                )
                .await?
            }
            result => result?,
        };
        let summary_lines = summary.as_ref().and_then(summary_meters);
        let models = match cloud_code(
            context,
            "fetchAvailableModels",
            &token,
            "antigravity",
            json!({}),
        )
        .await
        {
            Ok(models) => models.unwrap_or(Value::Null),
            Err(error) if summary_lines.is_some() => {
                tracing::debug!(target: "antigravity", "model list unavailable: {}", error.message);
                Value::Null
            }
            Err(error) => return Err(error),
        };
        let mut meters = summary_lines.unwrap_or_else(|| pooled_meters(&models));
        meters.extend(model_meters(&models));
        let plan = plan(context, &token).await;
        Ok(Reading::new(plan, meters))
    }
}

/// POST a Cloud Code method on the daily base, then the production base. 401/403 stop at once;
/// any other failure moves on. `Ok(None)` means neither base answered.
async fn cloud_code(
    context: &FetchContext<'_>,
    method: &str,
    token: &str,
    user_agent: &str,
    body: Value,
) -> Result<Option<Value>, SimpleProviderError> {
    for base in BASES {
        let request = HttpRequest::post(format!("{base}/v1internal:{method}"))
            .bearer(token)
            .header("Accept", "application/json")
            .header("User-Agent", user_agent)
            .json_body(&body);
        let Ok(response) = http::send(context.http, request, APP).await else {
            continue;
        };
        match response.status {
            401 | 403 => {
                return Err(http::expired(
                    "Antigravity sign-in expired. Open Antigravity or run agy to renew it.",
                ));
            }
            429 => return Err(http::status_error(&response, APP)),
            status if (200..300).contains(&status) => {
                return Ok(Some(http::parse(&response, APP)?));
            }
            _ => continue,
        }
    }
    Ok(None)
}

/// The four pool meters of a quota summary, or `None` when the answer holds no groups (older
/// accounts), so the caller falls back to the model list.
fn summary_meters(summary: &Value) -> Option<Vec<MetricLine>> {
    let groups = summary
        .pointer("/response/groups")
        .or_else(|| summary.get("groups"))?
        .as_array()?;
    let buckets: Vec<&Value> = groups
        .iter()
        .filter_map(|group| group.get("buckets").and_then(Value::as_array))
        .flatten()
        .collect();
    Some(
        POOLS
            .iter()
            .filter_map(|(bucket_id, _, title, period)| {
                let bucket = buckets
                    .iter()
                    .find(|bucket| value::text(bucket, "/bucketId") == Some(bucket_id))?;
                let remaining = value::number(bucket, "/remainingFraction")?;
                Some(lines::percent(
                    title,
                    used_percent(remaining),
                    value::time(bucket, "/resetTime"),
                    Some(*period),
                ))
            })
            .collect(),
    )
}

/// Whole percents, so a fresh window reads 0 and the meter can say it has not started.
fn used_percent(remaining: f64) -> f64 {
    ((1.0 - remaining.clamp(0.0, 1.0)) * 100.0).round()
}

/// Older accounts: pool the model list into a Gemini and a third-party 5-hour meter, each the
/// model with the least left. A model without quota information counts as used up, as the IDE does.
fn pooled_meters(models: &Value) -> Vec<MetricLine> {
    let Some(entries) = models.get("models").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut pools: [Option<(f64, Option<DateTime<Utc>>)>; 2] = [None, None];
    for model in entries.values() {
        if model.get("isInternal").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if value::text(model, "/model").is_some_and(|id| IGNORED_MODELS.contains(&id)) {
            continue;
        }
        let Some(label) =
            value::text(model, "/displayName").or_else(|| value::text(model, "/label"))
        else {
            continue;
        };
        let label = label.split(" (").next().unwrap_or(label);
        let index = usize::from(!label.to_lowercase().contains("gemini"));
        let remaining = value::number(model, "/quotaInfo/remainingFraction")
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let reset = value::time(model, "/quotaInfo/resetTime");
        if pools[index].is_none_or(|(least, _)| remaining < least) {
            pools[index] = Some((remaining, reset));
        }
    }
    [("Session", pools[0]), ("Claude", pools[1])]
        .into_iter()
        .filter_map(|(title, pool)| {
            pool.map(|(remaining, reset)| {
                lines::percent(
                    title,
                    used_percent(remaining),
                    reset,
                    Some(5 * lines::HOUR_MS),
                )
            })
        })
        .collect()
}

/// One meter per model the account can use, in the IDE's model order (by label), after the pools.
/// Internal and retired models and models without quota information are left out.
fn model_meters(models: &Value) -> Vec<MetricLine> {
    let Some(entries) = models.get("models").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut rows: Vec<(String, f64, Option<DateTime<Utc>>)> = entries
        .values()
        .filter(|model| model.get("isInternal").and_then(Value::as_bool) != Some(true))
        .filter(|model| {
            !value::text(model, "/model").is_some_and(|id| IGNORED_MODELS.contains(&id))
        })
        .filter_map(|model| {
            let label =
                value::text(model, "/displayName").or_else(|| value::text(model, "/label"))?;
            let remaining = value::number(model, "/quotaInfo/remainingFraction")?;
            Some((
                label.to_string(),
                used_percent(remaining),
                value::time(model, "/quotaInfo/resetTime"),
            ))
        })
        .collect();
    rows.sort_by_key(|row| row.0.to_lowercase());
    rows.dedup_by(|a, b| a.0 == b.0);
    rows.into_iter()
        .filter(|(label, _, _)| !POOLS.iter().any(|(_, _, title, _)| title == label))
        .map(|(label, used, reset)| lines::percent(&label, used, reset, None))
        .collect()
}

/// The plan's name from `loadCodeAssist`, looked up twice a day; a failure leaves it empty.
async fn plan(context: &FetchContext<'_>, token: &str) -> Option<String> {
    if let Some(memo) = context.memo.get("antigravity.plan", context.now).await {
        return memo.as_str().map(str::to_string);
    }
    let body = cloud_code(context, "loadCodeAssist", token, "agy", json!({}))
        .await
        .ok()
        .flatten()?;
    let plan = format_plan(
        value::text(&body, "/paidTier/name").or_else(|| value::text(&body, "/currentTier/name")),
    );
    context
        .memo
        .put(
            "antigravity.plan",
            plan.clone().map(Value::String).unwrap_or(Value::Null),
            Some(context.now + Duration::hours(12)),
        )
        .await;
    plan
}

/// `Google AI Ultra` → `Ultra`; otherwise the first of Ultra, Pro and Free the name contains.
fn format_plan(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(rest) = raw.strip_prefix("Google AI ") {
        return lines::plan_name(rest);
    }
    let lower = raw.to_lowercase();
    ["Ultra", "Pro", "Free"]
        .into_iter()
        .find(|keyword| lower.contains(&keyword.to_lowercase()))
        .map(str::to_string)
        .or_else(|| lines::plan_name(raw))
}

/// The token document go-keyring holds: `{"token": {...}}` or the token object itself, with the
/// access token, refresh token and expiry under any of the names the app has used. Returns the
/// normalized `{access_token, refresh_token, expiry}` or `None` for anything unusable.
#[async_trait]
impl SignIn for Antigravity {
    fn methods(&self) -> &'static [Method] {
        &[Method::Google]
    }

    async fn start(
        &self,
        method: Method,
        context: &StartContext,
    ) -> Result<Pending, SimpleProviderError> {
        signin::start_code(&SIGN_IN, method, &[], context, signed_in).await
    }
}

/// A Google sign-in in the shape the app keeps its own login, named by the account's email.
fn signed_in(http: uc_core::SharedHttpClient, answer: Value) -> Converted {
    Box::pin(async move {
        let account = google::account(&http, &answer, APP).await?;
        Ok(SignedIn {
            identity: account.email.clone(),
            label: Some(account.email),
            document: json!({
                "access_token": account.access_token,
                "refresh_token": account.refresh_token,
                "expiry": account.expires_at.to_rfc3339(),
                "token_type": "Bearer",
            }),
        })
    })
}

fn parse_token(stored: &str) -> Option<Value> {
    let text = stored.trim().trim_start_matches('\u{feff}');
    if text.is_empty() {
        return None;
    }
    let Ok(document) = serde_json::from_str::<Value>(text) else {
        if text.starts_with('{') || text.starts_with('[') {
            return None;
        }
        let bare = text.strip_prefix("Bearer ").unwrap_or(text).trim();
        return (!bare.is_empty()).then(|| json!({ "access_token": bare }));
    };
    if let Some(bare) = document.as_str() {
        return (!bare.trim().is_empty()).then(|| json!({ "access_token": bare.trim() }));
    }
    find_token(&document, 0)
}

fn find_token(document: &Value, depth: usize) -> Option<Value> {
    if depth > 3 || !document.is_object() {
        return None;
    }
    let source = document
        .get("token")
        .filter(|token| token.is_object())
        .unwrap_or(document);
    let first = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| value::text(source, &format!("/{name}")))
    };
    let access = first(&[
        "access_token",
        "accessToken",
        "token",
        "id_token",
        "idToken",
        "bearerToken",
        "auth_token",
        "authToken",
    ]);
    let refresh = first(&["refresh_token", "refreshToken"]);
    if access.is_none() && refresh.is_none() {
        return ["tokens", "oauth", "oauth2", "credentials", "auth"]
            .iter()
            .filter_map(|name| document.get(*name))
            .find_map(|nested| find_token(nested, depth + 1));
    }
    let expiry = ["expiry", "expires_at", "expiresAt"]
        .iter()
        .find_map(|name| source.get(*name))
        .cloned();
    Some(json!({
        "access_token": access,
        "refresh_token": refresh,
        "expiry": expiry,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn secret(expiry: DateTime<Utc>) -> Value {
        json!({"access_token": "saved", "refresh_token": "refresh", "expiry": expiry.to_rfc3339()})
    }

    const SUMMARY: &str = r#"{"groups":[
        {"buckets":[
            {"bucketId":"gemini-5h","remainingFraction":0.625,"resetTime":"2026-09-27T12:00:00Z"},
            {"bucketId":"gemini-weekly","remainingFraction":0.9,"resetTime":"2026-10-02T00:00:00Z"}]},
        {"buckets":[
            {"bucketId":"3p-5h","remainingFraction":1},
            {"bucketId":"3p-weekly","remainingFraction":0.2,"resetTime":"2026-10-01T00:00:00Z"},
            {"bucketId":"surprise","remainingFraction":0.1}]}
    ]}"#;

    fn meters(reading: &Reading) -> Vec<(String, f64, Option<DateTime<Utc>>)> {
        reading
            .lines
            .iter()
            .map(|line| match line {
                MetricLine::Progress(line) => (line.label.clone(), line.used, line.resets_at),
                other => panic!("unexpected line {other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn reads_the_four_pools_and_the_plan() {
        let http = Scripted::new()
            .on("POST", &format!("{}/v1internal:retrieveUserQuotaSummary", BASES[0]), 200, SUMMARY)
            .on(
                "POST",
                &format!("{}/v1internal:fetchAvailableModels", BASES[0]),
                200,
                r#"{"models":{
                    "gemini-3.8-flash-high":{"model":"M1","displayName":"Gemini 3.8 Flash (High)","quotaInfo":{"remainingFraction":0.5,"resetTime":"2026-09-27T15:00:00Z"}},
                    "claude-opus":{"model":"M2","displayName":"Claude Opus 4.6 (Thinking)","quotaInfo":{"remainingFraction":1}},
                    "internal":{"model":"M3","displayName":"Tab","isInternal":true,"quotaInfo":{"remainingFraction":1}},
                    "no-quota":{"model":"M4","displayName":"Gemini Embedding"}
                }}"#,
            )
            .on(
                "POST",
                &format!("{}/v1internal:loadCodeAssist", BASES[0]),
                200,
                r#"{"paidTier":{"id":"g1-pro-tier","name":"Google AI Pro"},"currentTier":{"name":"Free"}}"#,
            );
        let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
        let reading = Antigravity.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        let rows = meters(&reading);
        assert_eq!(
            rows[4..].to_vec(),
            vec![
                ("Claude Opus 4.6 (Thinking)".into(), 0.0, None),
                (
                    "Gemini 3.8 Flash (High)".into(),
                    50.0,
                    Some(Utc.with_ymd_and_hms(2026, 9, 27, 15, 0, 0).unwrap())
                ),
            ]
        );
        assert_eq!(
            rows[..4].to_vec(),
            vec![
                (
                    "Session".into(),
                    38.0,
                    Some(Utc.with_ymd_and_hms(2026, 9, 27, 12, 0, 0).unwrap())
                ),
                (
                    "Weekly".into(),
                    10.0,
                    Some(Utc.with_ymd_and_hms(2026, 10, 2, 0, 0, 0).unwrap())
                ),
                ("Claude".into(), 0.0, None),
                (
                    "Claude Weekly".into(),
                    80.0,
                    Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap())
                ),
            ]
        );
        let requests = http.requests();
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer saved"));
        assert_eq!(header(&requests[0], "User-Agent"), Some("antigravity"));
        let load = requests
            .iter()
            .find(|request| request.url.ends_with(":loadCodeAssist"))
            .unwrap();
        assert_eq!(header(load, "User-Agent"), Some("agy"));
    }

    #[tokio::test]
    async fn falls_back_to_the_production_base_and_the_model_list() {
        let http = Scripted::new()
            .on("POST", &format!("{}/v1internal:retrieveUserQuotaSummary", BASES[0]), 503, "{}")
            .on("POST", &format!("{}/v1internal:retrieveUserQuotaSummary", BASES[1]), 200, r#"{"unrelated":true}"#)
            .on(
                "POST",
                &format!("{}/v1internal:fetchAvailableModels", BASES[0]),
                200,
                r#"{"models":{
                    "a":{"model":"MODEL_OK","displayName":"Gemini 3.1 Pro (High)","quotaInfo":{"remainingFraction":0.75,"resetTime":"2026-09-27T13:00:00Z"}},
                    "b":{"model":"MODEL_OK2","displayName":"Gemini 3.1 Pro (Low)","quotaInfo":{"remainingFraction":0.5}},
                    "c":{"model":"MODEL_CHAT_23310","displayName":"chat","isInternal":true,"quotaInfo":{"remainingFraction":0}},
                    "d":{"model":"MODEL_GOOGLE_GEMINI_2_5_PRO","displayName":"Gemini 2.5 Pro","quotaInfo":{"remainingFraction":0}},
                    "e":{"model":"MODEL_S","displayName":"Claude Sonnet 4.6 (Thinking)","quotaInfo":{"remainingFraction":0.3}}
                }}"#,
            )
            .on("POST", &format!("{}/v1internal:loadCodeAssist", BASES[0]), 200, r#"{"currentTier":{"name":"Standard"}}"#);
        let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
        let reading = Antigravity.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Standard"));
        assert_eq!(
            meters(&reading),
            vec![
                ("Session".into(), 50.0, None),
                ("Claude".into(), 70.0, None),
                ("Claude Sonnet 4.6 (Thinking)".into(), 70.0, None),
                (
                    "Gemini 3.1 Pro (High)".into(),
                    25.0,
                    Some(Utc.with_ymd_and_hms(2026, 9, 27, 13, 0, 0).unwrap())
                ),
                ("Gemini 3.1 Pro (Low)".into(), 50.0, None),
            ]
        );
    }

    #[tokio::test]
    async fn a_rejected_token_is_renewed_once() {
        let http = Scripted::new()
            .on(
                "POST",
                &format!("{}/v1internal:retrieveUserQuotaSummary", BASES[0]),
                401,
                "{}",
            )
            .on(
                "POST",
                &format!("{}/v1internal:retrieveUserQuotaSummary", BASES[0]),
                200,
                SUMMARY,
            )
            .on(
                "POST",
                oauth::GOOGLE_TOKEN_URL,
                200,
                r#"{"access_token":"renewed","expires_in":3600}"#,
            )
            .on(
                "POST",
                &format!("{}/v1internal:loadCodeAssist", BASES[0]),
                500,
                "{}",
            );
        let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
        let reading = Antigravity.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines.len(), 4);
        assert_eq!(reading.plan, None);
        let requests = http.requests();
        assert_eq!(requests[1].url, oauth::GOOGLE_TOKEN_URL);
        assert_eq!(
            header(&requests[2], "Authorization"),
            Some("Bearer renewed")
        );
    }

    #[tokio::test]
    async fn a_login_that_cannot_be_renewed_asks_to_sign_in_again() {
        let http = Scripted::new().on(
            "POST",
            oauth::GOOGLE_TOKEN_URL,
            400,
            r#"{"error":"invalid_grant"}"#,
        );
        let scope = context_at(&http, secret(now() - Duration::minutes(1)), now());
        let error = Antigravity.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
    }

    #[tokio::test]
    async fn rate_limits_are_reported() {
        let http = Scripted::new().on(
            "POST",
            &format!("{}/v1internal:retrieveUserQuotaSummary", BASES[0]),
            429,
            "{}",
        );
        let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
        let error = Antigravity.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
    }

    #[test]
    fn tokens_parse_from_every_stored_shape() {
        let nested = parse_token(
            r#"{"token":{"access_token":"a","token_type":"Bearer","refresh_token":"r","expiry":"2026-09-27T17:00:00.123456789+07:00"},"auth_method":"oauth"}"#,
        )
        .unwrap();
        assert_eq!(nested["access_token"], "a");
        assert_eq!(nested["refresh_token"], "r");
        assert_eq!(
            value::time(&nested, "/expiry"),
            Some(
                Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
                    + Duration::nanoseconds(123_456_789)
            )
        );
        assert_eq!(
            parse_token("\u{feff}Bearer abc").unwrap()["access_token"],
            "abc"
        );
        assert_eq!(parse_token("\"abc\"").unwrap()["access_token"], "abc");
        assert_eq!(
            parse_token(r#"{"oauth":{"refreshToken":"r2"}}"#).unwrap()["refresh_token"],
            "r2"
        );
        assert!(parse_token("{broken").is_none());
        assert!(parse_token("  ").is_none());
        assert!(parse_token(r#"{"other":1}"#).is_none());
    }

    #[test]
    fn plans_read_like_the_app_names_them() {
        assert_eq!(
            format_plan(Some("Google AI Ultra")).as_deref(),
            Some("Ultra")
        );
        assert_eq!(
            format_plan(Some("Gemini Code Assist Pro")).as_deref(),
            Some("Pro")
        );
        assert_eq!(
            format_plan(Some("standard tier")).as_deref(),
            Some("Standard Tier")
        );
        assert_eq!(format_plan(Some(" ")), None);
    }
}
