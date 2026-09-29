//! Cline: the Session (5-hour), Weekly and Monthly usage limits of a Cline plan, as the share of
//! each window used. The login Cline keeps in `providers.json` is found the same way on Windows,
//! macOS and Linux: `CLINE_PROVIDER_SETTINGS_PATH` when it is set, else `settings/providers.json`
//! under `CLINE_DATA_DIR`, else `data/settings/providers.json` under `CLINE_DIR` or `~/.cline`.
//! Its WorkOS access token (`auth.accessToken`) is taken before a saved API key (`apiKey` or
//! `auth.apiKey`). A Cline API key can also come from `CLINE_API_KEY` or `CLINEPASS_API_KEY` or be
//! saved in Quota Control.
//!
//! A refresh sends one request, `GET https://api.cline.bot/api/v1/users/me/plan/usage-limits`,
//! with the key or token as a bearer token. A login token is used as saved and never renewed here:
//! an expired one is not sent, and an expired or refused login asks for Cline to be opened once.
//! After it, `GET https://api.cline.bot/api/v1/users/me/plan` (the account's current plan, as the
//! Cline SDK's `fetchCurrentUserPlan` reads it) names the plan and when its period ends; it is
//! looked up twice a day, and its failure leaves the usage rows as they are.
//!
//! Signing in from Quota Control uses the device sign-in of the Cline CLI and extension: WorkOS's
//! page for Cline's client opens with the code in it (the user signs in there with Google, GitHub or
//! email), and the WorkOS tokens are registered with Cline for its own, as Cline does. Those are the
//! card's own, so it renews them shortly before they expire and keeps the refresh token Cline hands
//! back.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::signin::{
    self, Converted, DeviceClient, Method, Pending, SignIn, SignedIn, StartContext,
};
use crate::support::{http, jwt, lines, oauth, value};

pub(crate) struct Cline;

const NAME: &str = "Cline";
const URL: &str = "https://api.cline.bot/api/v1/users/me/plan/usage-limits";
const PLAN_URL: &str = "https://api.cline.bot/api/v1/users/me/plan";
const EXPIRED: &str = "The Cline login expired. Open Cline once to renew it.";
/// Where Cline turns WorkOS tokens into its own, and renews its own.
const REGISTER: &str = "https://api.cline.bot/api/v1/auth/register";
const REFRESH: &str = "https://api.cline.bot/api/v1/auth/refresh";
/// How long before its expiry a sign-in made here is renewed, as Cline renews its own.
const RENEW_BEFORE_MINUTES: i64 = 5;

/// The WorkOS device sign-in of Cline's own client.
const DEVICE: DeviceClient = DeviceClient {
    device_url: "https://api.workos.com/user_management/authorize/device",
    token_url: "https://api.workos.com/user_management/authenticate",
    client_id: "client_01K3A541FN8TA3EPPHTD2325AR",
    scope: "",
    headers: &[],
};

/// Each window: its `type` in the answer, the widget id, the row title and the window's length.
const WINDOWS: [(&str, &str, &str, i64); 3] = [
    ("five_hour", "session", "Session", 5 * lines::HOUR_MS),
    ("weekly", "weekly", "Weekly", lines::WEEK_MS),
    ("monthly", "monthly", "Monthly", lines::MONTH_MS),
];

