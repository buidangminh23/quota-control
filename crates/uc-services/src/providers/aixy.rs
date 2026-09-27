//! Aixy: the budgets, weekly spend, requests and tokens of an Aixy gateway API key.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `AIXY_API_KEY`
//! environment variable or from a key saved in Quota Control, with an optional server address. The
//! public gateway `https://api.aixy-gateway.com` is the default, and a custom gateway may also use
//! plain HTTP to this computer or a private network. A refresh sends one request,
//! `GET <server>/v1/usage` with the key as a bearer token (a server address may already end in
//! `/v1`); a gateway that answers 404 is reported as not providing usage.
//!
//! The card leads with the first two budgets whose spend is known, hard ones before monitored ones
//! and fuller ones first, as Budget and Secondary Budget, then the spend, requests and tokens of
//! the last seven days, then every budget on a row of its own titled by its scope, interval,
//! sharing and enforcement. A hard budget counts reserved spend as used, and a budget whose spend
//! is unknown shows as Unavailable rather than as zero.

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uc_core::{
    HttpRequest, MetricKind, MetricLine, Provider, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{endpoint, http, lines};

pub(crate) struct Aixy;

const NAME: &str = "Aixy";

#[async_trait]
impl Service for Aixy {
    fn id(&self) -> &'static str {
        "aixy"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["AIXY_API_KEY"],
            url: "https://api.aixy-gateway.com",
            fields: &[("baseUrl", "Server address")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut descriptors = vec![];
        for (suffix, title) in [
            ("budget", "Budget"),
            ("secondaryBudget", "Secondary Budget"),
        ] {
            descriptors.push(
                WidgetDescriptor::bounded_dollars(
                    format!("{}.{s}", provider.id, s = suffix),
                    provider,
                    title,
                    None,
                    100.0,
                    None,
                    None,
                )
                .exporting_progress(suffix, "usd"),
            );
        }
        for (suffix, title, kind) in [
            ("spend", "Weekly Spend", MetricKind::Dollars),
            ("requests", "Requests", MetricKind::Count),
            ("tokens", "Tokens", MetricKind::Count),
        ] {
            descriptors.push(WidgetDescriptor::values(
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
        descriptors
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("Enter the Aixy API key."))?;
        let base = endpoint::base_url(
            context.secret.str("/baseUrl"),
            Some("https://api.aixy-gateway.com"),
            endpoint::Policy::HttpsOrPrivateNetworkHttp,
            NAME,
        )?;
        let base = base.strip_suffix("/v1").unwrap_or(&base);
        let response = http::send(
            context.http,
            HttpRequest::get(format!("{base}/v1/usage"))
                .header("Authorization", format!("Bearer {key}"))
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        if response.status == 404 {
            return Err(http::not_available(
                "This Aixy gateway does not provide usage. Update it or check the server address.",
            ));
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        parse(&http::parse(&response, NAME)?)
    }
}

/// The trimmed text in `field` of `object`, which must not be empty.
fn text<'a>(object: &'a Value, field: &str) -> Result<&'a str, SimpleProviderError> {
    object[field]
        .as_str()
        .map(str::trim)
        .filter(|trimmed| !trimmed.is_empty())
        .ok_or_else(|| http::decoding(NAME))
}

/// A non-negative amount, given as a number or as plain decimal text; `None` for a null.
fn amount(value: &Value) -> Result<Option<f64>, SimpleProviderError> {
    if value.is_null() {
        return Ok(None);
    }
    let number = if let Some(decimal) = value.as_str() {
        if decimal.is_empty()
            || !decimal
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
            || decimal.starts_with('.')
            || decimal.ends_with('.')
        {
            return Err(http::decoding(NAME));
        }
        decimal.parse().ok()
    } else {
        value.as_f64()
    };
    number
        .filter(|number: &f64| number.is_finite() && *number >= 0.0)
        .map(Some)
        .ok_or_else(|| http::decoding(NAME))
}

/// An RFC 3339 time; `None` for a null.
fn date(value: &Value) -> Result<Option<DateTime<Utc>>, SimpleProviderError> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
        .map(|time| Some(time.with_timezone(&Utc)))
        .ok_or_else(|| http::decoding(NAME))
}

/// One budget of the key, with the dollars it has used when they are known.
struct Budget {
    id: String,
    title: String,
    hard: bool,
    used: Option<f64>,
    limit: f64,
    reset: Option<DateTime<Utc>>,
    period: Option<i64>,
}

impl Budget {
    /// The budget's meter under `label`, or an Unavailable badge when its spend is unknown.
    fn line(&self, label: &str) -> MetricLine {
        match self.used {
            Some(used) => lines::dollars(label, used, self.limit, self.reset, self.period),
            None => lines::badge(label, "Unavailable"),
        }
    }
}

/// The card's rows from a `key.usage` answer; an answer that fails any of its checks is refused.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    if body["object"] != "key.usage" || body["currency"] != "USD" || date(&body["as_of"])?.is_none()
    {
        return Err(http::decoding(NAME));
    }
    let key = &body["key"];
    let id = text(key, "id")?;
    text(key, "project_id")?;
    let entries = body["budgets"]
        .as_array()
        .filter(|entries| entries.len() <= 64)
        .ok_or_else(|| http::decoding(NAME))?;
    let mut ids = HashSet::new();
    let mut budgets = vec![];
    for entry in entries {
        let budget_id = text(entry, "id")?;
        if !ids.insert(budget_id) {
            return Err(http::decoding(NAME));
        }
        let scope = text(entry, "scope")?;
        let interval = text(entry, "interval")?;
        if !["organization", "project", "team", "user", "api_key"].contains(&scope)
            || !["daily", "weekly", "monthly", "lifetime"].contains(&interval)
        {
            return Err(http::decoding(NAME));
        }
        let hard = match text(entry, "enforcement")? {
            "hard" => true,
            "monitor" => false,
            _ => return Err(http::decoding(NAME)),
        };
        let shared = entry["shared"]
            .as_bool()
            .ok_or_else(|| http::decoding(NAME))?;
        let limit = amount(&entry["limit_usd"])?
            .filter(|limit| *limit > 0.0)
            .ok_or_else(|| http::decoding(NAME))?;
        let applies_to = entry["applies_to"]
            .as_array()
            .filter(|targets| !targets.is_empty())
            .ok_or_else(|| http::decoding(NAME))?;
        if applies_to
            .iter()
            .any(|target| target["api_key_id"] != id || target["project_id"] != key["project_id"])
        {
            return Err(http::decoding(NAME));
        }
        let availability = &entry["availability"];
        let status = text(availability, "status")?;
        if !["available", "unavailable", "not_enforced"].contains(&status)
            || !["available", "unavailable"].contains(&text(entry, "spend_status")?)
        {
            return Err(http::decoding(NAME));
        }
        let known = if hard {
            status == "available"
        } else {
            text(entry, "spend_status")? == "available"
        };
        let spent = amount(if hard {
            &availability["spent_usd"]
        } else {
            &entry["spend_usd"]
        })?;
        let reserved = if hard {
            amount(&availability["reserved_usd"])?
        } else {
            Some(0.0)
        };
        let remaining = amount(if hard {
            &availability["remaining_usd"]
        } else {
            &entry["remaining_usd"]
        })?;
        let used = if known {
            let used = spent
                .zip(reserved)
                .filter(|_| remaining.is_some())
                .map(|(spent, reserved)| spent + reserved)
                .filter(|used| used.is_finite())
                .ok_or_else(|| http::decoding(NAME))?;
            Some(used)
        } else {
            None
        };
        let start = date(&entry["starts_at"])?;
        let reset = date(&entry["resets_at"])?;
        let period = match (start, reset) {
            (Some(start), Some(reset)) if reset > start => Some((reset - start).num_milliseconds()),
            (Some(_), Some(_)) => return Err(http::decoding(NAME)),
            _ => None,
        };
        let scope = if scope == "api_key" {
            "Key".to_string()
        } else {
            lines::plan_name(scope).unwrap_or_default()
        };
        let title = format!(
            "{} · {} · {} · {}",
            scope,
            lines::plan_name(interval).unwrap_or_default(),
            if shared { "Shared" } else { "Personal" },
            if hard { "Hard" } else { "Monitor" }
        );
        budgets.push(Budget {
            id: budget_id.into(),
            title,
            hard,
            used,
            limit,
            reset,
            period,
        });
    }
    budgets.sort_by(|left, right| {
        right
            .hard
            .cmp(&left.hard)
            .then(right.used.is_some().cmp(&left.used.is_some()))
            .then(
                (right.used.unwrap_or(0.0) / right.limit)
                    .total_cmp(&(left.used.unwrap_or(0.0) / left.limit)),
            )
            .then(left.id.cmp(&right.id))
    });
    let mut rows = vec![];
    for (budget, title) in budgets
        .iter()
        .filter(|budget| budget.used.is_some())
        .take(2)
        .zip(["Budget", "Secondary Budget"])
    {
        rows.push(budget.line(title));
    }
    let usage = &body["usage"];
    if !usage.is_null() {
        if usage["window"] != "7d" {
            return Err(http::decoding(NAME));
        }
        let count = |field: &str| {
            amount(&usage[field])?
                .filter(|number| number.fract() == 0.0 && *number <= 9007199254740991.0)
                .ok_or_else(|| http::decoding(NAME))
        };
        let requests = count("requests")?;
        let tokens = count("total_tokens")?;
        let attributed = count("attributed_requests")?;
        let partial = count("partial_requests")?;
        let spend = amount(&usage["spend_usd"])?;
        if attributed > requests
            || partial > attributed
            || (spend.is_some_and(|spend| spend > 0.0) && attributed == 0.0)
        {
            return Err(http::decoding(NAME));
        }
        if let Some(spend) = spend {
            rows.push(lines::dollar_value("Weekly Spend", spend));
        }
        rows.push(lines::count_value("Requests", requests, "requests"));
        rows.push(lines::count_value("Tokens", tokens, "tokens"));
    }
    let mut titles = HashSet::new();
    for budget in budgets {
        let mut title = budget.title.clone();
        let mut number = 2;
        while !titles.insert(title.clone()) {
            title = format!("{} ({i})", budget.title, i = number);
            number += 1;
        }
        rows.push(budget.line(&title));
    }
    Ok(Reading::new(None, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    fn fixture() -> Value {
        json!({"object":"key.usage","currency":"USD","as_of":"2026-09-24T12:00:00Z","key":{"id":"key-1","project_id":"project-1"},"budgets":[{"id":"b1","scope":"project","interval":"monthly","enforcement":"hard","shared":true,"limit_usd":"100","applies_to":[{"api_key_id":"key-1","project_id":"project-1"}],"spend_status":"available","availability":{"status":"available","spent_usd":"20","reserved_usd":"10","remaining_usd":"70"},"starts_at":"2026-09-01T00:00:00Z","resets_at":"2026-10-01T00:00:00Z"}],"usage":{"window":"7d","requests":12,"total_tokens":1200,"attributed_requests":10,"partial_requests":2,"spend_usd":1.25}})
    }

    #[tokio::test]
    async fn a_hard_budget_counts_its_reserved_spend_and_the_key_goes_as_a_bearer_token() {
        let http = Scripted::new().on(
            "GET",
            "https://api.aixy-gateway.com/v1/usage",
            200,
            &fixture().to_string(),
        );
        let scope = context_at(
            &http,
            json!({"apiKey":"fixture"}),
            "2026-09-27T00:00:00Z".parse().unwrap(),
        );
        let reading = Aixy.fetch(&scope.context()).await.unwrap();
        let reset = Some("2026-10-01T00:00:00Z".parse().unwrap());
        assert_eq!(reading.plan, None);
        assert_eq!(
            reading.lines,
            vec![
                lines::dollars("Budget", 30.0, 100.0, reset, Some(30 * lines::DAY_MS)),
                lines::dollar_value("Weekly Spend", 1.25),
                lines::count_value("Requests", 12.0, "requests"),
                lines::count_value("Tokens", 1200.0, "tokens"),
                lines::dollars(
                    "Project · Monthly · Shared · Hard",
                    30.0,
                    100.0,
                    reset,
                    Some(30 * lines::DAY_MS)
                )
            ]
        );
        assert_eq!(http.requests().len(), 1);
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer fixture")
        );
    }

    #[test]
    fn a_wrong_currency_key_date_count_or_duplicate_budget_is_refused() {
        for field in ["currency", "key", "date", "coverage", "duplicate"] {
            let mut body = fixture();
            match field {
                "currency" => body["currency"] = json!("EUR"),
                "key" => body["key"]["id"] = json!("other"),
                "date" => body["budgets"][0]["resets_at"] = json!("bad"),
                "coverage" => body["usage"]["attributed_requests"] = json!(13),
                _ => {
                    let budget = body["budgets"][0].clone();
                    body["budgets"].as_array_mut().unwrap().push(budget);
                }
            }
            assert!(parse(&body).is_err(), "{field}");
        }
    }

    #[test]
    fn unknown_is_not_zero_and_monitor_excludes_reservations() {
        let mut body = fixture();
        body["budgets"][0]["availability"]["status"] = json!("unavailable");
        let reading = parse(&body).unwrap();
        assert_eq!(
            reading.lines.last(),
            Some(&lines::badge(
                "Project · Monthly · Shared · Hard",
                "Unavailable"
            ))
        );
        body["budgets"][0]["enforcement"] = json!("monitor");
        body["budgets"][0]["spend_usd"] = json!(40);
        body["budgets"][0]["remaining_usd"] = json!(60);
        let reading = parse(&body).unwrap();
        assert_eq!(
            reading.lines[0],
            lines::dollars(
                "Budget",
                40.0,
                100.0,
                Some("2026-10-01T00:00:00Z".parse().unwrap()),
                Some(30 * lines::DAY_MS)
            )
        );
    }

    #[tokio::test]
    async fn refusals_rate_limits_and_gateways_without_usage_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (404, ErrorCategory::NotAvailable),
        ] {
            let http = Scripted::new().on(
                "GET",
                "https://api.aixy-gateway.com/v1/usage",
                status,
                "secret",
            );
            let scope = context_at(
                &http,
                json!({"apiKey":"fixture"}),
                "2026-09-27T00:00:00Z".parse().unwrap(),
            );
            let error = Aixy.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }
}
