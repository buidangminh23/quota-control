//! Bifrost: the budgets and rate limits of a Bifrost virtual key, and what each model spent.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the card takes the virtual key and the
//! gateway's server address (HTTPS, or plain HTTP on a private network), both saved in Quota
//! Control. A refresh sends one request, `GET <server>/api/governance/virtual-keys/quota` with the
//! key in the `x-bf-vk` header. No token is renewed and no model is run.
//!
//! The answer carries `budgets` and `rate_limits` (or a single `rate_limit`) for the key and for
//! each entry of its `provider_configs` and `model_configs`. Ordered by period, the key's own first
//! two budgets with a limit fill the Budget and Secondary Budget meters, an active
//! `override_amount` raising a limit, and its first budget gives the Total Usage row and a row per
//! model from `per_model_usage`. The key's own first token and request limits fill the Tokens and
//! Requests meters, and every budget and rate limit then follows as its own row. An inactive key
//! adds a Key Status row, or reports that it has nothing to show when it has no quota. A reset is
//! shown only for a fixed `reset_duration` such as `1h30m`, counted from `last_reset`; calendar
//! periods (days, weeks, months, quarters and years) are not guessed.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use uc_core::{
    HttpRequest, MetricKind, MetricLine, MetricValue, Provider, SimpleProviderError,
    WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{endpoint, http, lines, value};

pub(crate) struct Bifrost;

const NAME: &str = "Bifrost";

