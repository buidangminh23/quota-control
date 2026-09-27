//! Qwen Cloud: the five-hour, weekly and monthly usage of a Qwen Cloud individual token plan, and
//! the plan's name.
//!
//! Nothing is read from disk or from a browser's cookie store on Windows, macOS or Linux, and no
//! dashboard is scraped to renew a session: the user pastes the `login_aliyunid_ticket` cookie and
//! the `sec_token` of a signed-in Qwen Cloud session, and may add the `login_aliyunid_csrf`
//! cookie. A refresh asks for the usage, and for the subscription when it was not read in the last
//! 12 hours. Each is a `POST https://cs-data.qwencloud.com/data/api.json` to the
//! `IntlBroadScopeAspnGateway` action of the `sfm_bailian` product that calls OneConsole's
//! `zeldaHttp.apikeyMgr./tokenplan/personal/api/v2/usage` or `…/subscription`. It carries the
//! sec_token and the call's parameters in a form body, the cookies in `Cookie`, the site's Origin
//! and Referer, `X-Requested-With: XMLHttpRequest` and, when one is saved, the CSRF value as
//! `x-xsrf-token` and `x-csrf-token`. An answer whose `success` is false means the session is no
//! longer accepted.

use async_trait::async_trait;
use chrono::Duration;
use serde_json::{Value, json};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Qwen;

const BASE: &str = "https://cs-data.qwencloud.com/data/api.json";
const DASH: &str = "https://home.qwencloud.com/billing/subscription/token-plan-individual";

#[async_trait]
impl Service for Qwen {
    fn id(&self) -> &'static str {
        "qwen"
    }

    fn name(&self) -> &'static str {
        "Qwen Cloud"
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (login_aliyunid_ticket)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: DASH,
            fields: &[
                ("secToken", "Security token (sec_token)"),
                ("csrfToken", "CSRF cookie (login_aliyunid_csrf)"),
            ],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [
            ("session", "Session"),
            ("weekly", "Weekly"),
            ("monthly", "Monthly"),
        ]
        .into_iter()
        .map(|(id, title)| {
            WidgetDescriptor::percent(format!("{}.{id}", provider.id), provider, title, None, None)
                .exporting_progress(id, "percent")
        })
        .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let data = query(context, "usage").await?;
        let mut rows = vec![];
        for (used, reset, label, period) in [
            (
                "per5HourPercentage",
                "per5HourResetTime",
                "Session",
                5 * lines::HOUR_MS,
            ),
            (
                "per1WeekPercentage",
                "per1WeekResetTime",
                "Weekly",
                lines::WEEK_MS,
            ),
            (
                "per1MonthPercentage",
                "per1MonthResetTime",
                "Monthly",
                lines::MONTH_MS,
            ),
        ] {
            if let Some(ratio) = value::number(&data, &format!("/{used}")) {
                rows.push(lines::percent(
                    label,
                    ratio * 100.0,
                    value::time(&data, &format!("/{reset}")),
                    Some(period),
                ));
            }
        }
        if rows.is_empty() {
            return Err(http::decoding("Qwen Cloud"));
        }
        let subscription =
            if let Some(remembered) = context.memo.get("qwen-subscription", context.now).await {
                remembered
            } else {
                let fresh = query(context, "subscription").await?;
                context
                    .memo
                    .put(
                        "qwen-subscription",
                        fresh.clone(),
                        Some(context.now + Duration::hours(12)),
                    )
                    .await;
                fresh
            };
        Ok(Reading::new(
            value::text(&subscription, "/specCode").and_then(lines::plan_name),
            rows,
        ))
    }
}

