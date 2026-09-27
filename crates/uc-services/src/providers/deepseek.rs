//! DeepSeek: the prepaid balance of a DeepSeek platform API key.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `DEEPSEEK_API_KEY` environment variable or from a key saved in Quota Control. The DeepSeek TUI's
//! `~/.deepseek/config.toml` is left alone on purpose: tools such as 9router point it at their own
//! proxy, so its `api_key` may belong to another service. A refresh sends one request,
//! `GET https://api.deepseek.com/user/balance` with the key as a bearer token, and shows the
//! balance of each funded currency on one row (dollars for USD, a `CNY` amount for yuan), with a
//! warning when DeepSeek reports the balance too low for API calls.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{
    HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, MetricValue, Provider,
    ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct DeepSeek;

const NAME: &str = "DeepSeek";
const BALANCE_URL: &str = "https://api.deepseek.com/user/balance";
const BALANCE: &str = "Balance";
/// The unit word of a yuan balance, which its limits export also matches.
const CNY: &str = "CNY";
const LOW_BALANCE: &str = "The balance is not enough for DeepSeek API calls. Top up to continue.";
const KEY_REFUSED: &str = "DeepSeek refused this API key. Check it or create a new one.";

#[async_trait]
impl Service for DeepSeek {
    fn id(&self) -> &'static str {
        "deepseek"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://status.deepseek.com"),
            ProviderLink::new("Usage", "https://platform.deepseek.com/usage"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["DEEPSEEK_API_KEY"],
            url: "https://platform.deepseek.com/api_keys",
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
                LimitResourceSource::Value {
                    kind: MetricKind::Dollars,
                    label: None,
                },
                false,
            )
            .exporting_limit(
                "balanceCny",
                LimitResourceKind::Balance,
                "cny",
                LimitResourceSource::Value {
                    kind: MetricKind::Count,
                    label: Some(CNY.into()),
                },
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context.secret.key().ok_or_else(|| {
            http::invalid("The DeepSeek API key is missing. Add it again in Accounts.")
        })?;
        let response = http::send(
            context.http,
            HttpRequest::get(BALANCE_URL)
                .bearer(key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        if matches!(response.status, 401 | 403) {
            return Err(http::invalid(KEY_REFUSED));
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let body = http::parse(&response, NAME)?;
        let balances = balances(&body).ok_or_else(|| http::decoding(NAME))?;
        let lines = if balances.is_empty() {
            Vec::new()
        } else {
            vec![lines::values(BALANCE, balances)]
        };
        let warning =
            (value::flag(&body, "/is_available") == Some(false)).then(|| LOW_BALANCE.to_string());
        Ok(Reading::new(None, lines).with_warning(warning))
    }
}

/// The balance of each currency, dollars first. A currency with nothing left is dropped when
/// another one has money, so a funded account reads "$12.50 left" rather than "$12.50 · 0 CNY";
/// an account with no money anywhere keeps its first currency, so the row shows a real zero.
/// `None` when the answer carries no balance list, or a list none of whose entries can be read,
/// so a renamed field reads as a version problem rather than as an account without money.
fn balances(body: &Value) -> Option<Vec<MetricValue>> {
    let infos = body.get("balance_infos")?.as_array()?;
    let mut totals: Vec<(String, f64)> = Vec::new();
    for info in infos {
        let (Some(currency), Some(total)) = (
            value::text(info, "/currency"),
            value::number(info, "/total_balance"),
        ) else {
            continue;
        };
        let currency = currency.to_ascii_uppercase();
        let total = total.max(0.0);
        match totals.iter_mut().find(|(known, _)| *known == currency) {
            Some((_, sum)) => *sum += total,
            None => totals.push((currency, total)),
        }
    }
    if totals.is_empty() && !infos.is_empty() {
        return None;
    }
    totals.sort_by_key(|(currency, _)| currency.as_str() != "USD");
    if totals.iter().any(|(_, total)| *total > 0.0) {
        totals.retain(|(_, total)| *total > 0.0);
    } else {
        totals.truncate(1);
    }
    Some(
        totals
            .into_iter()
            .map(|(currency, total)| match currency.as_str() {
                "USD" => MetricValue::dollars(total),
                _ => MetricValue::count(total, currency),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Roots;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{DateTime, TimeZone, Utc};
    use serde_json::json;
    use uc_core::{ErrorCategory, MetricLine};

    const KEY: &str = "sk-test-0123456789abcdef";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn balance_row(values: Vec<MetricValue>) -> Vec<MetricLine> {
        vec![lines::values("Balance", values)]
    }

    async fn answer(status: u16, body: &str) -> (Scripted, Result<Reading, SimpleProviderError>) {
        let http = Scripted::new().on("GET", BALANCE_URL, status, body);
        let scope = context_at(&http, json!({ "apiKey": KEY }), now());
        let result = DeepSeek.fetch(&scope.context()).await;
        (http, result)
    }

    async fn read(body: &str) -> Reading {
        answer(200, body).await.1.unwrap()
    }

    async fn fail(status: u16, body: &str) -> SimpleProviderError {
        answer(status, body).await.1.unwrap_err()
    }

    #[tokio::test]
    async fn a_funded_dollar_account_shows_its_balance_without_the_empty_yuan_wallet() {
        let reading = read(
            r#"{"is_available":true,"balance_infos":[
                {"currency":"USD","total_balance":"12.50","granted_balance":"2.50","topped_up_balance":"10.00"},
                {"currency":"CNY","total_balance":"0.00","granted_balance":"0.00","topped_up_balance":"0.00"}
            ]}"#,
        )
        .await;
        assert_eq!(
            reading,
            Reading::new(None, balance_row(vec![MetricValue::dollars(12.5)]))
        );
    }

    #[tokio::test]
    async fn a_yuan_account_shows_its_balance_in_cny() {
        let reading = read(
            r#"{"is_available":true,"balance_infos":[
                {"currency":"CNY","total_balance":"110.00","granted_balance":"10.00","topped_up_balance":"100.00"}
            ]}"#,
        )
        .await;
        assert_eq!(
            reading,
            Reading::new(None, balance_row(vec![MetricValue::count(110.0, "CNY")]))
        );
    }

    #[tokio::test]
    async fn two_funded_currencies_share_the_row_with_dollars_first() {
        let reading = read(
            r#"{"is_available":true,"balance_infos":[
                {"currency":"cny","total_balance":"20.00","granted_balance":"0.00","topped_up_balance":"20.00"},
                {"currency":"USD","total_balance":3.4,"granted_balance":0,"topped_up_balance":3.4}
            ]}"#,
        )
        .await;
        assert_eq!(
            reading.lines,
            balance_row(vec![
                MetricValue::dollars(3.4),
                MetricValue::count(20.0, "CNY")
            ])
        );
        assert_eq!(reading.warning, None);
    }