#[async_trait]
impl Service for Cline {
    fn id(&self) -> &'static str {
        "cline"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::login("Cline").or_api_key(ApiKeyHelp {
            env: &["CLINE_API_KEY", "CLINEPASS_API_KEY"],
            url: "https://app.cline.bot",
            fields: &[],
        })
    }

    fn sign_in(&self) -> Option<&'static dyn SignIn> {
        Some(&Cline)
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let env_path = |name: &str| {
            roots.var(name).map(|raw| {
                if raw == "~" {
                    roots.home.clone()
                } else if let Some(tail) = raw.strip_prefix("~/") {
                    roots.home.join(tail)
                } else {
                    std::path::PathBuf::from(raw)
                }
            })
        };
        let path = env_path("CLINE_PROVIDER_SETTINGS_PATH").unwrap_or_else(|| {
            env_path("CLINE_DATA_DIR")
                .unwrap_or_else(|| {
                    env_path("CLINE_DIR")
                        .unwrap_or_else(|| roots.home.join(".cline"))
                        .join("data")
                })
                .join("settings/providers.json")
        });
        let Some(body) = value::read_json(&path, 1024 * 1024) else {
            return Vec::new();
        };
        let settings = &body["providers"]["cline"]["settings"];
        let (token, oauth) = if let Some(v) = value::text(settings, "/auth/accessToken") {
            (
                if v.starts_with("workos:") {
                    v.to_string()
                } else {
                    format!("workos:{v}")
                },
                true,
            )
        } else if let Some(key) =
            value::text(settings, "/apiKey").or_else(|| value::text(settings, "/auth/apiKey"))
        {
            (key.into(), false)
        } else {
            return Vec::new();
        };
        let identity: String = Sha256::digest(token.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        vec![Login::new(
            identity,
            "Cline",
            &path,
            Secret::new(json!({"apiKey":token,"oauth":oauth})),
        )]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        WINDOWS
            .iter()
            .map(|(_, id, title, _)| {
                WidgetDescriptor::percent(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    None,
                )
                .exporting_progress(id, "percent")
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let renewed = renew_if_due(context).await?;
        let fresh;
        let secret = match renewed {
            Some(document) => {
                context.keep_renewed(document.clone()).await?;
                fresh = Secret::owned(document);
                &fresh
            }
            None => context.secret,
        };
        let expired = || {
            http::expired(if secret.is_owned() {
                oauth::SIGN_IN_EXPIRED
            } else {
                EXPIRED
            })
        };
        let key = secret
            .key()
            .ok_or_else(|| http::invalid("The Cline API key is missing."))?;
        let oauth = secret.value()["oauth"] == true;
        if oauth && expiry(secret).is_some_and(|expiry| expiry <= context.now) {
            return Err(expired());
        }
        let response = http::send(
            context.http,
            HttpRequest::get(URL)
                .bearer(key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        if oauth && matches!(response.status, 401 | 403) {
            return Err(expired());
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let body = http::parse(&response, NAME)?;
        if body["success"] != true {
            return Err(http::decoding(NAME));
        }
        let limits = body["data"]["limits"]
            .as_array()
            .ok_or_else(|| http::decoding(NAME))?;
        let mut rows = Vec::new();
        for (kind, _, title, period) in WINDOWS {
            let mut last = None;
            for item in limits {
                let item_kind = item["type"].as_str().ok_or_else(|| http::decoding(NAME))?;
                if item_kind != kind {
                    continue;
                }
                let used = item["percentUsed"]
                    .as_f64()
                    .filter(|percent| percent.is_finite())
                    .ok_or_else(|| http::decoding(NAME))?;
                let reset = if item["resetsAt"].is_null() {
                    None
                } else {
                    Some(value::time(item, "/resetsAt").ok_or_else(|| http::decoding(NAME))?)
                };
                last = Some(lines::percent(title, used, reset, Some(period)));
            }
            rows.extend(last);
        }
        let current = current_plan(context, key).await;
        let plan = value::text(&current, "/plan/displayName")
            .map(str::to_string)
            .or_else(|| value::text(&current, "/plan/name").and_then(lines::plan_name));
        let ends_at = value::time(&current, "/cancelAt")
            .or_else(|| value::time(&current, "/currentPeriodEnd"));
        Ok(
            Reading::new(plan, rows).with_plan_term(ends_at.map(|ends_at| {
                uc_core::PlanTerm::Stated {
                    ends_at,
                    checked_at: None,
                }
            })),
        )
    }
}

/// The account's current plan (`data` of Cline's envelope), looked up twice a day. Best-effort: any
/// failure gives `Null` and never affects the usage rows.
async fn current_plan(context: &FetchContext<'_>, key: &str) -> Value {
    if let Some(memo) = context.memo.get("cline.plan", context.now).await {
        return memo;
    }
    let response = match http::send(
        context.http,
        HttpRequest::get(PLAN_URL)
            .bearer(key)
            .header("Accept", "application/json"),
        NAME,
    )
    .await
    {
        Ok(response) if response.is_success() => response,
        Ok(response) => {
            tracing::debug!(target: "cline", "current plan answered {}", response.status);
            return Value::Null;
        }
        Err(error) => {
            tracing::debug!(target: "cline", "current plan unavailable: {}", error.message);
            return Value::Null;
        }
    };
    let Ok(body) = http::parse(&response, NAME) else {
        return Value::Null;
    };
    let current = if body["success"].is_boolean() {
        if body["success"] != true {
            return Value::Null;
        }
        body["data"].clone()
    } else {
        body
    };
    context
        .memo
        .put(
            "cline.plan",
            current.clone(),
            Some(context.now + Duration::hours(12)),
        )
        .await;
    current
}

#[async_trait]
impl SignIn for Cline {
    /// WorkOS's page for Cline offers both, and the choice is made there.
    fn methods(&self) -> &'static [Method] {
        &[Method::Google, Method::GitHub]
    }

    async fn start(
        &self,
        _method: Method,
        context: &StartContext,
    ) -> Result<Pending, SimpleProviderError> {
        signin::start_device(&DEVICE, context, registered).await
    }
}

/// An approved WorkOS sign-in registered with Cline, in the shape the card reads Cline's own login,
/// named by the Cline account.
fn registered(http: uc_core::SharedHttpClient, tokens: Value) -> Converted {
    Box::pin(async move {
        let (Some(access), Some(refresh)) = (
            value::text(&tokens, "/access_token"),
            value::text(&tokens, "/refresh_token"),
        ) else {
            return Err(signin::invalid("Cline returned no usable sign-in."));
        };
        let response = http
            .send(
                HttpRequest::post(REGISTER)
                    .json_body(&json!({ "accessToken": access, "refreshToken": refresh }))
                    .header("Accept", "application/json")
                    .timeout(signin::REQUEST_TIMEOUT),
            )
            .await
            .map_err(|_| signin::network(NAME))?;
        if !response.is_success() {
            return Err(signin::unfinished(response.status));
        }
        let body: Value = response.json().unwrap_or(Value::Null);
        let data = &body["data"];
        let document = token_document(data, None)
            .filter(|_| body["success"] == true)
            .ok_or_else(|| signin::invalid("Cline returned no usable sign-in."))?;
        let email = value::text(data, "/userInfo/email").map(str::to_string);
        let identity = value::text(data, "/userInfo/clineUserId")
            .or_else(|| value::text(data, "/userInfo/subject"))
            .map(str::to_string)
            .or_else(|| email.clone())
            .ok_or_else(|| signin::invalid("Cline did not say which account signed in."))?;
        Ok(SignedIn {
            identity,
            label: email,
            document,
        })
    })
}

/// Cline's token answer as the card keeps it: the access token as the bearer Cline sends, and the
/// refresh token (the previous one when Cline sends none) and the expiry for renewing it.
fn token_document(data: &Value, previous_refresh: Option<&str>) -> Option<Value> {
    let access = value::text(data, "/accessToken")?;
    let refresh = value::text(data, "/refreshToken").or(previous_refresh)?;
    Some(json!({
        "apiKey": format!("workos:{}", access.strip_prefix("workos:").unwrap_or(access)),
        "oauth": true,
        "refreshToken": refresh,
        "expiresAt": data.get("expiresAt").cloned().unwrap_or(Value::Null),
    }))
}

/// When the card's token expires: its stated expiry, else the one the token carries.
fn expiry(secret: &Secret) -> Option<DateTime<Utc>> {
    value::time(secret.value(), "/expiresAt").or_else(|| {
        let key = secret.key()?;
        jwt::expires_at(key.strip_prefix("workos:").unwrap_or(key))
    })
}

/// A sign-in made in Quota Control, renewed shortly before it expires as Cline renews its own.
/// Another app's login is never renewed.
async fn renew_if_due(context: &FetchContext<'_>) -> Result<Option<Value>, SimpleProviderError> {
    let secret = context.secret;
    if !secret.is_owned() {
        return Ok(None);
    }
    if expiry(secret)
        .is_none_or(|expiry| expiry > context.now + Duration::minutes(RENEW_BEFORE_MINUTES))
    {
        return Ok(None);
    }
    let refresh = secret
        .str("/refreshToken")
        .ok_or_else(|| http::expired(oauth::SIGN_IN_EXPIRED))?;
    let response = http::send(
        context.http,
        HttpRequest::post(REFRESH)
            .json_body(&json!({ "refreshToken": refresh, "grantType": "refresh_token" }))
            .header("Accept", "application/json"),
        NAME,
    )
    .await?;
    if matches!(response.status, 400 | 401 | 403) {
        return Err(http::expired(oauth::SIGN_IN_REVOKED));
    }
    if !response.is_success() {
        return Err(http::status_error(&response, NAME));
    }
    let body = http::parse(&response, NAME)?;
    if body["success"] != true {
        return Err(http::decoding(NAME));
    }
    token_document(&body["data"], Some(refresh))
        .map(Some)
        .ok_or_else(|| http::decoding(NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header, owned_context_at};
    use uc_core::ErrorCategory;

    const KEY_AND_LOGIN: &str = r#"{"providers":{"cline":{"settings":{"apiKey":"old-key","auth":{"accessToken":"access-fixture"}}}}}"#;
    const THREE_WINDOWS: &str = r#"{"success":true,"data":{"limits":[{"type":"five_hour","percentUsed":25,"resetsAt":"2026-10-01T00:00:00Z"},{"type":"weekly","percentUsed":12.5},{"type":"monthly","percentUsed":2}]}}"#;

    fn now() -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn body(request: &HttpRequest) -> Value {
        serde_json::from_slice(request.body.as_deref().unwrap_or_default()).unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn a_device_sign_in_registers_the_workos_tokens_with_cline() {
        let http = Scripted::new()
            .on(
                "POST",
                DEVICE.device_url,
                200,
                r#"{"device_code":"dc-1","user_code":"ABCD-EFGH",
                    "verification_uri":"https://authkit.cline.bot/device",
                    "verification_uri_complete":"https://authkit.cline.bot/device?user_code=ABCD-EFGH",
                    "expires_in":300,"interval":5}"#,
            )
            .on(
                "POST",
                DEVICE.token_url,
                400,
                r#"{"error":"authorization_pending"}"#,
            )
            .on(
                "POST",
                DEVICE.token_url,
                200,
                r#"{"access_token":"workos-at","refresh_token":"workos-rt","token_type":"Bearer"}"#,
            )
            .on(
                "POST",
                REGISTER,
                200,
                r#"{"success":true,"data":{"accessToken":"cline-at","refreshToken":"cline-rt",
                    "tokenType":"Bearer","expiresAt":"2026-09-27T11:00:00Z",
                    "userInfo":{"subject":"user_01","email":"me@example.com","name":"Minh",
                        "clineUserId":"cline-9","accounts":null}}}"#,
            );
        let context = StartContext {
            http: http.shared(),
            language: uc_core::loopback::LoginLanguage::English,
            product: NAME,
        };
        let pending = Cline.start(Method::GitHub, &context).await.unwrap();
        assert_eq!(
            pending.url,
            "https://authkit.cline.bot/device?user_code=ABCD-EFGH"
        );
        let account = pending.finish.await.result.unwrap();
        assert_eq!(account.identity, "cline-9");
        assert_eq!(account.label.as_deref(), Some("me@example.com"));
        assert_eq!(
            account.document,
            json!({
                "apiKey": "workos:cline-at",
                "oauth": true,
                "refreshToken": "cline-rt",
                "expiresAt": "2026-09-27T11:00:00Z"
            })
        );
        let requests = http.requests();
        let start = String::from_utf8(requests[0].body.clone().unwrap()).unwrap();
        assert_eq!(start, "client_id=client_01K3A541FN8TA3EPPHTD2325AR");
        assert_eq!(
            body(&requests[3]),
            json!({"accessToken": "workos-at", "refreshToken": "workos-rt"})
        );
    }

    #[tokio::test]
    async fn a_sign_in_made_here_is_renewed_before_it_expires() {
        let http = Scripted::new()
            .on(
                "POST",
                REFRESH,
                200,
                r#"{"success":true,"data":{"accessToken":"cline-at-2","tokenType":"Bearer",
                    "expiresAt":"2026-09-27T11:00:00Z","userInfo":{"email":"me@example.com"}}}"#,
            )
            .on("GET", URL, 200, THREE_WINDOWS);
        let scope = owned_context_at(
            &http,
            json!({
                "apiKey": "workos:cline-at",
                "oauth": true,
                "refreshToken": "cline-rt",
                "expiresAt": (now() + Duration::minutes(2)).to_rfc3339()
            }),
            now(),
        );
        let reading = Cline.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines.len(), 3);
        let requests = http.requests();
        assert_eq!(
            body(&requests[0]),
            json!({"refreshToken": "cline-rt", "grantType": "refresh_token"})
        );
        assert_eq!(
            header(&requests[1], "Authorization"),
            Some("Bearer workos:cline-at-2")
        );
        assert_eq!(
            scope.renewed().await.unwrap(),
            json!({
                "apiKey": "workos:cline-at-2",
                "oauth": true,
                "refreshToken": "cline-rt",
                "expiresAt": "2026-09-27T11:00:00Z"
            })
        );
    }

    #[tokio::test]
    async fn a_refused_renewal_asks_to_sign_in_again_in_accounts() {
        let http = Scripted::new().on("POST", REFRESH, 401, r#"{"error":"invalid_grant"}"#);
        let scope = owned_context_at(
            &http,
            json!({
                "apiKey": "workos:cline-at",
                "oauth": true,
                "refreshToken": "cline-rt",
                "expiresAt": now().to_rfc3339()
            }),
            now(),
        );
        let error = Cline.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, oauth::SIGN_IN_REVOKED);
        assert!(scope.renewed().await.is_none());
    }

    #[test]
    fn discovery_prefers_the_login_token_and_gives_it_the_workos_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        assert!(Cline.discover(&roots).is_empty());
        let path = roots.home.join(".cline/data/settings/providers.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, KEY_AND_LOGIN).unwrap();
        let entries = Cline.discover(&roots);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].identity,
            Sha256::digest(b"workos:access-fixture")
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert_eq!(entries[0].secret.key(), Some("workos:access-fixture"));
        assert!(entries[0].label.is_none());
    }

    #[tokio::test]
    async fn each_window_becomes_a_row_from_one_bearer_authorized_get() {
        let http = Scripted::new().on("GET", URL, 200, THREE_WINDOWS);
        let scope = context_at(
            &http,
            json!({"apiKey":"workos:fixture","oauth":true}),
            chrono::Utc::now(),
        );
        assert_eq!(
            Cline.fetch(&scope.context()).await.unwrap(),
            Reading::new(
                None,
                vec![
                    lines::percent(
                        "Session",
                        25.0,
                        Some("2026-10-01T00:00:00Z".parse().unwrap()),
                        Some(5 * lines::HOUR_MS)
                    ),
                    lines::percent("Weekly", 12.5, None, Some(lines::WEEK_MS)),
                    lines::percent("Monthly", 2.0, None, Some(lines::MONTH_MS))
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, URL);
        assert_eq!(
            header(&requests[0], "authorization"),
            Some("Bearer workos:fixture")
        );
        assert_eq!(requests[1].url, PLAN_URL);
    }

    #[tokio::test]
    async fn the_current_plan_names_the_plan_and_its_period_end() {
        let http = Scripted::new().on("GET", URL, 200, THREE_WINDOWS).on(
            "GET",
            PLAN_URL,
            200,
            r#"{"success":true,"data":{"currentPeriodStart":"2026-09-10T00:00:00Z",
                "currentPeriodEnd":"2026-10-10T00:00:00Z",
                "plan":{"id":"p1","name":"pro","displayName":"Pro","interval":"month"}}}"#,
        );
        let scope = context_at(
            &http,
            json!({"apiKey":"workos:fixture","oauth":true}),
            now(),
        );
        let reading = Cline.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(
            reading.plan_term,
            Some(uc_core::PlanTerm::Stated {
                ends_at: "2026-10-10T00:00:00Z".parse().unwrap(),
                checked_at: None,
            })
        );
        assert_eq!(reading.lines.len(), 3);
    }

    #[tokio::test]
    async fn failures_are_reported_after_one_request_without_renewing_the_login() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
            (
                200,
                r#"{"success":true,"data":{"limits":[{"type":"weekly","percentUsed":"bad"}]}}"#,
                ErrorCategory::Decoding,
            ),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(
                &http,
                json!({"apiKey":"fixture","oauth":true}),
                chrono::Utc::now(),
            );
            assert_eq!(
                Cline.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
            assert_eq!(http.requests().len(), 1);
        }
    }
}
