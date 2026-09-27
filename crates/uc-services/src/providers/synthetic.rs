//! Synthetic: the session, weekly and web search quotas of a Synthetic API key, with its credits
//! when the answer carries them.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `SYNTHETIC_API_KEY` environment variable or from a key saved in Quota Control. A refresh sends
//! one read-only request, `GET https://api.synthetic.new/v2/quotas` with the key as a bearer token.
//! The `rollingFiveHourLimit`, `weeklyTokenLimit` and `search.hourly` quotas, at the top of the
//! answer or under its `data`, become the Session, Weekly and Web Searches meters. An answer
//! without them is searched for objects that read as quotas: the first three fill those meters and
//! the others become extra rows, and a quota with a name of its own also gets a row under that
//! name. A quota's share comes from a percent field or from its used, remaining and limit counts,
//! and its reset time and window from the fields that name them. The first quota with
//! `maxCredits` also gives the Credits meter and, with `nextRegenCredits`, the next credit refill;
//! a quota with a tick percentage gets a refill row, and the plan comes from a field such as
//! `plan`, `tier` or `packageName`.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, MetricLine, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Synthetic;

const NAME: &str = "Synthetic";
const URL: &str = "https://api.synthetic.new/v2/quotas";

#[async_trait]
impl Service for Synthetic {
    fn id(&self) -> &'static str {
        "synthetic"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["SYNTHETIC_API_KEY"],
            url: "https://dev.synthetic.new",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut descriptors: Vec<_> = [
            ("session", "Session"),
            ("weekly", "Weekly"),
            ("search", "Web Searches"),
        ]
        .into_iter()
        .map(|(id, title)| {
            WidgetDescriptor::percent(format!("{}.{id}", provider.id), provider, title, None, None)
                .exporting_progress(id, "percent")
        })
        .collect();
        descriptors.push(WidgetDescriptor::bounded_dollars(
            format!("{}.credits", provider.id),
            provider,
            "Credits",
            None,
            0.0,
            None,
            None,
        ));
        descriptors
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Synthetic API key is missing."))?;
        let body = http::json(context.http, HttpRequest::get(URL).bearer(key), NAME).await?;
        parse(&body)
    }
}

/// The meters, extra rows and plan of a quotas answer.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    let data = body.get("data").unwrap_or(body);
    let mut rows = Vec::new();
    let mut cost = None;
    let mut details = Vec::new();
    for (pointer, title, period) in [
        ("/rollingFiveHourLimit", "Session", Some(5 * lines::HOUR_MS)),
        ("/weeklyTokenLimit", "Weekly", Some(lines::WEEK_MS)),
        ("/search/hourly", "Web Searches", Some(lines::HOUR_MS)),
    ] {
        if let Some(entry) = body.pointer(pointer).or_else(|| data.pointer(pointer)) {
            let line = quota(entry, title, period)?;
            enrich(entry, &line, &mut cost, &mut details);
            rows.push(line);
        }
    }
    if rows.is_empty() {
        let mut entries = Vec::new();
        collect(body, &mut entries, 0);
        for (index, entry) in entries.iter().enumerate() {
            let resource = first_text(
                entry,
                &["name", "label", "type", "period", "scope", "title", "id"],
            );
            let label = ["Session", "Weekly", "Web Searches"]
                .get(index)
                .copied()
                .unwrap_or(resource.unwrap_or("Quota"));
            let line = quota(entry, label, None)?;
            enrich(entry, &line, &mut cost, &mut details);
            if index < 3 {
                rows.push(line);
            } else {
                details.push(line);
            }
            if let Some(resource) = resource.filter(|name| *name != label) {
                details.push(quota(entry, resource, None)?);
            }
        }
    }
    if rows.is_empty() {
        return Err(http::decoding(NAME));
    }
    rows.extend(cost);
    rows.extend(details);
    Ok(Reading::new(
        first_text(
            body,
            &[
                "plan",
                "planName",
                "plan_name",
                "tier",
                "subscription",
                "subscriptionPlan",
                "package",
                "packageName",
            ],
        )
        .or_else(|| {
            first_text(
                data,
                &[
                    "plan",
                    "planName",
                    "plan_name",
                    "tier",
                    "subscription",
                    "subscriptionPlan",
                    "package",
                    "packageName",
                ],
            )
        })
        .and_then(lines::plan_name),
        rows,
    ))
}

/// The first of `keys` that holds an amount, as a number or as text that may carry `$` signs and
/// thousands commas.
fn currency(object: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|field| {
            if let Some(text) = field.as_str() {
                text.trim()
                    .replace(['$', ','], "")
                    .parse::<f64>()
                    .ok()
                    .filter(|amount| amount.is_finite())
            } else {
                value::as_number(field)
            }
        })
    })
}

