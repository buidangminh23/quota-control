//! Windows, macOS and Linux: Hermes auth.json files or pasted Nous access token;
//! GET /api/oauth/account at a trusted Nous portal host, which also names the plan, the account's
//! `user.email` and the subscription's `current_period_end`.
//! Hermes owns token renewal; this reader never refreshes rotating tokens.

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, PlanTerm, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{endpoint, http, jwt, lines, value};

const NAME: &str = "Nous Portal";
#[cfg(test)]
const URL: &str = "https://portal.nousresearch.com/api/oauth/account";
const EXPIRED: &str = "The Hermes Agent login expired. Open Hermes Agent once to renew it.";

pub(crate) struct Nous;

#[async_trait]
impl Service for Nous {
    fn id(&self) -> &'static str {
        "nous"
    }

    fn name(&self) -> &'static str {
        "Nous Portal"
    }

    fn connection(&self) -> Connection {
        Connection::login("Hermes Agent").or_api_key(ApiKeyHelp {
            env: &["NOUS_PORTAL_ACCESS_TOKEN"],
            url: "https://portal.nousresearch.com/usage",
            fields: &[("baseUrl", "Portal URL")],
        })
    }

    fn key_label(&self) -> &'static str {
        "Access token"
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let root = roots.dir_from("HERMES_HOME", roots.home.join(".hermes"));
        ["auth.json", "shared/nous_auth.json"]
            .into_iter()
            .filter_map(|file| {
                let path = root.join(file);
                let document = value::read_json(&path, 1024 * 1024)?;
                let state = document
                    .pointer("/providers/nous")
                    .filter(|v| value::text(v, "/access_token").is_some())
                    .or_else(|| {
                        document
                            .pointer("/credential_pool/nous")
                            .and_then(Value::as_array)
                            .and_then(|entries| {
                                entries
                                    .iter()
                                    .filter(|v| value::text(v, "/access_token").is_some())
                                    .max_by_key(|v| {
                                        (
                                            value::time(v, "/agent_key_expires_at"),
                                            value::time(v, "/expires_at"),
                                            std::cmp::Reverse(
                                                v.get("priority")
                                                    .and_then(Value::as_i64)
                                                    .unwrap_or(0),
                                            ),
                                        )
                                    })
                            })
                    })
                    .unwrap_or(&document);
                let token = value::text(state, "/access_token")?;
                let identity = jwt::claim(token, &["sub"]).unwrap_or_else(|| {
                    Sha256::digest(token.as_bytes())
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect()
                });
                let base = value::text(state, "/portal_base_url")
                    .filter(|base| trusted(base))
                    .unwrap_or("https://portal.nousresearch.com");
                Some(Login::new(
                    identity,
                    "Hermes Agent",
                    &path,
                    Secret::new(
                        json!({"apiKey":token,"baseUrl":base,"expiresAt":state.get("expires_at")}),
                    ),
                ))
            })
            .collect()
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::bounded_dollars(
                format!("{}.monthly", provider.id),
                provider,
                "Monthly",
                None,
                0.0,
                None,
                None,
            )
            .exporting_progress("monthly", "usd"),
            WidgetDescriptor::dollar_balance(
                format!("{}.balance", provider.id),
                provider,
                "Balance",
                None,
                "left",
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let token = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Nous Portal access token is missing."))?;
        if value::time(context.secret.value(), "/expiresAt")
            .or_else(|| jwt::expires_at(token))
            .is_some_and(|expiry| expiry <= context.now + chrono::Duration::seconds(60))
        {
            return Err(http::expired(EXPIRED));
        }
        let base = endpoint::base_url(
            context.secret.str("/baseUrl"),
            Some("https://portal.nousresearch.com"),
            endpoint::Policy::Https,
            NAME,
        )?;
        if !trusted(&base) {
            return Err(http::invalid(
                "The Nous Portal address must use a nousresearch.com host.",
            ));
        }
        let response = http::send(
            context.http,
            HttpRequest::get(format!("{base}/api/oauth/account")).bearer(token),
            NAME,
        )
        .await?;
        if response.status == 401 {
            return Err(http::expired(EXPIRED));
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let body = http::parse(&response, NAME)?;
        if body
            .get("error")
            .is_some_and(|v| !v.is_null() && v != false)
        {
            return Err(http::decoding(NAME));
        }
        let monthly = value::number(&body, "/subscription/monthly_credits");
        let remaining = value::number(&body, "/subscription/credits_remaining").or_else(|| {
            value::number(&body, "/paid_service_access/subscription_credits_remaining")
        });
        let purchased = value::number(&body, "/purchased_credits_remaining")
            .or_else(|| value::number(&body, "/paid_service_access/purchased_credits_remaining"));
        let total = value::number(&body, "/paid_service_access/total_usable_credits");
        let mut output = Vec::new();
        if let (Some(limit), Some(left)) = (monthly, remaining) {
            if limit < 0.0 {
                return Err(http::decoding(NAME));
            }
            output.push(lines::dollars(
                "Monthly",
                (limit - left.max(0.0)).max(0.0),
                limit,
                value::time(&body, "/subscription/current_period_end"),
                Some(lines::MONTH_MS),
            ));
        }
        if let Some(balance) = total.or(purchased).or(remaining) {
            output.push(lines::dollar_value("Balance", balance));
        }
        if output.is_empty() {
            return Err(http::decoding(NAME));
        }
        Ok(Reading::new(
            value::text(&body, "/subscription/plan").and_then(lines::plan_name),
            output,
        )
        .with_account(value::text(&body, "/user/email"))
        .with_plan_term(
            value::time(&body, "/subscription/current_period_end").map(|ends_at| {
                PlanTerm::Stated {
                    ends_at,
                    checked_at: None,
                }
            }),
        ))
    }
}

