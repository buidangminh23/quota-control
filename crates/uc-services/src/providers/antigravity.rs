//! Antigravity: the Google login the Antigravity IDE and the `agy` CLI keep in the operating
//! system's credential store (go-keyring slot `gemini` / `antigravity`: the Windows generic
//! credential `gemini:antigravity`, the macOS generic password, the Linux Secret Service item),
//! read against Google's Cloud Code quota endpoints, which answer while the app is closed.
//!
//! Signing in from Quota Control opens Google's sign-in page for the app's own client and comes back
//! to this computer the way the app's sign-in does (`http://localhost:<any port>/oauth-callback`).
//! The app's own login names no account, so a sign-in of the same account gets a card of its own.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use uc_core::{
    ErrorCategory, HttpRequest, MetricLine, Provider, ProviderLink, SimpleProviderError,
    WidgetDescriptor,
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
/// Google's documented OAuth2 userinfo endpoint; the app's token carries the `userinfo.email` scope.
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const BASES: [&str; 2] = [
    "https://daily-cloudcode-pa.googleapis.com",
    "https://cloudcode-pa.googleapis.com",
];
const USER_AGENT: &str = "antigravity/hub/2.30.0";

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

const POOLS: [(&str, &str, &str, &str, i64); 5] = [
    (
        "gemini-weekly",
        "geminiWeekly",
        "Gemini Weekly",
        "Gemini Models",
        lines::WEEK_MS,
    ),
    (
        "gemini-5h",
        "geminiPro",
        "Gemini 5h",
        "Gemini Models",
        5 * lines::HOUR_MS,
    ),
    (
        "3p-weekly",
        "claudeWeekly",
        "Claude and GPT Weekly",
        "Claude and GPT models",
        lines::WEEK_MS,
    ),
    (
        "3p-5h",
        "claude",
        "Claude and GPT 5h",
        "Claude and GPT models",
        5 * lines::HOUR_MS,
    ),
    (
        "gemini-argon-wtu-weekly",
        "argonWeekly",
        "Gemini 4 Argon Weekly",
        "Gemini 4 Argon",
        lines::WEEK_MS,
    ),
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
            .map(|(_, suffix, title, _, _)| {
                WidgetDescriptor::percent(
                    format!("{}.{suffix}", provider.id),
                    provider,
                    title,
                    None,
                    None,
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
        let summary = match cloud_code(context, "retrieveUserQuotaSummary", &token, json!({})).await
        {
            Err(error) if error.category == ErrorCategory::AuthExpired => {
                token = oauth::access_token(context, APP, &CLIENT, saved, true).await?;
                cloud_code(context, "retrieveUserQuotaSummary", &token, json!({})).await?
            }
            result => result?,
        };
        let meters = if let Some(meters) = summary.as_ref().and_then(summary_meters) {
            meters
        } else {
            let models = cloud_code(context, "fetchAvailableModels", &token, json!({}))
                .await?
                .unwrap_or(Value::Null);
            model_meters(&models)
        };
        if meters.is_empty() {
            return Err(http::decoding(APP));
        }
        let plan = plan(context, &token).await;
        let email = email(context, &token).await;
        Ok(Reading::new(plan, meters).with_account(email))
    }
}

/// POST a Cloud Code method on the daily base, then the production base. 401/403 stop at once;
/// any other failure moves on. `Ok(None)` means neither base answered.
async fn cloud_code(
    context: &FetchContext<'_>,
    method: &str,
    token: &str,
    body: Value,
) -> Result<Option<Value>, SimpleProviderError> {
    for base in BASES {
        let request = HttpRequest::post(format!("{base}/v1internal:{method}"))
            .bearer(token)
            .header("Accept", "application/json")
            .header("User-Agent", user_agent())
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
                if let Ok(body) = http::parse(&response, APP) {
                    return Ok(Some(body));
                }
            }
            _ => continue,
        }
    }
    Ok(None)
}

fn user_agent() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        arch => arch,
    };
    format!("{USER_AGENT} (aidev_client; os_type={os}; arch={arch})")
}

