//! StepFun: the rolling session and weekly limits, or the credit pools, of a StepFun plan, and the
//! plan's name.
//!
//! Nothing is read from disk on Windows, macOS or Linux, and the card neither signs in with a
//! password nor renews the login: the user pastes the `Oasis-Token` cookie of a signed-in
//! platform.stepfun.com session, or sets it in `STEPFUN_TOKEN`. The value holds tokens joined by
//! `...`; once the first one has expired the card asks the user to open StepFun again without
//! sending anything, and the device ID comes from a `device_id` claim.
//!
//! A refresh sends the read-only dashboard call
//! `POST https://platform.stepfun.com/api/step.openapi.devcenter.Dashboard/QueryStepPlanRateLimit`,
//! then, at most once every 12 hours, `GetStepPlanStatus` under the same dashboard address for the
//! plan name. Each carries an empty JSON body, the cookie as
//! `Oasis-Token=<token>; Oasis-Webid=<device ID>` and the `oasis-appid`, `oasis-platform` and
//! `oasis-webid` headers. A plan with rolling windows shows its Session (five hours) and Weekly
//! meters; a credit plan shows a monthly Credits meter and a row for each credit pool.

use async_trait::async_trait;
use serde_json::{Value, json};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, jwt, lines, value};

pub(crate) struct StepFun;

const BASE: &str = "https://platform.stepfun.com/api/step.openapi.devcenter.Dashboard";

#[async_trait]
impl Service for StepFun {
    fn id(&self) -> &'static str {
        "stepfun"
    }

    fn name(&self) -> &'static str {
        "StepFun"
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (Oasis-Token)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["STEPFUN_TOKEN"],
            url: "https://platform.stepfun.com",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [
            ("session", "Session"),
            ("weekly", "Weekly"),
            ("credits", "Credits"),
        ]
        .into_iter()
        .map(|(id, title)| {
            WidgetDescriptor::percent(format!("{}.{id}", provider.id), provider, title, None, None)
                .exporting_progress(id, "percent")
        })
        .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let token = context
            .secret
            .key()
            .filter(|token| {
                !token
                    .chars()
                    .any(|character| character.is_control() || character == ';')
            })
            .ok_or_else(|| http::invalid("The StepFun session token is missing or invalid."))?;
        let access = token.split("...").next().unwrap_or(token);
        if jwt::expires_at(access).is_some_and(|expiry| expiry <= context.now) {
            return Err(http::expired(
                "The StepFun login expired. Open StepFun once to renew it.",
            ));
        }
        let device = token
            .rsplit("...")
            .find_map(|part| jwt::claim(part, &["device_id"]))
            .ok_or_else(|| {
                http::invalid(
                    "The StepFun token has no device ID. Paste the complete Oasis-Token value.",
                )
            })?;
        if device
            .chars()
            .any(|character| character.is_control() || character == ';')
        {
            return Err(http::invalid("The StepFun token has an invalid device ID."));
        }
        let limits = call(context, token, &device, "QueryStepPlanRateLimit").await?;
        let mut rows = vec![];
        let rolling = value::time(&limits, "/five_hour_usage_reset_time").is_some()
            || value::time(&limits, "/weekly_usage_reset_time").is_some();
        if rolling {
            for (field, reset, label, period) in [
                (
                    "five_hour_usage_left_rate",
                    "five_hour_usage_reset_time",
                    "Session",
                    5 * lines::HOUR_MS,
                ),
                (
                    "weekly_usage_left_rate",
                    "weekly_usage_reset_time",
                    "Weekly",
                    lines::WEEK_MS,
                ),
            ] {
                if let Some(left) = value::number(&limits, &format!("/{field}"))
                    && let Some(reset) = value::time(&limits, &format!("/{reset}"))
                {
                    rows.push(lines::percent(
                        label,
                        (1.0 - left) * 100.0,
                        Some(reset),
                        Some(period),
                    ));
                }
            }
        } else if let Some(pool) = limits.get("plan_credit_rate_limit") {
            let buckets = pool.get("credit_buckets").and_then(Value::as_array);
            let mut total = 0.0;
            let mut remaining = 0.0;
            let mut valid = buckets.is_some_and(|buckets| !buckets.is_empty());
            if let Some(buckets) = buckets {
                for bucket in buckets {
                    match (
                        value::number(bucket, "/credit_total"),
                        value::number(bucket, "/credit_residual"),
                    ) {
                        (Some(size), Some(residual))
                            if size > 0.0 && residual >= 0.0 && residual <= size =>
                        {
                            total += size;
                            remaining += residual
                        }
                        _ => valid = false,
                    }
                }
            }
            let left = if valid {
                Some(remaining / total)
            } else {
                value::number(pool, "/subscription_credit_left_rate")
                    .or_else(|| value::number(pool, "/topup_credit_left_rate"))
            };
            if let Some(left) = left {
                rows.push(lines::percent(
                    "Credits",
                    (1.0 - left) * 100.0,
                    value::time(pool, "/subscription_credit_reset_time"),
                    Some(lines::MONTH_MS),
                ));
            }
            if let Some(buckets) = buckets {
                for (index, bucket) in buckets.iter().enumerate() {
                    if let (Some(total), Some(left)) = (
                        value::number(bucket, "/credit_total"),
                        value::number(bucket, "/credit_residual"),
                    ) {
                        rows.push(lines::count(
                            &format!("Credit Pool {}", index + 1),
                            total - left,
                            total,
                            "credits",
                            value::time(bucket, "/next_reset_at")
                                .or_else(|| value::time(bucket, "/expire_at")),
                            None,
                        ));
                    }
                }
            }
        }
        if rows.is_empty() {
            return Err(http::decoding("StepFun"));
        }
        let plan = if let Some(saved) = context.memo.get("stepfun-plan", context.now).await {
            saved
        } else {
            let status = call(context, token, &device, "GetStepPlanStatus").await?;
            context
                .memo
                .put(
                    "stepfun-plan",
                    status.clone(),
                    Some(context.now + chrono::Duration::hours(12)),
                )
                .await;
            status
        };
        Ok(Reading::new(
            value::text(&plan, "/subscription/name").map(str::to_owned),
            rows,
        ))
    }
}

