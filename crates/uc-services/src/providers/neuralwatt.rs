//! Neuralwatt: the energy a Neuralwatt subscription has used of its period's allowance, the credit
//! balance and the credits used, the share of the API key's spending allowance used, and this
//! month's and lifetime cost, energy, requests and tokens.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `NEURALWATT_API_KEY` environment variable or from a key saved in Quota Control, which may also
//! hold an HTTPS server address to use in place of `https://api.neuralwatt.com`. A refresh sends
//! one request, `GET <server>/v1/quota` with the key as a bearer token (`/v1` is added when the
//! address does not already end with it), and nothing is renewed. When the answer leaves out the
//! credit total, the credits used or the balance, the missing one is worked out from the other
//! two; the kWh used and the kWh included are worked out the same way. A key Neuralwatt reports
//! as blocked shows its allowance as fully used.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{
    endpoint::{self, Policy},
    http, lines, value,
};

pub(crate) struct Neuralwatt;

const NAME: &str = "Neuralwatt";

#[async_trait]
impl Service for Neuralwatt {
    fn id(&self) -> &'static str {
        "neuralwatt"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["NEURALWATT_API_KEY"],
            url: "https://portal.neuralwatt.com/dashboard",
            fields: &[("baseUrl", "Server address")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::percent(
                format!("{}.energy", provider.id),
                provider,
                "Energy",
                None,
                None,
            )
            .exporting_progress("energy", "percent"),
            WidgetDescriptor::dollar_balance(
                format!("{}.balance", provider.id),
                provider,
                "Balance",
                None,
                "left",
            ),
            WidgetDescriptor::bounded_dollars(
                format!("{}.credits", provider.id),
                provider,
                "Credits Used",
                None,
                100.0,
                None,
                None,
            )
            .exporting_progress("credits", "usd"),
            WidgetDescriptor::percent(
                format!("{}.allowance", provider.id),
                provider,
                "Key Allowance",
                None,
                None,
            )
            .exporting_progress("allowance", "percent"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let token = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Neuralwatt API key is missing."))?;
        let mut base = endpoint::base_url(
            context.secret.str("/baseUrl"),
            Some("https://api.neuralwatt.com"),
            Policy::Https,
            NAME,
        )?;
        if !base.ends_with("/v1") {
            base.push_str("/v1");
        }
        let root = http::json(
            context.http,
            HttpRequest::get(format!("{base}/quota")).bearer(token),
            NAME,
        )
        .await?;
        if !root["balance"].is_object() {
            return Err(http::decoding(NAME));
        }
        let balance = &root["balance"];
        let subscription = &root["subscription"];
        let allowance = &root["key"]["allowance"];
        for section in [
            subscription,
            &root["key"],
            allowance,
            &root["usage"],
            &root["usage"]["lifetime"],
            &root["usage"]["current_month"],
        ] {
            if !section.is_null() && !section.is_object() {
                return Err(http::decoding(NAME));
            }
        }
        for (section, field) in [
            (subscription, "auto_renew"),
            (subscription, "in_overage"),
            (allowance, "blocked"),
        ] {
            if !section[field].is_null() && !section[field].is_boolean() {
                return Err(http::decoding(NAME));
            }
        }
        let remaining = number(balance, "credits_remaining_usd")?;
        let used = number(balance, "credits_used_usd")?;
        let total = number(balance, "total_credits_usd")?
            .filter(|total| *total > 0.0)
            .or_else(|| {
                used.zip(remaining)
                    .map(|(used, remaining)| used + remaining)
                    .filter(|total| total.is_finite() && *total > 0.0)
            });
        if remaining.is_none() && used.is_none() && total.is_none() {
            return Err(http::decoding(NAME));
        }
        let used = used.or_else(|| {
            total
                .zip(remaining)
                .map(|(total, remaining)| (total - remaining).max(0.0))
        });
        let remaining =
            remaining.or_else(|| total.zip(used).map(|(total, used)| (total - used).max(0.0)));
        let consumed = number(subscription, "kwh_used")?;
        let available = number(subscription, "kwh_remaining")?;
        let included = number(subscription, "kwh_included")?
            .filter(|included| *included > 0.0)
            .or_else(|| {
                consumed
                    .zip(available)
                    .map(|(consumed, available)| consumed + available)
                    .filter(|included| included.is_finite() && *included > 0.0)
            });
        let consumed = consumed.or_else(|| {
            included
                .zip(available)
                .map(|(included, available)| (included - available).max(0.0))
        });
        let parse_date = |field: &str| -> Result<Option<DateTime<Utc>>, SimpleProviderError> {
            if subscription[field].is_null() {
                Ok(None)
            } else {
                let text = subscription[field]
                    .as_str()
                    .ok_or_else(|| http::decoding(NAME))?;
                DateTime::parse_from_rfc3339(text)
                    .ok()
                    .map(|time| Some(time.with_timezone(&Utc)))
                    .ok_or_else(|| http::decoding(NAME))
            }
        };
        let start = parse_date("current_period_start")?;
        let end = parse_date("current_period_end")?;
        let period = start
            .zip(end)
            .map(|(start, end)| (end - start).num_milliseconds())
            .filter(|length| *length > 0);
        let mut rows = Vec::new();
        if let Some(row) = consumed.zip(included).and_then(|(consumed, included)| {
            lines::percent_of("Energy", consumed, included, end, period)
        }) {
            rows.push(row);
        }
        if let Some(remaining) = remaining {
            rows.push(lines::dollar_value("Balance", remaining));
        }
        if let Some(used) = used {
            rows.push(if let Some(total) = total {
                lines::dollars("Credits Used", used, total, None, None)
            } else {
                lines::dollar_value("Credits Used", used)
            });
        }
        let limit = number(allowance, "limit_usd")?;
        let spent = number(allowance, "spent_usd")?;
        number(allowance, "remaining_usd")?;
        if allowance["blocked"] == true {
            rows.push(lines::percent("Key Allowance", 100.0, None, None));
        } else if let Some(row) = spent
            .zip(limit)
            .and_then(|(spent, limit)| lines::percent_of("Key Allowance", spent, limit, None, None))
        {
            rows.push(row);
        }
        for (period, label) in [("current_month", "This Month"), ("lifetime", "Lifetime")] {
            let data = &root["usage"][period];
            for (field, title, unit) in [
                ("cost_usd", "Cost", "usd"),
                ("energy_kwh", "Energy", "kWh"),
                ("requests", "Requests", "requests"),
                ("tokens", "Tokens", "tokens"),
            ] {
                if let Some(amount) = number(data, field)? {
                    if matches!(field, "requests" | "tokens") && amount.fract() != 0.0 {
                        return Err(http::decoding(NAME));
                    }
                    let title = format!("{label} {title}");
                    rows.push(if unit == "usd" {
                        lines::dollar_value(&title, amount)
                    } else {
                        lines::count_value(&title, amount, unit)
                    });
                }
            }
        }
        let plan = value::text(subscription, "/plan").and_then(lines::plan_name);
        Ok(
            Reading::new(plan, rows).with_plan_term(end.map(|ends_at| uc_core::PlanTerm::Stated {
                ends_at,
                checked_at: None,
            })),
        )
    }
}

/// The number at `field` of `section`: `None` when it is missing, null or negative, and a
/// decoding error when it is not a finite number.
fn number(section: &Value, field: &str) -> Result<Option<f64>, SimpleProviderError> {
    if section[field].is_null() {
        return Ok(None);
    }
    section[field]
        .as_f64()
        .filter(|amount| amount.is_finite())
        .map(|amount| (amount >= 0.0).then_some(amount))
        .ok_or_else(|| http::decoding(NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn infers_the_balance_from_the_total_and_shows_a_blocked_key_at_its_full_allowance() {
        let body = r#"{"balance":{"total_credits_usd":10,"credits_used_usd":3},"subscription":{"plan":"power_pro","kwh_included":20,"kwh_remaining":15,"current_period_start":"2026-09-01T00:00:00Z","current_period_end":"2026-10-01T00:00:00Z"},"key":{"allowance":{"blocked":true}},"usage":{"current_month":{"tokens":42}}}"#;
        let http = Scripted::new().on("GET", "https://quota.example/v1/quota", 200, body);
        let scope = context_at(
            &http,
            json!({"apiKey":"fixture","baseUrl":"https://quota.example/v1/"}),
            chrono::Utc::now(),
        );
        let reading = Neuralwatt.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, Some("Power Pro".into()));
        assert_eq!(
            reading.plan_term,
            Some(uc_core::PlanTerm::Stated {
                ends_at: "2026-10-01T00:00:00Z".parse().unwrap(),
                checked_at: None,
            })
        );
        assert_eq!(reading.lines[1], lines::dollar_value("Balance", 7.0));
        assert_eq!(
            reading.lines[3],
            lines::percent("Key Allowance", 100.0, None, None)
        );
        assert_eq!(
            reading.lines[4],
            lines::count_value("This Month Tokens", 42.0, "tokens")
        );
        assert_eq!(
            header(&http.requests()[0], "authorization"),
            Some("Bearer fixture")
        );
    }

    #[tokio::test]
    async fn failures_keep_their_categories_and_unsafe_server_addresses_send_nothing() {
        let http = Scripted::new().on(
            "GET",
            "https://api.neuralwatt.com/v1/quota",
            200,
            r#"{"balance":{"credits_remaining_usd":0}}"#,
        );
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        assert_eq!(
            Neuralwatt.fetch(&scope.context()).await.unwrap().lines,
            vec![lines::dollar_value("Balance", 0.0)]
        );
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
            (
                200,
                r#"{"balance":{"credits_used_usd":"1"}}"#,
                ErrorCategory::Decoding,
            ),
        ] {
            let http = Scripted::new().on("GET", "https://quota.example", status, body);
            let scope = context_at(
                &http,
                json!({"apiKey":"fixture","baseUrl":"https://quota.example"}),
                chrono::Utc::now(),
            );
            assert_eq!(
                Neuralwatt
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                category
            );
        }
        for url in [
            "http://127.0.0.1",
            "https://a:b@quota.example",
            "https://quota.example?token=x",
        ] {
            let http = Scripted::new();
            let scope = context_at(
                &http,
                json!({"apiKey":"fixture","baseUrl":url}),
                chrono::Utc::now(),
            );
            assert_eq!(
                Neuralwatt
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                ErrorCategory::AuthInvalid
            );
            assert!(http.requests().is_empty());
        }
    }
}
