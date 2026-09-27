//! Kilo: the credits of a Kilo account (its credit blocks and their balances) and its Kilo Pass
//! subscription. The login Kilo keeps in `~/.local/share/kilo/auth.json` (its `kilo.access`
//! token) is read from the home folder on Windows, macOS and Linux alike; a Kilo API key can
//! instead come from `KILO_API_KEY` or be saved in Quota Control. A token is used as saved and
//! never renewed here: an expired one is not sent, and one the answer calls `UNAUTHORIZED` or
//! `FORBIDDEN` also asks for Kilo to be opened once.
//!
//! A refresh sends one request, a read-only tRPC batch of `user.getCreditBlocks` and
//! `kiloPass.getState`: `GET https://app.kilo.ai/api/trpc/user.getCreditBlocks,kiloPass.getState`
//! with `batch=1`, a `null` input for both queries and the key or token as a bearer token. The
//! Credits row shows how much of all credit blocks is used (their amounts come in millionths of a
//! dollar), the Kilo Pass row this period's usage of its base and bonus credits until the next
//! billing date, and every credit block with a description or name gets a balance row of its own.
//! The pass's `tier_19`, `tier_49` and `tier_199` read as the Starter, Pro and Expert plans.

use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{http, jwt, lines, value};

pub(crate) struct Kilo;

const URL: &str = "https://app.kilo.ai/api/trpc/user.getCreditBlocks,kiloPass.getState";

