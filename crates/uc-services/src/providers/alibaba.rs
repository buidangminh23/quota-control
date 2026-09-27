//! Alibaba Model Studio: the request quotas of a Coding Plan, its Session (5-hour), Weekly and
//! Monthly windows, read with a Coding Plan API key. Nothing is read from disk on Windows, macOS
//! or Linux: the key comes from the `ALIBABA_CODING_PLAN_API_KEY` or `DASHSCOPE_API_KEY`
//! environment variable or from a key saved in Quota Control.
//!
//! A refresh asks a Model Studio console for the plan with `POST <console>/data/api.json`, the
//! `queryCodingPlanInstanceInfoV2` action of the `broadscope-bailian` product. There are two
//! consoles: the international one, `https://modelstudio.console.alibabacloud.com` (region
//! `ap-southeast-1`), and `https://bailian.console.aliyun.com` (region `cn-beijing`). The key goes
//! out as a bearer token and in the `x-api-key` and `X-DashScope-API-Key` headers, together with
//! the console's `Origin` and `Referer` and the region's Coding Plan commodity code in the JSON
//! body. The console that last gave a reading is remembered for 12 hours and asked first (the
//! international one when none is remembered); when it refuses the key with HTTP 401 or 403, the
//! other console is asked in the same refresh, so a refresh sends one or two requests.
//!
//! The answer's active plan instance (the first whose status is `VALID`, `ACTIVE`, `NORMAL` or
//! `RUNNING`, else the first listed) gives the plan name. The quotas come from its
//! `codingPlanQuotaInfo`, or from the answer's own when the instance has none: for each window,
//! the requests used, the request limit and, when the answer includes it, the time it refills.

use async_trait::async_trait;
use serde_json::{Value, json};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Alibaba;

const NAME: &str = "Alibaba Model Studio";
const HOSTS: [&str; 2] = [
    "https://modelstudio.console.alibabacloud.com",
    "https://bailian.console.aliyun.com",
];
const REGIONS: [&str; 2] = ["ap-southeast-1", "cn-beijing"];

/// The Coding Plan query of the console at `index` in [`HOSTS`], for that console's region.
fn url(index: usize) -> String {
    format!(
        "{}/data/api.json?action=zeldaEasy.broadscope-bailian.codingPlan.queryCodingPlanInstanceInfoV2&product=broadscope-bailian&api=queryCodingPlanInstanceInfoV2&currentRegionId={}",
        HOSTS[index], REGIONS[index]
    )
}

#[async_trait]
impl Service for Alibaba {
    fn id(&self) -> &'static str {
        "alibaba"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Coding Plan API key"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["ALIBABA_CODING_PLAN_API_KEY", "DASHSCOPE_API_KEY"],
            url: "https://modelstudio.console.alibabacloud.com",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [
            ("session", "Session", 5 * lines::HOUR_MS),
            ("weekly", "Weekly", lines::WEEK_MS),
            ("monthly", "Monthly", lines::MONTH_MS),
        ]
        .into_iter()
        .map(|(id, title, period)| {
            WidgetDescriptor::bounded_count(
                format!("{}.{id}", provider.id),
                provider,
                title,
                None,
                0.0,
                "requests",
                Some(period),
            )
            .exporting_progress(id, "percent")
        })
        .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Alibaba Coding Plan API key is missing."))?;
        let first = context
            .memo
            .get("alibaba-region", context.now)
            .await
            .and_then(|remembered| remembered.as_u64())
            .filter(|index| *index < 2)
            .unwrap_or(0) as usize;
        for index in [first, 1 - first] {
            let commodity = if index == 0 {
                "sfm_codingplan_public_intl"
            } else {
                "sfm_codingplan_public_cn"
            };
            let body = json!({"queryCodingPlanInstanceInfoRequest":{"commodityCode":commodity}});
            let request = HttpRequest::post(url(index))
                .bearer(key)
                .header("x-api-key", key)
                .header("X-DashScope-API-Key", key)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .header("Origin", HOSTS[index])
                .header(
                    "Referer",
                    format!(
                        "{}/{}/?tab={}#/efm/coding_plan",
                        HOSTS[index],
                        REGIONS[index],
                        if index == 0 { "coding-plan" } else { "model" }
                    ),
                )
                .json_body(&body);
            let response = http::send(context.http, request, NAME).await?;
            if matches!(response.status, 401 | 403) && index == first {
                continue;
            }
            if !response.is_success() {
                return Err(http::status_error(&response, NAME));
            }
            let reading = parse(&http::parse(&response, NAME)?)?;
            context
                .memo
                .put(
                    "alibaba-region",
                    json!(index),
                    Some(context.now + chrono::Duration::hours(12)),
                )
                .await;
            return Ok(reading);
        }
        Err(http::expired(
            "Alibaba Model Studio refused the saved login or key. Sign in again or replace the key.",
        ))
    }
}

