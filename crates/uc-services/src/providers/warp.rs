//! Warp: the request credits of a Warp account, its monthly allowance and any bonus grants.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `WARP_API_KEY` or
//! `WARP_TOKEN` environment variable or from a key saved in Quota Control. A refresh sends one
//! read-only GraphQL query, `GetRequestLimitInfo`, as
//! `POST https://app.warp.dev/graphql/v2?op=GetRequestLimitInfo` with the key as a bearer token,
//! the user agent `Warp/1.0`, the client id `warp-app` and this computer's system (Windows, macOS
//! or Linux) as the OS category and name. The monthly row counts the credits used since the last
//! refresh against the request limit until the next refresh; an unlimited plan shows the credits
//! used alone and reads as Unlimited. The bonus rows add up the grants of the account and of its
//! workspaces, then list each grant with its expiration. An answer carrying GraphQL errors is
//! reported as unreadable.

use async_trait::async_trait;
use serde_json::{Value, json};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Warp;

const URL: &str = "https://app.warp.dev/graphql/v2?op=GetRequestLimitInfo";
const QUERY: &str = "query GetRequestLimitInfo($requestContext: RequestContext!) { user(requestContext: $requestContext) { __typename ... on UserOutput { user { requestLimitInfo { isUnlimited nextRefreshTime requestLimit requestsUsedSinceLastRefresh } bonusGrants { requestCreditsGranted requestCreditsRemaining expiration } workspaces { bonusGrantsInfo { grants { requestCreditsGranted requestCreditsRemaining expiration } } } } } } }";

#[async_trait]
impl Service for Warp {
    fn id(&self) -> &'static str {
        "warp"
    }

    fn name(&self) -> &'static str {
        "Warp"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["WARP_API_KEY", "WARP_TOKEN"],
            url: "https://app.warp.dev",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::bounded_count(
                format!("{}.monthly", provider.id),
                provider,
                "Monthly",
                None,
                0.0,
                "credits",
                Some(lines::MONTH_MS),
            ),
            WidgetDescriptor::bounded_count(
                format!("{}.bonus", provider.id),
                provider,
                "Bonus Credits",
                None,
                0.0,
                "credits",
                None,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Warp API key is missing."))?;
        let os = if cfg!(windows) {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "Linux"
        };
        let body = http::json(
            context.http,
            HttpRequest::post(URL)
                .bearer(key)
                .header("User-Agent", "Warp/1.0")
                .header("x-warp-client-id", "warp-app")
                .header("x-warp-os-category", os)
                .header("x-warp-os-name", os)
                .json_body(&json!({
                    "operationName": "GetRequestLimitInfo",
                    "query": QUERY,
                    "variables": {
                        "requestContext": {
                            "clientContext": {},
                            "osContext": { "category": os, "name": os, "version": "unknown" }
                        }
                    }
                })),
            "Warp",
        )
        .await?;
        if body
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty())
        {
            return Err(http::decoding("Warp"));
        }
        let user = body
            .pointer("/data/user/user")
            .ok_or_else(|| http::decoding("Warp"))?;
        let info = user
            .get("requestLimitInfo")
            .ok_or_else(|| http::decoding("Warp"))?;
        let used = value::number(info, "/requestsUsedSinceLastRefresh")
            .ok_or_else(|| http::decoding("Warp"))?;
        let unlimited = value::flag(info, "/isUnlimited") == Some(true);
        let mut out = vec![if unlimited {
            lines::count_value("Monthly", used, "credits")
        } else {
            lines::count(
                "Monthly",
                used,
                value::number(info, "/requestLimit").ok_or_else(|| http::decoding("Warp"))?,
                "credits",
                value::time(info, "/nextRefreshTime"),
                Some(lines::MONTH_MS),
            )
        }];
        let mut grants: Vec<&Value> = user
            .get("bonusGrants")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();
        if let Some(workspaces) = user.get("workspaces").and_then(Value::as_array) {
            for workspace in workspaces {
                grants.extend(
                    workspace
                        .pointer("/bonusGrantsInfo/grants")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten(),
                );
            }
        }
        let mut total = 0.0;
        let mut left = 0.0;
        let mut reset = None;
        for grant in &grants {
            total += value::number(grant, "/requestCreditsGranted")
                .ok_or_else(|| http::decoding("Warp"))?;
            let remaining = value::number(grant, "/requestCreditsRemaining")
                .ok_or_else(|| http::decoding("Warp"))?;
            left += remaining;
            if remaining > 0.0
                && let Some(expiration) = value::time(grant, "/expiration")
            {
                reset = Some(
                    reset.map_or(expiration, |earliest: chrono::DateTime<chrono::Utc>| {
                        earliest.min(expiration)
                    }),
                );
            }
        }
        if !grants.is_empty() {
            out.push(lines::count(
                "Bonus Credits",
                total - left,
                total,
                "credits",
                reset,
                None,
            ));
        }
        for (index, grant) in grants.iter().enumerate() {
            if let (Some(total), Some(left)) = (
                value::number(grant, "/requestCreditsGranted"),
                value::number(grant, "/requestCreditsRemaining"),
            ) {
                out.push(lines::count(
                    &format!("Bonus Grant {}", index + 1),
                    total - left,
                    total,
                    "credits",
                    value::time(grant, "/expiration"),
                    None,
                ));
            }
        }
        Ok(Reading::new(unlimited.then(|| "Unlimited".into()), out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use uc_core::ErrorCategory;

    const LIMITED_PLAN: &str = r#"{"data":{"user":{"__typename":"UserOutput","user":{"requestLimitInfo":{"isUnlimited":false,"requestLimit":100,"requestsUsedSinceLastRefresh":25,"nextRefreshTime":"2026-10-01T00:00:00Z"},"bonusGrants":[{"requestCreditsGranted":20,"requestCreditsRemaining":15}],"workspaces":[]}}}}"#;

    #[tokio::test]
    async fn a_limited_plan_shows_its_monthly_credits_and_each_bonus_grant_from_one_query() {
        let http = Scripted::new().on("POST", URL, 200, LIMITED_PLAN);
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Warp.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(
            reading.lines,
            vec![
                lines::count(
                    "Monthly",
                    25.0,
                    100.0,
                    "credits",
                    value::as_time(&json!("2026-10-01T00:00:00Z")),
                    Some(lines::MONTH_MS)
                ),
                lines::count("Bonus Credits", 5.0, 20.0, "credits", None, None),
                lines::count("Bonus Grant 1", 5.0, 20.0, "credits", None, None)
            ]
        );
        assert_eq!(header(&http.requests()[0], "User-Agent"), Some("Warp/1.0"));
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer test")
        );
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn failed_queries_keep_their_category_and_never_repeat_the_answer() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
            (
                200,
                r#"{"errors":[{"message":"secret"}]}"#,
                ErrorCategory::Decoding,
            ),
        ] {
            let http = Scripted::new().on("POST", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let error = Warp.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }
}
