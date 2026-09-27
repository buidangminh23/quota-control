//! Doubao: how much of a Volcengine account's Coding Plan and Agent Plan is used, as the share of
//! each plan's 5-hour, weekly and monthly windows, plus the Agent Plan's daily window.
//!
//! Nothing is read from disk on Windows, macOS or Linux, and nothing is renewed: the account's
//! access key ID, its secret access key and an optional region (`cn-beijing` when left empty) are
//! saved in Accounts. The ID and the region may hold only ASCII letters, digits, `-` and `_`. An
//! Ark API key cannot read plan usage, so a key saved without an access key ID is refused before
//! anything is sent; no Ark API key is created and `arkcli` is never run.
//!
//! A refresh sends two empty-body `POST`s to `https://open.volcengineapi.com` in this order, each
//! signed with Volcengine's `HMAC-SHA256` scheme for the `ark` service in the saved region:
//! - `/?Action=GetCodingPlanUsage&Version=2024-01-01` for the Coding Plan's `QuotaUsage` levels
//!   (session, weekly and monthly percents, with reset times in seconds);
//! - `/?Action=GetAFPUsage&Version=2024-01-01` for the Agent Plan's `AFPFiveHour`, `AFPWeekly`,
//!   `AFPMonthly` and `AFPDaily` windows (`Used` of `Quota`, with reset times in milliseconds).
//!
//! A failed Coding Plan answer stops the refresh before the second request, while a failed Agent
//! Plan answer is dropped when the Coding Plan has rows. An error in an answer's
//! `ResponseMetadata` reads, by its code, as a refused key, as rate limiting or as usage Doubao
//! could not return.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uc_core::{
    HttpRequest, MetricLine, Provider, ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines};

pub(crate) struct Doubao;

const NAME: &str = "Doubao";
const HOST: &str = "open.volcengineapi.com";
const CONTENT_TYPE: &str = "application/x-www-form-urlencoded; charset=utf-8";
const SIGNED: &str = "content-type;host;x-content-sha256;x-date";

const MISSING_ACCESS_KEY_ID: &str = "Enter a Volcengine access key ID and secret access key. Ark API keys cannot read plan usage here.";
const ACCESS_REFUSED: &str =
    "Doubao refused the access key or its plan permissions. Check the saved credentials.";

#[async_trait]
impl Service for Doubao {
    fn id(&self) -> &'static str {
        "doubao"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Secret access key"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://console.volcengine.com/iam/keymanage/",
            fields: &[
                ("accessKeyId", "Access key ID"),
                ("region", "Region (default: cn-beijing)"),
            ],
        })
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![ProviderLink::new(
            "Usage",
            "https://console.volcengine.com/ark/region:ark+cn-beijing/openManagement",
        )]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [
            ("session", "Session"),
            ("weekly", "Weekly"),
            ("monthly", "Monthly"),
            ("agentSession", "Agent Session"),
            ("agentWeekly", "Agent Weekly"),
            ("agentMonthly", "Agent Monthly"),
        ]
        .into_iter()
        .map(|(s, title)| {
            WidgetDescriptor::percent(format!("{}.{s}", provider.id), provider, title, None, None)
                .exporting_progress(s, "percent")
        })
        .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let id = context
            .secret
            .str("/accessKeyId")
            .ok_or_else(|| http::invalid(MISSING_ACCESS_KEY_ID))?;
        let secret = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("Enter the Doubao secret access key."))?;
        let region = context.secret.str("/region").unwrap_or("cn-beijing");
        if !safe_header_id(id) || !safe_header_id(region) {
            return Err(http::invalid("Check the Doubao access key ID and region."));
        }
        let coding = fetch(context, "GetCodingPlanUsage", id, secret, region).await?;
        let mut meters = coding_lines(&coding)?;
        let has_coding = !meters.is_empty();
        let mut has_agent = false;
        let agent = fetch(context, "GetAFPUsage", id, secret, region).await;
        match agent.and_then(|body| agent_lines(&body)) {
            Ok(agent) => {
                has_agent = !agent.is_empty();
                meters.extend(agent);
            }
            Err(error) if meters.is_empty() => return Err(error),
            Err(_) => {}
        }
        if meters.is_empty() {
            return Err(http::not_available(
                "No active Doubao Coding or Agent Plan quota is available.",
            ));
        }
        Ok(Reading::new(
            Some(
                match (has_coding, has_agent) {
                    (true, true) => "Coding / Agent Plans",
                    (true, false) => "Coding Plan",
                    _ => "Agent Plan",
                }
                .into(),
            ),
            meters,
        ))
    }
}

