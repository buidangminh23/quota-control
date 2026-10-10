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
//!
//! A second query, `GetAccountInfo`, sent the same way to
//! `https://app.warp.dev/graphql/v2?op=GetAccountInfo`, reads fields of Warp's own GraphQL schema
//! (`crates/warp_graphql_schema/api/schema.graphql` in `warpdotdev/warp`): the user's
//! `profile.email`, and the `billingMetadata` of the user, else of its first workspace that has
//! one, whose `tier.name` names the plan and whose active or canceled service agreement's
//! `currentPeriodEnd` is the plan's renewal or end date. It is asked at most every 12 hours (every
//! hour after a failure), and a failure never affects the credits.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use uc_core::{HttpRequest, PlanTerm, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Warp;

const URL: &str = "https://app.warp.dev/graphql/v2?op=GetRequestLimitInfo";
const ACCOUNT_URL: &str = "https://app.warp.dev/graphql/v2?op=GetAccountInfo";
const ACCOUNT_QUERY: &str = "query GetAccountInfo($requestContext: RequestContext!) { user(requestContext: $requestContext) { __typename ... on UserOutput { user { profile { email } billingMetadata { tier { name } serviceAgreements { currentPeriodEnd status } } workspaces { billingMetadata { tier { name } serviceAgreements { currentPeriodEnd status } } } } } } }";
/// Memo key holding the last account answer: `{"email", "plan", "endsAt"}`.
const ACCOUNT_MEMO: &str = "warp.account";
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
        let body = http::json(
            context.http,
            graphql(URL, key, "GetRequestLimitInfo", QUERY),
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
        let account = account(context, key).await;
        let plan = value::text(&account, "/plan")
            .map(str::to_string)
            .or_else(|| unlimited.then(|| "Unlimited".into()));
        Ok(Reading::new(plan, out)
            .with_plan_checked_at(value::time(&account, "/checkedAt"))
            .with_plan_term(
                value::time(&account, "/endsAt").map(|ends_at| PlanTerm::Stated {
                    ends_at,
                    checked_at: value::time(&account, "/checkedAt"),
                }),
            )
            .with_account(value::text(&account, "/email")))
    }
}

/// A GraphQL `POST` as the Warp app sends it, with this computer's system as the OS context.
fn graphql(url: &str, key: &str, operation: &str, query: &str) -> HttpRequest {
    let os = if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    };
    HttpRequest::post(url)
        .bearer(key)
        .header("User-Agent", "Warp/1.0")
        .header("x-warp-client-id", "warp-app")
        .header("x-warp-os-category", os)
        .header("x-warp-os-name", os)
        .json_body(&json!({
            "operationName": operation,
            "query": query,
            "variables": {
                "requestContext": {
                    "clientContext": {},
                    "osContext": { "category": os, "name": os, "version": "unknown" }
                }
            }
        }))
}

/// The account's email, plan and plan end as `{"email", "plan", "endsAt"}`, remembered for 12
/// hours (an hour after a failed query, then as an empty object). Never fails the card.
async fn account(context: &FetchContext<'_>, key: &str) -> Value {
    if let Some(memo) = context.memo.get(ACCOUNT_MEMO, context.now).await
        && (value::time(&memo, "/endsAt").is_none() || value::time(&memo, "/checkedAt").is_some())
    {
        return memo;
    }
    let answer = http::json(
        context.http,
        graphql(ACCOUNT_URL, key, "GetAccountInfo", ACCOUNT_QUERY),
        "Warp",
    )
    .await
    .ok()
    .filter(|body| {
        body.get("errors")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
    })
    .and_then(|body| {
        body.pointer("/data/user/user")
            .map(|user| read_account(user, context.now))
    });
    let keep = if answer.is_some() {
        Duration::hours(12)
    } else {
        Duration::hours(1)
    };
    let mut found = answer.unwrap_or_else(|| json!({}));
    if found.get("plan").is_some_and(|plan| !plan.is_null()) {
        found["checkedAt"] = json!(context.now.to_rfc3339());
    }
    let expires = value::time(&found, "/endsAt")
        .filter(|end| *end > context.now)
        .map_or(context.now + keep, |end| end.min(context.now + keep));
    context
        .memo
        .put(ACCOUNT_MEMO, found.clone(), Some(expires))
        .await;
    found
}

