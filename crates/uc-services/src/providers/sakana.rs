//! Sakana AI: the Session (5-hour) and Weekly shares of a Sakana AI console plan and the plan's
//! name, with the credit balance and the extra usage spent when the pay-as-you-go tab shows them.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the value of the console's `session`
//! cookie is pasted into Accounts. A refresh sends up to two `GET`s for the console's HTML billing
//! page, each with `Cookie: session=<value>`:
//! - `https://console.sakana.ai/billing` for the `5-hour` and `Weekly` blocks (their `N% used` and
//!   `Resets on <date> at <time>`, read as UTC) and the plan in the card title. A redirect there
//!   means the session expired.
//! - `https://console.sakana.ai/billing?tab=payAsYouGo`, once the first page has been read, for
//!   the `Credit balance` and the usage `Total:`. When it fails, the card keeps its meters without
//!   those two rows.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines};

pub(crate) struct Sakana;

const NAME: &str = "Sakana AI";
const URL: &str = "https://console.sakana.ai/billing";

#[async_trait]
impl Service for Sakana {
    fn id(&self) -> &'static str {
        "sakana"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (session)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://console.sakana.ai/billing",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut descriptors: Vec<_> = [("session", "Session"), ("weekly", "Weekly")]
            .into_iter()
            .map(|(id, title)| {
                WidgetDescriptor::percent(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    None,
                )
                .exporting_progress(id, "percent")
            })
            .collect();
        descriptors.push(WidgetDescriptor::dollar_balance(
            format!("{}.balance", provider.id),
            provider,
            "Balance",
            None,
            "left",
        ));
        descriptors.push(WidgetDescriptor::values(
            format!("{}.extraUsage", provider.id),
            provider,
            "Extra Usage",
            None,
            Some(uc_core::MetricKind::Dollars),
            None,
            false,
            None,
            false,
        ));
        descriptors
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Sakana AI session cookie is missing."))?;
        let response = http::send(
            context.http,
            HttpRequest::get(URL)
                .header("Cookie", format!("session={key}"))
                .header("Accept", "text/html,application/xhtml+xml")
                .header("Accept-Language", "en-US,en;q=0.9"),
            NAME,
        )
        .await?;
        if (300..400).contains(&response.status) {
            return Err(http::expired(
                "The Sakana AI session expired. Sign in again and replace the session cookie.",
            ));
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let mut reading = parse(&Value::String(
            String::from_utf8_lossy(&response.body).into_owned(),
        ))?;
        if let Ok(response) = http::send(
            context.http,
            HttpRequest::get(format!("{URL}?tab=payAsYouGo"))
                .header("Cookie", format!("session={key}"))
                .header("Accept", "text/html,application/xhtml+xml")
                .header("Accept-Language", "en-US,en;q=0.9"),
            NAME,
        )
        .await
            && response.status == 200
        {
            reading
                .lines
                .extend(extra_usage(&String::from_utf8_lossy(&response.body)));
        }
        Ok(reading)
    }
}

/// The Balance and Extra Usage rows of the pay-as-you-go tab: the dollar amount shown after
/// `Credit balance`, and the `Total:` under `Usage`. No rows when the balance cannot be read.
fn extra_usage(html: &str) -> Vec<uc_core::MetricLine> {
    let html = html.replace("<!-- -->", "");
    let money = |text: &str| {
        text.trim()
            .trim_start_matches('$')
            .replace(',', "")
            .parse::<f64>()
            .ok()
            .filter(|amount| amount.is_finite() && *amount >= 0.0)
    };
    let Some((_, tail)) = html.split_once("Credit balance") else {
        return vec![];
    };
    let bounded = tail.chars().take(900).collect::<String>();
    let balance = bounded
        .split_once("tabular-nums")
        .and_then(|(_, tail)| tail.split_once('>'))
        .and_then(|(_, text)| text.split_once("</p>"))
        .and_then(|(text, _)| money(text));
    let Some(balance) = balance else {
        return vec![];
    };
    let mut rows = vec![lines::dollar_value("Balance", balance)];
    if let Some((_, tail)) = html.split_once(">Usage</h2>")
        && let Some(usage) = elements(tail, "span")
            .iter()
            .find_map(|span| span.trim().strip_prefix("Total:").and_then(money))
    {
        rows.push(lines::dollar_value("Extra Usage", usage));
    }
    rows
}

