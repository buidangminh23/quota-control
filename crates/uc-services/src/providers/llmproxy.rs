//! LLM Proxy: the quota left on a self-hosted LLM Proxy, its requests, tokens and approximate
//! spend, and a row for each provider it routes to, busiest first.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the card takes an API key and the proxy's
//! server address (HTTPS, or plain HTTP on a private network), both saved in Quota Control. A
//! refresh sends one request, `GET <server>/v1/quota-stats` (a `/v1` the address already ends with
//! is not repeated) with the key as a bearer token. The Quota row is the lowest
//! `remaining_percent` among the providers' `quota_groups`, resetting at the earliest future
//! `reset_time`; the totals come from the `summary` when it has them, else from the providers. No
//! credential is renewed and no model is run.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uc_core::{
    HttpRequest, MetricKind, MetricValue, Provider, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{endpoint, http, lines, value};

pub(crate) struct LlmProxy;

const NAME: &str = "LLM Proxy";

#[async_trait]
impl Service for LlmProxy {
    fn id(&self) -> &'static str {
        "llmproxy"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "",
            fields: &[("baseUrl", "Server address")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut widgets = vec![
            WidgetDescriptor::percent(
                format!("{}.quota", provider.id),
                provider,
                "Quota",
                None,
                None,
            )
            .exporting_progress("quota", "percent"),
        ];
        for (suffix, title, kind) in [
            ("requests", "Requests", MetricKind::Count),
            ("tokens", "Tokens", MetricKind::Count),
            ("spend", "Total Usage", MetricKind::Dollars),
        ] {
            widgets.push(WidgetDescriptor::values(
                format!("{}.{s}", provider.id, s = suffix),
                provider,
                title,
                None,
                Some(kind),
                None,
                true,
                None,
                false,
            ));
        }
        widgets
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("Enter the LLM Proxy API key."))?;
        let base = endpoint::base_url(
            context.secret.str("/baseUrl"),
            None,
            endpoint::Policy::HttpsOrPrivateNetworkHttp,
            NAME,
        )?;
        let base = base.strip_suffix("/v1").unwrap_or(&base);
        let body = http::json(
            context.http,
            HttpRequest::get(format!("{base}/v1/quota-stats"))
                .header("Authorization", format!("Bearer {key}"))
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        parse(&body, context.now)
    }
}

/// The number in `field` of `object`, `None` when the field is absent or null. Any other value
/// that is not a finite number, or not a whole one when `integer` is set, cannot be read.
fn number(object: &Value, field: &str, integer: bool) -> Result<Option<f64>, SimpleProviderError> {
    if object[field].is_null() {
        return Ok(None);
    }
    object[field]
        .as_f64()
        .filter(|amount| amount.is_finite() && (!integer || amount.fract() == 0.0))
        .map(Some)
        .ok_or_else(|| http::decoding(NAME))
}

