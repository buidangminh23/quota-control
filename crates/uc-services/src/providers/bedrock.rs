//! AWS Bedrock: this month's Bedrock spend from Cost Explorer and the last 14 days of Claude
//! activity from CloudWatch.
//!
//! The card finds the AWS CLI's credentials the same way on Windows, macOS and Linux. The profile
//! named by `AWS_PROFILE` or `AWS_DEFAULT_PROFILE` (else `default`) is read from `~/.aws/config`
//! and then `~/.aws/credentials`, or from the files `AWS_CONFIG_FILE` and
//! `AWS_SHARED_CREDENTIALS_FILE` name, and the region comes from `AWS_REGION`,
//! `AWS_DEFAULT_REGION` or the profile, else `us-east-1`. A profile with a session token whose
//! expiry has passed or cannot be read is skipped. An access key pair can also be pasted into
//! Accounts, with its region and an optional session token.
//!
//! A refresh sends at most four requests, each signed with AWS Signature Version 4, and none when
//! the saved session token has expired. Cost Explorer's `GetCostAndUsage`, sent to
//! `https://ce.us-east-1.amazonaws.com`, gives this month's unblended cost per service over at
//! most three pages; each service whose name contains "Bedrock" gets its own spend row, and their
//! sum is Spend This Month. An answer that needs more pages is reported as unavailable rather
//! than shown in part. CloudWatch's `GetMetricData`, sent to `https://monitoring.<region>.<domain>`
//! (`amazonaws.com`, or the partition's own domain for regions such as `cn-north-1`), then gives
//! the input tokens, output tokens and invocations of Claude models, summed over the last 14 days,
//! for the Tokens and Requests rows. When CloudWatch fails or answers incompletely, the spend is
//! still shown, with a warning and without those two rows.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Utc};
use ring::hmac;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{http, lines, value};

pub(crate) struct Bedrock;

const NAME: &str = "AWS Bedrock";
const CLOUDWATCH_WARNING: &str =
    "CloudWatch activity is unavailable or incomplete; monthly spend is current.";
const EXPIRED: &str = "The AWS CLI login expired. Open AWS CLI once to renew it.";

