//! Amp: the free allowance, the subscription's Monthly and Orb meters and the credit balances Amp
//! reports for an account. Nothing is read from disk on Windows, macOS or Linux: the key comes
//! from the `AMP_API_KEY` environment variable or from a key saved in Quota Control.
//!
//! A refresh sends one request, `POST https://ampcode.com/api/internal?userDisplayBalanceInfo`,
//! with the key as a bearer token and the JSON body
//! `{"method":"userDisplayBalanceInfo","params":{}}`. Amp answers with a balance text
//! (`result.displayText`), read line by line once its ANSI escape codes and `**` markers are
//! dropped: `Amp Free` gives the dollars (or the percentage) left of the free allowance,
//! `Individual credits` the Balance row, each `Workspace <name>` a dollar row of its own, and the
//! subscription or tier line the plan name and the Monthly and Orb meters, which reset when the
//! billing period it names ends or after the renewal it counts down to. An `auth-required` error
//! code means Amp rejected the key.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Amp;

const NAME: &str = "Amp";
const URL: &str = "https://ampcode.com/api/internal?userDisplayBalanceInfo";

#[async_trait]
impl Service for Amp {
    fn id(&self) -> &'static str {
        "amp"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["AMP_API_KEY"],
            url: "https://ampcode.com/settings",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [
            ("free", "Amp Free"),
            ("subscription", "Monthly"),
            ("orb", "Orb"),
            ("balance", "Balance"),
        ]
        .into_iter()
        .map(|(id, title)| {
            WidgetDescriptor::combined(
                format!("{}.{id}", provider.id),
                provider,
                title,
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
            .ok_or_else(|| http::invalid("The Amp API key is missing."))?;
        let body = http::json(
            context.http,
            HttpRequest::post(URL)
                .bearer(key)
                .header("Accept", "application/json")
                .json_body(&serde_json::json!({"method":"userDisplayBalanceInfo","params":{}})),
            NAME,
        )
        .await?;
        parse(&body, context)
    }
}

/// The rows Amp's balance text describes; `context` gives the time a subscription's "renewal in"
/// countdown starts from.
fn parse(body: &Value, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
    if value::text(body, "/error/code") == Some("auth-required") {
        return Err(http::expired("The Amp API key was rejected."));
    }
    if body.get("ok") != Some(&Value::Bool(true)) {
        return Err(http::decoding(NAME));
    }
    let text =
        strip_ansi(value::text(body, "/result/displayText").ok_or_else(|| http::decoding(NAME))?)
            .replace("**", "");
    let mut free = None;
    let mut subscription = None;
    let mut orb = None;
    let mut balance = None;
    let mut plan = None;
    let mut workspaces = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((label, details)) = line.split_once(':') else {
            continue;
        };
        if label == "Amp Free" {
            if let Some((left, total)) = details.split_once('/') {
                if let (Some(left), Some(total)) = (amount(left), amount(total)) {
                    let period = details
                        .split_once("replenishes +")
                        .and_then(|(_, text)| amount(text))
                        .filter(|rate| *rate > 0.0)
                        .map(|rate| (total / rate).round().max(1.0) * lines::HOUR_MS as f64)
                        .filter(|millis| millis.is_finite() && *millis <= i64::MAX as f64)
                        .map(|millis| millis as i64);
                    free = Some(lines::dollars(
                        "Amp Free",
                        (total - left).max(0.0),
                        total,
                        None,
                        period,
                    ));
                }
            } else if let Some((left, _)) = details.split_once('%')
                && let Some(left) = amount(left)
            {
                free = Some(lines::percent(
                    "Amp Free",
                    100.0 - left,
                    None,
                    Some(lines::DAY_MS),
                ));
            }
        } else if label == "Individual credits" {
            balance = amount(details).map(|remaining| lines::dollar_value("Balance", remaining));
        } else if let Some(name) = label.strip_prefix("Workspace ")
            && let Some(remaining) = amount(details)
        {
            workspaces.push(lines::dollar_value(name, remaining));
        } else if label.starts_with("Subscription ")
            || label.ends_with(" Subscription")
            || label.ends_with(" Tier")
        {
            plan = Some(
                label
                    .trim_start_matches("Subscription ")
                    .trim_start_matches("Amp ")
                    .trim_end_matches(" Subscription")
                    .trim_end_matches(" Tier")
                    .to_string(),
            );
            let period = details.split_once("period ").and_then(|(_, rest)| {
                let mut words = rest.split_whitespace();
                let start = chrono::NaiveDate::parse_from_str(words.next()?, "%Y-%m-%d").ok()?;
                if words.next()? != "to" {
                    return None;
                }
                let end = chrono::NaiveDate::parse_from_str(
                    words.next()?.trim_end_matches([',', ')']),
                    "%Y-%m-%d",
                )
                .ok()?;
                (end > start).then_some((start, end))
            });
            let reset = period
                .and_then(|(_, end)| end.and_hms_opt(0, 0, 0))
                .map(|midnight| midnight.and_utc())
                .or_else(|| {
                    details.split_once("renewal in ").and_then(|(_, rest)| {
                        let units = amount(rest)?;
                        if units < 0.0 || units > u32::MAX as f64 {
                            return None;
                        }
                        if rest.contains("month") {
                            context
                                .now
                                .checked_add_months(chrono::Months::new(units as u32))
                        } else {
                            context
                                .now
                                .checked_add_signed(chrono::Duration::try_days(units as i64)?)
                        }
                    })
                });
            let duration = period
                .map(|(start, end)| (end - start).num_milliseconds())
                .or(Some(lines::MONTH_MS));
            let dollars = details
                .split('$')
                .skip(1)
                .filter_map(amount)
                .collect::<Vec<_>>();
            if details.contains("agent usage") && dollars.len() >= 2 {
                subscription = Some(lines::dollars(
                    "Monthly",
                    (dollars[1] - dollars[0]).max(0.0),
                    dollars[1],
                    reset,
                    duration,
                ));
                if let Some((_, rest)) = details.split_once("orb usage ") {
                    let mut words = rest.split_whitespace();
                    if let (Some(left), Some("of"), Some(limit)) = (
                        words.next().and_then(amount),
                        words.next(),
                        words.next().and_then(amount),
                    ) && limit > 0.0
                    {
                        orb = Some(lines::count(
                            "Orb",
                            (limit - left).max(0.0),
                            limit,
                            "hours",
                            reset,
                            duration,
                        ));
                    }
                }
            } else {
                let segments = details.split('%').collect::<Vec<_>>();
                if segments.len() >= 3 {
                    if let Some(left) = amount(segments[0]) {
                        subscription = Some(lines::percent(
                            "Monthly",
                            100.0 - left,
                            reset,
                            Some(lines::MONTH_MS),
                        ));
                    }
                    if let Some(left) = segments[1].split_whitespace().rev().find_map(amount) {
                        orb = Some(lines::percent(
                            "Orb",
                            100.0 - left,
                            reset,
                            Some(lines::MONTH_MS),
                        ));
                    }
                }
            }
        }
    }
    let mut rows = Vec::new();
    rows.extend(free);
    rows.extend(subscription);
    rows.extend(orb);
    rows.extend(balance);
    rows.extend(workspaces);
    if rows.is_empty() {
        return Err(http::decoding(NAME));
    }
    Ok(Reading::new(plan, rows))
}

/// The number `text` starts with, after spaces and a `$`, with its thousands commas dropped.
fn amount(text: &str) -> Option<f64> {
    let raw = text
        .trim()
        .trim_start_matches('$')
        .chars()
        .take_while(|character| {
            character.is_ascii_digit() || *character == '.' || *character == ','
        })
        .filter(|character| *character != ',')
        .collect::<String>();
    raw.parse::<f64>().ok().filter(|number| number.is_finite())
}

fn strip_ansi(text: &str) -> String {
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        } else {
            plain.push(character);
        }
    }
    plain
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const FREE_CREDITS_AND_WORKSPACE: &str = r###"{"ok":true,"result":{"displayText":"Amp Free: $3 / $10 remaining (replenishes +$1 / hour)\nIndividual credits: $12.50 remaining\nWorkspace Team: $45 remaining"}}"###;
    const TIER_WITH_PERIOD: &str = "\u{1b}[32mAmp Pro Tier: agent usage $30 of $100 remaining, orb usage 8h of 10h a1.small orb hours remaining, period 2026-01-01 to 2026-02-01 resets upon renewal in 1 months\u{1b}[0m";
    const PERCENT_SUBSCRIPTION: &str = "Subscription Pro: 60% other usage and 20% orb usage remaining - resets upon renewal in 1 months";

    #[tokio::test]
    async fn the_free_allowance_credits_and_workspaces_come_from_one_authorized_post() {
        let http = Scripted::new().on("POST", URL, 200, FREE_CREDITS_AND_WORKSPACE);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = Amp.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                None,
                vec![
                    lines::dollars("Amp Free", 7.0, 10.0, None, Some(10 * lines::HOUR_MS)),
                    lines::dollar_value("Balance", 12.5),
                    lines::dollar_value("Team", 45.0)
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer test"));
        assert_eq!(
            serde_json::from_slice::<Value>(requests[0].body.as_ref().unwrap()).unwrap(),
            json!({"method":"userDisplayBalanceInfo","params":{}})
        );
    }

    #[tokio::test]
    async fn failed_or_unreadable_answers_keep_their_categories_without_the_body() {
        for (status, body, category) in [
            (401, "secret", ErrorCategory::AuthExpired),
            (403, "secret", ErrorCategory::AuthExpired),
            (429, "", ErrorCategory::RateLimited),
            (500, "", ErrorCategory::Http5xx),
            (200, "{}", ErrorCategory::Decoding),
            (200, "bad json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("POST", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let error = Amp.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn subscription_meters_reset_when_the_period_ends_or_a_calendar_month_later() {
        let http = Scripted::new();
        let scope = context_at(
            &http,
            json!({}),
            Utc.with_ymd_and_hms(2026, 1, 31, 10, 0, 0).unwrap(),
        );
        let body = json!({"ok":true,"result":{"displayText":TIER_WITH_PERIOD}});
        let reset = Some(Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap());
        assert_eq!(
            parse(&body, &scope.context()).unwrap(),
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::dollars("Monthly", 70.0, 100.0, reset, Some(31 * lines::DAY_MS)),
                    lines::count("Orb", 2.0, 10.0, "hours", reset, Some(31 * lines::DAY_MS))
                ]
            )
        );
        let body = json!({"ok":true,"result":{"displayText":PERCENT_SUBSCRIPTION}});
        assert_eq!(
            parse(&body, &scope.context()).unwrap(),
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::percent(
                        "Monthly",
                        40.0,
                        Some(Utc.with_ymd_and_hms(2026, 2, 28, 10, 0, 0).unwrap()),
                        Some(lines::MONTH_MS)
                    ),
                    lines::percent(
                        "Orb",
                        80.0,
                        Some(Utc.with_ymd_and_hms(2026, 2, 28, 10, 0, 0).unwrap()),
                        Some(lines::MONTH_MS)
                    )
                ]
            )
        );
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Amp.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