#[async_trait]
impl Service for Kilo {
    fn id(&self) -> &'static str {
        "kilo"
    }

    fn name(&self) -> &'static str {
        "Kilo"
    }

    fn connection(&self) -> Connection {
        Connection::login("Kilo").or_api_key(ApiKeyHelp {
            env: &["KILO_API_KEY"],
            url: "https://app.kilo.ai",
            fields: &[],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let path = roots.home.join(".local/share/kilo/auth.json");
        let Some(document) = value::read_json(&path, 1_048_576) else {
            return vec![];
        };
        let Some(token) = value::text(&document, "/kilo/access") else {
            return vec![];
        };
        vec![Login::new(
            Sha256::digest(token.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            "Kilo",
            &path,
            Secret::api_key(token),
        )]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::bounded_dollars(
                format!("{}.credits", provider.id),
                provider,
                "Credits",
                None,
                0.0,
                Some("credits"),
                Some("used"),
            ),
            WidgetDescriptor::bounded_dollars(
                format!("{}.pass", provider.id),
                provider,
                "Kilo Pass",
                None,
                0.0,
                Some("credits"),
                Some("used"),
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Kilo key or token is missing."))?;
        if jwt::expires_at(key).is_some_and(|expiry| expiry <= context.now) {
            return Err(http::expired(
                "The Kilo login expired. Open Kilo once to renew it.",
            ));
        }
        let mut url = url::Url::parse(URL).map_err(|_| http::decoding("Kilo"))?;
        url.query_pairs_mut()
            .append_pair("batch", "1")
            .append_pair("input", r#"{"0":{"json":null},"1":{"json":null}}"#);
        let body = http::json(
            context.http,
            HttpRequest::get(url.as_str())
                .bearer(key)
                .header("Accept", "application/json"),
            "Kilo",
        )
        .await?;
        let entries = body.as_array().ok_or_else(|| http::decoding("Kilo"))?;
        for entry in entries {
            if entry.get("error").is_some() {
                let code = value::text(entry, "/error/json/data/code");
                return Err(if matches!(code, Some("UNAUTHORIZED" | "FORBIDDEN")) {
                    http::expired("The Kilo login expired. Open Kilo once to renew it.")
                } else {
                    http::decoding("Kilo")
                });
            }
        }
        let payload = |index: usize| {
            entries.get(index).and_then(|entry| {
                entry
                    .pointer("/result/data/json")
                    .or_else(|| entry.pointer("/result/json"))
            })
        };
        let credits = payload(0).ok_or_else(|| http::decoding("Kilo"))?;
        let blocks = credits
            .get("creditBlocks")
            .and_then(Value::as_array)
            .ok_or_else(|| http::decoding("Kilo"))?;
        let mut total = 0.0;
        let mut balance = 0.0;
        for block in blocks {
            total += value::number(block, "/amount_mUsd").ok_or_else(|| http::decoding("Kilo"))?;
            balance +=
                value::number(block, "/balance_mUsd").ok_or_else(|| http::decoding("Kilo"))?;
        }
        let balance = value::number(credits, "/totalBalance_mUsd").unwrap_or(balance);
        let mut rows = vec![lines::dollars(
            "Credits",
            (total - balance).max(0.0) / 1_000_000.0,
            total / 1_000_000.0,
            None,
            None,
        )];
        let mut plan = None;
        if let Some(subscription) = payload(1)
            .and_then(|state| state.get("subscription"))
            .filter(|subscription| !subscription.is_null())
        {
            let used = value::number(subscription, "/currentPeriodUsageUsd")
                .ok_or_else(|| http::decoding("Kilo"))?;
            let base = value::number(subscription, "/currentPeriodBaseCreditsUsd")
                .ok_or_else(|| http::decoding("Kilo"))?;
            let bonus = value::number(subscription, "/currentPeriodBonusCreditsUsd").unwrap_or(0.0);
            rows.push(lines::dollars(
                "Kilo Pass",
                used,
                base + bonus,
                value::time(subscription, "/nextBillingAt"),
                Some(lines::MONTH_MS),
            ));
            plan = value::text(subscription, "/tier").map(|tier| {
                match tier {
                    "tier_19" => "Starter",
                    "tier_49" => "Pro",
                    "tier_199" => "Expert",
                    _ => tier,
                }
                .to_owned()
            });
        }
        for block in blocks {
            if let Some(label) =
                value::text(block, "/description").or_else(|| value::text(block, "/name"))
                && let Some(balance) = value::number(block, "/balance_mUsd")
            {
                rows.push(lines::dollar_value(label, balance / 1_000_000.0));
            }
        }
        Ok(Reading::new(plan, rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    const CREDITS_AND_PASS: &str = r#"[{"result":{"data":{"json":{"creditBlocks":[{"amount_mUsd":10000000,"balance_mUsd":7500000,"description":"Purchased"}],"totalBalance_mUsd":7500000}}}},{"result":{"data":{"json":{"subscription":{"tier":"tier_49","currentPeriodUsageUsd":5,"currentPeriodBaseCreditsUsd":20,"currentPeriodBonusCreditsUsd":2,"nextBillingAt":"2026-10-01T00:00:00Z"}}}}}]"#;

    #[tokio::test]
    async fn credits_kilo_pass_and_block_balances_come_from_one_batched_get() {
        let http = Scripted::new().on("GET", URL, 200, CREDITS_AND_PASS);
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Kilo.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(
            reading.lines,
            vec![
                lines::dollars("Credits", 2.5, 10.0, None, None),
                lines::dollars(
                    "Kilo Pass",
                    5.0,
                    22.0,
                    value::as_time(&json!("2026-10-01T00:00:00Z")),
                    Some(lines::MONTH_MS)
                ),
                lines::dollar_value("Purchased", 7.5)
            ]
        );
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer test")
        );
        assert!(http.requests()[0].url.contains("batch=1"));
    }

    #[tokio::test]
    async fn refused_rate_limited_and_unreadable_answers_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, "{}");
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            assert_eq!(
                Kilo.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }

    #[test]
    fn the_access_token_in_kilo_auth_json_becomes_one_login() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        assert!(Kilo.discover(&roots).is_empty());
        let folder = roots.home.join(".local/share/kilo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("auth.json"), r#"{"kilo":{"access":"test"}}"#).unwrap();
        let logins = Kilo.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity.len(), 64);
        assert_eq!(logins[0].secret.key(), Some("test"));
    }
}
