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
//! The pass's `tier_19`, `tier_49` and `tier_199` read as the Starter, Pro and Expert plans, and its
//! `nextBillingAt` is the plan's renewal date. A second request, `GET https://api.kilo.ai/api/profile`
//! with the same bearer token (the profile the Kilo CLI reads, `{"user": {"email", ...}}`), names
//! the account; when it fails the card shows without the email.
//!
//! Signing in from Quota Control uses the device sign-in of the Kilo CLI: app.kilo.ai's page opens
//! with the code in it, the user signs in there (Google, GitHub and the others Kilo offers) and
//! approves, and the long-lived token Kilo hands over is saved in Quota Control as the CLI saves it.

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, PlanTerm, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::signin::{
    self, Converted, Look, Method, Pending, Poll, Polling, SignIn, SignedIn, StartContext,
};
use crate::support::{http, jwt, lines, oauth, value};

pub(crate) struct Kilo;

const URL: &str = "https://app.kilo.ai/api/trpc/user.getCreditBlocks,kiloPass.getState";
const EXPIRED: &str = "The Kilo login expired. Open Kilo once to renew it.";
/// The account profile the Kilo CLI reads for the signed-in email.
const PROFILE_URL: &str = "https://api.kilo.ai/api/profile";
/// The device sign-in of the Kilo CLI: a code, and where the app asks whether it was approved.
const DEVICE_CODES: &str = "https://api.kilo.ai/api/device-auth/codes";
const DEVICE_TOKEN: &str = "https://api.kilo.ai/api/device-auth/token";

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

    fn sign_in(&self) -> Option<&'static dyn SignIn> {
        Some(&Kilo)
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
        let expired = || {
            http::expired(if context.secret.is_owned() {
                oauth::SIGN_IN_EXPIRED
            } else {
                EXPIRED
            })
        };
        if jwt::expires_at(key).is_some_and(|expiry| expiry <= context.now) {
            return Err(expired());
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
                    expired()
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
        let mut renews_at = None;
        if let Some(subscription) = payload(1)
            .and_then(|state| state.get("subscription"))
            .filter(|subscription| !subscription.is_null())
        {
            let used = value::number(subscription, "/currentPeriodUsageUsd")
                .ok_or_else(|| http::decoding("Kilo"))?;
            let base = value::number(subscription, "/currentPeriodBaseCreditsUsd")
                .ok_or_else(|| http::decoding("Kilo"))?;
            let bonus = value::number(subscription, "/currentPeriodBonusCreditsUsd").unwrap_or(0.0);
            renews_at = value::time(subscription, "/nextBillingAt");
            rows.push(lines::dollars(
                "Kilo Pass",
                used,
                base + bonus,
                renews_at,
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
        let account = http::json(
            context.http,
            HttpRequest::get(PROFILE_URL)
                .bearer(key)
                .header("Accept", "application/json"),
            "Kilo",
        )
        .await
        .ok()
        .and_then(|profile| {
            value::text(&profile, "/user/email")
                .or_else(|| value::text(&profile, "/email"))
                .map(str::to_string)
        });
        Ok(Reading::new(plan, rows)
            .with_plan_term(renews_at.map(|ends_at| PlanTerm::Stated {
                ends_at,
                checked_at: None,
            }))
            .with_account(account))
    }
}

#[async_trait]
impl SignIn for Kilo {
    /// Kilo's page offers both, and the choice is made there.
    fn methods(&self) -> &'static [Method] {
        &[Method::Google, Method::GitHub]
    }

    async fn start(
        &self,
        _method: Method,
        context: &StartContext,
    ) -> Result<Pending, SimpleProviderError> {
        let response = context
            .http
            .send(
                HttpRequest::post(DEVICE_CODES)
                    .header("Accept", "application/json")
                    .timeout(signin::REQUEST_TIMEOUT),
            )
            .await
            .map_err(|_| signin::network("Kilo"))?;
        let body: Value = response.json().unwrap_or(Value::Null);
        let device_code = value::text(&body, "/device_code").map(str::to_string);
        let page = value::text(&body, "/verificationUrl")
            .and_then(|raw| signin::page_on(raw, &["kilo.ai"]));
        let (Some(device_code), Some(page), true) = (device_code, page, response.is_success())
        else {
            return Err(SimpleProviderError::new(
                uc_core::ErrorCategory::http(response.status),
                "Kilo did not start a sign-in. Try again later.",
            ));
        };
        let lifetime = value::number(&body, "/expiresIn").unwrap_or(600.0).max(0.0) as u64;
        let requests = context.http.clone();
        let look = move || -> Look {
            let http = requests.clone();
            let device_code = device_code.clone();
            Box::pin(async move { look_device(&http, &device_code).await })
        };
        Ok(signin::polled(
            page,
            None,
            Polling {
                interval: std::time::Duration::from_secs(3),
                slow_down: std::time::Duration::from_secs(3),
                lifetime: std::time::Duration::from_secs(lifetime),
            },
            context.http.clone(),
            look,
            signed_in,
        ))
    }
}

/// One look at a device sign-in: 202 while it waits for the user, 403 when refused, 410 once the
/// code expired or was used.
async fn look_device(http: &uc_core::SharedHttpClient, device_code: &str) -> Poll {
    let Ok(response) = http
        .send(
            HttpRequest::post(DEVICE_TOKEN)
                .json_body(&json!({ "deviceCode": device_code, "supportsRefresh": false }))
                .header("Accept", "application/json")
                .timeout(signin::REQUEST_TIMEOUT),
        )
        .await
    else {
        return Poll::Waiting;
    };
    match response.status {
        200 => Poll::Approved(response.json().unwrap_or(Value::Null)),
        202 => Poll::Waiting,
        403 => Poll::Failed(signin::denied()),
        410 => Poll::Failed(uc_core::loopback::expired()),
        429 => Poll::SlowDown,
        status if status >= 500 => Poll::Waiting,
        status => Poll::Failed(signin::unfinished(status)),
    }
}

/// An approved device sign-in's token, saved as the Kilo CLI saves it and named by its user.
fn signed_in(_: uc_core::SharedHttpClient, answer: Value) -> Converted {
    Box::pin(async move {
        let token = value::text(&answer, "/token")
            .ok_or_else(|| signin::invalid("Kilo returned no usable sign-in."))?;
        let email = value::text(&answer, "/userEmail").map(str::to_string);
        let identity = value::text(&answer, "/userId")
            .map(str::to_string)
            .or_else(|| email.clone())
            .ok_or_else(|| signin::invalid("Kilo did not say which account signed in."))?;
        Ok(SignedIn {
            identity,
            label: email,
            document: json!({ "apiKey": token }),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header, owned_context_at};
    use chrono::Utc;
    use uc_core::ErrorCategory;

    fn start_context(http: &Scripted) -> StartContext {
        StartContext {
            http: http.shared(),
            language: uc_core::loopback::LoginLanguage::English,
            product: "Kilo",
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_device_sign_in_saves_the_token_the_page_approved() {
        let http = Scripted::new()
            .on(
                "POST",
                DEVICE_CODES,
                200,
                r#"{"code":"ABCD-EFGH","user_code":"ABCD-EFGH","device_code":"dc-1",
                    "verificationUrl":"https://app.kilo.ai/device-auth?code=ABCD-EFGH","expiresIn":600}"#,
            )
            .on("POST", DEVICE_TOKEN, 202, r#"{"status":"pending"}"#)
            .on(
                "POST",
                DEVICE_TOKEN,
                200,
                r#"{"status":"approved","token":"kilo-jwt","userId":"user-9","userEmail":"me@example.com"}"#,
            );
        let pending = Kilo
            .start(Method::Google, &start_context(&http))
            .await
            .unwrap();
        assert_eq!(
            pending.url,
            "https://app.kilo.ai/device-auth?code=ABCD-EFGH"
        );
        assert!(pending.user_code.is_none());
        let account = pending.finish.await.result.unwrap();
        assert_eq!(account.identity, "user-9");
        assert_eq!(account.label.as_deref(), Some("me@example.com"));
        assert_eq!(account.document, json!({"apiKey": "kilo-jwt"}));
        let requests = http.requests();
        let body: Value = serde_json::from_slice(requests[1].body.as_deref().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"deviceCode": "dc-1", "supportsRefresh": false})
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_or_expired_device_code_ends_the_sign_in() {
        for (status, message) in [
            (403, "The browser login was not authorized."),
            (410, "This login has expired. Start again."),
        ] {
            let http = Scripted::new()
                .on(
                    "POST",
                    DEVICE_CODES,
                    200,
                    r#"{"device_code":"dc-1","verificationUrl":"https://app.kilo.ai/device-auth?code=X"}"#,
                )
                .on("POST", DEVICE_TOKEN, status, "{}");
            let pending = Kilo
                .start(Method::GitHub, &start_context(&http))
                .await
                .unwrap();
            let error = pending.finish.await.result.unwrap_err();
            assert_eq!(error.message, message);
        }
    }

    #[tokio::test]
    async fn an_expired_sign_in_asks_to_sign_in_again_in_accounts() {
        let http = Scripted::new();
        let expired = format!(
            "e30.{}.sig",
            base64::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                json!({"exp": 1_000_000_000}).to_string()
            )
        );
        let scope = owned_context_at(&http, json!({"apiKey": expired}), Utc::now());
        let error = Kilo.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, oauth::SIGN_IN_EXPIRED);
    }

    const CREDITS_AND_PASS: &str = r#"[{"result":{"data":{"json":{"creditBlocks":[{"amount_mUsd":10000000,"balance_mUsd":7500000,"description":"Purchased"}],"totalBalance_mUsd":7500000}}}},{"result":{"data":{"json":{"subscription":{"tier":"tier_49","currentPeriodUsageUsd":5,"currentPeriodBaseCreditsUsd":20,"currentPeriodBonusCreditsUsd":2,"nextBillingAt":"2026-10-01T00:00:00Z"}}}}}]"#;

    #[tokio::test]
    async fn credits_kilo_pass_and_block_balances_come_from_one_batched_get() {
        let http = Scripted::new().on("GET", URL, 200, CREDITS_AND_PASS).on(
            "GET",
            PROFILE_URL,
            200,
            r#"{"user":{"name":"Fixture","email":"fixture@example.com"},"organizations":[]}"#,
        );
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Kilo.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.account.as_deref(), Some("fixture@example.com"));
        assert_eq!(
            reading.plan_term,
            value::as_time(&json!("2026-10-01T00:00:00Z")).map(|ends_at| PlanTerm::Stated {
                ends_at,
                checked_at: None,
            })
        );
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
        assert_eq!(
            header(&http.requests()[1], "Authorization"),
            Some("Bearer test")
        );
    }

    #[tokio::test]
    async fn a_failed_profile_leaves_only_the_email_out() {
        let http =
            Scripted::new()
                .on("GET", URL, 200, CREDITS_AND_PASS)
                .on("GET", PROFILE_URL, 500, "{}");
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Kilo.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.account, None);
        assert_eq!(reading.lines.len(), 3);
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
