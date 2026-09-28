//! Hyperbolic: the credit balance of a Hyperbolic API key, in dollars.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `HYPERBOLIC_API_KEY` environment variable or from a key saved in Quota Control. A refresh makes
//! no billable call: it sends one request, `GET https://api.hyperbolic.ai/v2/customer/balance`
//! with the key as a bearer token, and shows `balanceCents` as the dollars left and, when the
//! answer has one, `maxOverdraftCents` as the overdraft limit. Hyperbolic describes the endpoint in
//! https://hyperbolic.ai/docs/api-reference/openapi.json, and
//! https://hyperbolic.ai/docs/general/billing-payments.md says its credits equal dollars. Then, at
//! most once every 12 hours, `GET https://api.hyperbolic.ai/v2/users/me` (in the same OpenAPI
//! document) names the account's `email` and its `role`, shown as the plan when it is `pro` or
//! `elite`; when that request fails the balance is shown alone.

use async_trait::async_trait;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Hyperbolic;

const URL: &str = "https://api.hyperbolic.ai/v2/customer/balance";
const PROFILE: &str = "https://api.hyperbolic.ai/v2/users/me";
const PROFILE_MEMO: &str = "hyperbolic.profile";

#[async_trait]
impl Service for Hyperbolic {
    fn id(&self) -> &'static str {
        "hyperbolic"
    }

    fn name(&self) -> &'static str {
        "Hyperbolic"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["HYPERBOLIC_API_KEY"],
            url: "https://app.hyperbolic.ai/settings",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![WidgetDescriptor::dollar_balance(
            format!("{}.balance", provider.id),
            provider,
            "Balance",
            None,
            "left",
        )]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Hyperbolic API key is missing."))?;
        let body = http::json(
            context.http,
            HttpRequest::get(URL).bearer(key),
            "Hyperbolic",
        )
        .await?;
        let balance =
            value::number(&body, "/balanceCents").ok_or_else(|| http::decoding("Hyperbolic"))?;
        let mut rows = vec![lines::dollar_value("Balance", balance / 100.0)];
        if let Some(overdraft) = value::number(&body, "/maxOverdraftCents") {
            rows.push(lines::dollar_value("Overdraft limit", overdraft / 100.0));
        }
        let profile = profile(context, key).await;
        let plan = profile
            .as_ref()
            .and_then(|profile| value::text(profile, "/role"))
            .filter(|role| !role.eq_ignore_ascii_case("user"))
            .and_then(lines::plan_name);
        let email = profile
            .as_ref()
            .and_then(|profile| value::text(profile, "/email"));
        Ok(Reading::new(plan, rows).with_account(email))
    }
}

/// The key's user from `GET /v2/users/me`, kept for 12 hours; `None` when that request fails, which
/// leaves the balance as it is.
async fn profile(context: &FetchContext<'_>, key: &str) -> Option<serde_json::Value> {
    if let Some(profile) = context.memo.get(PROFILE_MEMO, context.now).await {
        return Some(profile);
    }
    let profile = http::json(
        context.http,
        HttpRequest::get(PROFILE).bearer(key),
        "Hyperbolic",
    )
    .await
    .ok()
    .filter(serde_json::Value::is_object)?;
    context
        .memo
        .put(
            PROFILE_MEMO,
            profile.clone(),
            Some(context.now + chrono::Duration::hours(12)),
        )
        .await;
    Some(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn balance_and_overdraft_cents_show_as_dollars_even_when_the_balance_is_negative() {
        for (cents, expected) in [(1234.0, 12.34), (-50.0, -0.5), (0.0, 0.0)] {
            let body = json!({"balanceCents":cents,"maxOverdraftCents":2500}).to_string();
            let http = Scripted::new().on("GET", URL, 200, &body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let reading = Hyperbolic.fetch(&scope.context()).await.unwrap();
            assert_eq!(
                reading.lines,
                vec![
                    lines::dollar_value("Balance", expected),
                    lines::dollar_value("Overdraft limit", 25.0)
                ]
            );
            assert_eq!(http.requests().len(), 2);
            assert_eq!(reading.plan, None);
            assert_eq!(reading.account, None);
            assert_eq!(
                header(&http.requests()[0], "Authorization"),
                Some("Bearer test")
            );
        }
    }

    #[tokio::test]
    async fn the_profile_names_the_account_and_a_paid_tier_and_is_kept_for_12_hours() {
        let http = Scripted::new()
            .on("GET", URL, 200, r#"{"balanceCents":500}"#)
            .on(
                "GET",
                PROFILE,
                200,
                r#"{"id":"u-1","email":"me@example.com","name":null,"role":"pro","roles":[]}"#,
            );
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Hyperbolic.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.account.as_deref(), Some("me@example.com"));
        assert_eq!(reading.lines, vec![lines::dollar_value("Balance", 5.0)]);
        let again = Hyperbolic.fetch(&scope.context()).await.unwrap();
        assert_eq!(again, reading);
        assert_eq!(http.requests().len(), 3);
        assert_eq!(
            header(&http.requests()[1], "Authorization"),
            Some("Bearer test")
        );
    }

    #[tokio::test]
    async fn the_basic_tier_names_no_plan() {
        let http = Scripted::new()
            .on("GET", URL, 200, r#"{"balanceCents":500}"#)
            .on(
                "GET",
                PROFILE,
                200,
                r#"{"email":"me@example.com","role":"user"}"#,
            );
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Hyperbolic.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(reading.account.as_deref(), Some("me@example.com"));
    }

    #[tokio::test]
    async fn refused_keys_rate_limits_server_errors_and_missing_balances_keep_their_category() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (503, ErrorCategory::Http5xx),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, "{}");
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            assert_eq!(
                Hyperbolic
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                category
            );
        }
    }
}