#[async_trait]
impl Service for Bedrock {
    fn id(&self) -> &'static str {
        "bedrock"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Secret access key"
    }

    fn connection(&self) -> Connection {
        Connection::login("AWS CLI").or_api_key(ApiKeyHelp {
            env: &[],
            url: "https://console.aws.amazon.com/iam/",
            fields: &[
                ("accessKeyId", "Access key ID"),
                ("region", "AWS region"),
                ("sessionToken", "Session token (optional)"),
            ],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let profile = roots
            .var("AWS_PROFILE")
            .or_else(|| roots.var("AWS_DEFAULT_PROFILE"))
            .unwrap_or_else(|| "default".into());
        let credentials_path = roots.dir_from(
            "AWS_SHARED_CREDENTIALS_FILE",
            roots.home.join(".aws/credentials"),
        );
        let config_path = roots.dir_from("AWS_CONFIG_FILE", roots.home.join(".aws/config"));
        let mut settings = profile_values(
            &config_path,
            &if profile == "default" {
                profile.clone()
            } else {
                format!("profile {profile}")
            },
        );
        settings.extend(profile_values(&credentials_path, &profile));
        let Some(access) = settings.get("aws_access_key_id") else {
            return vec![];
        };
        let Some(secret) = settings.get("aws_secret_access_key") else {
            return vec![];
        };
        let token = settings.get("aws_session_token");
        let expiry = [
            "expiration",
            "aws_session_expiration",
            "aws_credentials_expiration",
        ]
        .into_iter()
        .find_map(|name| settings.get(name));
        if token.is_some()
            && expiry.is_some_and(|text| {
                value::as_time(&json!(text)).is_none_or(|expires_at| expires_at <= Utc::now())
            })
        {
            return vec![];
        }
        let region = roots
            .var("AWS_REGION")
            .or_else(|| roots.var("AWS_DEFAULT_REGION"))
            .or_else(|| settings.get("region").cloned())
            .unwrap_or_else(|| "us-east-1".into());
        let location = if credentials_path.is_file() {
            &credentials_path
        } else {
            &config_path
        };
        let saved = Secret::new(json!({
            "apiKey": secret,
            "accessKeyId": access,
            "region": region,
            "sessionToken": token,
            "expiresAt": expiry
        }));
        vec![Login::new(access, "AWS CLI", location, saved).with_label(Some(profile))]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::values(
                format!("{}.monthly", provider.id),
                provider,
                "Spend This Month",
                None,
                Some(uc_core::MetricKind::Dollars),
                None,
                false,
                None,
                false,
            ),
            WidgetDescriptor::values(
                format!("{}.tokens", provider.id),
                provider,
                "Tokens",
                None,
                Some(uc_core::MetricKind::Count),
                None,
                false,
                None,
                false,
            ),
            WidgetDescriptor::values(
                format!("{}.requests", provider.id),
                provider,
                "Requests",
                None,
                Some(uc_core::MetricKind::Count),
                None,
                false,
                None,
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let secret = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The AWS secret access key is missing."))?;
        let access = context
            .secret
            .str("/accessKeyId")
            .ok_or_else(|| http::invalid("The AWS access key ID is missing."))?;
        let region = context.secret.str("/region").unwrap_or("us-east-1");
        let host = cloudwatch_host(region)?;
        let token = context.secret.str("/sessionToken");
        if token.is_some()
            && value::time(context.secret.value(), "/expiresAt")
                .is_some_and(|expires_at| expires_at <= context.now)
        {
            return Err(http::expired(EXPIRED));
        }
        let credentials = Credentials {
            access,
            secret,
            token,
        };
        let start = context
            .now
            .with_day(1)
            .ok_or_else(|| http::decoding(NAME))?
            .format("%Y-%m-%d")
            .to_string();
        let end = (context.now + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let mut costs = BTreeMap::<String, f64>::new();
        let mut next_page = None;
        let mut seen_pages = BTreeSet::new();
        let mut calls = 0;
        loop {
            let mut body = json!({
                "TimePeriod": {"Start": start, "End": end},
                "Granularity": "MONTHLY",
                "Metrics": ["UnblendedCost"],
                "GroupBy": [{"Type": "DIMENSION", "Key": "SERVICE"}]
            });
            if let Some(ref cursor) = next_page {
                body["NextPageToken"] = json!(cursor)
            }
            let request = signed(
                &credentials,
                context.now,
                "ce.us-east-1.amazonaws.com",
                "us-east-1",
                "ce",
                "AWSInsightsIndexService.GetCostAndUsage",
                &body,
            );
            let response = http::send(context.http, request, NAME).await?;
            calls += 1;
            if !response.is_success() {
                if response.status == 401 || response.status == 403 {
                    return Err(http::expired(EXPIRED));
                }
                return Err(http::status_error(&response, NAME));
            }
            let page = http::parse(&response, NAME)?;
            let rows = page
                .get("ResultsByTime")
                .and_then(Value::as_array)
                .ok_or_else(|| http::decoding(NAME))?;
            for row in rows {
                let groups = row
                    .get("Groups")
                    .and_then(Value::as_array)
                    .ok_or_else(|| http::decoding(NAME))?;
                for group in groups {
                    let keys = group
                        .get("Keys")
                        .and_then(Value::as_array)
                        .ok_or_else(|| http::decoding(NAME))?;
                    let Some(name) = keys
                        .iter()
                        .filter_map(Value::as_str)
                        .find(|key| key.to_lowercase().contains("bedrock"))
                    else {
                        continue;
                    };
                    let amount = value::number(group, "/Metrics/UnblendedCost/Amount")
                        .ok_or_else(|| http::decoding(NAME))?;
                    if value::text(group, "/Metrics/UnblendedCost/Unit") != Some("USD") {
                        return Err(http::decoding(NAME));
                    }
                    let sum = costs.entry(name.to_string()).or_default();
                    *sum += amount;
                    if !sum.is_finite() {
                        return Err(http::decoding(NAME));
                    }
                }
            }
            next_page = value::text(&page, "/NextPageToken")
                .filter(|token| !token.is_empty())
                .map(str::to_string);
            if let Some(ref cursor) = next_page {
                if calls >= 3 || !seen_pages.insert(cursor.clone()) {
                    return Err(http::not_available(
                        "Cost Explorer returned more pages than this refresh can safely read. No partial spend is shown.",
                    ));
                }
            } else {
                break;
            }
        }
        let total: f64 = costs.values().sum();
        if !total.is_finite() {
            return Err(http::decoding(NAME));
        }
        let mut found = vec![lines::dollar_value("Spend This Month", total)];
        let mut reading = Reading::new(None, vec![]);
        let queries: [(&str, &str); 3] = [
            ("inputTokens", "InputTokenCount"),
            ("outputTokens", "OutputTokenCount"),
            ("requests", "Invocations"),
        ];
        let query_values: Vec<Value> = queries
            .iter()
            .map(|(id, metric)| {
                let expression = format!(
                    "SUM(SEARCH('{{AWS/Bedrock,ModelId}} MetricName=\"{metric}\" claude', 'Sum', 86400))"
                );
                json!({"Id": id, "Expression": expression, "ReturnData": true})
            })
            .collect();
        let mut body = json!({
            "StartTime": (context.now - chrono::Duration::days(14)).timestamp(),
            "EndTime": context.now.timestamp(),
            "ScanBy": "TimestampAscending",
            "MetricDataQueries": query_values
        });
        let mut totals = BTreeMap::<String, f64>::new();
        let mut seen_activity_pages = BTreeSet::new();
        loop {
            let request = signed(
                &credentials,
                context.now,
                &host,
                region,
                "monitoring",
                "GraniteServiceVersion20100801.GetMetricData",
                &body,
            );
            let answer = http::json(context.http, request, NAME).await;
            calls += 1;
            match answer.and_then(|page| parse_activity(&page).map(|values| (page, values))) {
                Ok((page, values)) => {
                    for (id, amount) in values {
                        *totals.entry(id).or_default() += amount;
                    }
                    if totals
                        .values()
                        .any(|total| !total.is_finite() || *total > 9007199254740991.0)
                    {
                        reading.warning = Some(CLOUDWATCH_WARNING.into());
                        break;
                    }
                    if let Some(cursor) =
                        value::text(&page, "/NextToken").filter(|token| !token.is_empty())
                    {
                        if calls >= 4 || !seen_activity_pages.insert(cursor.to_string()) {
                            reading.warning = Some(CLOUDWATCH_WARNING.into());
                            break;
                        }
                        body["NextToken"] = json!(cursor);
                        continue;
                    }
                    if let (Some(input), Some(output)) =
                        (totals.get("inputTokens"), totals.get("outputTokens"))
                    {
                        found.push(lines::values(
                            "Tokens",
                            vec![
                                uc_core::MetricValue::count(*input, "Claude input (14 days)"),
                                uc_core::MetricValue::count(*output, "Claude output (14 days)"),
                            ],
                        ));
                    }
                    if let Some(requests) = totals.get("requests") {
                        found.push(lines::count_value(
                            "Requests",
                            *requests,
                            "Claude requests (14 days)",
                        ));
                    }
                    break;
                }
                Err(_) => {
                    reading.warning = Some(CLOUDWATCH_WARNING.into());
                    break;
                }
            }
        }
        found.extend(
            costs
                .into_iter()
                .map(|(name, amount)| lines::dollar_value(&name, amount)),
        );
        reading.lines = found;
        Ok(reading)
    }
}

/// The settings in the section of an AWS CLI config or credentials file whose header names
/// `profile`, keyed by lower-case name. Empty when the file is missing, larger than 1 MiB,
/// unreadable or without that section.
fn profile_values(path: &Path, profile: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    if !std::fs::metadata(path).is_ok_and(|metadata| metadata.len() <= 1024 * 1024) {
        return values;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return values;
    };
    let mut active = false;
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.starts_with('[') && line.ends_with(']') {
            active = line[1..line.len() - 1].trim() == profile;
            continue;
        }
        if !active || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some((name, setting)) = line.split_once('=') {
            let setting = setting.trim();
            if !setting.is_empty() {
                values.insert(name.trim().to_lowercase(), setting.into());
            }
        }
    }
    values
}

/// The CloudWatch host of `region`, on its partition's domain. A region that is not at least
/// three dash-separated parts of lower-case letters and digits, the last one a number, is refused
/// before anything is sent.
fn cloudwatch_host(region: &str) -> Result<String, SimpleProviderError> {
    let parts: Vec<&str> = region.split('-').collect();
    if parts.len() < 3
        || parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
        || !parts
            .last()
            .is_some_and(|last| last.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(http::invalid("The AWS region is invalid."));
    }
    let suffix = if region.starts_with("cn-") {
        "amazonaws.com.cn"
    } else if region.starts_with("eusc-") {
        "amazonaws.eu"
    } else if region.starts_with("us-iso-") {
        "c2s.ic.gov"
    } else if region.starts_with("us-isob-") {
        "sc2s.sgov.gov"
    } else if region.starts_with("eu-isoe-") {
        "cloud.adc-e.uk"
    } else if region.starts_with("us-isof-") {
        "csp.hci.ic.gov"
    } else {
        "amazonaws.com"
    };
    Ok(format!("monitoring.{region}.{suffix}"))
}

/// The summed values of each query of a CloudWatch `GetMetricData` answer. An answer with
/// messages, an unknown query id, a query that is not complete or a value that is not a finite,
/// non-negative number is a decoding error.
fn parse_activity(body: &Value) -> Result<BTreeMap<String, f64>, SimpleProviderError> {
    if body
        .get("Messages")
        .and_then(Value::as_array)
        .is_some_and(|messages| !messages.is_empty())
    {
        return Err(http::decoding(NAME));
    }
    let rows = body
        .get("MetricDataResults")
        .and_then(Value::as_array)
        .ok_or_else(|| http::decoding(NAME))?;
    let mut totals = BTreeMap::<String, f64>::new();
    for row in rows {
        let id = value::text(row, "/Id")
            .filter(|id| ["inputTokens", "outputTokens", "requests"].contains(id))
            .ok_or_else(|| http::decoding(NAME))?;
        if value::text(row, "/StatusCode") != Some("Complete") {
            return Err(http::decoding(NAME));
        }
        let values = row
            .get("Values")
            .and_then(Value::as_array)
            .ok_or_else(|| http::decoding(NAME))?;
        let sum = totals.entry(id.into()).or_default();
        for datapoint in values {
            let amount = datapoint
                .as_f64()
                .filter(|amount| amount.is_finite() && *amount >= 0.0)
                .ok_or_else(|| http::decoding(NAME))?;
            *sum += amount;
        }
    }
    Ok(totals)
}

struct Credentials<'a> {
    access: &'a str,
    secret: &'a str,
    token: Option<&'a str>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 digest of `bytes`, in lower-case hex.
fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The HMAC-SHA256 of `text` under `key`.
fn mac(key: &[u8], text: &str) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), text.as_bytes())
        .as_ref()
        .to_vec()
}