/// Adds what a quota says beyond its share: the Credits meter and the next credit refill from the
/// first quota with `maxCredits`, and a refill row for a quota with a tick percentage.
fn enrich(
    entry: &Value,
    line: &MetricLine,
    cost: &mut Option<MetricLine>,
    details: &mut Vec<MetricLine>,
) {
    let MetricLine::Progress(progress) = line else {
        return;
    };
    if cost.is_none()
        && let Some(limit) =
            currency(entry, &["maxCredits", "max_credits"]).filter(|amount| *amount >= 0.0)
    {
        let used = currency(entry, &["usedCredits", "used_credits"])
            .or_else(|| {
                currency(entry, &["remainingCredits", "remaining_credits"])
                    .map(|remaining| (limit - remaining).max(0.0))
            })
            .unwrap_or(progress.used * limit / 100.0);
        *cost = Some(lines::dollars(
            "Credits",
            used,
            limit,
            progress.resets_at,
            Some(lines::WEEK_MS),
        ));
        if let Some(amount) = currency(entry, &["nextRegenCredits", "next_regen_credits"]) {
            details.push(lines::dollar_value("Next Credit Refill", amount));
        }
    }
    if let Some(percent) = first_number(
        entry,
        &[
            "tickPercent",
            "tick_percent",
            "nextTickPercent",
            "next_tick_percent",
        ],
    ) {
        details.push(lines::count_value(
            &format!("{} Refill", progress.label),
            if percent <= 1.0 {
                percent * 100.0
            } else {
                percent
            },
            "%",
        ));
    }
}

fn first_number(object: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(value::as_number))
}

fn first_text<'a>(object: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    })
}

/// One quota as a percent meter: its share from a percent field or from its counts, its reset
/// time, and its window from `period` or from the fields that name one.
fn quota(
    entry: &Value,
    label: &str,
    period: Option<i64>,
) -> Result<MetricLine, SimpleProviderError> {
    let normalized = |share: f64| if share <= 1.0 { share * 100.0 } else { share };
    let mut percent = first_number(
        entry,
        &[
            "percentUsed",
            "usedPercent",
            "usagePercent",
            "usage_percent",
            "used_percent",
            "percent_used",
            "percent",
        ],
    )
    .map(normalized)
    .or_else(|| {
        first_number(
            entry,
            &[
                "percentRemaining",
                "remainingPercent",
                "remaining_percent",
                "percent_remaining",
            ],
        )
        .map(|remaining| 100.0 - normalized(remaining))
    });
    if percent.is_none() {
        let mut limit = first_number(
            entry,
            &[
                "limit",
                "messageLimit",
                "message_limit",
                "messages",
                "maxRequests",
                "max_requests",
                "requestLimit",
                "request_limit",
                "quota",
                "max",
                "total",
                "capacity",
                "allowance",
            ],
        );
        let mut used = first_number(
            entry,
            &[
                "used",
                "usage",
                "usedMessages",
                "used_messages",
                "messagesUsed",
                "messages_used",
                "requests",
                "requestCount",
                "request_count",
                "consumed",
                "spent",
            ],
        );
        let left = first_number(entry, &["remaining", "left", "available", "balance"]);
        if let (Some(used), Some(left)) = (used, left) {
            limit = limit.or(Some(used + left));
        }
        if let (Some(total), Some(left)) = (limit, left) {
            used = used.or(Some(total - left));
        }
        if let (Some(used), Some(total)) = (used, limit.filter(|total| *total > 0.0)) {
            percent = Some(used / total * 100.0);
        }
    }
    let percent = percent.ok_or_else(|| http::decoding(NAME))?;
    let reset = [
        "resetAt",
        "reset_at",
        "resetsAt",
        "resets_at",
        "renewAt",
        "renew_at",
        "renewsAt",
        "renews_at",
        "nextTickAt",
        "next_tick_at",
        "nextRegenAt",
        "next_regen_at",
        "periodEnd",
        "period_end",
        "expiresAt",
        "expires_at",
        "endAt",
        "end_at",
    ]
    .iter()
    .find_map(|key| entry.get(*key).and_then(value::as_time));
    let period = period
        .or_else(|| {
            first_number(
                entry,
                &[
                    "windowMinutes",
                    "window_minutes",
                    "periodMinutes",
                    "period_minutes",
                ],
            )
            .map(|minutes| (minutes * 60000.0) as i64)
        })
        .or_else(|| {
            first_number(
                entry,
                &["windowHours", "window_hours", "periodHours", "period_hours"],
            )
            .map(|hours| (hours * 3600000.0) as i64)
        })
        .or_else(|| {
            first_number(
                entry,
                &["windowDays", "window_days", "periodDays", "period_days"],
            )
            .map(|days| (days * lines::DAY_MS as f64) as i64)
        })
        .or_else(|| {
            first_number(
                entry,
                &[
                    "windowSeconds",
                    "window_seconds",
                    "periodSeconds",
                    "period_seconds",
                ],
            )
            .map(|seconds| ((seconds / 60.0).round() * 60_000.0) as i64)
        })
        .or_else(|| {
            let text = first_text(
                entry,
                &[
                    "window",
                    "windowLabel",
                    "window_label",
                    "period",
                    "periodLabel",
                    "period_label",
                ],
            )?;
            let compact = text.to_lowercase().replace(char::is_whitespace, "");
            let split =
                compact.find(|character: char| !character.is_ascii_digit() && character != '.')?;
            let count = compact[..split].parse::<f64>().ok()?;
            let multiplier = match &compact[split..] {
                "m" | "min" | "mins" | "minute" | "minutes" => 60_000.0,
                "h" | "hr" | "hrs" | "hour" | "hours" => lines::HOUR_MS as f64,
                "d" | "day" | "days" => lines::DAY_MS as f64,
                _ => return None,
            };
            (count.is_finite() && count > 0.0).then_some((count * multiplier) as i64)
        })
        .filter(|milliseconds| *milliseconds > 0);
    Ok(lines::percent(label, percent, reset, period))
}