/// Posts the dashboard method `path` and returns its answer when StepFun reports success
/// (`status` 1); a `code` of 401 or 403 means the login expired.
async fn call(
    context: &FetchContext<'_>,
    token: &str,
    device: &str,
    path: &str,
) -> Result<Value, SimpleProviderError> {
    let body = http::json(
        context.http,
        HttpRequest::post(format!("{BASE}/{path}"))
            .header("oasis-appid", "10300")
            .header("oasis-platform", "web")
            .header("oasis-webid", device)
            .header(
                "Cookie",
                format!("Oasis-Token={token}; Oasis-Webid={device}"),
            )
            .json_body(&json!({})),
        "StepFun",
    )
    .await?;
    if value::number(&body, "/status") == Some(1.0) {
        Ok(body)
    } else if matches!(value::number(&body, "/code"), Some(401.0 | 403.0)) {
        Err(http::expired(
            "The StepFun login expired. Open StepFun once to renew it.",
        ))
    } else {
        Err(http::decoding("StepFun"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use base64::Engine;
    use chrono::Utc;
    use uc_core::ErrorCategory;

    const ROLLING_LIMITS: &str = r#"{"status":1,"five_hour_usage_left_rate":0.75,"five_hour_usage_reset_time":"1790503200","weekly_usage_left_rate":0.5,"weekly_usage_reset_time":"1790812800"}"#;
    const CREDIT_LIMITS: &str = r#"{"status":1,"plan_credit_rate_limit":{"credit_buckets":[{"credit_total":100,"credit_residual":25}],"subscription_credit_left_rate":0.9}}"#;

    fn token() -> String {
        format!(
            "h.{}.s",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(r#"{"device_id":"device-test"}"#)
        )
    }

    #[tokio::test]
    async fn a_rolling_plan_shows_session_and_weekly_meters_and_the_plan_name() {
        let http = Scripted::new()
            .on(
                "POST",
                &format!("{BASE}/QueryStepPlanRateLimit"),
                200,
                ROLLING_LIMITS,
            )
            .on(
                "POST",
                &format!("{BASE}/GetStepPlanStatus"),
                200,
                r#"{"status":1,"subscription":{"name":"Step Pro"}}"#,
            );
        let scope = context_at(&http, json!({"apiKey":token()}), Utc::now());
        let reading = StepFun.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Step Pro"));
        assert_eq!(
            reading.lines,
            vec![
                lines::percent(
                    "Session",
                    25.0,
                    value::as_time(&json!(1790503200)),
                    Some(5 * lines::HOUR_MS)
                ),
                lines::percent(
                    "Weekly",
                    50.0,
                    value::as_time(&json!(1790812800)),
                    Some(lines::WEEK_MS)
                )
            ]
        );
        assert_eq!(
            header(&http.requests()[0], "oasis-webid"),
            Some("device-test")
        );
        assert_eq!(http.requests().len(), 2);
    }

    #[tokio::test]
    async fn refused_throttled_or_unsuccessful_answers_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("POST", BASE, status, "{}");
            let scope = context_at(&http, json!({"apiKey":token()}), Utc::now());
            assert_eq!(
                StepFun.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }

    #[tokio::test]
    async fn a_credit_plan_shows_the_credits_used_across_its_pools_and_each_pool() {
        let http = Scripted::new()
            .on(
                "POST",
                &format!("{BASE}/QueryStepPlanRateLimit"),
                200,
                CREDIT_LIMITS,
            )
            .on(
                "POST",
                &format!("{BASE}/GetStepPlanStatus"),
                200,
                r#"{"status":1}"#,
            );
        let scope = context_at(&http, json!({"apiKey":token()}), Utc::now());
        let reading = StepFun.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::percent("Credits", 75.0, None, Some(lines::MONTH_MS)),
                lines::count("Credit Pool 1", 75.0, 100.0, "credits", None, None)
            ]
        );
    }
}
