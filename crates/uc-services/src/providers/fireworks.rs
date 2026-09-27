//! Fireworks: what a Fireworks account was billed over the last 30 days.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `FIREWORKS_API_KEY` environment variable or from a key saved in Quota Control, with an optional
//! account slug saved beside it. Every request carries the key as a bearer token, and a refresh
//! sends at most four. Without a saved slug, a refresh uses the account that a successful refresh
//! with the same key read in the last 12 hours, or else looks it up with
//! `GET https://api.fireworks.ai/v1/accounts`, following `nextPageToken` pages; the key must reach
//! exactly one account. The spend comes from
//! `GET https://api.fireworks.ai/v1/accounts/{slug}/billing/summary` with a `startTime` 30 days
//! back and an `endTime` of now: the line items' `totalCost` in the first currency met, in dollars
//! for USD, with a warning when other currencies were left out. A summary that answers 404 forgets
//! the remembered account and looks the account up again for one more try.

use std::collections::BTreeSet;

use async_trait::async_trait;
use chrono::{Duration, SecondsFormat};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, HttpResponse, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines};

pub(crate) struct Fireworks;

const NAME: &str = "Fireworks";
const BASE: &str = "https://api.fireworks.ai/v1/accounts";
const SELECT: &str = "Set the Fireworks account slug to select one billing account.";
const BUDGET: &str = "Fireworks account discovery needs more pages. Enter the account slug.";

