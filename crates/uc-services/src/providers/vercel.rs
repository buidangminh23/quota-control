//! Vercel AI Gateway: the AI Gateway Credits balance of the team an API key belongs to, and the
//! team's lifetime spend.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `AI_GATEWAY_API_KEY` environment variable or from a key saved in Quota Control. It is an AI
//! Gateway API key, which Vercel ties to one team, or a Vercel OIDC token, which lasts 12 hours. A
//! refresh sends one request, `GET https://ai-gateway.vercel.sh/v1/credits` with the key as a
//! bearer token, which answers `{"balance": "95.50", "total_used": "4.50"}` in US dollars. The
//! endpoint has no allowance, reset date or plan, so the card shows both amounts as plain values,
//! without a meter.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{
    HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, MetricLine, Provider,
    ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, jwt, lines, value};

pub(crate) struct Vercel;

const NAME: &str = "Vercel AI Gateway";
const CREDITS_URL: &str = "https://ai-gateway.vercel.sh/v1/credits";
/// Vercel's dashboard deep links: the dashboard fills `[team]` with the team the user picks.
const API_KEYS_URL: &str =
    "https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai-gateway%2Fapi-keys&title=AI+Gateway+API+Keys";
const DASHBOARD_URL: &str =
    "https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai-gateway&title=Go+to+AI+Gateway";
const BALANCE: &str = "Balance";
const TOTAL_SPENT: &str = "Total Spent";
const NO_KEY: &str = "The Vercel AI Gateway API key is missing. Add it again in Accounts.";
/// A 401, which Vercel documents as an invalid API key or OIDC token.
const REFUSED: &str = "Vercel AI Gateway refused this API key. Check it or create a new one.";
const OIDC_EXPIRED: &str = "Vercel AI Gateway no longer accepts this OIDC token. OIDC tokens last 12 hours; save an AI Gateway API key instead.";
/// A 403, which Vercel documents as an endpoint the team's plan does not include: the key itself
/// was accepted.
const NOT_SHOWN: &str = "Vercel AI Gateway does not let this key read the team's credit balance.";