/// Every object under `node` that reads as a quota, searching down to 24 levels deep and keeping
/// at most 128 of them.
fn collect<'a>(node: &'a Value, found: &mut Vec<&'a Value>, depth: u8) {
    if depth > 24 || found.len() >= 128 {
        return;
    }
    if node.is_object() && quota(node, "", None).is_ok() {
        found.push(node);
        return;
    }
    match node {
        Value::Array(items) => {
            for item in items {
                collect(item, found, depth + 1)
            }
        }
        Value::Object(items) => {
            for item in items.values() {
                collect(item, found, depth + 1)
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const PRO_QUOTAS: &str = r###"{"plan":"pro","rollingFiveHourLimit":{"used":25,"limit":100,"resetAt":"2026-09-27T15:00:00Z"},"weeklyTokenLimit":{"used":200,"limit":1000},"search":{"hourly":{"percentUsed":0.1}}}"###;

    #[tokio::test]
    async fn the_session_weekly_and_search_quotas_show_as_meters_from_one_bearer_request() {
        let http = Scripted::new().on("GET", URL, 200, PRO_QUOTAS);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = Synthetic.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::percent(
                        "Session",
                        25.0,
                        Some(Utc.with_ymd_and_hms(2026, 9, 27, 15, 0, 0).unwrap()),
                        Some(5 * lines::HOUR_MS)
                    ),
                    lines::percent("Weekly", 20.0, None, Some(lines::WEEK_MS)),
                    lines::percent("Web Searches", 10.0, None, Some(lines::HOUR_MS))
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer test"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn refusals_rate_limits_and_bad_answers_keep_their_category_without_echoing_the_body() {
        for (status, body, category) in [
            (401, "secret", ErrorCategory::AuthExpired),
            (403, "secret", ErrorCategory::AuthExpired),
            (429, "", ErrorCategory::RateLimited),
            (500, "", ErrorCategory::Http5xx),
            (200, "{}", ErrorCategory::Decoding),
            (200, "bad json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let error = Synthetic.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn an_unknown_quota_list_falls_back_to_meters_with_credits_refills_and_named_rows() {
        let body = json!({
            "packageName": "pro",
            "quotas": [{
                "name": "Model A",
                "used": 20,
                "limit": 100,
                "window": "2 days",
                "maxCredits": "$100.00",
                "remainingCredits": "$80",
                "nextRegenCredits": "$5",
                "tickPercent": 0.1
            }]
        });
        assert_eq!(
            parse(&body).unwrap(),
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::percent("Session", 20.0, None, Some(2 * lines::DAY_MS)),
                    lines::dollars("Credits", 20.0, 100.0, None, Some(lines::WEEK_MS)),
                    lines::dollar_value("Next Credit Refill", 5.0),
                    lines::count_value("Session Refill", 10.0, "%"),
                    lines::percent("Model A", 20.0, None, Some(2 * lines::DAY_MS))
                ]
            )
        );
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Synthetic
                .fetch(&scope.context())
                .await
                .unwrap_err()
                .category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
