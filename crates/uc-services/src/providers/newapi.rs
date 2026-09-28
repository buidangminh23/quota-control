//! OpenAI-compatible relay (One API and New API): the balance left and the amount used on a relay
//! key, in US dollars.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the card takes the relay's address (HTTPS,
//! or plain HTTP on a private network) and one of its `sk-` keys, both saved in Quota Control. The
//! relays keep amounts as whole quota units, and `GET <relay>/api/status` (no key) says how many
//! units make one US dollar (`quota_per_unit`), so amounts are converted here from raw units rather
//! than taken from the relays' `*_usd` billing fields, whose unit changes with the site's display
//! settings (US dollars, yuan or tokens).
//!
//! - New API: `GET <relay>/api/usage/token/` with the key as a bearer answers the key's
//!   `total_used` and `total_available` units, and whether it is `unlimited_quota`.
//! - One API, which has no such route (404): `GET <relay>/v1/dashboard/billing/subscription`
//!   (`hard_limit_usd`, the units granted) and `GET <relay>/v1/dashboard/billing/usage`
//!   (`total_usage`, units used times 100). One API divides both by `quota_per_unit` itself only
//!   when `display_in_currency` is on, which `/api/status` also reports, so either way the amounts
//!   end up in US dollars. A limit of 100000000 units or more marks a key without a limit.
//!
//! Sources: https://github.com/songquanpeng/one-api/blob/main/controller/billing.go,
//! https://github.com/QuantumNous/new-api/blob/main/controller/token.go and each project's
//! `controller/misc.go`. No model is run and nothing is written.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{
    HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, Provider, SimpleProviderError,
    WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{endpoint, http, lines, value};

pub(crate) struct NewApi;

const NAME: &str = "OpenAI-compatible relay";
const BALANCE: &str = "Balance";
const USED: &str = "Used";
/// The units a relay reports for a key without a limit.
const UNLIMITED_UNITS: f64 = 100_000_000.0;
const KEY_REFUSED: &str = "The relay refused the key. Paste one of its sk- keys.";
const NO_RATE: &str = "The relay does not say how many quota units make a dollar (/api/status), so amounts cannot be shown safely.";

#[async_trait]
impl Service for NewApi {
    fn id(&self) -> &'static str {
        "newapi"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "",
            fields: &[("baseUrl", "Relay address")],
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
            ),
            WidgetDescriptor::values(
                format!("{}.used", provider.id),
                provider,
                USED,
                None,
                Some(MetricKind::Dollars),
                None,
                true,
                None,
                false,
            )
            .exporting_limit(
                "used",
                LimitResourceKind::Consumption,
                "usd",
                LimitResourceSource::Value {
                    kind: MetricKind::Dollars,
                    label: None,
                },
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("Enter the relay's API key."))?;
        let base = endpoint::base_url(
            context.secret.str("/baseUrl"),
            None,
            endpoint::Policy::HttpsOrPrivateNetworkHttp,
            NAME,
        )?;
        let base = base.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        let status = relay_status(context, base).await?;
        let response = http::send(
            context.http,
            keyed(&format!("{base}/api/usage/token/"), key),
            NAME,
        )
        .await?;
        match response.status {
            401 | 403 => Err(http::invalid(KEY_REFUSED)),
            404 | 405 => one_api(context, base, key, &status).await,
            _ if !response.is_success() => Err(http::status_error(&response, NAME)),
            _ => new_api(&http::parse(&response, NAME)?, &status),
        }
    }
}

/// What `/api/status` says about the relay's amounts.
struct RelayStatus {
    /// Quota units in one US dollar.
    per_dollar: f64,
    /// Whether One API's billing answers are already in dollars.
    in_currency: bool,
}

async fn relay_status(
    context: &FetchContext<'_>,
    base: &str,
) -> Result<RelayStatus, SimpleProviderError> {
    let body = http::json(
        context.http,
        HttpRequest::get(format!("{base}/api/status")).header("Accept", "application/json"),
        NAME,
    )
    .await?;
    let per_dollar = value::number(&body, "/data/quota_per_unit")
        .filter(|units| units.is_finite() && *units > 0.0)
        .ok_or_else(|| http::not_available(NO_RATE))?;
    let in_currency = body
        .pointer("/data/display_in_currency")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(RelayStatus {
        per_dollar,
        in_currency,
    })
}

fn keyed(url: &str, key: &str) -> HttpRequest {
    HttpRequest::get(url)
        .bearer(key)
        .header("Accept", "application/json")
}

/// New API's own answer about the key, in raw units.
fn new_api(body: &Value, status: &RelayStatus) -> Result<Reading, SimpleProviderError> {
    let data = body.get("data").ok_or_else(|| http::decoding(NAME))?;
    let used = value::number(data, "/total_used").ok_or_else(|| http::decoding(NAME))?;
    let unlimited = data
        .get("unlimited_quota")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let left = value::number(data, "/total_available")
        .filter(|units| !unlimited && *units < UNLIMITED_UNITS);
    Ok(reading(
        left.map(|units| units / status.per_dollar),
        used / status.per_dollar,
    ))
}