#[async_trait]
impl Service for Vercel {
    fn id(&self) -> &'static str {
        "vercel"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://www.vercel-status.com"),
            ProviderLink::new("Dashboard", DASHBOARD_URL),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["AI_GATEWAY_API_KEY"],
            url: API_KEYS_URL,
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::dollar_balance(
                format!("{}.balance", provider.id),
                provider,
                BALANCE,
                None,
                "left",
            )
            .exporting_limit(
                "balance",
                LimitResourceKind::Balance,
                "usd",
                dollars(),
                false,
            ),
            WidgetDescriptor::values(
                format!("{}.totalSpent", provider.id),
                provider,
                TOTAL_SPENT,
                None,
                Some(MetricKind::Dollars),
                Some("spent"),
                false,
                None,
                false,
            )
            .exporting_limit(
                "totalSpent",
                LimitResourceKind::Consumption,
                "usd",
                dollars(),
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context.secret.key().ok_or_else(|| http::invalid(NO_KEY))?;
        let oidc = jwt::claims(key).is_some();
        if oidc && jwt::expires_at(key).is_some_and(|expires| expires <= context.now) {
            return Err(http::expired(OIDC_EXPIRED));
        }
        let response = http::send(
            context.http,
            HttpRequest::get(CREDITS_URL)
                .bearer(key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        match response.status {
            401 if oidc => return Err(http::expired(OIDC_EXPIRED)),
            401 => return Err(http::invalid(REFUSED)),
            403 => return Err(http::not_available(NOT_SHOWN)),
            _ => {}
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let body = http::parse(&response, NAME)?;
        let rows = amounts(&body).ok_or_else(|| http::decoding(NAME))?;
        Ok(Reading::new(None, rows))
    }
}

/// Where both widgets' amounts appear in the local limits API.
fn dollars() -> LimitResourceSource {
    LimitResourceSource::Value {
        kind: MetricKind::Dollars,
        label: None,
    }
}

/// The balance row, then the lifetime spend row when the answer carries a usable spend. The
/// balance keeps its sign, so an overdrawn team reads as negative rather than as zero. `None`
/// when the balance is missing or unreadable: it never becomes a made-up zero.
fn amounts(body: &Value) -> Option<Vec<MetricLine>> {
    let balance = value::number(body, "/balance")?;
    let mut rows = vec![lines::dollar_value(BALANCE, balance)];
    if let Some(spent) = value::number(body, "/total_used").filter(|spent| *spent >= 0.0) {
        rows.push(lines::dollar_value(TOTAL_SPENT, spent));
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Roots;
    use crate::testing::{Scripted, context_at, header};
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use chrono::{DateTime, Duration, TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const KEY: &str = "vck_test_0123456789abcdef";
    const FUNDED: &str = r#"{"balance":"95.50","total_used":"4.50"}"#;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    async fn answer_with(
        key: &str,
        status: u16,
        body: &str,
    ) -> (Scripted, Result<Reading, SimpleProviderError>) {
        let http = Scripted::new().on("GET", CREDITS_URL, status, body);
        let scope = context_at(&http, json!({ "apiKey": key }), now());
        let result = Vercel.fetch(&scope.context()).await;
        (http, result)
    }

    async fn read(body: &str) -> Reading {
        answer_with(KEY, 200, body).await.1.unwrap()
    }

    async fn fail(status: u16, body: &str) -> SimpleProviderError {
        answer_with(KEY, status, body).await.1.unwrap_err()
    }

    fn rows(balance: f64, spent: Option<f64>) -> Vec<MetricLine> {
        let mut rows = vec![lines::dollar_value("Balance", balance)];
        rows.extend(spent.map(|spent| lines::dollar_value("Total Spent", spent)));
        rows
    }

    /// A token shaped like the `VERCEL_OIDC_TOKEN` that `vercel env pull` writes (a JWT).
    fn oidc_token(expires: DateTime<Utc>) -> String {
        let claims = json!({
            "iss": "https://oidc.vercel.com/acme",
            "sub": "owner:acme:project:app:environment:development",
            "exp": expires.timestamp()
        });
        format!(
            "eyJhbGciOiJSUzI1NiJ9.{}.signature",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    #[tokio::test]
    async fn shows_the_team_balance_and_its_lifetime_spend_in_dollars() {
        let reading = read(FUNDED).await;
        assert_eq!(reading, Reading::new(None, rows(95.5, Some(4.5))));
        assert_eq!(reading.warning, None);
    }

    #[tokio::test]
    async fn sends_the_key_as_a_bearer_token_to_the_credits_endpoint() {
        let (http, result) = answer_with(KEY, 200, FUNDED).await;
        result.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, "https://ai-gateway.vercel.sh/v1/credits");
        assert_eq!(
            header(request, "Authorization"),
            Some("Bearer vck_test_0123456789abcdef")
        );
        assert_eq!(header(request, "Accept"), Some("application/json"));
        assert_eq!(request.body, None);
        assert_eq!(request.timeout, HttpRequest::DEFAULT_TIMEOUT);
    }

    #[tokio::test]
    async fn zero_and_negative_balances_and_numeric_amounts_are_kept() {
        assert_eq!(
            read(r#"{"balance":"0","total_used":"0"}"#).await.lines,
            rows(0.0, Some(0.0))
        );
        assert_eq!(
            read(r#"{"balance":"-0.25","total_used":"5.25"}"#)
                .await
                .lines,
            rows(-0.25, Some(5.25))
        );
        assert_eq!(
            read(r#"{"balance":12,"total_used":0.5}"#).await.lines,
            rows(12.0, Some(0.5))
        );
    }

    #[tokio::test]
    async fn a_missing_or_unusable_spend_leaves_only_the_balance() {
        for body in [
            r#"{"balance":"3.00"}"#,
            r#"{"balance":"3.00","total_used":null}"#,
            r#"{"balance":"3.00","total_used":"n/a"}"#,
            r#"{"balance":"3.00","total_used":"-1.00"}"#,
        ] {
            assert_eq!(read(body).await.lines, rows(3.0, None), "{body}");
        }
    }

    #[tokio::test]
    async fn an_answer_without_a_readable_balance_is_a_decoding_error() {
        for body in [
            r#"{"total_used":"4.50"}"#,
            r#"{"balance":"lots","total_used":"4.50"}"#,
            r#"{"balance":null}"#,
            "<html>busy</html>",
        ] {
            let error = fail(200, body).await;
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
            assert_eq!(
                error.message,
                "Vercel AI Gateway returned usage data this version cannot read."
            );
        }
    }

    #[tokio::test]
    async fn a_refused_key_is_reported_without_revealing_it() {
        let error = fail(
            401,
            r#"{"error":{"message":"Invalid API key","type":"authentication_error"}}"#,
        )
        .await;
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "Vercel AI Gateway refused this API key. Check it or create a new one."
        );
        assert!(!error.message.contains(KEY));
    }

    #[tokio::test]
    async fn a_forbidden_answer_says_the_balance_is_not_available_instead_of_blaming_the_key() {
        let body =
            r#"{"error":{"message":"This endpoint requires a paid plan","type":"forbidden"}}"#;
        let live_oidc = oidc_token(now() + Duration::hours(6));
        for key in [KEY, live_oidc.as_str()] {
            let (http, result) = answer_with(key, 403, body).await;
            let error = result.unwrap_err();
            assert_eq!(error.category, ErrorCategory::NotAvailable);
            assert_eq!(
                error.message,
                "Vercel AI Gateway does not let this key read the team's credit balance."
            );
            assert!(!error.message.contains(key));
            assert_eq!(http.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn rate_limits_server_errors_and_other_statuses_keep_their_categories() {
        let limited = fail(
            429,
            r#"{"error":{"message":"Too many requests","type":"rate_limit_exceeded"}}"#,
        )
        .await;
        assert_eq!(limited.category, ErrorCategory::RateLimited);
        assert_eq!(
            limited.message,
            "Vercel AI Gateway is rate limiting usage requests. Waiting before retrying."
        );
        let unavailable = fail(503, r#"{"error":{"message":"Service unavailable"}}"#).await;
        assert_eq!(unavailable.category, ErrorCategory::Http5xx);
        assert_eq!(
            unavailable.message,
            "Vercel AI Gateway answered with HTTP 503."
        );
        let missing = fail(404, r#"{"error":{"message":"Not found"}}"#).await;
        assert_eq!(missing.category, ErrorCategory::Http4xx);
        assert_eq!(missing.message, "Vercel AI Gateway answered with HTTP 404.");
    }

    #[tokio::test]
    async fn an_expired_oidc_token_is_reported_without_a_request() {
        let token = oidc_token(now() - Duration::minutes(1));
        let (http, result) = answer_with(&token, 200, FUNDED).await;
        let error = result.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "Vercel AI Gateway no longer accepts this OIDC token. OIDC tokens last 12 hours; save an AI Gateway API key instead."
        );
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn a_live_oidc_token_is_used_and_its_refusal_reads_as_expired() {
        let token = oidc_token(now() + Duration::hours(6));
        let (http, result) = answer_with(&token, 200, FUNDED).await;
        assert_eq!(result.unwrap().lines, rows(95.5, Some(4.5)));
        let bearer = format!("Bearer {token}");
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some(bearer.as_str())
        );
        let (_, refused) = answer_with(&token, 401, "{}").await;
        let error = refused.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(error.message, OIDC_EXPIRED);
        assert!(!error.message.contains(&token));
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), now());
        let error = Vercel.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "The Vercel AI Gateway API key is missing. Add it again in Accounts."
        );
        assert!(http.requests().is_empty());
    }

    #[test]
    fn connects_with_an_api_key_and_links_to_vercel() {
        assert_eq!(Vercel.id(), "vercel");
        assert_eq!(Vercel.name(), "Vercel AI Gateway");
        let connection = Vercel.connection();
        assert_eq!(connection.login_from, None);
        let help = connection.api_key.expect("an API key connection");
        assert_eq!(help.env, ["AI_GATEWAY_API_KEY"]);
        assert_eq!(
            help.url,
            "https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai-gateway%2Fapi-keys&title=AI+Gateway+API+Keys"
        );
        assert!(help.fields.is_empty());
        assert_eq!(
            Vercel.links(),
            [
                ProviderLink::new("Status", "https://www.vercel-status.com"),
                ProviderLink::new(
                    "Dashboard",
                    "https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai-gateway&title=Go+to+AI+Gateway"
                ),
            ]
        );
        let dir = tempfile::tempdir().unwrap();
        assert!(Vercel.discover(&Roots::under(dir.path())).is_empty());
    }

    #[test]
    fn widgets_read_the_balance_and_spend_rows_and_export_both() {
        let provider = Provider::new("vercel@abc", "Vercel AI Gateway");
        let descriptors = Vercel.descriptors(&provider);
        let widgets: Vec<_> = descriptors
            .iter()
            .map(|descriptor| {
                (
                    descriptor.id.as_str(),
                    descriptor.metric_label.as_str(),
                    descriptor.template.kind,
                    descriptor.template.limit,
                    descriptor.template.unbounded_value_word.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            widgets,
            [
                (
                    "vercel@abc.balance",
                    "Balance",
                    MetricKind::Dollars,
                    None,
                    Some("left")
                ),
                (
                    "vercel@abc.totalSpent",
                    "Total Spent",
                    MetricKind::Dollars,
                    None,
                    Some("spent")
                ),
            ]
        );
        let exports: Vec<_> = descriptors
            .iter()
            .flat_map(|descriptor| &descriptor.limit_resources)
            .map(|resource| {
                (
                    resource.key.as_str(),
                    resource.kind,
                    resource.unit.as_str(),
                    &resource.source,
                    resource.estimated,
                )
            })
            .collect();
        let source = dollars();
        assert_eq!(
            exports,
            [
                ("balance", LimitResourceKind::Balance, "usd", &source, false),
                (
                    "totalSpent",
                    LimitResourceKind::Consumption,
                    "usd",
                    &source,
                    false
                ),
            ]
        );
    }
}