#[async_trait]
impl Service for Fireworks {
    fn id(&self) -> &'static str {
        "fireworks"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["FIREWORKS_API_KEY"],
            url: "https://app.fireworks.ai",
            fields: &[("accountSlug", "Account slug")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![WidgetDescriptor::values(
            format!("{}.spend", provider.id),
            provider,
            "Last 30 Days",
            None,
            None,
            None,
            true,
            None,
            false,
        )]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Fireworks API key is missing."))?;
        let configured = context
            .secret
            .str("/accountSlug")
            .map(str::trim)
            .filter(|slug| !slug.is_empty());
        if configured.is_some_and(|slug| !valid_slug(slug)) {
            return Err(http::invalid(
                "Invalid Fireworks account slug. Check the account slug in Settings.",
            ));
        }
        let fingerprint: String = Sha256::digest(key.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let cached = context
            .memo
            .get("account", context.now)
            .await
            .filter(|saved| saved["fingerprint"] == fingerprint)
            .and_then(|saved| {
                saved["slug"]
                    .as_str()
                    .filter(|slug| valid_slug(slug))
                    .map(str::to_string)
            });
        let mut requests_left = 4;
        let mut slug = if let Some(configured) = configured {
            configured.to_string()
        } else if let Some(cached) = cached {
            cached
        } else {
            discover(context, key, &mut requests_left).await?
        };
        let mut response = summary(context, key, &slug, &mut requests_left).await?;
        if response.status == 404 {
            context.memo.remove("account").await;
            slug = discover(context, key, &mut requests_left).await?;
            response = summary(context, key, &slug, &mut requests_left).await?;
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let body = http::parse(&response, NAME)?;
        if !body.is_object() {
            return Err(http::decoding(NAME));
        }
        let items = match &body["lineItems"] {
            Value::Null => &[][..],
            Value::Array(items) => items.as_slice(),
            _ => return Err(http::decoding(NAME)),
        };
        let mut currency = None;
        let mut total = 0.0;
        let mut mixed = false;
        for item in items {
            if !item.is_object() {
                return Err(http::decoding(NAME));
            }
            let cost = &item["totalCost"];
            if cost.is_null() {
                continue;
            }
            if !cost.is_object() {
                return Err(http::decoding(NAME));
            }
            if (!cost["units"].is_null() && !cost["units"].is_string())
                || (!cost["currencyCode"].is_null() && !cost["currencyCode"].is_string())
                || (!cost["nanos"].is_null() && cost["nanos"].as_i64().is_none())
            {
                return Err(http::decoding(NAME));
            }
            let (Some(units), Some(nanos), Some(code)) = (
                cost["units"]
                    .as_str()
                    .and_then(|units| units.trim().parse::<f64>().ok())
                    .filter(|units| units.is_finite()),
                cost["nanos"].as_i64(),
                cost["currencyCode"]
                    .as_str()
                    .map(str::trim)
                    .filter(|code| !code.is_empty()),
            ) else {
                continue;
            };
            let chosen = *currency.get_or_insert(code);
            if chosen == code {
                total += units + nanos as f64 / 1e9;
            } else {
                mixed = true;
            }
        }
        if !total.is_finite() {
            return Err(http::decoding(NAME));
        }
        let code = currency.ok_or_else(|| {
            http::not_available("Fireworks has no rated billing costs for the last 30 days.")
        })?;
        context
            .memo
            .put(
                "account",
                json!({"fingerprint": fingerprint, "slug": slug}),
                Some(context.now + Duration::hours(12)),
            )
            .await;
        let line = if code.eq_ignore_ascii_case("USD") {
            lines::dollar_value("Last 30 Days", total)
        } else {
            lines::count_value("Last 30 Days", total, code)
        };
        Ok(Reading::new(None, vec![line]).with_warning(mixed.then(|| {
            "Fireworks returned multiple currencies. Only the first currency is shown.".into()
        })))
    }
}

/// Whether `slug` can name an account in a request path: 1 to 256 ASCII letters, digits, dots,
/// underscores or hyphens, other than `.` and `..`.
fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 256
        && slug != "."
        && slug != ".."
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

/// One request out of the refresh's budget. A 404 is handed back for the caller to judge; any
/// other failed status is an error.
async fn get(
    context: &FetchContext<'_>,
    key: &str,
    url: String,
    requests_left: &mut usize,
) -> Result<HttpResponse, SimpleProviderError> {
    if *requests_left == 0 {
        return Err(http::not_available(BUDGET));
    }
    *requests_left -= 1;
    let response = http::send(context.http, HttpRequest::get(url).bearer(key), NAME).await?;
    if response.status != 404 && !response.is_success() {
        return Err(http::status_error(&response, NAME));
    }
    Ok(response)
}

/// The slug of the one account the key reaches, read page by page while the budget still leaves
/// a request for the billing summary.
async fn discover(
    context: &FetchContext<'_>,
    key: &str,
    requests_left: &mut usize,
) -> Result<String, SimpleProviderError> {
    let mut slugs = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut token = String::new();
    loop {
        if *requests_left <= 1 {
            return Err(http::not_available(BUDGET));
        }
        let mut url = url::Url::parse(BASE).map_err(|_| http::decoding(NAME))?;
        if !token.is_empty() {
            url.query_pairs_mut().append_pair("pageToken", &token);
        }
        let response = get(context, key, url.to_string(), requests_left).await?;
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let body = http::parse(&response, NAME)?;
        if !body.is_object() {
            return Err(http::decoding(NAME));
        }
        if let Some(accounts) = body.get("accounts").filter(|accounts| !accounts.is_null()) {
            for account in accounts.as_array().ok_or_else(|| http::decoding(NAME))? {
                if !account.is_object() {
                    return Err(http::decoding(NAME));
                }
                for field in ["accountId", "id", "name"] {
                    if !account[field].is_null() && !account[field].is_string() {
                        return Err(http::decoding(NAME));
                    }
                }
                if let Some(slug) = ["accountId", "id", "name"]
                    .iter()
                    .find_map(|field| {
                        account[field]
                            .as_str()
                            .map(str::trim)
                            .filter(|name| !name.is_empty())
                    })
                    .and_then(|name| name.rsplit('/').find(|part| !part.is_empty()))
                    .filter(|slug| valid_slug(slug))
                {
                    slugs.insert(slug.to_string());
                }
            }
        }
        token = if body["nextPageToken"].is_null() {
            String::new()
        } else {
            body["nextPageToken"]
                .as_str()
                .ok_or_else(|| http::decoding(NAME))?
                .trim()
                .to_string()
        };
        if token.is_empty() {
            return if slugs.len() == 1 {
                slugs
                    .into_iter()
                    .next()
                    .ok_or_else(|| http::invalid(SELECT))
            } else {
                Err(http::invalid(SELECT))
            };
        }
        if !seen.insert(token.clone()) {
            return Err(http::decoding(NAME));
        }
    }
}

/// The account's billing summary from 30 days ago to now.
async fn summary(
    context: &FetchContext<'_>,
    key: &str,
    slug: &str,
    requests_left: &mut usize,
) -> Result<HttpResponse, SimpleProviderError> {
    let mut url = url::Url::parse(&format!("{BASE}/{slug}/billing/summary"))
        .map_err(|_| http::decoding(NAME))?;
    url.query_pairs_mut()
        .append_pair(
            "startTime",
            &(context.now - Duration::days(30)).to_rfc3339_opts(SecondsFormat::Secs, true),
        )
        .append_pair(
            "endTime",
            &context.now.to_rfc3339_opts(SecondsFormat::Secs, true),
        );
    get(context, key, url.to_string(), requests_left).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use uc_core::ErrorCategory;

    const BILL: &str = r#"{"lineItems":[{"totalCost":{"units":"2","nanos":500000000,"currencyCode":"USD"}},{"totalCost":{"units":"1","nanos":0,"currencyCode":"USD"}}]}"#;

    #[tokio::test]
    async fn the_only_account_is_discovered_once_and_nanos_add_to_units() {
        let http = Scripted::new()
            .on(
                "GET",
                BASE,
                200,
                r#"{"accounts":[{"name":"accounts/acct"}]}"#,
            )
            .on("GET", &format!("{BASE}/acct/billing"), 200, BILL);
        let scope = context_at(&http, json!({"apiKey": "fixture"}), chrono::Utc::now());
        let reading = Fireworks.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![lines::dollar_value("Last 30 Days", 3.5)]
        );
        Fireworks.fetch(&scope.context()).await.unwrap();
        assert_eq!(http.requests().len(), 3);
        assert_eq!(
            header(&http.requests()[0], "authorization"),
            Some("Bearer fixture")
        );
        assert!(http.requests()[1].url.contains("startTime="));
    }

    #[tokio::test]
    async fn discovery_never_exceeds_budget_or_selects_ambiguous_account() {
        let http = Scripted::new()
            .on(
                "GET",
                BASE,
                200,
                r#"{"accounts":[{"id":"a"}],"nextPageToken":"1"}"#,
            )
            .on(
                "GET",
                &format!("{BASE}?pageToken=1"),
                200,
                r#"{"accounts":[{"id":"a"}],"nextPageToken":"2"}"#,
            )
            .on(
                "GET",
                &format!("{BASE}?pageToken=2"),
                200,
                r#"{"accounts":[{"id":"a"}],"nextPageToken":"3"}"#,
            );
        let scope = context_at(&http, json!({"apiKey": "fixture"}), chrono::Utc::now());
        assert_eq!(
            Fireworks
                .fetch(&scope.context())
                .await
                .unwrap_err()
                .category,
            ErrorCategory::NotAvailable
        );
        assert_eq!(http.requests().len(), 3);
        let http = Scripted::new().on("GET", BASE, 200, r#"{"accounts":[{"id":"a"},{"id":"b"}]}"#);
        let scope = context_at(&http, json!({"apiKey": "fixture"}), chrono::Utc::now());
        assert_eq!(
            Fireworks
                .fetch(&scope.context())
                .await
                .unwrap_err()
                .category,
            ErrorCategory::AuthInvalid
        );
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_stale_saved_slug_is_looked_up_again_and_failures_keep_their_categories() {
        let http = Scripted::new()
            .on("GET", &format!("{BASE}/old/billing"), 404, "{}")
            .on("GET", BASE, 200, r#"{"accounts":[{"id":"acct"}]}"#)
            .on("GET", &format!("{BASE}/acct/billing"), 200, BILL);
        let scope = context_at(
            &http,
            json!({"apiKey": "fixture", "accountSlug": "old"}),
            chrono::Utc::now(),
        );
        assert!(Fireworks.fetch(&scope.context()).await.is_ok());
        assert_eq!(http.requests().len(), 3);
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", BASE, status, body);
            let scope = context_at(
                &http,
                json!({"apiKey": "fixture", "accountSlug": "acct"}),
                chrono::Utc::now(),
            );
            assert_eq!(
                Fireworks
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                category
            );
        }
    }
}