/// Whether `text` can go into a signed header as it is: 1 to 128 ASCII letters, digits, `-` or
/// `_`.
fn safe_header_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// HMAC-SHA256 of `message` under `key`.
fn hmac(key: &[u8], message: &[u8]) -> Vec<u8> {
    let key = if key.len() > 64 {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    let mut inner = [0x36u8; 64];
    let mut outer = [0x5cu8; 64];
    for (index, byte) in key.iter().enumerate() {
        inner[index] ^= byte;
        outer[index] ^= byte;
    }
    let mut hash = Sha256::new();
    hash.update(inner);
    hash.update(message);
    let inner = hash.finalize();
    let mut hash = Sha256::new();
    hash.update(outer);
    hash.update(inner);
    hash.finalize().to_vec()
}

/// The empty-body `POST` of `action`, signed with Volcengine's `HMAC-SHA256` scheme: the signing
/// key is the secret run through the date, the region, `ark` and `request` in turn.
fn signed_request(
    action: &str,
    id: &str,
    secret: &str,
    region: &str,
    now: DateTime<Utc>,
) -> HttpRequest {
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();
    let payload = hex(&Sha256::digest([]));
    let query = format!("Action={action}&Version=2024-01-01");
    let canonical = format!(
        "POST\n/\n{query}\ncontent-type:{CONTENT_TYPE}\nhost:{HOST}\nx-content-sha256:{payload}\nx-date:{stamp}\n\n{SIGNED}\n{payload}"
    );
    let scope = format!("{date}/{region}/ark/request");
    let to_sign = format!(
        "HMAC-SHA256\n{stamp}\n{scope}\n{}",
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let mut key = secret.as_bytes().to_vec();
    for part in [date.as_str(), region, "ark", "request"] {
        key = hmac(&key, part.as_bytes());
    }
    let signature = hex(&hmac(&key, to_sign.as_bytes()));
    HttpRequest::post(format!("https://{HOST}/?{query}"))
        .body(vec![])
        .header("Accept", "application/json")
        .header("Content-Type", CONTENT_TYPE)
        .header("Host", HOST)
        .header("X-Date", stamp)
        .header("X-Content-Sha256", payload)
        .header(
            "Authorization",
            format!(
                "HMAC-SHA256 Credential={id}/{scope}, SignedHeaders={SIGNED}, Signature={signature}"
            ),
        )
}

/// The answer to `action`, or the card error the code of its `ResponseMetadata.Error` stands for.
async fn fetch(
    context: &FetchContext<'_>,
    action: &str,
    id: &str,
    secret: &str,
    region: &str,
) -> Result<Value, SimpleProviderError> {
    let body = http::json(
        context.http,
        signed_request(action, id, secret, region, context.now),
        NAME,
    )
    .await?;
    if !body
        .pointer("/ResponseMetadata/Error")
        .is_none_or(Value::is_null)
    {
        return Err(
            match body
                .pointer("/ResponseMetadata/Error/Code")
                .and_then(Value::as_str)
            {
                Some(
                    "AccessDenied"
                    | "SignatureDoesNotMatch"
                    | "InvalidAccessKeyId"
                    | "InvalidAccessKey"
                    | "AuthenticationFailed",
                ) => http::expired(ACCESS_REFUSED),
                Some("Throttling" | "ThrottlingException") => SimpleProviderError::new(
                    uc_core::ErrorCategory::RateLimited,
                    "Doubao is rate limiting usage requests. Waiting before retrying.",
                ),
                _ => http::not_available("Doubao could not return plan usage. Try again later."),
            },
        );
    }
    Ok(body)
}

/// The `Result` of `GetCodingPlanUsage`.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Coding {
    #[serde(default)]
    quota_usage: Vec<Quota>,
}

/// One Coding Plan level: its used percent and, in seconds, when it resets.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Quota {
    level: String,
    percent: f64,
    reset_timestamp: Option<f64>,
}