/// The value of the first of `keys` in `node`, else the first one found in its fields or items,
/// searched depth-first no more than 24 levels down.
fn find<'a>(node: &'a Value, keys: &[&str], depth: usize) -> Option<&'a Value> {
    if depth > 24 {
        return None;
    }
    if let Some(object) = node.as_object() {
        for key in keys {
            if let Some(found) = object.get(*key) {
                return Some(found);
            }
        }
        for item in object.values() {
            if let Some(found) = find(item, keys, depth + 1) {
                return Some(found);
            }
        }
    } else if let Some(array) = node.as_array() {
        for item in array {
            if let Some(found) = find(item, keys, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

/// The request quotas of the active plan instance, one row per window the answer fills, and its
/// plan name.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    let active = find(
        body,
        &["codingPlanInstanceInfos", "coding_plan_instance_infos"],
        0,
    )
    .and_then(Value::as_array)
    .and_then(|instances| {
        instances
            .iter()
            .find(|instance| {
                ["VALID", "ACTIVE", "NORMAL", "RUNNING"].contains(
                    &value::text(instance, "/status")
                        .unwrap_or("")
                        .to_uppercase()
                        .as_str(),
                )
            })
            .or_else(|| instances.first())
    });
    let source = active
        .filter(|instance| {
            find(
                instance,
                &["codingPlanQuotaInfo", "coding_plan_quota_info"],
                0,
            )
            .is_some()
        })
        .unwrap_or(body);
    let quota = find(
        source,
        &["codingPlanQuotaInfo", "coding_plan_quota_info"],
        0,
    )
    .unwrap_or(source);
    let mut rows = Vec::new();
    for (label, used, limit, reset, period) in [
        (
            "Session",
            ["per5HourUsedQuota", "perFiveHourUsedQuota"],
            ["per5HourTotalQuota", "perFiveHourTotalQuota"],
            [
                "per5HourQuotaNextRefreshTime",
                "perFiveHourQuotaNextRefreshTime",
            ],
            5 * lines::HOUR_MS,
        ),
        (
            "Weekly",
            ["perWeekUsedQuota", "perWeekUsedQuota"],
            ["perWeekTotalQuota", "perWeekTotalQuota"],
            ["perWeekQuotaNextRefreshTime", "perWeekQuotaNextRefreshTime"],
            lines::WEEK_MS,
        ),
        (
            "Monthly",
            ["perBillMonthUsedQuota", "perMonthUsedQuota"],
            ["perBillMonthTotalQuota", "perMonthTotalQuota"],
            [
                "perBillMonthQuotaNextRefreshTime",
                "perMonthQuotaNextRefreshTime",
            ],
            lines::MONTH_MS,
        ),
    ] {
        let first_number = |keys: [&str; 2]| {
            keys.into_iter()
                .find_map(|k| value::number(quota, &format!("/{k}")))
        };
        if let (Some(used), Some(limit)) = (first_number(used), first_number(limit)) {
            if used < 0.0 || limit < 0.0 {
                return Err(http::decoding(NAME));
            }
            let reset = reset
                .into_iter()
                .find_map(|k| value::time(quota, &format!("/{k}")));
            rows.push(lines::count(
                label,
                used,
                limit,
                "requests",
                reset,
                Some(period),
            ));
        }
    }
    if rows.is_empty() {
        return Err(http::not_available(
            "No Coding Plan quotas were returned. Token Plan usage requires a browser session that this API key reader does not support.",
        ));
    }
    let plan = find(
        active.unwrap_or(body),
        &["planName", "plan_name", "packageName", "package_name"],
        0,
    )
    .and_then(Value::as_str)
    .and_then(lines::plan_name);
    Ok(Reading::new(plan, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use uc_core::ErrorCategory;

    const PRO_PLAN: &str = r#"{"data":{"codingPlanInstanceInfos":[{"planName":"pro"}],"codingPlanQuotaInfo":{"per5HourUsedQuota":52,"per5HourTotalQuota":1000,"per5HourQuotaNextRefreshTime":1700000300000,"perWeekUsedQuota":800,"perWeekTotalQuota":5000,"perBillMonthUsedQuota":1200,"perBillMonthTotalQuota":20000}}}"#;

    #[tokio::test]
    async fn a_key_the_international_console_refuses_is_read_in_cn_beijing_and_remembered() {
        let http = Scripted::new()
            .on("POST", HOSTS[0], 401, "")
            .on("POST", HOSTS[1], 200, PRO_PLAN);
        let scope = context_at(
            &http,
            json!({"apiKey":"test"}),
            Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap(),
        );
        let reading = Alibaba.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::count(
                        "Session",
                        52.0,
                        1000.0,
                        "requests",
                        chrono::DateTime::from_timestamp(1700000300, 0),
                        Some(5 * lines::HOUR_MS)
                    ),
                    lines::count(
                        "Weekly",
                        800.0,
                        5000.0,
                        "requests",
                        None,
                        Some(lines::WEEK_MS)
                    ),
                    lines::count(
                        "Monthly",
                        1200.0,
                        20000.0,
                        "requests",
                        None,
                        Some(lines::MONTH_MS)
                    )
                ]
            )
        );
        Alibaba.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].url, url(1));
        assert_eq!(header(&requests[1], "Authorization"), Some("Bearer test"));
        assert_eq!(header(&requests[1], "X-DashScope-API-Key"), Some("test"));
        assert_eq!(
            serde_json::from_slice::<Value>(requests[1].body.as_ref().unwrap()).unwrap(),
            json!({"queryCodingPlanInstanceInfoRequest":{"commodityCode":"sfm_codingplan_public_cn"}})
        );
    }

    #[tokio::test]
    async fn failed_or_empty_answers_keep_their_categories_without_the_body() {
        for (status, body, category) in [
            (401, "secret", ErrorCategory::AuthExpired),
            (403, "secret", ErrorCategory::AuthExpired),
            (429, "", ErrorCategory::RateLimited),
            (500, "", ErrorCategory::Http5xx),
            (200, "{}", ErrorCategory::NotAvailable),
            (200, "bad json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new()
                .on("POST", HOSTS[0], status, body)
                .on("POST", HOSTS[1], status, body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let error = Alibaba.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Alibaba.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