/// One API's billing answers, in dollars when the relay shows currency and in raw units otherwise.
async fn one_api(
    context: &FetchContext<'_>,
    base: &str,
    key: &str,
    status: &RelayStatus,
) -> Result<Reading, SimpleProviderError> {
    let billing = |path: &str| keyed(&format!("{base}/v1/dashboard/billing/{path}"), key);
    let subscription = billing_json(context, billing("subscription")).await?;
    let usage = billing_json(context, billing("usage")).await?;
    let limit =
        value::number(&subscription, "/hard_limit_usd").ok_or_else(|| http::decoding(NAME))?;
    let used = value::number(&usage, "/total_usage").ok_or_else(|| http::decoding(NAME))? / 100.0;
    let (limit, used) = if status.in_currency {
        (limit, used)
    } else {
        (limit / status.per_dollar, used / status.per_dollar)
    };
    let unlimited = limit * status.per_dollar >= UNLIMITED_UNITS;
    let left = (!unlimited).then(|| (limit - used).max(0.0));
    Ok(reading(left, used))
}

async fn billing_json(
    context: &FetchContext<'_>,
    request: HttpRequest,
) -> Result<Value, SimpleProviderError> {
    let response = http::send(context.http, request, NAME).await?;
    match response.status {
        401 | 403 => Err(http::invalid(KEY_REFUSED)),
        _ if !response.is_success() => Err(http::status_error(&response, NAME)),
        _ => http::parse(&response, NAME),
    }
}

fn reading(left: Option<f64>, used: f64) -> Reading {
    let mut rows = Vec::new();
    if let Some(left) = left {
        rows.push(lines::dollar_value(BALANCE, left.max(0.0)));
    }
    rows.push(lines::dollar_value(USED, used.max(0.0)));
    Reading::new(None, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;

    const RELAY: &str = "https://relay.example";

    fn status(per_unit: f64, in_currency: bool) -> String {
        json!({"success": true, "data": {"quota_per_unit": per_unit, "display_in_currency": in_currency}})
            .to_string()
    }

    fn secret() -> Value {
        json!({"apiKey": "sk-test", "baseUrl": "https://relay.example/v1/"})
    }

    async fn read(http: &Scripted) -> Result<Reading, SimpleProviderError> {
        let scope = context_at(http, secret(), Utc::now());
        NewApi.fetch(&scope.context()).await
    }

    #[tokio::test]
    async fn new_api_amounts_are_converted_from_raw_units() {
        let http = Scripted::new()
            .on("GET", &format!("{RELAY}/api/status"), 200, &status(500000.0, false))
            .on(
                "GET",
                &format!("{RELAY}/api/usage/token/"),
                200,
                r#"{"code":true,"data":{"total_granted":5000000,"total_used":1250000,"total_available":3750000,"unlimited_quota":false}}"#,
            );
        let reading = read(&http).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::dollar_value(BALANCE, 7.5),
                lines::dollar_value(USED, 2.5)
            ]
        );
        assert_eq!(
            header(&http.requests()[1], "Authorization"),
            Some("Bearer sk-test")
        );
    }

    #[tokio::test]
    async fn an_unlimited_new_api_key_shows_only_what_it_used() {
        let http = Scripted::new()
            .on("GET", &format!("{RELAY}/api/status"), 200, &status(500000.0, false))
            .on(
                "GET",
                &format!("{RELAY}/api/usage/token/"),
                200,
                r#"{"data":{"total_used":500000,"total_available":100000000,"unlimited_quota":true}}"#,
            );
        let reading = read(&http).await.unwrap();
        assert_eq!(reading.lines, vec![lines::dollar_value(USED, 1.0)]);
    }

    fn one_api(in_currency: bool, limit: f64, usage: f64) -> Scripted {
        Scripted::new()
            .on(
                "GET",
                &format!("{RELAY}/api/status"),
                200,
                &status(500000.0, in_currency),
            )
            .on(
                "GET",
                &format!("{RELAY}/api/usage/token/"),
                404,
                "404 page not found",
            )
            .on(
                "GET",
                &format!("{RELAY}/v1/dashboard/billing/subscription"),
                200,
                &json!({"object": "billing_subscription", "hard_limit_usd": limit}).to_string(),
            )
            .on(
                "GET",
                &format!("{RELAY}/v1/dashboard/billing/usage"),
                200,
                &json!({"object": "list", "total_usage": usage}).to_string(),
            )
    }

    #[tokio::test]
    async fn one_api_in_currency_is_already_in_dollars() {
        let reading = read(&one_api(true, 10.0, 250.0)).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::dollar_value(BALANCE, 7.5),
                lines::dollar_value(USED, 2.5)
            ]
        );
    }

    #[tokio::test]
    async fn one_api_in_raw_units_is_converted_with_the_relays_rate() {
        let reading = read(&one_api(false, 5_000_000.0, 125_000_000.0))
            .await
            .unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::dollar_value(BALANCE, 7.5),
                lines::dollar_value(USED, 2.5)
            ]
        );
    }

    #[tokio::test]
    async fn a_relay_without_a_dollar_rate_shows_nothing_rather_than_a_wrong_unit() {
        let http = Scripted::new().on(
            "GET",
            &format!("{RELAY}/api/status"),
            200,
            r#"{"success":true,"data":{}}"#,
        );
        let error = read(&http).await.unwrap_err();
        assert_eq!(error.message, NO_RATE);
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_refused_key_says_which_key_to_paste() {
        let http = Scripted::new()
            .on(
                "GET",
                &format!("{RELAY}/api/status"),
                200,
                &status(500000.0, false),
            )
            .on("GET", &format!("{RELAY}/api/usage/token/"), 401, "{}");
        let error = read(&http).await.unwrap_err();
        assert_eq!(error.message, KEY_REFUSED);
    }

    #[test]
    fn the_relay_takes_a_key_and_its_address() {
        let connection = NewApi.connection();
        assert!(connection.api_key.is_some());
        assert_eq!(
            NewApi.descriptors(&Provider::new("newapi@x", NAME)).len(),
            2
        );
    }
}