/// A positive epoch time, counted in milliseconds or else in seconds.
fn epoch(timestamp: Option<f64>, milliseconds: bool) -> Option<DateTime<Utc>> {
    let timestamp = timestamp.filter(|timestamp| timestamp.is_finite() && *timestamp > 0.0)?;
    let millis = if milliseconds {
        timestamp
    } else {
        timestamp * 1000.0
    };
    if millis > i64::MAX as f64 {
        return None;
    }
    DateTime::from_timestamp_millis(millis as i64)
}

/// The Coding Plan's Session, Weekly and Monthly meters, in that order: the first quota of each
/// level counts, and levels of any other name are skipped.
fn coding_lines(body: &Value) -> Result<Vec<MetricLine>, SimpleProviderError> {
    let usage: Coding =
        serde_json::from_value(body["Result"].clone()).map_err(|_| http::decoding(NAME))?;
    let mut selected = std::collections::BTreeMap::new();
    for quota in usage.quota_usage {
        if !quota.percent.is_finite() {
            return Err(http::decoding(NAME));
        }
        let (title, period, order) = match quota.level.to_ascii_lowercase().as_str() {
            "session" | "5-hour" | "five_hour" | "5h" => ("Session", 5 * lines::HOUR_MS, 0),
            "weekly" | "week" => ("Weekly", lines::WEEK_MS, 1),
            "monthly" | "month" => ("Monthly", lines::MONTH_MS, 2),
            _ => continue,
        };
        selected.entry(order).or_insert_with(|| {
            lines::percent(
                title,
                quota.percent,
                epoch(quota.reset_timestamp, false),
                Some(period),
            )
        });
    }
    Ok(selected.into_values().collect())
}

/// One Agent Plan window: `Used` of `Quota`, resetting at `ResetTime` in milliseconds.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AgentWindow {
    quota: f64,
    used: f64,
    reset_time: Option<f64>,
}