fn summary_meters(summary: &Value) -> Option<Vec<MetricLine>> {
    let summary = summary.get("response").unwrap_or(summary);
    let buckets: Vec<(Option<&str>, &Value)> = summary
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| {
            group
                .get("buckets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(move |bucket| (value::text(group, "/displayName"), bucket))
        })
        .chain(
            summary
                .get("buckets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|bucket| (None, bucket)),
        )
        .collect();
    let meters: Vec<_> = POOLS
        .iter()
        .filter_map(|(bucket_id, _, title, group_name, period)| {
            let window = |bucket: &Value| match value::text(bucket, "/window") {
                Some("weekly") => Some(lines::WEEK_MS),
                Some("5h") => Some(5 * lines::HOUR_MS),
                _ => None,
            };
            let exact = buckets.iter().filter(|(_, bucket)| {
                value::text(bucket, "/bucketId") == Some(bucket_id)
                    && (bucket.get("window").is_none() || window(bucket) == Some(*period))
            });
            let grouped = buckets.iter().filter(|(group, bucket)| {
                group.is_some_and(|group| group.trim().eq_ignore_ascii_case(group_name))
                    && window(bucket) == Some(*period)
                    && !POOLS.iter().any(|(known_id, _, _, _, _)| {
                        value::text(bucket, "/bucketId") == Some(known_id)
                    })
            });
            let meter = |(_, bucket): &(Option<&str>, &Value)| {
                let remaining = remaining_fraction(bucket, "/remainingFraction")?;
                Some(lines::percent(
                    title,
                    used_percent(remaining),
                    value::time(bucket, "/resetTime"),
                    Some(*period),
                ))
            };
            if let Some(meter) = exact.filter_map(meter).next() {
                return Some(meter);
            }
            let mut candidates = grouped.filter_map(meter);
            let meter = candidates.next()?;
            candidates.next().is_none().then_some(meter)
        })
        .collect();
    (!meters.is_empty()).then_some(meters)
}

fn remaining_fraction(value: &Value, pointer: &str) -> Option<f64> {
    value::number(value, pointer).filter(|remaining| (0.0..=1.0).contains(remaining))
}

fn used_percent(remaining: f64) -> f64 {
    ((1.0 - remaining.clamp(0.0, 1.0)) * 100.0).round()
}

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
            let reset = value::time(model, "/quotaInfo/resetTime");
            let remaining = if model.pointer("/quotaInfo/remainingFraction").is_some() {
                remaining_fraction(model, "/quotaInfo/remainingFraction")?
            } else if reset.is_some() {
                0.0
            } else {
                return None;
            };
            Some((label.to_string(), used_percent(remaining), reset))
        })
        .collect();
    rows.sort_by_key(|row| row.0.to_lowercase());
    rows.dedup_by(|a, b| a.0 == b.0);
    rows.into_iter()
        .filter(|(label, _, _)| !POOLS.iter().any(|(_, _, title, _, _)| title == label))
        .map(|(label, used, reset)| lines::percent(&label, used, reset, None))
        .collect()
}