    #[tokio::test]
    async fn an_empty_account_shows_a_real_zero_and_warns_that_calls_will_fail() {
        let reading = read(
            r#"{"is_available":false,"balance_infos":[
                {"currency":"CNY","total_balance":"0.00","granted_balance":"0.00","topped_up_balance":"0.00"},
                {"currency":"USD","total_balance":"-0.35","granted_balance":"0.00","topped_up_balance":"-0.35"}
            ]}"#,
        )
        .await;
        assert_eq!(
            reading,
            Reading::new(None, balance_row(vec![MetricValue::dollars(0.0)])).with_warning(Some(
                "The balance is not enough for DeepSeek API calls. Top up to continue.".into()
            ))
        );
    }

    #[tokio::test]
    async fn an_empty_balance_list_reads_as_no_data_rather_than_an_error() {
        let reading = read(r#"{"is_available":true,"balance_infos":[]}"#).await;
        assert_eq!(reading, Reading::new(None, Vec::new()));
    }

    #[tokio::test]
    async fn sends_the_key_as_a_bearer_token_to_the_balance_endpoint() {
        let (http, result) = answer(
            200,
            r#"{"is_available":true,"balance_infos":[{"currency":"USD","total_balance":"1.00"}]}"#,
        )
        .await;
        result.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, "https://api.deepseek.com/user/balance");
        assert_eq!(
            header(request, "Authorization"),
            Some("Bearer sk-test-0123456789abcdef")
        );
        assert_eq!(header(request, "Accept"), Some("application/json"));
        assert_eq!(request.body, None);
        assert_eq!(request.timeout, HttpRequest::DEFAULT_TIMEOUT);
    }

    #[tokio::test]
    async fn a_refused_key_is_reported_without_revealing_it() {
        for status in [401, 403] {
            let error = fail(
                status,
                r#"{"error":{"message":"Authentication Fails, Your api key: ****cdef is invalid","type":"authentication_error"}}"#,
            )
            .await;
            assert_eq!(error.category, ErrorCategory::AuthInvalid, "{status}");
            assert_eq!(
                error.message,
                "DeepSeek refused this API key. Check it or create a new one."
            );
            assert!(!error.message.contains(KEY));
        }
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_keep_their_categories() {
        assert_eq!(
            fail(429, r#"{"error":{"message":"Rate Limit Reached"}}"#)
                .await
                .category,
            ErrorCategory::RateLimited
        );
        let overloaded = fail(503, "Server Overloaded").await;
        assert_eq!(overloaded.category, ErrorCategory::Http5xx);
        assert_eq!(overloaded.message, "DeepSeek answered with HTTP 503.");
    }

    #[tokio::test]
    async fn an_answer_without_a_balance_list_is_a_decoding_error() {
        for body in [r#"{"is_available":true}"#, "<html>busy</html>"] {
            let error = fail(200, body).await;
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
            assert_eq!(
                error.message,
                "DeepSeek returned usage data this version cannot read."
            );
        }
    }

    #[tokio::test]
    async fn a_balance_list_with_no_readable_entry_is_a_decoding_error() {
        let error = fail(
            200,
            r#"{"is_available":true,"balance_infos":[{"currency":"USD","balance":"12.50"}]}"#,
        )
        .await;
        assert_eq!(error.category, ErrorCategory::Decoding);
        assert_eq!(
            error.message,
            "DeepSeek returned usage data this version cannot read."
        );
    }

    #[tokio::test]
    async fn unreadable_entries_are_skipped_when_another_one_reads() {
        let reading = read(
            r#"{"is_available":true,"balance_infos":[
                {"currency":"USD","total_balance":"n/a"},
                {"total_balance":"5.00"},
                "CNY",
                {"currency":"CNY","total_balance":"42.10","granted_balance":"0.00","topped_up_balance":"42.10"}
            ]}"#,
        )
        .await;
        assert_eq!(
            reading,
            Reading::new(None, balance_row(vec![MetricValue::count(42.1, "CNY")]))
        );
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), now());
        let error = DeepSeek.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert!(http.requests().is_empty());
    }

    #[test]
    fn connects_with_an_api_key_only() {
        let connection = DeepSeek.connection();
        assert_eq!(connection.login_from, None);
        let help = connection.api_key.expect("an API key connection");
        assert_eq!(help.env, ["DEEPSEEK_API_KEY"]);
        assert_eq!(help.url, "https://platform.deepseek.com/api_keys");
        assert!(help.fields.is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(DeepSeek.discover(&Roots::under(dir.path())).is_empty());
    }

    #[test]
    fn the_balance_widget_reads_the_balance_row_and_exports_both_currencies() {
        let provider = Provider::new("deepseek@abc", "DeepSeek");
        let descriptors = DeepSeek.descriptors(&provider);
        assert_eq!(descriptors.len(), 1);
        let balance = &descriptors[0];
        assert_eq!(balance.id, "deepseek@abc.balance");
        assert_eq!(balance.metric_label, "Balance");
        assert_eq!(balance.template.limit, None);
        assert_eq!(
            balance.template.unbounded_value_word.as_deref(),
            Some("left")
        );
        let exports: Vec<_> = balance
            .limit_resources
            .iter()
            .map(|resource| {
                (
                    resource.key.as_str(),
                    resource.unit.as_str(),
                    &resource.source,
                )
            })
            .collect();
        assert_eq!(
            exports,
            [
                (
                    "balance",
                    "usd",
                    &LimitResourceSource::Value {
                        kind: MetricKind::Dollars,
                        label: None
                    }
                ),
                (
                    "balanceCny",
                    "cny",
                    &LimitResourceSource::Value {
                        kind: MetricKind::Count,
                        label: Some("CNY".into())
                    }
                ),
            ]
        );
        assert!(
            balance
                .limit_resources
                .iter()
                .all(|resource| resource.kind == LimitResourceKind::Balance)
        );
    }

    /// The limits API takes, for each export, the row's first value of the export's kind (and of
    /// its label when it names one); both currencies of a funded account must be found that way.
    #[tokio::test]
    async fn each_limit_export_finds_its_currency_in_the_balance_row() {
        let reading = read(
            r#"{"is_available":true,"balance_infos":[
                {"currency":"CNY","total_balance":"20.00","granted_balance":"0.00","topped_up_balance":"20.00"},
                {"currency":"USD","total_balance":"3.40","granted_balance":"0.00","topped_up_balance":"3.40"}
            ]}"#,
        )
        .await;
        let [MetricLine::Values(row)] = reading.lines.as_slice() else {
            panic!("one values row, got {:?}", reading.lines)
        };
        let descriptors = DeepSeek.descriptors(&Provider::new("deepseek@abc", "DeepSeek"));
        let exported: Vec<(&str, f64)> = descriptors[0]
            .limit_resources
            .iter()
            .map(|resource| {
                let LimitResourceSource::Value { kind, label } = &resource.source else {
                    panic!("a value export")
                };
                let value = row
                    .values
                    .iter()
                    .find(|value| {
                        value.kind == *kind
                            && label
                                .as_ref()
                                .is_none_or(|label| value.label.as_ref() == Some(label))
                    })
                    .expect("a value for every export");
                (resource.key.as_str(), value.number)
            })
            .collect();
        assert_eq!(exported, [("balance", 3.4), ("balanceCny", 20.0)]);
    }
}
