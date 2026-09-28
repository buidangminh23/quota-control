//! Droid: the usage windows, token allowances and extra-usage balance of a Factory account, read
//! with a Factory API key.
//!
//! On Windows, macOS and Linux the key comes from the `FACTORY_API_KEY` environment variable, from
//! a key saved in Quota Control, or from the `FACTORY_API_KEY=` line of `~/.factory/.env`, which is
//! found as a Droid login identified by the key's SHA-256 digest. Browser sessions and rotating
//! WorkOS tokens are never read. Every request goes to `https://api.factory.ai` with the key as a
//! bearer token and the origin, referer and client headers of Factory's web app. A refresh sends
//! `GET /api/billing/limits` and, at most once every 12 hours, `GET /api/app/auth/me` for the plan,
//! the user id and the account's email. A plan billed by token rate limits shows its session, weekly and monthly
//! windows from the limits answer; an older plan also sends
//! `GET /api/organization/subscription/usage?useCache=true&userId=<id>` and shows how much of its
//! standard and premium token allowances is used. The extra-usage balance follows, then the core
//! windows when the limits answer has both standard and core windows.

use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{http, lines, value};

pub(crate) struct Factory;

const NAME: &str = "Droid";
const BASE: &str = "https://api.factory.ai";

#[async_trait]
impl Service for Factory {
    fn id(&self) -> &'static str {
        "factory"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::login("Droid").or_api_key(ApiKeyHelp {
            env: &["FACTORY_API_KEY"],
            url: "https://app.factory.ai/settings/api-keys",
            fields: &[],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let path = roots.home.join(".factory/.env");
        if std::fs::metadata(&path)
            .ok()
            .is_none_or(|metadata| metadata.len() > 1_048_576)
        {
            return vec![];
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            return vec![];
        };
        text.lines()
            .find_map(|line| {
                let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
                let (name, key) = line.split_once('=')?;
                if name.trim() != "FACTORY_API_KEY" {
                    return None;
                }
                let key = key.trim().trim_matches(['\'', '"']);
                (!key.is_empty()).then(|| {
                    Login::new(
                        Sha256::digest(key.as_bytes())
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>(),
                        "Droid",
                        &path,
                        Secret::api_key(key),
                    )
                })
            })
            .into_iter()
            .collect()
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut descriptors: Vec<_> = [
            ("session", "Session"),
            ("weekly", "Weekly"),
            ("monthly", "Monthly"),
            ("standard", "Standard"),
            ("premium", "Premium"),
        ]
        .into_iter()
        .map(|(id, title)| {
            WidgetDescriptor::percent(format!("{}.{id}", provider.id), provider, title, None, None)
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
        descriptors
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Droid API key is missing."))?;
        let limits = get(context, key, "/api/billing/limits").await?;
        let account = if let Some(account) = context.memo.get("factory-account", context.now).await
        {
            account
        } else {
            let account = get(context, key, "/api/app/auth/me").await?;
            context
                .memo
                .put(
                    "factory-account",
                    account.clone(),
                    Some(context.now + chrono::Duration::hours(12)),
                )
                .await;
            account
        };
        let plan = value::text(
            &account,
            "/organization/subscription/orbSubscription/plan/name",
        )
        .or_else(|| value::text(&account, "/organization/subscription/factoryTier"))
        .and_then(lines::plan_name);
        let mut out = vec![];
        if value::flag(&limits, "/usesTokenRateLimitsBilling") == Some(true) {
            for (f, title, period) in [
                ("fiveHour", "Session", 5 * lines::HOUR_MS),
                ("weekly", "Weekly", lines::WEEK_MS),
                ("monthly", "Monthly", lines::MONTH_MS),
            ] {
                if let Some(window) = limits
                    .pointer(&format!("/limits/standard/{f}"))
                    .or_else(|| limits.pointer(&format!("/limits/core/{f}")))
                    && let Some(used) = value::number(window, "/usedPercent")
                {
                    let reset = value::time(window, "/windowEnd").or_else(|| {
                        value::number(window, "/secondsRemaining").and_then(|seconds| {
                            context.now.checked_add_signed(chrono::Duration::seconds(
                                seconds.clamp(0.0, 31536000.0) as i64,
                            ))
                        })
                    });
                    out.push(lines::percent(title, used, reset, Some(period)));
                }
            }
        } else {
            let id =
                value::text(&account, "/userProfile/id").ok_or_else(|| http::decoding(NAME))?;
            let mut url = url::Url::parse(&format!("{BASE}/api/organization/subscription/usage"))
                .map_err(|_| http::decoding(NAME))?;
            url.query_pairs_mut()
                .append_pair("useCache", "true")
                .append_pair("userId", id);
            let usage = http::json(context.http, request(key, url.as_str()), NAME).await?;
            for (f, title) in [("standard", "Standard"), ("premium", "Premium")] {
                if let Some(allowance) = usage.pointer(&format!("/usage/{f}")) {
                    let ratio = value::number(allowance, "/usedRatio").or_else(|| {
                        let limit = value::number(allowance, "/totalAllowance")?;
                        let used = value::number(allowance, "/userTokens")
                            .or_else(|| value::number(allowance, "/orgTotalTokensUsed"))?;
                        (limit > 0.0).then_some(used / limit)
                    });
                    if let Some(ratio) = ratio {
                        out.push(lines::percent(
                            title,
                            ratio * 100.0,
                            value::time(&usage, "/usage/endDate"),
                            Some(lines::MONTH_MS),
                        ));
                    }
                }
            }
        }
        if let Some(cents) = value::number(&limits, "/extraUsageBalanceCents") {
            out.push(lines::dollar_value("Balance", cents / 100.0));
        }
        for (f, title) in [
            ("fiveHour", "Core Session"),
            ("weekly", "Core Weekly"),
            ("monthly", "Core Monthly"),
        ] {
            if limits.pointer("/limits/standard").is_some()
                && let Some(window) = limits.pointer(&format!("/limits/core/{f}"))
                && let Some(used) = value::number(window, "/usedPercent")
            {
                out.push(lines::percent(
                    title,
                    used,
                    value::time(window, "/windowEnd"),
                    None,
                ));
            }
        }
        if out.is_empty() {
            return Err(http::decoding(NAME));
        }
        Ok(Reading::new(plan, out).with_account(value::text(&account, "/userProfile/email")))
    }
}

/// A GET of `url` with the key as a bearer token and the origin, referer and client headers of
/// Factory's web app.
fn request(key: &str, url: &str) -> HttpRequest {
    HttpRequest::get(url)
        .bearer(key)
        .header("Accept", "application/json")
        .header("Origin", "https://app.factory.ai")
        .header("Referer", "https://app.factory.ai/")
        .header("x-factory-client", "web-app")
}

/// The JSON answer of `path` on the Factory API.
async fn get(
    context: &FetchContext<'_>,
    key: &str,
    path: &str,
) -> Result<Value, SimpleProviderError> {
    http::json(context.http, request(key, &format!("{BASE}{path}")), NAME).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    const LIMITS: &str = r#"{"usesTokenRateLimitsBilling":true,"limits":{"standard":{"fiveHour":{"usedPercent":25,"windowEnd":"2026-09-28T00:00:00Z"}},"core":{"fiveHour":{"usedPercent":10}}},"extraUsageBalanceCents":1234}"#;
    const ACCOUNT: &str = r#"{"organization":{"subscription":{"factoryTier":"pro"}},"userProfile":{"id":"u-1","email":"me@example.com"}}"#;

    #[tokio::test]
    async fn a_token_rate_limit_plan_shows_its_windows_balance_and_core_windows() {
        let http = Scripted::new()
            .on("GET", &format!("{BASE}/api/billing/limits"), 200, LIMITS)
            .on("GET", &format!("{BASE}/api/app/auth/me"), 200, ACCOUNT);
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Factory.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.account.as_deref(), Some("me@example.com"));
        assert_eq!(
            reading.lines,
            vec![
                lines::percent(
                    "Session",
                    25.0,
                    value::as_time(&json!("2026-09-28T00:00:00Z")),
                    Some(5 * lines::HOUR_MS)
                ),
                lines::dollar_value("Balance", 12.34),
                lines::percent("Core Session", 10.0, None, None)
            ]
        );
        assert_eq!(http.requests().len(), 2);
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer test")
        );
        assert_eq!(
            header(&http.requests()[0], "x-factory-client"),
            Some("web-app")
        );
    }

    #[tokio::test]
    async fn refused_keys_and_rate_limits_keep_their_category() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (403, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
        ] {
            let http = Scripted::new().on("GET", BASE, status, "{}");
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            assert_eq!(
                Factory.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }

    #[test]
    fn the_env_file_key_is_found_as_a_droid_login_identified_by_its_sha256_digest() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        assert!(Factory.discover(&roots).is_empty());
        let folder = roots.home.join(".factory");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(".env"), "export FACTORY_API_KEY='test-key'\n").unwrap();
        let logins = Factory.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(
            logins[0].identity,
            Sha256::digest(b"test-key")
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert_eq!(logins[0].secret.key(), Some("test-key"));
    }
}