fn trusted(base: &str) -> bool {
    url::Url::parse(base).ok().is_some_and(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(|host| {
                host == "nousresearch.com" || host.ends_with(".nousresearch.com")
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;
    #[test]
    fn discovers_without_network_and_ignores_untrusted_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        assert!(Nous.discover(&roots).is_empty());
        let folder = dir.path().join(".hermes");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("auth.json"),r#"{"providers":{"nous":{"access_token":"test","portal_base_url":"https://evil.example"}}}"#).unwrap();
        let found = Nous.discover(&roots);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].identity.len(), 64);
        assert_eq!(
            found[0].secret.str("/baseUrl"),
            Some("https://portal.nousresearch.com")
        );
    }
    #[tokio::test]
    async fn exact_credits_and_request() {
        let http=Scripted::new().on("GET",URL,200,r#"{"subscription":{"monthly_credits":100,"credits_remaining":75,"plan":"pro","current_period_end":"2026-10-01T00:00:00Z"},"paid_service_access":{"total_usable_credits":90},"user":{"email":"me@example.com"}}"#);
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        assert_eq!(
            Nous.fetch(&scope.context()).await.unwrap(),
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::dollars(
                        "Monthly",
                        25.0,
                        100.0,
                        Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()),
                        Some(lines::MONTH_MS)
                    ),
                    lines::dollar_value("Balance", 90.0)
                ]
            )
            .with_account(Some("me@example.com"))
            .with_plan_term(Some(PlanTerm::Stated {
                ends_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
                checked_at: None,
            }))
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer test"));
        assert_eq!(requests[0].body, None);
    }
    #[tokio::test]
    async fn failures_do_not_leak_secrets() {
        for (status, body, category) in [
            (401, "test", ErrorCategory::AuthExpired),
            (429, "", ErrorCategory::RateLimited),
            (500, "", ErrorCategory::Http5xx),
            (200, "{}", ErrorCategory::Decoding),
            (200, "not json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let error = Nous.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("test"));
        }
    }
}