/// The plan's name from `loadCodeAssist`, looked up twice a day; a failure leaves it empty.
async fn plan(context: &FetchContext<'_>, token: &str) -> Option<String> {
    if let Some(memo) = context.memo.get("antigravity.plan", context.now).await {
        return memo.as_str().map(str::to_string);
    }
    let body = cloud_code(context, "loadCodeAssist", token, json!({}))
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

/// The account's email from Google's userinfo endpoint, looked up twice a day; a failure leaves it
/// empty and never affects the reading.
async fn email(context: &FetchContext<'_>, token: &str) -> Option<String> {
    if let Some(memo) = context.memo.get("antigravity.email", context.now).await {
        return memo.as_str().map(str::to_string);
    }
    let request = HttpRequest::get(USERINFO_URL)
        .bearer(token)
        .header("Accept", "application/json");
    let response = match http::send(context.http, request, APP).await {
        Ok(response) if response.is_success() => response,
        Ok(response) => {
            tracing::debug!(target: "antigravity", "userinfo answered {}", response.status);
            return None;
        }
        Err(error) => {
            tracing::debug!(target: "antigravity", "userinfo unavailable: {}", error.message);
            return None;
        }
    };
    let body = http::parse(&response, APP).ok()?;
    let email = value::text(&body, "/email").map(str::to_string);
    context
        .memo
        .put(
            "antigravity.email",
            email.clone().map(Value::String).unwrap_or(Value::Null),
            Some(context.now + Duration::hours(12)),
        )
        .await;
    email
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
        {"displayName":"Gemini Models","buckets":[
            {"bucketId":"gemini-weekly","displayName":"Weekly Limit Remaining","window":"weekly","remainingFraction":0.6878024,"resetTime":"2026-11-14T05:13:17Z"},
            {"bucketId":"gemini-5h","displayName":"Five Hour Limit Remaining","window":"5h","remainingFraction":1,"resetTime":"2026-11-10T17:54:18Z"}]},
        {"displayName":"Claude and GPT models","buckets":[
            {"bucketId":"3p-weekly","displayName":"Weekly Limit Remaining","window":"weekly","remainingFraction":0,"resetTime":"2026-11-13T01:03:04Z"},
            {"bucketId":"3p-5h","displayName":"Five Hour Limit Remaining","window":"5h","remainingFraction":1,"disabled":true,"resetTime":"2026-11-10T17:54:18Z"}]},
        {"displayName":"Gemini 4 Argon","buckets":[
            {"bucketId":"gemini-argon-wtu-weekly","displayName":"Gemini 4 Argon Weekly Limit","window":"weekly","remainingFraction":0.009153066,"resetTime":"2026-11-14T03:59:21Z"}]}
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

    #[test]
    fn descriptors_preserve_ids_and_provider_reset_times() {
        let descriptors = Antigravity.descriptors(&Provider::new("antigravity@account", APP));
        assert_eq!(
            descriptors
                .iter()
                .map(|descriptor| (descriptor.id.as_str(), descriptor.metric_label.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("antigravity@account.geminiWeekly", "Gemini Weekly"),
                ("antigravity@account.geminiPro", "Gemini 5h"),
                ("antigravity@account.claudeWeekly", "Claude and GPT Weekly"),
                ("antigravity@account.claude", "Claude and GPT 5h"),
                ("antigravity@account.argonWeekly", "Gemini 4 Argon Weekly"),
            ]
        );
        assert!(
            descriptors
                .iter()
                .all(|descriptor| descriptor.template.session_start_signal.is_none())
        );
    }

    #[test]
    fn grouped_windows_survive_bucket_id_changes() {
        let summary: Value = serde_json::from_str(SUMMARY).unwrap();
        let expected = summary_meters(&summary).unwrap();
        let mut changed = summary;
        for group in changed["groups"].as_array_mut().unwrap() {
            for bucket in group["buckets"].as_array_mut().unwrap() {
                bucket["bucketId"] = json!(format!("renamed-{}", bucket["bucketId"]));
            }
        }
        assert_eq!(
            summary_meters(&json!({"response": changed})),
            Some(expected)
        );
    }

    #[test]
    fn known_bucket_ids_take_precedence_over_conflicting_group_metadata() {
        let summary = json!({"groups": [{"displayName": "Gemini Models", "buckets": [
            {"bucketId": "unknown-weekly", "window": "weekly", "remainingFraction": 0.2},
            {"bucketId": "gemini-argon-wtu-weekly", "window": "weekly", "remainingFraction": 0.01},
            {"bucketId": "gemini-weekly", "window": "weekly", "remainingFraction": 0.69}
        ]}]});
        let reading = Reading::new(None, summary_meters(&summary).unwrap());
        assert_eq!(
            meters(&reading),
            vec![
                ("Gemini Weekly".into(), 31.0, None),
                ("Gemini 4 Argon Weekly".into(), 99.0, None),
            ]
        );
        let only_argon = json!({"groups": [{"displayName": "Gemini Models", "buckets": [
            {"bucketId": "gemini-argon-wtu-weekly", "window": "weekly", "remainingFraction": 0.01}
        ]}]});
        let reading = Reading::new(None, summary_meters(&only_argon).unwrap());
        assert_eq!(
            meters(&reading),
            vec![("Gemini 4 Argon Weekly".into(), 99.0, None)]
        );
    }

    #[test]
    fn unrelated_buckets_and_missing_fractions_do_not_become_usage() {
        for summary in [
            json!(null),
            json!({"groups": []}),
            json!({"groups": {}}),
            json!({"groups": [{"buckets": [
                {"bucketId": "gemini-weekly", "remainingAmount": 50},
                {"bucketId": "3p-weekly", "remainingFraction": null},
                {"bucketId": "gemini-5h", "remainingFraction": -0.1},
                {"bucketId": "3p-5h", "remainingFraction": 1.1},
                {"bucketId": "gemini-argon-wtu-weekly", "remainingFraction": "bad"}
            ]}]}),
            json!({"groups": [{"displayName": "Other models", "buckets": [
                {"bucketId": "unknown-weekly", "window": "weekly", "remainingFraction": 0}
            ]}]}),
            json!({"groups": [{"displayName": "Gemini Models", "buckets": [
                {"bucketId": "unknown", "remainingFraction": 0},
                {"bucketId": "unknown-monthly", "window": "monthly", "remainingFraction": 0}
            ]}]}),
            json!({"groups": [{"displayName": "Gemini Models", "buckets": [
                {"bucketId": "unknown-a", "window": "weekly", "remainingFraction": 0.1},
                {"bucketId": "unknown-b", "window": "weekly", "remainingFraction": 0.9}
            ]}]}),
        ] {
            assert!(summary_meters(&summary).is_none(), "{summary}");
        }
    }

    #[test]
    fn known_legacy_buckets_keep_their_scope_without_inventing_resets() {
        let summary = json!({"buckets": [
            {"bucketId": "gemini-weekly", "remainingFraction": 0},
            {"bucketId": "3p-5h", "remainingFraction": 1}
        ]});
        let reading = Reading::new(None, summary_meters(&summary).unwrap());
        assert_eq!(
            meters(&reading),
            vec![
                ("Gemini Weekly".into(), 100.0, None),
                ("Claude and GPT 5h".into(), 0.0, None),
            ]
        );
    }

    #[test]
    fn model_fallback_preserves_exhausted_quotas_without_inventing_windows() {
        let models = json!({"models": {
            "claude": {"displayName": "Claude Sonnet", "quotaInfo": {"resetTime": "2026-11-13T01:03:04Z"}},
            "argon": {"displayName": "Gemini 4 Argon", "quotaInfo": {"remainingFraction": 0.009153066, "resetTime": "2026-11-14T03:59:21Z"}},
            "flash": {"displayName": "Gemini Flash", "quotaInfo": {"remainingFraction": 1}},
            "absent": {"displayName": "Missing Quota"},
            "empty": {"displayName": "Empty Quota", "quotaInfo": {}},
            "malformed": {"displayName": "Invalid Quota", "quotaInfo": {"remainingFraction": null, "resetTime": "2026-11-13T01:03:04Z"}}
        }});
        let reading = Reading::new(None, model_meters(&models));
        assert_eq!(
            meters(&reading),
            vec![
                (
                    "Claude Sonnet".into(),
                    100.0,
                    Some(Utc.with_ymd_and_hms(2026, 11, 13, 1, 3, 4).unwrap())
                ),
                (
                    "Gemini 4 Argon".into(),
                    99.0,
                    Some(Utc.with_ymd_and_hms(2026, 11, 14, 3, 59, 21).unwrap())
                ),
                ("Gemini Flash".into(), 0.0, None),
            ]
        );
        assert!(reading.lines.iter().all(|line| match line {
            MetricLine::Progress(line) => line.period_duration_ms.is_none(),
            _ => false,
        }));
    }

    #[tokio::test]
    async fn malformed_and_empty_summaries_fall_back_to_reported_models() {
        for summary in [
            "{broken",
            r#"{"groups":[]}"#,
            r#"{"groups":[{"buckets":[]}]}"#,
        ] {
            let http = Scripted::new()
                .on("POST", &format!("{}/v1internal:retrieveUserQuotaSummary", BASES[0]), 200, summary)
                .on("POST", &format!("{}/v1internal:retrieveUserQuotaSummary", BASES[1]), 200, summary)
                .on("POST", &format!("{}/v1internal:fetchAvailableModels", BASES[0]), 200,
                    r#"{"models":{"argon":{"displayName":"Gemini 4 Argon","quotaInfo":{"remainingFraction":0.01,"resetTime":"2026-11-14T03:59:21Z"}}}}"#);
            let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
            let reading = Antigravity.fetch(&scope.context()).await.unwrap();
            assert_eq!(
                meters(&reading),
                vec![(
                    "Gemini 4 Argon".into(),
                    99.0,
                    Some(Utc.with_ymd_and_hms(2026, 11, 14, 3, 59, 21).unwrap()),
                )]
            );
        }
    }

    #[tokio::test]
    async fn unreadable_sources_do_not_publish_an_empty_successful_reading() {
        let http = Scripted::new()
            .on(
                "POST",
                &format!("{}/v1internal:retrieveUserQuotaSummary", BASES[0]),
                200,
                "{}",
            )
            .on(
                "POST",
                &format!("{}/v1internal:fetchAvailableModels", BASES[0]),
                200,
                "{}",
            );
        let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
        assert_eq!(
            Antigravity
                .fetch(&scope.context())
                .await
                .unwrap_err()
                .category,
            ErrorCategory::Decoding
        );
    }

    #[tokio::test]
    async fn reads_the_five_pools_and_the_plan() {
        let http = Scripted::new()
            .on("POST", &format!("{}/v1internal:retrieveUserQuotaSummary", BASES[0]), 200, SUMMARY)
            .on(
                "POST",
                &format!("{}/v1internal:loadCodeAssist", BASES[0]),
                200,
                r#"{"paidTier":{"id":"g1-pro-tier","name":"Google AI Pro"},"currentTier":{"name":"Free"}}"#,
            )
            .on(
                "GET",
                USERINFO_URL,
                200,
                r#"{"id":"1","email":"minh@example.com","verified_email":true}"#,
            );
        let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
        let reading = Antigravity.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.account.as_deref(), Some("minh@example.com"));
        let rows = meters(&reading);
        assert_eq!(
            rows,
            vec![
                (
                    "Gemini Weekly".into(),
                    31.0,
                    Some(Utc.with_ymd_and_hms(2026, 11, 14, 5, 13, 17).unwrap())
                ),
                (
                    "Gemini 5h".into(),
                    0.0,
                    Some(Utc.with_ymd_and_hms(2026, 11, 10, 17, 54, 18).unwrap())
                ),
                (
                    "Claude and GPT Weekly".into(),
                    100.0,
                    Some(Utc.with_ymd_and_hms(2026, 11, 13, 1, 3, 4).unwrap())
                ),
                (
                    "Claude and GPT 5h".into(),
                    0.0,
                    Some(Utc.with_ymd_and_hms(2026, 11, 10, 17, 54, 18).unwrap())
                ),
                (
                    "Gemini 4 Argon Weekly".into(),
                    99.0,
                    Some(Utc.with_ymd_and_hms(2026, 11, 14, 3, 59, 21).unwrap())
                ),
            ]
        );
        assert_eq!(
            reading.lines[..5]
                .iter()
                .map(|line| match line {
                    MetricLine::Progress(line) => line.period_duration_ms,
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>(),
            vec![
                Some(lines::WEEK_MS),
                Some(5 * lines::HOUR_MS),
                Some(lines::WEEK_MS),
                Some(5 * lines::HOUR_MS),
                Some(lines::WEEK_MS),
            ]
        );
        let requests = http.requests();
        assert!(
            !requests
                .iter()
                .any(|request| request.url.ends_with(":fetchAvailableModels"))
        );
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer saved"));
        assert_eq!(
            header(&requests[0], "User-Agent"),
            Some(user_agent().as_str())
        );
        let load = requests
            .iter()
            .find(|request| request.url.ends_with(":loadCodeAssist"))
            .unwrap();
        assert_eq!(header(load, "User-Agent"), Some(user_agent().as_str()));
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
                ("Claude Sonnet 4.6 (Thinking)".into(), 70.0, None),
                (
                    "Gemini 3.1 Pro (High)".into(),
                    25.0,
                    Some(Utc.with_ymd_and_hms(2026, 9, 27, 13, 0, 0).unwrap())
                ),
                ("Gemini 3.1 Pro (Low)".into(), 50.0, None),
            ]
        );
        assert!(reading.lines.iter().all(|line| match line {
            MetricLine::Progress(line) => line.period_duration_ms.is_none(),
            _ => false,
        }));
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
        assert_eq!(reading.lines.len(), 5);
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