/// Calls the token plan's `operation` (`usage` or `subscription`) through the OneConsole gateway
/// with the pasted session, and returns the answer's `data`, parsed when it comes as JSON text.
async fn query(context: &FetchContext<'_>, operation: &str) -> Result<Value, SimpleProviderError> {
    let ticket = context
        .secret
        .key()
        .filter(|ticket| {
            !ticket
                .chars()
                .any(|character| character.is_control() || character == ';')
        })
        .ok_or_else(|| http::invalid("The Qwen Cloud session cookie is missing or invalid."))?;
    let sec_token = context.secret.str("/secToken").ok_or_else(|| {
        http::invalid("Enter the Qwen Cloud sec_token from a signed-in usage request.")
    })?;
    let api = format!("zeldaHttp.apikeyMgr./tokenplan/personal/api/v2/{operation}");
    let mut url = url::Url::parse(BASE).map_err(|_| http::decoding("Qwen Cloud"))?;
    url.query_pairs_mut()
        .append_pair("action", "IntlBroadScopeAspnGateway")
        .append_pair("product", "sfm_bailian")
        .append_pair("api", &api)
        .append_pair("_v", "undefined");
    let mut data = json!({
        "cornerstoneParam": {
            "feTraceId": format!(
                "00000000-0000-4000-8000-{:012x}",
                context.now.timestamp_millis() & 0xffffffffffff
            ),
            "feURL": DASH,
            "protocol": "V2",
            "console": "ONE_CONSOLE",
            "productCode": "p_efm",
            "domain": "home.qwencloud.com",
            "consoleSite": "QWENCLOUD",
            "userNickName": "",
            "userPrincipalName": "",
            "xsp_lang": "en-US"
        }
    });
    if operation == "subscription" {
        data["commodityCode"] = json!("sfm_tokenplansolo_public_intl");
    }
    let params = json!({"Api": api, "V": "1.0", "Data": data}).to_string();
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("product", "sfm_bailian")
        .append_pair("action", "IntlBroadScopeAspnGateway")
        .append_pair("sec_token", sec_token)
        .append_pair("region", "ap-southeast-1")
        .append_pair("language", "en-US")
        .append_pair("params", &params)
        .finish();
    let mut cookie = format!("login_aliyunid_ticket={ticket}");
    let mut request = HttpRequest::post(url.as_str())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Origin", "https://home.qwencloud.com")
        .header("Referer", DASH)
        .header("X-Requested-With", "XMLHttpRequest")
        .body(body.into_bytes());
    if let Some(csrf) = context.secret.str("/csrfToken") {
        if csrf
            .chars()
            .any(|character| character.is_control() || character == ';')
        {
            return Err(http::invalid("The Qwen Cloud CSRF cookie is invalid."));
        }
        cookie.push_str(&format!("; login_aliyunid_csrf={csrf}"));
        request = request
            .header("x-xsrf-token", csrf)
            .header("x-csrf-token", csrf);
    }
    let answer = http::json(context.http, request.header("Cookie", cookie), "Qwen Cloud").await?;
    if value::flag(&answer, "/success") == Some(false)
        || value::flag(&answer, "/Success") == Some(false)
    {
        return Err(http::expired(
            "Qwen Cloud rejected the session. Sign in and paste fresh session values.",
        ));
    }
    let data = answer
        .get("data")
        .or_else(|| answer.get("Data"))
        .ok_or_else(|| http::decoding("Qwen Cloud"))?;
    if let Some(text) = data.as_str() {
        serde_json::from_str(text).map_err(|_| http::decoding("Qwen Cloud"))
    } else {
        Ok(data.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use uc_core::ErrorCategory;

    const USAGE: &str = r#"{"data":{"per5HourPercentage":0.03,"per5HourResetTime":1700003600000,"per1WeekPercentage":0.01,"per1WeekResetTime":1700086400000}}"#;
    const SUBSCRIPTION: &str = r#"{"data":{"specCode":"standard"}}"#;

    #[tokio::test]
    async fn reads_the_meters_and_the_plan_sending_the_cookies_and_the_sec_token_in_the_form() {
        let http = Scripted::new()
            .on("POST", BASE, 200, USAGE)
            .on("POST", BASE, 200, SUBSCRIPTION);
        let scope = context_at(
            &http,
            json!({"apiKey":"ticket","secToken":"sec+&=","csrfToken":"csrf"}),
            Utc::now(),
        );
        let reading = Qwen.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Standard"));
        assert_eq!(
            reading.lines,
            vec![
                lines::percent(
                    "Session",
                    3.0,
                    value::as_time(&json!(1700003600000i64)),
                    Some(5 * lines::HOUR_MS)
                ),
                lines::percent(
                    "Weekly",
                    1.0,
                    value::as_time(&json!(1700086400000i64)),
                    Some(lines::WEEK_MS)
                )
            ]
        );
        let request = &http.requests()[0];
        assert_eq!(
            header(request, "Cookie"),
            Some("login_aliyunid_ticket=ticket; login_aliyunid_csrf=csrf")
        );
        let body = url::form_urlencoded::parse(request.body.as_ref().unwrap())
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            body.get("sec_token").map(|token| token.as_ref()),
            Some("sec+&=")
        );
        assert_eq!(http.requests().len(), 2);
    }

    #[tokio::test]
    async fn refused_rate_limited_and_unreadable_answers_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("POST", BASE, status, "{}");
            let scope = context_at(&http, json!({"apiKey":"ticket","secToken":"s"}), Utc::now());
            assert_eq!(
                Qwen.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