/// The email, the plan's tier name and its agreement's period end from a `GetAccountInfo` user.
fn read_account(user: &Value, now: DateTime<Utc>) -> Value {
    let billing = std::iter::once(user.get("billingMetadata"))
        .chain(
            user.get("workspaces")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|workspace| workspace.get("billingMetadata")),
        )
        .flatten()
        .find(|billing| value::text(billing, "/tier/name").is_some());
    let ends_at = billing
        .and_then(|billing| billing.get("serviceAgreements"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|agreement| {
            matches!(
                value::text(agreement, "/status"),
                Some("ACTIVE" | "CANCELED")
            )
        })
        .filter_map(|agreement| value::time(agreement, "/currentPeriodEnd"))
        .find(|end| *end > now);
    json!({
        "email": value::text(user, "/profile/email"),
        "plan": billing.and_then(|billing| value::text(billing, "/tier/name")),
        "endsAt": ends_at.map(|ends_at| ends_at.to_rfc3339()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use uc_core::ErrorCategory;

    #[test]
    fn an_expired_agreement_does_not_hide_the_current_paid_period() {
        let now = Utc.with_ymd_and_hms(2026, 10, 10, 10, 0, 0).unwrap();
        let current_end = now + Duration::days(10);
        let user = json!({"billingMetadata": {
            "tier": {"name": "Pro"},
            "serviceAgreements": [
                {"status": "CANCELED", "currentPeriodEnd": (now - Duration::days(1)).to_rfc3339()},
                {"status": "UNPAID", "currentPeriodEnd": (now + Duration::days(20)).to_rfc3339()},
                {"status": "ACTIVE", "currentPeriodEnd": current_end.to_rfc3339()}
            ]
        }});
        assert_eq!(
            value::time(&read_account(&user, now), "/endsAt"),
            Some(current_end)
        );
    }

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
        // The account query found no answer, so the card shows without it.
        assert_eq!(http.requests().len(), 2);
        assert_eq!(reading.account, None);
        assert_eq!(reading.plan_term, None);
    }

    const ACCOUNT: &str = r#"{"data":{"user":{"__typename":"UserOutput","user":{"profile":{"email":"fixture@example.com"},"billingMetadata":null,"workspaces":[{"billingMetadata":{"tier":{"name":"Build"},"serviceAgreements":[{"currentPeriodEnd":"2026-09-01T00:00:00Z","status":"UNPAID"},{"currentPeriodEnd":"2026-10-15T00:00:00Z","status":"ACTIVE"}]}}]}}}}"#;

    #[tokio::test]
    async fn the_account_query_names_the_email_the_tier_and_the_period_end_and_is_remembered() {
        let http = Scripted::new().on("POST", URL, 200, LIMITED_PLAN).on(
            "POST",
            ACCOUNT_URL,
            200,
            ACCOUNT,
        );
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Warp.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Build"));
        assert_eq!(reading.account.as_deref(), Some("fixture@example.com"));
        assert_eq!(
            reading.plan_term,
            value::as_time(&json!("2026-10-15T00:00:00Z")).map(|ends_at| PlanTerm::Stated {
                ends_at,
                checked_at: Some(scope.context().now),
            })
        );
        let requests = http.requests();
        assert_eq!(requests[1].url, ACCOUNT_URL);
        assert_eq!(header(&requests[1], "Authorization"), Some("Bearer test"));
        let mut later = scope.context();
        later.now += Duration::hours(1);
        let again = Warp.fetch(&later).await.unwrap();
        assert_eq!(again, reading);
        assert_eq!(again.plan_checked_at, Some(scope.context().now));
        assert_eq!(http.requests().len(), 3, "the account answer is remembered");
    }

    #[tokio::test]
    async fn a_failed_account_query_leaves_the_credits_alone() {
        let http = Scripted::new().on("POST", URL, 200, LIMITED_PLAN).on(
            "POST",
            ACCOUNT_URL,
            200,
            r#"{"errors":[{"message":"no"}]}"#,
        );
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Warp.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(reading.account, None);
        assert_eq!(reading.lines.len(), 3);
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