/// The Agent Plan's Session, Weekly and Monthly meters, then its Daily one. A window without a
/// positive quota is left out.
fn agent_lines(body: &Value) -> Result<Vec<MetricLine>, SimpleProviderError> {
    let result = body["Result"]
        .as_object()
        .ok_or_else(|| http::decoding(NAME))?;
    let mut meters = vec![];
    let mut daily = vec![];
    for (field, title, period) in [
        ("AFPFiveHour", "Agent Session", 5 * lines::HOUR_MS),
        ("AFPWeekly", "Agent Weekly", lines::WEEK_MS),
        ("AFPMonthly", "Agent Monthly", lines::MONTH_MS),
        ("AFPDaily", "Agent Daily", lines::DAY_MS),
    ] {
        let Some(raw) = result.get(field).filter(|raw| !raw.is_null()) else {
            continue;
        };
        let window: AgentWindow =
            serde_json::from_value(raw.clone()).map_err(|_| http::decoding(NAME))?;
        if !window.quota.is_finite() || !window.used.is_finite() {
            return Err(http::decoding(NAME));
        }
        if window.quota <= 0.0 {
            continue;
        }
        let percent = window.used / window.quota * 100.0;
        if !percent.is_finite() {
            return Err(http::decoding(NAME));
        }
        let line = lines::percent(title, percent, epoch(window.reset_time, true), Some(period));
        if title == "Agent Daily" {
            daily.push(line)
        } else {
            meters.push(line)
        }
    }
    meters.extend(daily);
    Ok(meters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const CODING_USAGE: &str = r#"{"Result":{"Status":"Running","QuotaUsage":[{"Level":"session","Percent":12.5,"ResetTimestamp":1782226478},{"Level":"weekly","Percent":3.182143,"ResetTimestamp":0}]}}"#;
    const AGENT_USAGE: &str = r#"{"Result":{"AFPFiveHour":{"Quota":100,"Used":20,"ResetTime":1782226478000},"AFPDaily":{"Quota":200,"Used":50,"ResetTime":-1}}}"#;
    const SIGNED_AUTHORIZATION: &str = "HMAC-SHA256 Credential=AKLTTEST/20260617/cn-beijing/ark/request, SignedHeaders=content-type;host;x-content-sha256;x-date, Signature=220f360943ab513c639db31ee72aeee7fa8b915812cde28ce104d6496b0bd24d";

    fn now() -> DateTime<Utc> {
        "2026-06-17T00:00:00Z".parse().unwrap()
    }

    fn secret() -> Value {
        json!({"apiKey":"secret","accessKeyId":"AKLTTEST"})
    }

    #[tokio::test]
    async fn both_plans_are_read_with_two_signed_empty_posts_and_the_daily_window_last() {
        let http = Scripted::new()
            .on(
                "POST",
                "https://open.volcengineapi.com/?Action=GetCodingPlanUsage",
                200,
                CODING_USAGE,
            )
            .on(
                "POST",
                "https://open.volcengineapi.com/?Action=GetAFPUsage",
                200,
                AGENT_USAGE,
            );
        let scope = context_at(&http, secret(), now());
        let reading = Doubao.fetch(&scope.context()).await.unwrap();
        let reset = DateTime::from_timestamp(1782226478, 0);
        assert_eq!(reading.plan.as_deref(), Some("Coding / Agent Plans"));
        assert_eq!(
            reading.lines,
            vec![
                lines::percent("Session", 12.5, reset, Some(5 * lines::HOUR_MS)),
                lines::percent("Weekly", 3.182143, None, Some(lines::WEEK_MS)),
                lines::percent("Agent Session", 20.0, reset, Some(5 * lines::HOUR_MS)),
                lines::percent("Agent Daily", 25.0, None, Some(lines::DAY_MS))
            ]
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].body.as_deref(), Some(&[][..]));
        assert_eq!(header(&requests[0], "X-Date"), Some("20260617T000000Z"));
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some(SIGNED_AUTHORIZATION)
        );
    }

    #[tokio::test]
    async fn the_coding_plan_survives_a_failed_agent_answer_and_an_agent_plan_reads_alone() {
        for (coding, agent, status, expected) in [
            (
                r#"{"Result":{"QuotaUsage":[{"Level":"session","Percent":7}]}}"#,
                "secret-echo",
                503,
                lines::percent("Session", 7.0, None, Some(5 * lines::HOUR_MS)),
            ),
            (
                r#"{"Result":{"Status":"Reclaimed"}}"#,
                r#"{"Result":{"AFPWeekly":{"Quota":200,"Used":50}}}"#,
                200,
                lines::percent("Agent Weekly", 25.0, None, Some(lines::WEEK_MS)),
            ),
        ] {
            let http = Scripted::new()
                .on(
                    "POST",
                    "https://open.volcengineapi.com/?Action=GetCodingPlanUsage",
                    200,
                    coding,
                )
                .on(
                    "POST",
                    "https://open.volcengineapi.com/?Action=GetAFPUsage",
                    status,
                    agent,
                );
            let scope = context_at(&http, secret(), now());
            assert_eq!(
                Doubao.fetch(&scope.context()).await.unwrap().lines,
                vec![expected]
            );
        }
    }

    #[tokio::test]
    async fn failed_coding_answers_stop_the_refresh_and_an_ark_key_alone_sends_nothing() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (403, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
        ] {
            let http = Scripted::new().on(
                "POST",
                "https://open.volcengineapi.com/?Action=GetCodingPlanUsage",
                status,
                "secret-echo",
            );
            let scope = context_at(&http, secret(), now());
            let error = Doubao.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret-echo"));
            assert_eq!(http.requests().len(), 1);
        }
        let http = Scripted::new();
        let scope = context_at(&http, json!({"apiKey":"ark-key"}), now());
        assert_eq!(
            Doubao.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }

    #[test]
    fn malformed_windows_fail_and_absent_or_zero_quota_windows_add_no_rows() {
        assert!(
            coding_lines(&json!({"Result":{"QuotaUsage":[{"Level":"session","Percent":"bad"}]}}))
                .is_err()
        );
        assert!(agent_lines(&json!({"Result":{"AFPWeekly":{"Quota":100}}})).is_err());
        assert_eq!(
            coding_lines(&json!({"Result":{"Status":"Reclaimed"}})).unwrap(),
            vec![]
        );
        assert!(
            agent_lines(&json!({"Result":{"AFPWeekly":{"Quota":0,"Used":0}}}))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn hmac_matches_standard_vector() {
        assert_eq!(
            hex(&hmac(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }
}
