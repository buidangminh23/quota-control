//! Xiaomi MiMo: the balance of a Xiaomi MiMo platform account, the share of its token plan used
//! this month and the tokens each model has used of its monthly allowance.
//!
//! Nothing is read from disk on Windows, macOS or Linux, and browser cookie stores are never
//! read: the values of the console's `api-platform_serviceToken` and `userId` cookies are pasted
//! into Accounts. A refresh sends three `GET`s to `https://platform.xiaomimimo.com/api/v1`, each
//! with both cookies, the console's `Origin` and `Referer` and `x-timeZone: UTC+00:00`:
//! - `/balance` for the balance and its currency (shown in dollars for USD);
//! - `/tokenPlan/detail` for the plan and the end of the current period;
//! - `/tokenPlan/usage` for the month's share used and each model's `used` of `limit` tokens.
//!
//! Each answer's `code` is 0 beside the `data`, or 401 or 403 once the session has expired.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct MiMo;

const NAME: &str = "Xiaomi MiMo";
const BASE: &str = "https://platform.xiaomimimo.com/api/v1";

#[async_trait]
impl Service for MiMo {
    fn id(&self) -> &'static str {
        "mimo"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (api-platform_serviceToken)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://platform.xiaomimimo.com",
            fields: &[("userId", "User ID cookie (userId)")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::combined(
                format!("{}.balance", provider.id),
                provider,
                "Balance",
                None,
                false,
            ),
            WidgetDescriptor::percent(
                format!("{}.monthly", provider.id),
                provider,
                "Monthly",
                None,
                None,
            )
            .exporting_progress("monthly", "percent"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let token = context
            .secret
            .key()
            .filter(|token| safe_cookie(token))
            .ok_or_else(|| http::invalid("The MiMo session cookie is missing or invalid."))?;
        let user = context
            .secret
            .str("/userId")
            .filter(|user| safe_cookie(user))
            .ok_or_else(|| http::invalid("The MiMo userId cookie is missing or invalid."))?;
        let cookie = format!("api-platform_serviceToken={token}; userId={user}");
        let balance = get(context, &cookie, "balance").await?;
        let amount = value::number(&balance, "/balance").ok_or_else(|| http::decoding(NAME))?;
        let currency = value::text(&balance, "/currency").ok_or_else(|| http::decoding(NAME))?;
        let mut found = vec![if currency.eq_ignore_ascii_case("USD") {
            lines::dollar_value("Balance", amount)
        } else {
            lines::count_value("Balance", amount, currency)
        }];
        let detail = get(context, &cookie, "tokenPlan/detail").await?;
        let usage = get(context, &cookie, "tokenPlan/usage").await?;
        let reset = value::time(&detail, "/currentPeriodEnd");
        if let Some(percent) = value::number(&usage, "/monthUsage/percent") {
            found.push(lines::percent(
                "Monthly",
                percent,
                reset,
                Some(lines::MONTH_MS),
            ));
        }
        if let Some(items) = usage.pointer("/monthUsage/items").and_then(Value::as_array) {
            for item in items {
                if let (Some(name), Some(used), Some(limit)) = (
                    value::text(item, "/name"),
                    value::number(item, "/used"),
                    value::number(item, "/limit"),
                ) {
                    found.push(lines::count(
                        name,
                        used,
                        limit,
                        "tokens",
                        reset,
                        Some(lines::MONTH_MS),
                    ));
                }
            }
        }
        Ok(Reading::new(
            value::text(&detail, "/planCode").and_then(lines::plan_name),
            found,
        ))
    }
}

/// Whether `text` can go into the Cookie header without starting another cookie or header line.
fn safe_cookie(text: &str) -> bool {
    !text
        .chars()
        .any(|character| character.is_control() || matches!(character, ';' | '\r' | '\n'))
}

/// The `data` of one console API answer, or the card error its `code` stands for.
async fn get(
    context: &FetchContext<'_>,
    cookie: &str,
    path: &str,
) -> Result<Value, SimpleProviderError> {
    let body = http::json(
        context.http,
        HttpRequest::get(format!("{BASE}/{path}"))
            .header("Cookie", cookie)
            .header("Origin", "https://platform.xiaomimimo.com")
            .header(
                "Referer",
                "https://platform.xiaomimimo.com/#/console/balance",
            )
            .header("x-timeZone", "UTC+00:00"),
        NAME,
    )
    .await?;
    match value::number(&body, "/code") {
        Some(0.0) => body
            .get("data")
            .cloned()
            .ok_or_else(|| http::decoding(NAME)),
        Some(401.0 | 403.0) => Err(http::expired(
            "The MiMo session expired. Sign in and paste fresh cookies.",
        )),
        _ => Err(http::decoding(NAME)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    const BALANCE: &str = r#"{"code":0,"data":{"balance":"12.50","currency":"CNY"}}"#;
    const DETAIL: &str = r#"{"code":0,"data":{"planCode":"pro","currentPeriodEnd":"2026-10-01 00:00:00","expired":false}}"#;
    const USAGE: &str = r#"{"code":0,"data":{"monthUsage":{"percent":25,"items":[{"name":"MiMo-V2","used":250,"limit":1000,"percent":25}]}}}"#;

    #[tokio::test]
    async fn reads_the_balance_the_monthly_share_and_each_model_with_both_cookies() {
        let http = Scripted::new()
            .on("GET", &format!("{BASE}/balance"), 200, BALANCE)
            .on("GET", &format!("{BASE}/tokenPlan/detail"), 200, DETAIL)
            .on("GET", &format!("{BASE}/tokenPlan/usage"), 200, USAGE);
        let scope = context_at(&http, json!({"apiKey":"test","userId":"u1"}), Utc::now());
        let reading = MiMo.fetch(&scope.context()).await.unwrap();
        let reset = value::as_time(&json!("2026-10-01T00:00:00Z"));
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(
            reading.lines,
            vec![
                lines::count_value("Balance", 12.5, "CNY"),
                lines::percent("Monthly", 25.0, reset, Some(lines::MONTH_MS)),
                lines::count(
                    "MiMo-V2",
                    250.0,
                    1000.0,
                    "tokens",
                    reset,
                    Some(lines::MONTH_MS)
                )
            ]
        );
        assert_eq!(http.requests().len(), 3);
        assert_eq!(
            header(&http.requests()[0], "Cookie"),
            Some("api-platform_serviceToken=test; userId=u1")
        );
    }

    #[tokio::test]
    async fn http_failures_an_expired_session_code_and_a_missing_code_keep_their_categories() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, r#"{"code":401}"#, ErrorCategory::AuthExpired),
            (200, "{}", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", BASE, status, body);
            let scope = context_at(&http, json!({"apiKey":"test","userId":"u1"}), Utc::now());
            assert_eq!(
                MiMo.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