#[async_trait]
impl Service for Bifrost {
    fn id(&self) -> &'static str {
        "bifrost"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Virtual key"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "",
            fields: &[("baseUrl", "Server address")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut widgets = vec![];
        for (suffix, title) in [
            ("budget", "Budget"),
            ("secondaryBudget", "Secondary Budget"),
        ] {
            widgets.push(
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
        for (suffix, title, unit) in [
            ("tokens", "Tokens", "tokens"),
            ("requests", "Requests", "requests"),
        ] {
            widgets.push(
                WidgetDescriptor::bounded_count(
                    format!("{}.{suffix}", provider.id),
                    provider,
                    title,
                    None,
                    1.0,
                    unit,
                    None,
                )
                .exporting_progress(suffix, unit),
            );
        }
        widgets.push(WidgetDescriptor::values(
            format!("{}.spend", provider.id),
            provider,
            "Total Usage",
            None,
            Some(MetricKind::Dollars),
            None,
            true,
            None,
            false,
        ));
        widgets
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("Enter the Bifrost virtual key."))?;
        let base = endpoint::base_url(
            context.secret.str("/baseUrl"),
            None,
            endpoint::Policy::HttpsOrPrivateNetworkHttp,
            NAME,
        )?;
        let body = http::json(
            context.http,
            HttpRequest::get(format!("{base}/api/governance/virtual-keys/quota"))
                .header("x-bf-vk", key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        parse(&body, context.now)
    }
}

/// The finite number in `field` of `object`, `None` when the field is absent or null; any other
/// value cannot be read.
fn number(object: &Value, field: &str) -> Result<Option<f64>, SimpleProviderError> {
    if object[field].is_null() {
        return Ok(None);
    }
    object[field]
        .as_f64()
        .filter(|amount| amount.is_finite())
        .map(Some)
        .ok_or_else(|| http::decoding(NAME))
}

/// The objects of a JSON array, none for null; anything else cannot be read.
fn array(list: &Value) -> Result<Vec<&Value>, SimpleProviderError> {
    if list.is_null() {
        return Ok(vec![]);
    }
    list.as_array()
        .filter(|items| items.iter().all(Value::is_object))
        .map(|items| items.iter().collect())
        .ok_or_else(|| http::decoding(NAME))
}

/// The seconds in a `reset_duration`, and whether they are a fixed length: `Y`, `Q`, `M`, `w` and
/// `d` count calendar periods, while a duration made of `h`, `m`, `s`, `ms`, `us` and `ns` parts
/// (such as `1h30m`) is fixed.
fn duration(text: &str) -> Option<(f64, bool)> {
    for (unit, scale) in [
        ("Y", 31536000.0),
        ("Q", 7776000.0),
        ("M", 2592000.0),
        ("w", 604800.0),
        ("d", 86400.0),
    ] {
        if let Some(count) = text.strip_suffix(unit) {
            if !count.is_empty()
                && count
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
            {
                let seconds = count.parse::<f64>().ok()? * scale;
                return (seconds.is_finite() && seconds > 0.0).then_some((seconds, false));
            }
            return None;
        }
    }
    let mut rest = text;
    let mut total = 0.0;
    while !rest.is_empty() {
        let end = rest.find(|character: char| !character.is_ascii_digit() && character != '.')?;
        let count = rest[..end].parse::<f64>().ok()?;
        rest = &rest[end..];
        let (unit, scale) = [
            ("ns", 1e-9),
            ("us", 1e-6),
            ("µs", 1e-6),
            ("μs", 1e-6),
            ("ms", 1e-3),
            ("s", 1.0),
            ("m", 60.0),
            ("h", 3600.0),
        ]
        .into_iter()
        .find(|(unit, _)| rest.starts_with(unit))?;
        total += count * scale;
        rest = &rest[unit.len()..];
    }
    (total.is_finite() && total > 0.0).then_some((total, true))
}

/// The period of the `reset_duration` under `prefix` in seconds and, when the period is fixed, the
/// first reset after `now` counted from `last_reset` and the period in milliseconds.
fn timing(
    object: &Value,
    prefix: &str,
    now: DateTime<Utc>,
) -> (Option<f64>, Option<DateTime<Utc>>, Option<i64>) {
    let field = format!("{prefix}reset_duration");
    let Some(raw) = object[&field].as_str() else {
        return (None, None, None);
    };
    let Some((seconds, fixed)) = duration(raw) else {
        return (None, None, None);
    };
    if !fixed {
        return (Some(seconds), None, None);
    }
    let millis = seconds * 1000.0;
    if millis < 1.0 || millis > i64::MAX as f64 {
        return (Some(seconds), None, None);
    }
    let period = millis as i64;
    let reset = value::time(object, &format!("/{prefix}last_reset")).and_then(|start| {
        let elapsed = (now - start).num_milliseconds().max(0);
        let periods = elapsed.checked_div(period)?.checked_add(1)?;
        let delta = periods.checked_mul(period)?;
        start.checked_add_signed(Duration::milliseconds(delta))
    });
    (Some(seconds), reset, Some(period))
}

/// A budget of the key or of one of its configs.
struct Budget<'a> {
    id: &'a str,
    title: String,
    /// Whether the budget belongs to the key itself rather than to one of its configs.
    root: bool,
    used: f64,
    limit: f64,
    /// The period in seconds, the next reset and the period in milliseconds, from [`timing`].
    time: (Option<f64>, Option<DateTime<Utc>>, Option<i64>),
    models: Vec<&'a Value>,
}

impl Budget<'_> {
    fn line(&self, title: &str) -> MetricLine {
        if self.limit > 0.0 {
            lines::dollars(title, self.used, self.limit, self.time.1, self.time.2)
        } else {
            lines::dollar_value(title, self.used)
        }
    }
}

fn parse(body: &Value, now: DateTime<Utc>) -> Result<Reading, SimpleProviderError> {
    if !body.is_object() || (!body["is_active"].is_null() && !body["is_active"].is_boolean()) {
        return Err(http::decoding(NAME));
    }
    let mut scopes = vec![(String::new(), body)];
    for (field, kind) in [("provider_configs", "Provider"), ("model_configs", "Model")] {
        for (index, config) in array(&body[field])?.into_iter().enumerate() {
            let provider = value::text(config, "/provider").unwrap_or("");
            let name = if kind == "Model" {
                value::text(config, "/model_name")
                    .map(str::to_string)
                    .unwrap_or_else(|| (index + 1).to_string())
            } else {
                String::new()
            };
            scopes.push((
                format!("{kind} {provider} {name}").trim().to_string(),
                config,
            ));
        }
    }
    let mut budgets = vec![];
    let mut limits = vec![];
    let mut primary_limits = std::collections::BTreeMap::new();
    for (scope, config) in &scopes {
        for raw in array(&config["budgets"])? {
            let Some(id) = value::text(raw, "/id") else {
                continue;
            };
            let amount = number(raw, "override_amount")?.unwrap_or(0.0);
            let cycles = number(raw, "override_cycles_remaining")?.unwrap_or(0.0);
            if cycles.fract() != 0.0 {
                return Err(http::decoding(NAME));
            }
            let active = amount > 0.0
                && (raw["override_mode"] == "forever"
                    || (raw["override_mode"] == "cycles" && cycles > 0.0));
            let limit =
                number(raw, "max_limit")?.unwrap_or(0.0) + if active { amount } else { 0.0 };
            if !limit.is_finite() {
                return Err(http::decoding(NAME));
            }
            let title = format!(
                "{} · {}",
                if scope.is_empty() {
                    "Budget Pool"
                } else {
                    scope
                },
                value::text(raw, "/source_name").unwrap_or(id)
            );
            budgets.push(Budget {
                id,
                title,
                root: scope.is_empty(),
                used: number(raw, "current_usage")?.unwrap_or(0.0),
                limit,
                time: timing(raw, "", now),
                models: array(&raw["per_model_usage"])?,
            });
        }
        let mut components = array(&config["rate_limits"])?;
        if components.is_empty() && !config["rate_limit"].is_null() {
            if !config["rate_limit"].is_object() {
                return Err(http::decoding(NAME));
            }
            components.push(&config["rate_limit"]);
        }
        for (index, limit) in components.into_iter().enumerate() {
            for (key, title, unit) in [
                ("token", "Tokens", "tokens"),
                ("request", "Requests", "requests"),
            ] {
                let max = number(limit, &format!("{key}_max_limit"))?;
                let used = number(limit, &format!("{key}_current_usage"))?.unwrap_or(0.0);
                if !max.is_some_and(|cap| cap > 0.0)
                    && value::text(limit, &format!("/{key}_reset_duration")).is_none()
                {
                    continue;
                }
                let label = format!(
                    "{}{}{} {title}",
                    if scope.is_empty() { "" } else { scope },
                    if scope.is_empty() { "" } else { " · " },
                    value::text(limit, "/source_name")
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("Pool {}", index + 1))
                );
                let time = timing(limit, &format!("{key}_"), now);
                if scope.is_empty()
                    && let Some(max) = max.filter(|cap| *cap > 0.0)
                {
                    primary_limits
                        .entry(title)
                        .or_insert_with(|| lines::count(title, used, max, unit, time.1, time.2));
                }
                limits.push(match max.filter(|cap| *cap > 0.0) {
                    Some(max) => lines::count(&label, used, max, unit, time.1, time.2),
                    None => lines::badge(&label, "Unavailable"),
                });
            }
        }
    }
    if body["is_active"] == false && budgets.is_empty() && limits.is_empty() {
        return Err(http::not_available(
            "Bifrost virtual key is inactive and has no quotas to display.",
        ));
    }
    budgets.sort_by(|left, right| {
        left.time
            .0
            .unwrap_or(f64::INFINITY)
            .total_cmp(&right.time.0.unwrap_or(f64::INFINITY))
            .then(left.id.cmp(right.id))
    });
    let mut rows = vec![];
    for (budget, title) in budgets
        .iter()
        .filter(|budget| budget.root && budget.limit > 0.0)
        .take(2)
        .zip(["Budget", "Secondary Budget"])
    {
        rows.push(budget.line(title));
    }
    let first = budgets.iter().find(|budget| budget.root);
    if let Some(first) = first {
        rows.push(lines::dollar_value("Total Usage", first.used));
    }
    rows.extend(primary_limits.into_values());
    if body["is_active"] == false {
        rows.push(lines::badge("Key Status", "Inactive"));
    }
    let mut titles = std::collections::HashSet::new();
    for budget in &budgets {
        let mut title = budget.title.clone();
        let mut copy = 2;
        while !titles.insert(title.clone()) {
            title = format!("{} ({i})", budget.title, i = copy);
            copy += 1;
        }
        rows.push(budget.line(&title));
    }
    rows.extend(limits);
    if let Some(first) = first {
        for model in &first.models {
            let cost = number(model, "total_cost")?;
            let tokens = number(model, "total_tokens")?;
            let requests = number(model, "total_requests")?;
            if cost.unwrap_or(0.0) == 0.0 && tokens.unwrap_or(0.0) == 0.0 {
                continue;
            }
            let name = value::text(model, "/model").unwrap_or("Model");
            let provider = value::text(model, "/provider").unwrap_or("");
            let title = format!("Model: {provider} {name}").trim().to_string();
            let mut values = vec![];
            if let Some(cost) = cost {
                values.push(MetricValue::dollars(cost));
            }
            if let Some(tokens) = tokens {
                values.push(MetricValue::count(tokens, "tokens"));
            }
            if let Some(requests) = requests {
                values.push(MetricValue::count(requests, "requests"));
            }
            rows.push(lines::values(&title, values));
        }
    }
    Ok(Reading::new(None, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    fn now() -> DateTime<Utc> {
        "2026-09-27T12:00:00Z".parse().unwrap()
    }

    #[tokio::test]
    async fn a_key_shows_its_budget_rate_limits_and_the_spend_of_each_model() {
        let body = json!({"budgets":[{"id":"month","max_limit":125,"current_usage":42.17,"reset_duration":"1M","last_reset":"2026-09-01T00:00:00Z","source_name":"Engineering","per_model_usage":[{"model":"gpt-4o","provider":"openai","total_cost":5,"total_tokens":1200000}]}],"rate_limit":{"token_max_limit":1},"rate_limits":[{"token_max_limit":1000000,"token_current_usage":345678,"token_reset_duration":"1d","request_max_limit":5000,"request_current_usage":120}]});
        let http = Scripted::new().on(
            "GET",
            "http://gateway.lan/api/governance/virtual-keys/quota",
            200,
            &body.to_string(),
        );
        let scope = context_at(
            &http,
            json!({"apiKey":"fixture","baseUrl":"http://gateway.lan"}),
            now(),
        );
        let reading = Bifrost.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(
            reading.lines,
            vec![
                lines::dollars("Budget", 42.17, 125.0, None, None),
                lines::dollar_value("Total Usage", 42.17),
                lines::count("Requests", 120.0, 5000.0, "requests", None, None),
                lines::count("Tokens", 345678.0, 1000000.0, "tokens", None, None),
                lines::dollars("Budget Pool · Engineering", 42.17, 125.0, None, None),
                lines::count("Pool 1 Tokens", 345678.0, 1000000.0, "tokens", None, None),
                lines::count("Pool 1 Requests", 120.0, 5000.0, "requests", None, None),
                lines::values(
                    "Model: openai gpt-4o",
                    vec![
                        MetricValue::dollars(5.0),
                        MetricValue::count(1200000.0, "tokens")
                    ]
                )
            ]
        );
        assert_eq!(header(&http.requests()[0], "x-bf-vk"), Some("fixture"));
        assert_eq!(http.requests().len(), 1);
    }

    #[test]
    fn an_active_override_raises_the_limit_and_a_fixed_duration_sets_the_reset() {
        for (mode, cycles, limit) in [
            ("forever", 0, 150.0),
            ("cycles", 2, 150.0),
            ("cycles", 0, 100.0),
        ] {
            let body = json!({"budgets":[{"id":"a","max_limit":100,"current_usage":200,"override_amount":50,"override_mode":mode,"override_cycles_remaining":cycles,"reset_duration":"1h30m","last_reset":"2026-09-27T09:00:00Z"}]});
            let reading = parse(&body, now()).unwrap();
            assert_eq!(
                reading.lines[0],
                lines::dollars(
                    "Budget",
                    200.0,
                    limit,
                    Some("2026-09-27T13:30:00Z".parse().unwrap()),
                    Some(90 * 60000)
                )
            );
        }
        assert_eq!(duration("1d2h"), None);
        assert_eq!(duration("0s"), None);
    }

    #[test]
    fn inactive_keys_show_their_status_and_malformed_budgets_are_rejected() {
        assert!(parse(&json!({"is_active":false}), now()).is_err());
        assert!(parse(&json!({"budgets":[{"id":"a","max_limit":"bad"}]}), now()).is_err());
        let reading = parse(
            &json!({"is_active":false,"budgets":[{"id":"a","current_usage":5}]}),
            now(),
        )
        .unwrap();
        assert_eq!(reading.lines[0], lines::dollar_value("Total Usage", 5.0));
        assert_eq!(reading.lines[1], lines::badge("Key Status", "Inactive"));
    }

    #[tokio::test]
    async fn http_errors_keep_their_categories_without_echoing_the_body() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
        ] {
            let http = Scripted::new().on(
                "GET",
                "https://gateway.example/api/governance/virtual-keys/quota",
                status,
                "secret",
            );
            let scope = context_at(
                &http,
                json!({"apiKey":"fixture","baseUrl":"https://gateway.example"}),
                now(),
            );
            let error = Bifrost.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }
}