fn parse(body: &Value, now: DateTime<Utc>) -> Result<Reading, SimpleProviderError> {
    let providers = body["providers"]
        .as_object()
        .ok_or_else(|| http::decoding(NAME))?;
    if !body["summary"].is_null() && !body["summary"].is_object() {
        return Err(http::decoding(NAME));
    }
    let (mut provider_rows, mut remaining, mut reset) =
        (Vec::new(), None::<f64>, None::<DateTime<Utc>>);
    let (mut total_requests, mut total_tokens, mut total_cost) = (0.0, 0.0, 0.0);
    for (name, stats) in providers {
        if !stats.is_object() || (!stats["tokens"].is_null() && !stats["tokens"].is_object()) {
            return Err(http::decoding(NAME));
        }
        for field in ["credential_count", "active_count", "exhausted_count"] {
            number(stats, field, true)?;
        }
        let requests = number(stats, "total_requests", true)?.unwrap_or(0.0);
        let mut tokens = 0.0;
        for field in ["input_cached", "input_uncached", "output"] {
            tokens += number(&stats["tokens"], field, true)?.unwrap_or(0.0);
        }
        let cost = number(stats, "approx_cost", false)?;
        total_requests += requests;
        total_tokens += tokens;
        total_cost += cost.unwrap_or(0.0);
        let groups: Vec<&Value> = match &stats["quota_groups"] {
            Value::Array(list) => list.iter().collect(),
            Value::Object(by_name) => by_name.values().collect(),
            _ => vec![],
        };
        let parsed: Result<Vec<_>, SimpleProviderError> = groups
            .into_iter()
            .map(|group| {
                if !group.is_object()
                    || (!group["reset_time"].is_null() && !group["reset_time"].is_string())
                {
                    return Err(http::decoding(NAME));
                }
                Ok((
                    number(group, "remaining_percent", false)?,
                    value::time(group, "/reset_time").filter(|time| *time > now),
                ))
            })
            .collect();
        if let Ok(groups) = parsed {
            for (left, time) in groups {
                if let Some(left) = left {
                    remaining = Some(remaining.map_or(left, |lowest| lowest.min(left)));
                }
                if let Some(time) = time {
                    reset = Some(reset.map_or(time, |earliest| earliest.min(time)));
                }
            }
        }
        let mut values = vec![
            MetricValue::count(requests, "requests"),
            MetricValue::count(tokens, "tokens"),
        ];
        if let Some(cost) = cost {
            values.push(MetricValue::dollars(cost));
        }
        let label = if ["Quota", "Requests", "Tokens", "Total Usage"].contains(&name.as_str()) {
            format!("Provider: {name}")
        } else {
            name.clone()
        };
        provider_rows.push((requests, label, values));
    }
    let total_requests =
        number(&body["summary"], "total_requests", true)?.unwrap_or(total_requests);
    let total_tokens = number(&body["summary"], "total_tokens", true)?.unwrap_or(total_tokens);
    let cost = number(&body["summary"], "approx_cost", false)?
        .or((total_cost > 0.0).then_some(total_cost));
    if ![total_requests, total_tokens, total_cost]
        .iter()
        .all(|total| total.is_finite())
    {
        return Err(http::decoding(NAME));
    }
    let mut rows = vec![];
    if let Some(left) = remaining {
        rows.push(lines::percent("Quota", 100.0 - left, reset, None));
    }
    rows.push(lines::count_value("Requests", total_requests, "requests"));
    rows.push(lines::count_value("Tokens", total_tokens, "tokens"));
    if let Some(cost) = cost {
        rows.push(lines::dollar_value("Total Usage", cost));
    }
    provider_rows.sort_by(|left, right| right.0.total_cmp(&left.0).then(left.1.cmp(&right.1)));
    rows.extend(
        provider_rows
            .into_iter()
            .map(|(_, label, values)| lines::values(&label, values)),
    );
    Ok(Reading::new(None, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    fn now() -> DateTime<Utc> {
        "2026-05-01T00:00:00Z".parse().unwrap()
    }

    #[tokio::test]
    async fn the_quota_totals_and_a_row_per_provider_come_from_one_quota_stats_request() {
        let body = json!({"providers":{"openai":{"total_requests":120,"tokens":{"input_cached":1000,"input_uncached":2000,"output":3000},"approx_cost":12.5,"quota_groups":{"default":{"remaining_percent":42,"reset_time":"2026-05-18T12:00:00Z"}}},"anthropic":{"total_requests":40,"tokens":{"output":1000},"approx_cost":3}},"summary":{"total_requests":160,"total_tokens":7000,"approx_cost":15.5}});
        let http = Scripted::new().on(
            "GET",
            "https://proxy.example/v1/quota-stats",
            200,
            &body.to_string(),
        );
        let scope = context_at(
            &http,
            json!({"apiKey":"fixture","baseUrl":"https://proxy.example/v1/"}),
            now(),
        );
        let reading = LlmProxy.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(
            reading.lines,
            vec![
                lines::percent(
                    "Quota",
                    58.0,
                    Some("2026-05-18T12:00:00Z".parse().unwrap()),
                    None
                ),
                lines::count_value("Requests", 160.0, "requests"),
                lines::count_value("Tokens", 7000.0, "tokens"),
                lines::dollar_value("Total Usage", 15.5),
                lines::values(
                    "openai",
                    vec![
                        MetricValue::count(120.0, "requests"),
                        MetricValue::count(6000.0, "tokens"),
                        MetricValue::dollars(12.5)
                    ]
                ),
                lines::values(
                    "anthropic",
                    vec![
                        MetricValue::count(40.0, "requests"),
                        MetricValue::count(1000.0, "tokens"),
                        MetricValue::dollars(3.0)
                    ]
                )
            ]
        );
        assert_eq!(http.requests().len(), 1);
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer fixture")
        );
    }

    #[tokio::test]
    async fn http_errors_keep_their_categories_without_echoing_the_body() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (403, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (500, ErrorCategory::http(500)),
        ] {
            let http = Scripted::new().on(
                "GET",
                "https://proxy.example/v1/quota-stats",
                status,
                "secret-echo",
            );
            let scope = context_at(
                &http,
                json!({"apiKey":"fixture","baseUrl":"https://proxy.example"}),
                now(),
            );
            let error = LlmProxy.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret-echo"));
        }
    }

    #[test]
    fn malformed_answers_fail_and_optional_fields_may_be_missing() {
        assert!(parse(&json!({}), now()).is_err());
        assert!(parse(&json!({"providers":{"a":{"total_requests":"3"}}}), now()).is_err());
        let body = json!({"providers":{"a":{"total_requests":2,"quota_groups":"invalid"}},"summary":{"approx_cost":0}});
        let reading = parse(&body, now()).unwrap();
        assert_eq!(
            reading.lines[0],
            lines::count_value("Requests", 2.0, "requests")
        );
        assert_eq!(reading.lines[2], lines::dollar_value("Total Usage", 0.0));
    }
}