/// A POST of `body` to `https://<host>` for the JSON API operation `target`, signed with AWS
/// Signature Version 4 for `region` and `service` at `now`.
fn signed(
    credentials: &Credentials<'_>,
    now: DateTime<Utc>,
    host: &str,
    region: &str,
    service: &str,
    target: &str,
    body: &Value,
) -> HttpRequest {
    let mut request = HttpRequest::post(format!("https://{host}")).json_body(body);
    let body_hash = hash(request.body.as_deref().unwrap_or_default());
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let day = now.format("%Y%m%d").to_string();
    let mut headers = BTreeMap::from([
        (
            "content-type",
            if service == "ce" {
                "application/x-amz-json-1.1".to_string()
            } else {
                "application/x-amz-json-1.0".to_string()
            },
        ),
        ("host", host.to_string()),
        ("x-amz-content-sha256", body_hash.clone()),
        ("x-amz-date", stamp.clone()),
        ("x-amz-target", target.to_string()),
    ]);
    if let Some(token) = credentials.token {
        headers.insert("x-amz-security-token", token.to_string());
    }
    let signed_headers = headers.keys().copied().collect::<Vec<_>>().join(";");
    let canonical_headers = headers
        .iter()
        .map(|(k, value)| {
            format!(
                "{k}:{}\n",
                value.split_whitespace().collect::<Vec<_>>().join(" ")
            )
        })
        .collect::<String>();
    let canonical_request =
        format!("POST\n/\n\n{canonical_headers}\n{signed_headers}\n{body_hash}");
    let scope = format!("{day}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}",
        hash(canonical_request.as_bytes())
    );
    let key = mac(format!("AWS4{}", credentials.secret).as_bytes(), &day);
    let key = mac(&key, region);
    let key = mac(&key, service);
    let key = mac(&key, "aws4_request");
    let signature = hex(&mac(&key, &string_to_sign));
    request.headers.clear();
    for (name, value) in headers {
        request = request.header(name, value)
    }
    request.header(
        "Authorization",
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            credentials.access
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use uc_core::ErrorCategory;

    const COST_EXPLORER_URL: &str = "https://ce.us-east-1.amazonaws.com";
    const COST: &str = r#"{"ResultsByTime":[{"Groups":[{"Keys":["Claude Sonnet (Bedrock Edition)"],"Metrics":{"UnblendedCost":{"Amount":"12.50","Unit":"USD"}}},{"Keys":["Amazon EC2"],"Metrics":{"UnblendedCost":{"Amount":"99","Unit":"USD"}}}]}]}"#;
    const ACTIVITY: &str = r#"{"MetricDataResults":[{"Id":"inputTokens","StatusCode":"Complete","Values":[100,200]},{"Id":"outputTokens","StatusCode":"Complete","Values":[50]},{"Id":"requests","StatusCode":"Complete","Values":[3]}]}"#;
    const TWO_PROFILES: &str = "[default]\naws_access_key_id = OTHER\naws_secret_access_key = ignored\n[work]\naws_access_key_id = AKIATEST\naws_secret_access_key = testSecret\n";
    const EXPIRED_SESSION: &str = "[work]\naws_access_key_id = AKIATEST\naws_secret_access_key = testSecret\naws_session_token = session\nexpiration = 2000-01-01T00:00:00Z\n";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn secret() -> Value {
        json!({
            "apiKey": "testSecret",
            "accessKeyId": "AKIATEST",
            "region": "us-east-1",
            "sessionToken": "session"
        })
    }

    #[tokio::test]
    async fn reads_bedrock_spend_and_claude_activity_with_signed_requests() {
        let http = Scripted::new().on("POST", COST_EXPLORER_URL, 200, COST).on(
            "POST",
            "https://monitoring.us-east-1.amazonaws.com",
            200,
            ACTIVITY,
        );
        let scope = context_at(&http, secret(), now());
        let reading = Bedrock.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                None,
                vec![
                    lines::dollar_value("Spend This Month", 12.5),
                    lines::values(
                        "Tokens",
                        vec![
                            uc_core::MetricValue::count(300.0, "Claude input (14 days)"),
                            uc_core::MetricValue::count(50.0, "Claude output (14 days)")
                        ]
                    ),
                    lines::count_value("Requests", 3.0, "Claude requests (14 days)"),
                    lines::dollar_value("Claude Sonnet (Bedrock Edition)", 12.5)
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(header(&requests[0], "x-amz-date"), Some("20260927T100000Z"));
        assert_eq!(
            header(&requests[0], "x-amz-security-token"),
            Some("session")
        );
        assert_eq!(
            header(&requests[0], "Content-Type"),
            Some("application/x-amz-json-1.1")
        );
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some(
                "AWS4-HMAC-SHA256 Credential=AKIATEST/20260927/us-east-1/ce/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token;x-amz-target, Signature=a682638e7471bce4273decf05ef71d3f6e4da0395d3881342e005eb513a37ac5"
            )
        );
        let body: Value = serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({
                "TimePeriod": {"Start": "2026-09-01", "End": "2026-09-28"},
                "Granularity": "MONTHLY",
                "Metrics": ["UnblendedCost"],
                "GroupBy": [{"Type": "DIMENSION", "Key": "SERVICE"}]
            })
        );
        assert!(
            header(&requests[0], "Authorization")
                .unwrap()
                .contains("Credential=AKIATEST/20260927/us-east-1/ce/aws4_request")
        );
    }

    #[tokio::test]
    async fn discovers_the_named_profile_and_skips_one_whose_session_expired() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path())
            .with_var("AWS_PROFILE", "work")
            .with_var("AWS_REGION", "ap-southeast-2");
        assert!(Bedrock.discover(&roots).is_empty());
        std::fs::create_dir_all(dir.path().join(".aws")).unwrap();
        std::fs::write(
            dir.path().join(".aws/config"),
            "[profile work]\nregion = eu-west-1\n",
        )
        .unwrap();
        std::fs::write(dir.path().join(".aws/credentials"), TWO_PROFILES).unwrap();
        let found = Bedrock.discover(&roots);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].identity, "AKIATEST");
        assert_eq!(found[0].secret.str("/region"), Some("ap-southeast-2"));
        assert_eq!(found[0].label.as_deref(), Some("work"));
        std::fs::write(dir.path().join(".aws/credentials"), EXPIRED_SESSION).unwrap();
        assert!(Bedrock.discover(&roots).is_empty());
    }

    #[tokio::test]
    async fn errors_keep_their_categories_and_a_repeated_page_token_stops_paging() {
        for (status, body, category) in [
            (401, "secret", ErrorCategory::AuthExpired),
            (403, "secret", ErrorCategory::AuthExpired),
            (429, "", ErrorCategory::RateLimited),
            (500, "", ErrorCategory::Http5xx),
            (200, "{}", ErrorCategory::Decoding),
            (200, "bad json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("POST", COST_EXPLORER_URL, status, body);
            let scope = context_at(&http, secret(), now());
            let error = Bedrock.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
        let http = Scripted::new().on(
            "POST",
            COST_EXPLORER_URL,
            200,
            r#"{"ResultsByTime":[],"NextPageToken":"same"}"#,
        );
        let scope = context_at(&http, secret(), now());
        assert_eq!(
            Bedrock.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::NotAvailable
        );
        assert_eq!(http.requests().len(), 2);
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), now());
        assert_eq!(
            Bedrock.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
        assert!(cloudwatch_host("us-east-1.evil.test").is_err());
    }

    #[tokio::test]
    async fn incomplete_cloudwatch_never_appears_as_zero() {
        let http = Scripted::new().on("POST", COST_EXPLORER_URL, 200, COST).on(
            "POST",
            "https://monitoring.us-east-1.amazonaws.com",
            200,
            r#"{"MetricDataResults":[{"Id":"requests","StatusCode":"PartialData","Values":[1]}]}"#,
        );
        let scope = context_at(&http, secret(), now());
        let reading = Bedrock.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.warning.as_deref(), Some(CLOUDWATCH_WARNING));
        assert_eq!(reading.lines.len(), 2);
    }
}