/// The Session and Weekly meters and the plan of the billing page. Each meter reads the
/// `% used` and `Resets on` paragraphs that follow its `5-hour` or `Weekly` paragraph.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    let html = body.as_str().ok_or_else(|| http::decoding(NAME))?;
    let paragraphs = elements(html, "p");
    let mut meters = Vec::new();
    for (label, title, period) in [
        ("5-hour", "Session", 5 * lines::HOUR_MS),
        ("Weekly", "Weekly", lines::WEEK_MS),
    ] {
        let Some(index) = paragraphs
            .iter()
            .position(|paragraph| paragraph.trim() == label)
        else {
            continue;
        };
        let group = paragraphs[index + 1..]
            .iter()
            .take_while(|paragraph| paragraph.trim() != "5-hour" && paragraph.trim() != "Weekly")
            .collect::<Vec<_>>();
        let percent = group
            .iter()
            .find_map(|paragraph| {
                paragraph
                    .trim()
                    .strip_suffix("% used")
                    .and_then(|number| number.trim().parse::<f64>().ok())
            })
            .filter(|used| used.is_finite() && *used >= 0.0 && *used <= 100.0)
            .ok_or_else(|| http::decoding(NAME))?;
        let reset = group
            .iter()
            .find_map(|paragraph| paragraph.trim().strip_prefix("Resets on "))
            .and_then(|date| {
                chrono::NaiveDateTime::parse_from_str(date, "%B %e, %Y at %I:%M %p").ok()
            })
            .map(|time| time.and_utc());
        meters.push(lines::percent(title, percent, reset, Some(period)));
    }
    if meters.is_empty() {
        return Err(http::decoding(NAME));
    }
    let plan = html
        .split_once("data-slot=\"card-title\"")
        .and_then(|(_, rest)| elements(rest, "span").into_iter().next())
        .and_then(|name| lines::plan_name(&name));
    Ok(Reading::new(plan, meters))
}

/// The trimmed text of the elements of `html` that open with `<tag`, skipping those that hold
/// other elements; at most 512.
fn elements(html: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut rest = html;
    let mut texts = Vec::new();
    while let Some((_, tail)) = rest.split_once(&open) {
        let Some((_, body)) = tail.split_once('>') else {
            break;
        };
        let Some((text, end)) = body.split_once(&close) else {
            break;
        };
        if !text.contains('<') {
            texts.push(text.trim().to_string());
        }
        rest = end;
        if texts.len() >= 512 {
            break;
        }
    }
    texts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const BILLING_PAGE: &str = r###"<div data-slot="card-title"><span>Pro</span></div><p>5-hour</p><p>25% used</p><p>Resets on September 27, 2026 at 03:00 PM</p><p>Weekly</p><p>40% used</p><p>Resets on October 1, 2026 at 12:00 AM</p>"###;

    #[tokio::test]
    async fn reads_the_session_and_weekly_meters_and_the_plan_from_the_billing_page() {
        let http = Scripted::new().on("GET", URL, 200, BILLING_PAGE);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = Sakana.fetch(&scope.context()).await.unwrap();
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
                    lines::percent(
                        "Weekly",
                        40.0,
                        Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()),
                        Some(lines::WEEK_MS)
                    )
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Cookie"), Some("session=test"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn failed_or_unreadable_pages_keep_their_categories_without_echoing_the_answer() {
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
            let error = Sakana.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[test]
    fn reads_the_balance_and_extra_usage_from_the_pay_as_you_go_tab() {
        let html = r#"<h2 class="font-semibold">Credit balance</h2><button aria-label="Credit updates may be delayed."></button><p class="font-semibold tabular-nums">$12.34</p><h2 class="font-semibold">Usage</h2><span class="text-muted-foreground">Total<!-- -->: <!-- -->$5.67</span>"#;
        assert_eq!(
            extra_usage(html),
            vec![
                lines::dollar_value("Balance", 12.34),
                lines::dollar_value("Extra Usage", 5.67)
            ]
        );
        assert_eq!(extra_usage("<h2>Credit balance</h2><p>$99</p>"), vec![]);
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Sakana.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
