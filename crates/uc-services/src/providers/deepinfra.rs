//! DeepInfra: the balance, the amount owed, this month's spend and the billing cycle of a
//! DeepInfra account.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `DEEPINFRA_API_KEY` environment variable or from a key saved in Quota Control. A refresh sends
//! two requests with the key as a bearer token, first
//! `GET https://api.deepinfra.com/payment/checklist?compute_owed=true` and then
//! `GET https://api.deepinfra.com/payment/usage?from=current`. The checklist's `stripe_balance`
//! plus its `recent` amount is what the account owes, and a negative amount owed is a balance
//! left. This month's spend is the `total_cost`, in cents, of the last month the usage answer
//! lists, or the `recent` amount when it lists none. An account with a positive `limit` also gets
//! a billing cycle meter of the `recent` amount against that limit, and a `suspended` account gets
//! a warning. A third request, `GET https://api.deepinfra.com/v1/me` (the account profile in
//! DeepInfra's OpenAPI description, https://api.deepinfra.com/openapi.json), names the account's
//! `email`; it waits two seconds at most, and its failure only leaves the email out.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines};

pub(crate) struct DeepInfra;

const NAME: &str = "DeepInfra";
const BASE: &str = "https://api.deepinfra.com/payment/";
const ME_URL: &str = "https://api.deepinfra.com/v1/me";
const SUSPENDED: &str = "DeepInfra has suspended this account. Check the billing dashboard.";

#[async_trait]
impl Service for DeepInfra {
    fn id(&self) -> &'static str {
        "deepinfra"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["DEEPINFRA_API_KEY"],
            url: "https://deepinfra.com/dash/api_keys",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let amount = |suffix: &str, title: &str| {
            WidgetDescriptor::values(
                format!("{}.{suffix}", provider.id),
                provider,
                title,
                None,
                Some(MetricKind::Dollars),
                None,
                true,
                None,
                false,
            )
        };
        vec![
            WidgetDescriptor::dollar_balance(
                format!("{}.balance", provider.id),
                provider,
                "Balance",
                None,
                "left",
            ),
            amount("owed", "Amount Owed"),
            amount("month", "Spend This Month"),
            WidgetDescriptor::bounded_dollars(
                format!("{}.cycle", provider.id),
                provider,
                "Billing Cycle",
                None,
                100.0,
                None,
                None,
            )
            .exporting_progress("cycle", "usd"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The DeepInfra API key is missing."))?;
        let checklist = http::json(
            context.http,
            HttpRequest::get(format!("{BASE}checklist?compute_owed=true"))
                .bearer(key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        let usage = http::json(
            context.http,
            HttpRequest::get(format!("{BASE}usage?from=current"))
                .bearer(key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        let number = |field: &Value| {
            field
                .as_f64()
                .filter(|amount| amount.is_finite())
                .ok_or_else(|| http::decoding(NAME))
        };
        let recent = number(&checklist["recent"])?.max(0.0);
        let owed = number(&checklist["stripe_balance"])? + recent;
        if !owed.is_finite()
            || checklist
                .get("suspended")
                .is_some_and(|suspended| !suspended.is_null() && !suspended.is_boolean())
        {
            return Err(http::decoding(NAME));
        }
        let months = usage["months"]
            .as_array()
            .ok_or_else(|| http::decoding(NAME))?;
        let mut spent = recent;
        for month in months {
            if month["period"].as_str().is_none() {
                return Err(http::decoding(NAME));
            }
            spent = (number(&month["total_cost"])? / 100.0).max(0.0);
        }
        let mut rows = vec![
            lines::dollar_value("Balance", (-owed).max(0.0)),
            lines::dollar_value("Amount Owed", owed.max(0.0)),
            lines::dollar_value("Spend This Month", spent),
        ];
        if let Some(limit) = checklist.get("limit").filter(|limit| !limit.is_null()) {
            let limit = number(limit)?;
            if limit > 0.0 {
                rows.push(lines::dollars("Billing Cycle", recent, limit, None, None));
            }
        }
        let email = http::json(
            context.http,
            HttpRequest::get(ME_URL)
                .bearer(key)
                .header("Accept", "application/json")
                .timeout(std::time::Duration::from_secs(2)),
            NAME,
        )
        .await
        .ok()
        .and_then(|me| me["email"].as_str().map(str::to_string));
        Ok(Reading::new(None, rows)
            .with_account(email)
            .with_warning((checklist["suspended"] == true).then(|| SUSPENDED.into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn the_checklist_and_usage_answers_fill_every_row_and_warn_of_a_suspension() {
        let http = Scripted::new()
            .on(
                "GET",
                &format!("{BASE}checklist"),
                200,
                r#"{"recent":2.5,"stripe_balance":-10,"limit":20,"suspended":true}"#,
            )
            .on(
                "GET",
                &format!("{BASE}usage"),
                200,
                r#"{"months":[{"period":"2026-09","total_cost":375}]}"#,
            )
            .on(
                "GET",
                ME_URL,
                200,
                r#"{"uid":"u-1","email":"dev@example.com","email_verified":true}"#,
            );
        let scope = context_at(&http, json!({"apiKey":"fixture"}), Utc::now());
        assert_eq!(
            DeepInfra.fetch(&scope.context()).await.unwrap(),
            Reading::new(
                None,
                vec![
                    lines::dollar_value("Balance", 7.5),
                    lines::dollar_value("Amount Owed", 0.0),
                    lines::dollar_value("Spend This Month", 3.75),
                    lines::dollars("Billing Cycle", 2.5, 20.0, None, None)
                ]
            )
            .with_account(Some("dev@example.com"))
            .with_warning(Some(SUSPENDED.into()))
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].url, ME_URL);
        assert_eq!(
            header(&requests[2], "authorization"),
            Some("Bearer fixture")
        );
        assert_eq!(
            requests[0].url,
            format!("{BASE}checklist?compute_owed=true")
        );
        assert_eq!(requests[1].url, format!("{BASE}usage?from=current"));
        assert_eq!(
            header(&requests[0], "authorization"),
            Some("Bearer fixture")
        );
    }

    #[tokio::test]
    async fn a_failed_profile_lookup_only_leaves_the_email_out() {
        let http = Scripted::new()
            .on(
                "GET",
                &format!("{BASE}checklist"),
                200,
                r#"{"recent":0,"stripe_balance":0}"#,
            )
            .on("GET", &format!("{BASE}usage"), 200, r#"{"months":[]}"#)
            .on("GET", ME_URL, 500, "{}");
        let scope = context_at(&http, json!({"apiKey":"fixture"}), Utc::now());
        let reading = DeepInfra.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.account, None);
        assert_eq!(reading.warning, None);
        assert_eq!(reading.lines.len(), 3);
    }

    #[tokio::test]
    async fn refused_keys_rate_limits_and_unreadable_answers_keep_their_category() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{", ErrorCategory::Decoding),
            (200, "{}", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", BASE, status, body);
            let scope = context_at(&http, json!({"apiKey":"fixture"}), Utc::now());
            assert_eq!(
                DeepInfra
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                category
            );
        }
    }
}
