//! Poe: the point balance of a Poe API key and the points it spent over the last 30 days.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `POE_API_KEY`
//! environment variable or from a key saved in Quota Control. A refresh sends
//! `GET https://api.poe.com/usage/current_balance` for the balance, then up to three
//! `GET https://api.poe.com/usage/points_history?limit=100` requests for the point history, each
//! page after the first asking for the entries after the previous page's cursor
//! (`starting_after`); every request carries the key as a bearer token.
//!
//! The history entries of the 30 days before the refresh add a Monthly row with the points they
//! cost and one row per bot. When the history cannot be read the balance still shows with a
//! warning, and when three pages do not reach its end the card warns that the history is partial.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Poe;

const NAME: &str = "Poe";
const URL: &str = "https://api.poe.com/usage/current_balance";

#[async_trait]
impl Service for Poe {
    fn id(&self) -> &'static str {
        "poe"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["POE_API_KEY"],
            url: "https://poe.com/api_key",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("balance", "Balance"), ("monthly", "Monthly")]
            .into_iter()
            .map(|(id, title)| {
                WidgetDescriptor::values(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    Some(uc_core::MetricKind::Count),
                    None,
                    false,
                    None,
                    false,
                )
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Poe API key is missing."))?;
        let body = http::json(context.http, HttpRequest::get(URL).bearer(key), NAME).await?;
        let mut reading = parse(&body)?;
        let mut models = std::collections::BTreeMap::<String, f64>::new();
        let mut cursor = None;
        let mut total = 0.0;
        let mut complete = false;
        for _ in 0..3 {
            let query = cursor
                .as_ref()
                .map(|cursor: &String| {
                    format!(
                        "&starting_after={}",
                        url::form_urlencoded::byte_serialize(cursor.as_bytes()).collect::<String>()
                    )
                })
                .unwrap_or_default();
            let response = http::json(
                context.http,
                HttpRequest::get(format!(
                    "https://api.poe.com/usage/points_history?limit=100{query}"
                ))
                .bearer(key),
                NAME,
            )
            .await;
            let Ok(history) = response else {
                reading.warning =
                    Some("Poe point history is unavailable; the balance is current.".into());
                break;
            };
            let Some(rows) = history
                .get("data")
                .or_else(|| history.get("items"))
                .or_else(|| history.get("results"))
                .and_then(Value::as_array)
            else {
                reading.warning =
                    Some("Poe point history is unavailable; the balance is current.".into());
                break;
            };
            for row in rows {
                let created = ["creation_time", "timestamp", "created_at"]
                    .iter()
                    .find_map(|field| row.get(*field).and_then(value::as_time));
                if !created.is_some_and(|created| {
                    created >= context.now - chrono::Duration::days(30) && created <= context.now
                }) {
                    continue;
                }
                let Some(points) = value::number(row, "/cost_points")
                    .or_else(|| value::number(row, "/points"))
                    .or_else(|| value::number(row, "/point_cost"))
                else {
                    continue;
                };
                let points = points.max(0.0);
                total += points;
                let name = value::text(row, "/bot_name").unwrap_or("Unknown Model");
                *models.entry(name.to_string()).or_default() += points;
            }
            let next = value::text(&history, "/next_cursor")
                .map(str::to_string)
                .or_else(|| {
                    (value::flag(&history, "/has_more") == Some(true))
                        .then(|| {
                            rows.last()
                                .and_then(|last| value::text(last, "/query_id"))
                                .map(str::to_string)
                        })
                        .flatten()
                });
            if next.is_none() {
                complete = true;
                break;
            }
            if next == cursor {
                break;
            }
            cursor = next;
        }
        if !models.is_empty() {
            reading
                .lines
                .push(lines::count_value("Monthly", total, "points"));
            for (name, points) in models {
                reading
                    .lines
                    .push(lines::count_value(&name, points, "points"));
            }
        }
        if !complete && reading.warning.is_none() {
            reading.warning =
                Some("Poe history is partial because the usage request limit was reached.".into());
        }
        Ok(reading)
    }
}

/// The Balance row of a balance answer.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    let balance =
        value::number(body, "/current_point_balance").ok_or_else(|| http::decoding(NAME))?;
    Ok(Reading::new(
        None,
        vec![lines::count_value("Balance", balance, "points")],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const HISTORY_BODY: &str = r#"{"data":[{"creation_time":"2026-09-27T09:00:00Z","cost_points":35,"bot_name":"Claude Sonnet"}],"has_more":false}"#;

    #[tokio::test]
    async fn shows_the_point_balance_then_the_points_each_bot_spent_over_30_days() {
        let http = Scripted::new().on("GET", URL, 200, r###"{"current_point_balance":250000}"###);
        let http = http.on(
            "GET",
            "https://api.poe.com/usage/points_history",
            200,
            HISTORY_BODY,
        );
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = Poe.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                None,
                vec![
                    lines::count_value("Balance", 250000.0, "points"),
                    lines::count_value("Monthly", 35.0, "points"),
                    lines::count_value("Claude Sonnet", 35.0, "points")
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer test"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn failed_or_unreadable_answers_keep_their_categories_without_echoing_the_body() {
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
            let error = Poe.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Poe.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
