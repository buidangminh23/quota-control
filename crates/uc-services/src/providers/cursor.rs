//! Cursor: the login the Cursor app keeps in its VS Code-style state database
//! `User/globalStorage/state.vscdb`, under `%APPDATA%\Cursor` on Windows,
//! `~/Library/Application Support/Cursor` on macOS and `$XDG_CONFIG_HOME/Cursor` (by default
//! `~/.config/Cursor`) on Linux: the `cursorAuth/accessToken` JWT, the account's cached email and
//! its membership type. The token is used as saved and never renewed here: Cursor rotates it and
//! writes the new one back itself, so an expired token asks for Cursor to be opened once.
//!
//! Endpoints: the dashboard's Connect RPCs on `api2.cursor.sh`
//! (`aiserver.v1.DashboardService/GetCurrentPeriodUsage` for the billing cycle, `GetPlanInfo`,
//! `GetSandUsageStatus` for Grok Bot and `GetCreditGrantsBalance`) with the token as a bearer, and
//! `cursor.com/api/auth/stripe` (the prepaid balance), `/api/usage-summary` and `/api/usage` (the
//! Enterprise and team fallback) with the `WorkosCursorSessionToken` cookie the web dashboard builds
//! from the same token. A refresh sends at most four requests; the plan and the credit balance are
//! remembered between refreshes.
//! The usage answer's `billingCycleEnd` is also shown as the end of a paid plan's current period.
//!
//! A key pasted in Quota Control is either the `WorkosCursorSessionToken` cookie of a signed-in
//! cursor.com tab (`<user id>%3A%3A<token>`, or the token alone), read exactly as the app's login,
//! or an Enterprise team's Admin API key, read against `POST https://api.cursor.com/teams/spend`
//! with the key as the Basic user name: the team's spend this billing cycle (`spendCents`) and its
//! fast premium requests, added up over every page of members (at most five pages of 100).
//!
//! Signing in from Quota Control uses the browser sign-in of the Cursor SDK and CLI: cursor.com's
//! page links a random id and a PKCE challenge to the account the user signs in with (Google,
//! GitHub or email, chosen on that page), and the app asks `api2.cursor.sh/auth/poll` for the tokens
//! until Cursor hands them over. Those are the same session tokens the app keeps, so the card reads
//! them the same way, and a sign-in of the account the app is signed in to shares its card. Being
//! the card's own, they are renewed the way the app renews its session, a week into its 60 days.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{
    HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, MetricLine, PlanTerm,
    Provider, ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::signin::{
    self, Converted, Look, Method, Pending, Poll, Polling, SignIn, SignedIn, StartContext,
};
use crate::support::{apps, http, jwt, lines, oauth, value};

pub(crate) struct Cursor;

const APP: &str = "Cursor";
/// The app's folder in the roaming app data folder.
const FOLDER: &str = "Cursor";
const ACCESS_TOKEN: &str = "cursorAuth/accessToken";
const REFRESH_TOKEN: &str = "cursorAuth/refreshToken";
const CACHED_EMAIL: &str = "cursorAuth/cachedEmail";
const MEMBERSHIP: &str = "cursorAuth/stripeMembershipType";

const DASHBOARD: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService";
const WEB: &str = "https://cursor.com/api";

/// The most requests one refresh sends.
const MAX_REQUESTS: usize = 4;

const TOTAL: &str = "Total usage";
const CURSOR_MODELS: &str = "Cursor Models";
const OTHER_MODELS: &str = "Other Models";
const GROK_BOT: &str = "Grok Bot usage";
const ON_DEMAND: &str = "On-demand";
const REQUESTS: &str = "Requests";
const CREDITS: &str = "Credits";

const EXPIRED: &str = "The Cursor login expired. Open Cursor once to renew it.";
const COOKIE_EXPIRED: &str =
    "The pasted Cursor session expired. Copy WorkosCursorSessionToken from cursor.com again.";
const ADMIN_REFUSED: &str = "Cursor refused the key. Paste an Enterprise team's Admin API key, or the WorkosCursorSessionToken cookie of cursor.com.";
const TEAM_SPEND_URL: &str = "https://api.cursor.com/teams/spend";
const TEAM_SPEND: &str = "Team Spend";
const TEAM_REQUESTS: &str = "Team Premium Requests";
/// Pages of 100 members read from `teams/spend` at most.
const MAX_SPEND_PAGES: u64 = 5;
const NO_SUBSCRIPTION: &str = "No active Cursor subscription.";
const ENTERPRISE_UNAVAILABLE: &str = "Enterprise usage data unavailable. Try again later.";
const TEAM_UNAVAILABLE: &str = "Team request-based usage data unavailable. Try again later.";
const REQUESTS_UNAVAILABLE: &str = "Cursor request-based usage data unavailable. Try again later.";

const PLAN_MEMO: &str = "cursor.plan";

/// The page that signs a browser in for the SDK and CLI, and where the app asks for the tokens.
const LOGIN_PAGE: &str = "https://cursor.com/loginDeepControl";
const POLL: &str = "https://api2.cursor.sh/auth/poll";
/// The endpoint and client the Cursor app renews its session with.
const TOKEN: &str = "https://api2.cursor.sh/oauth/token";
const CLIENT_ID: &str = "KbZUR41cY7W6zRSdpSUJ7I7mLYBKOCmB";
/// A session is renewed, as the app renews it, once fewer than 1272 hours (53 days) of it are left.
const RENEW_BEFORE_HOURS: i64 = 1272;
const POLICY: &str = "Your organization's settings do not allow this sign-in.";
const CREDITS_MEMO: &str = "cursor.credits";
const GROK_MEMO: &str = "cursor.grokBot";

#[async_trait]
impl Service for Cursor {
    fn id(&self) -> &'static str {
        "cursor"
    }

    fn name(&self) -> &'static str {
        APP
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://status.cursor.com/"),
            ProviderLink::new("Dashboard", "https://www.cursor.com/dashboard"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::login(APP).or_api_key(ApiKeyHelp {
            env: &[],
            url: "https://cursor.com/dashboard",
            fields: &[],
        })
    }

    fn key_label(&self) -> &'static str {
        "Admin API key or session cookie (WorkosCursorSessionToken)"
    }

    fn sign_in(&self) -> Option<&'static dyn SignIn> {
        Some(&Cursor)
    }

    /// The signed-in account, known by its JWT subject, else its cached email. A hash of a token is
    /// the last resort only: Cursor rotates its tokens, so it would move the card at every renewal.
    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let path = apps::state_db(roots, FOLDER);
        if !path.is_file() {
            return Vec::new();
        }
        let read = |key: &str| apps::item(roots, FOLDER, key).and_then(state_text);
        let access = read(ACCESS_TOKEN);
        let subject = access.as_deref().and_then(token_subject);
        let refresh = subject.is_none().then(|| read(REFRESH_TOKEN)).flatten();
        if access.is_none() && refresh.is_none() {
            return Vec::new();
        }
        let email = read(CACHED_EMAIL).or_else(|| {
            access
                .as_deref()
                .and_then(|token| jwt::claim(token, &["email"]))
        });
        let identity = subject
            .or_else(|| refresh.as_deref().and_then(token_subject))
            .or_else(|| email.clone())
            .unwrap_or_else(|| {
                sha256_hex(refresh.as_deref().or(access.as_deref()).unwrap_or_default())
            });
        let secret = json!({ "accessToken": access, "membershipType": read(MEMBERSHIP) });
        vec![Login::new(identity, APP, &path, Secret::new(secret)).with_label(email)]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let id = |suffix: &str| format!("{}.{suffix}", provider.id);
        vec![
            WidgetDescriptor::percent(id("usage"), provider, "Total Usage", Some(TOTAL), None)
                .exporting_progress("totalUsage", "percent"),
            WidgetDescriptor::percent(id("auto"), provider, CURSOR_MODELS, None, None)
                .exporting_progress("autoUsage", "percent"),
            WidgetDescriptor::percent(id("api"), provider, OTHER_MODELS, None, None)
                .exporting_progress("apiUsage", "percent"),
            WidgetDescriptor::percent(id("grokBot"), provider, "Grok Bot", Some(GROK_BOT), None)
                .exporting_progress("grokBot", "percent"),
            WidgetDescriptor::bounded_dollars(
                id("onDemand"),
                provider,
                "Extra Usage",
                Some(ON_DEMAND),
                100.0,
                None,
                Some("spent"),
            )
            .exporting_limit(
                "onDemand",
                LimitResourceKind::Consumption,
                "usd",
                LimitResourceSource::ProgressOrValue {
                    kind: MetricKind::Dollars,
                    label: None,
                },
                false,
            ),
            WidgetDescriptor::bounded_count(
                id("requests"),
                provider,
                REQUESTS,
                None,
                500.0,
                "requests",
                Some(lines::MONTH_MS),
            )
            .exporting_progress("requests", "requests"),
            WidgetDescriptor::dollar_balance(id("credits"), provider, CREDITS, None, "left")
                .exporting_limit(
                    "credits",
                    LimitResourceKind::Balance,
                    "usd",
                    LimitResourceSource::Value {
                        kind: MetricKind::Dollars,
                        label: None,
                    },
                    false,
                ),
            WidgetDescriptor::values(
                id("teamSpend"),
                provider,
                TEAM_SPEND,
                None,
                Some(MetricKind::Dollars),
                None,
                true,
                None,
                false,
            )
            .exporting_limit(
                "teamSpend",
                LimitResourceKind::Consumption,
                "usd",
                LimitResourceSource::Value {
                    kind: MetricKind::Dollars,
                    label: None,
                },
                false,
            ),
            WidgetDescriptor::values(
                id("teamRequests"),
                provider,
                TEAM_REQUESTS,
                None,
                Some(MetricKind::Count),
                None,
                true,
                None,
                false,
            )
            .exporting_limit(
                "teamPremiumRequests",
                LimitResourceKind::Consumption,
                "count",
                LimitResourceSource::Value {
                    kind: MetricKind::Count,
                    label: None,
                },
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let pasted_session;
        let pasted = context.secret.key();
        let base = match pasted {
            Some(key) => match cookie_token(key) {
                Some(token) => {
                    pasted_session = Secret::new(json!({ "accessToken": token }));
                    &pasted_session
                }
                None => return team_spend(context, key).await,
            },
            None => context.secret,
        };
        let renewed = renew_if_due(context).await;
        let fresh;
        let secret = match renewed {
            Some(document) => {
                context.keep_renewed(document.clone()).await?;
                fresh = Secret::owned(document);
                &fresh
            }
            None => base,
        };
        let expired = || {
            http::expired(if pasted.is_some() {
                COOKIE_EXPIRED
            } else if secret.is_owned() {
                oauth::SIGN_IN_EXPIRED
            } else {
                EXPIRED
            })
        };
        let token = secret.str("/accessToken").ok_or_else(expired)?;
        if jwt::expires_at(token).is_some_and(|expiry| expiry <= context.now) {
            return Err(expired());
        }
        // The usage call below is the first request.
        let mut budget = Budget(MAX_REQUESTS - 1);
        let usage = current_usage(context, token).await?;
        let plan = plan_name(context, token, &mut budget).await;
        let membership = secret.str("/membershipType").and_then(membership_label);
        let facts = Facts::of(&usage);
        let session = web_session(token);

        if let Some(reason) = fallback_reason(&facts, plan.as_deref()) {
            let summary = usage_summary(context, session.as_ref(), &mut budget).await;
            let requests = request_usage(context, session.as_ref(), &mut budget).await;
            let mut found = summary_lines(summary.as_ref(), requests.as_ref());
            if found.is_empty() {
                return Err(http::not_available(reason));
            }
            found.extend(grok_bot_line(context, token, &mut budget).await);
            found.extend(model_requests(
                requests.as_ref(),
                &Cycle::of_web(summary.as_ref(), requests.as_ref()),
            ));
            let label = plan_label(plan.as_deref())
                .or_else(|| {
                    summary
                        .as_ref()
                        .and_then(|summary| value::text(summary, "/membershipType"))
                        .and_then(membership_label)
                })
                .or(membership);
            let term = plan_term(&usage, label.as_deref());
            return Ok(Reading::new(label, found).with_plan_term(term));
        }

        let label = plan_label(plan.as_deref()).or(membership);
        let term = plan_term(&usage, label.as_deref());
        if facts.counts_requests() {
            let requests = request_usage(context, session.as_ref(), &mut budget).await;
            let mut found =
                request_lines(requests.as_ref(), &Cycle::of_requests(requests.as_ref()));
            if found.is_empty() {
                return Err(http::decoding(APP));
            }
            found.extend(grok_bot_line(context, token, &mut budget).await);
            found.extend(model_requests(
                requests.as_ref(),
                &Cycle::of_requests(requests.as_ref()),
            ));
            return Ok(Reading::new(label, found).with_plan_term(term));
        }

        let mut found = plan_lines(&usage, &facts, plan.as_deref())?;
        found.extend(grok_bot_line(context, token, &mut budget).await);
        found.extend(credits_line(context, token, session.as_ref(), &mut budget).await);
        Ok(Reading::new(label, found).with_plan_term(term))
    }
}

#[async_trait]
impl SignIn for Cursor {
    /// Cursor's page offers both, and the choice is made there.
    fn methods(&self) -> &'static [Method] {
        &[Method::Google, Method::GitHub]
    }

    async fn start(
        &self,
        _method: Method,
        context: &StartContext,
    ) -> Result<Pending, SimpleProviderError> {
        let verifier = signin::random_token(32)?;
        let uuid = signin::random_uuid()?;
        let mut page = url::Url::parse(LOGIN_PAGE).map_err(|_| http::decoding(APP))?;
        page.query_pairs_mut()
            .append_pair("challenge", &signin::challenge(&verifier))
            .append_pair("uuid", &uuid)
            .append_pair("mode", "login")
            .append_pair("redirectTarget", "sdk");
        let requests = context.http.clone();
        let by_get = Arc::new(AtomicBool::new(false));
        let look = move || -> Look {
            let http = requests.clone();
            let (uuid, verifier, by_get) = (uuid.clone(), verifier.clone(), by_get.clone());
            Box::pin(async move { look_login(&http, &uuid, &verifier, &by_get).await })
        };
        Ok(signin::polled(
            page.to_string(),
            None,
            Polling {
                interval: std::time::Duration::from_secs(1),
                slow_down: std::time::Duration::from_secs(2),
                lifetime: signin::FLOW_LIFETIME,
            },
            context.http.clone(),
            look,
            signed_in,
        ))
    }
}

/// One look at a browser sign-in: Cursor answers 404 until the user has signed in. The look is a
/// POST, so the verifier, which can mint a key, stays out of URLs; a server without that route is
/// asked by GET from then on, as the SDK falls back.
async fn look_login(
    http: &uc_core::SharedHttpClient,
    uuid: &str,
    verifier: &str,
    by_get: &AtomicBool,
) -> Poll {
    let request = if by_get.load(Ordering::Relaxed) {
        HttpRequest::get(format!("{POLL}?uuid={uuid}&verifier={verifier}"))
    } else {
        HttpRequest::post(POLL).json_body(&json!({ "uuid": uuid, "verifier": verifier }))
    };
    let Ok(response) = http
        .send(
            request
                .header("Accept", "application/json")
                .timeout(signin::REQUEST_TIMEOUT),
        )
        .await
    else {
        return Poll::Waiting;
    };
    let text = String::from_utf8_lossy(&response.body).to_ascii_lowercase();
    match response.status {
        200 => {
            let body: Value = response.json().unwrap_or(Value::Null);
            if value::text(&body, "/accessToken").is_some()
                && value::text(&body, "/refreshToken").is_some()
            {
                Poll::Approved(body)
            } else {
                Poll::Failed(signin::invalid("Cursor returned no usable sign-in."))
            }
        }
        404 => {
            if text.contains("route not found") {
                by_get.store(true, Ordering::Relaxed);
            }
            Poll::Waiting
        }
        403 if text.contains("sign_in_policy_violation") => Poll::Failed(signin::invalid(POLICY)),
        403 => Poll::Failed(signin::denied()),
        429 => Poll::SlowDown,
        status if status >= 500 => Poll::Waiting,
        status => Poll::Failed(signin::unfinished(status)),
    }
}

/// A browser sign-in's tokens in the shape the card reads the app's login, named by the session's
/// subject as the app's login is.
fn signed_in(http: uc_core::SharedHttpClient, answer: Value) -> Converted {
    Box::pin(async move {
        let access = value::text(&answer, "/accessToken")
            .ok_or_else(|| signin::invalid("Cursor returned no usable sign-in."))?
            .to_string();
        let refresh = value::text(&answer, "/refreshToken")
            .ok_or_else(|| signin::invalid("Cursor returned no usable sign-in."))?
            .to_string();
        let identity = token_subject(&access)
            .ok_or_else(|| signin::invalid("Cursor did not say which account signed in."))?;
        let label = match jwt::claim(&access, &["email"]) {
            Some(email) => Some(email),
            None => account_email(&http, &access).await,
        };
        Ok(SignedIn {
            identity,
            label,
            document: json!({
                "accessToken": access,
                "refreshToken": refresh,
                "membershipType": null
            }),
        })
    })
}

/// The account's email as the web dashboard's `auth/me` reports it; none when that fails.
async fn account_email(http: &uc_core::SharedHttpClient, token: &str) -> Option<String> {
    let session = web_session(token)?;
    let response = http
        .send(
            web("auth/me", &session)
                .header("Accept", "application/json")
                .timeout(signin::REQUEST_TIMEOUT),
        )
        .await
        .ok()?;
    if !response.is_success() {
        return None;
    }
    let body: Value = response.json().ok()?;
    value::text(&body, "/email").map(str::to_string)
}

/// A sign-in made in Quota Control renewed the way the Cursor app renews its own session: once
/// fewer than 53 of its 60 days are left, its refresh token is traded for a new session token, which
/// then serves as both. Until a renewal succeeds the current token is used as it is.
async fn renew_if_due(context: &FetchContext<'_>) -> Option<Value> {
    let secret = context.secret;
    if !secret.is_owned() {
        return None;
    }
    let token = secret.str("/accessToken")?;
    if jwt::expires_at(token)
        .is_some_and(|expiry| expiry > context.now + Duration::hours(RENEW_BEFORE_HOURS))
    {
        return None;
    }
    let refresh = secret.str("/refreshToken").unwrap_or(token);
    let response = http::send(
        context.http,
        HttpRequest::post(TOKEN)
            .json_body(&json!({
                "grant_type": "refresh_token",
                "client_id": CLIENT_ID,
                "refresh_token": refresh
            }))
            .header("Accept", "application/json"),
        APP,
    )
    .await
    .ok()?;
    if !response.is_success() {
        return None;
    }
    let body = http::parse(&response, APP).ok()?;
    if body.get("shouldLogout").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let renewed = value::text(&body, "/access_token")?;
    let mut document = secret.value().clone();
    document["accessToken"] = json!(renewed);
    document["refreshToken"] = json!(renewed);
    Some(document)
}

/// The requests this refresh may still send.
struct Budget(usize);

impl Budget {
    /// Spend `count` requests when that many are left.
    fn take(&mut self, count: usize) -> bool {
        let enough = self.0 >= count;
        if enough {
            self.0 -= count;
        }
        enough
    }
}

/// A Connect RPC of Cursor's dashboard service as the app sends it: JSON, the token as a bearer
/// and an empty request message.
fn connect(method: &str, token: &str) -> HttpRequest {
    HttpRequest::post(format!("{DASHBOARD}/{method}"))
        .bearer(token)
        .header("Connect-Protocol-Version", "1")
        .json_body(&json!({}))
}

/// The session token inside a pasted `WorkosCursorSessionToken` value (`<user>%3A%3A<token>`, with
/// or without the cookie's name), or a pasted token alone; `None` for anything that is not a JWT,
/// which is then taken for an Admin API key.
fn cookie_token(pasted: &str) -> Option<&str> {
    let value = pasted
        .strip_prefix("WorkosCursorSessionToken=")
        .unwrap_or(pasted);
    let token = value
        .rsplit_once("%3A%3A")
        .or_else(|| value.rsplit_once("::"))
        .map_or(value, |(_, token)| token);
    (token.split('.').count() == 3 && jwt::expires_at(token).is_some()).then_some(token)
}

/// An Enterprise team's spend and fast premium requests this billing cycle, over every page of
/// members, read with its Admin API key.
async fn team_spend(context: &FetchContext<'_>, key: &str) -> Result<Reading, SimpleProviderError> {
    use base64::Engine;
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("{key}:"));
    let mut cents = 0.0;
    let mut requests = 0.0;
    let mut members = None;
    let mut page = 1;
    loop {
        let request = HttpRequest::post(TEAM_SPEND_URL)
            .header("Authorization", format!("Basic {basic}"))
            .header("Accept", "application/json")
            .json_body(&json!({ "page": page, "pageSize": 100 }));
        let response = http::send(context.http, request, APP).await?;
        match response.status {
            401 | 403 => return Err(http::invalid(ADMIN_REFUSED)),
            _ if !response.is_success() => return Err(http::status_error(&response, APP)),
            _ => {}
        }
        let body = http::parse(&response, APP)?;
        let spend = body
            .get("teamMemberSpend")
            .and_then(Value::as_array)
            .ok_or_else(|| http::decoding(APP))?;
        for member in spend {
            cents += value::number(member, "/spendCents").unwrap_or(0.0).max(0.0);
            requests += value::number(member, "/fastPremiumRequests")
                .unwrap_or(0.0)
                .max(0.0);
        }
        members = members.or_else(|| value::number(&body, "/totalMembers"));
        let pages = value::number(&body, "/totalPages").unwrap_or(1.0);
        if (page as f64) >= pages || page >= MAX_SPEND_PAGES {
            break;
        }
        page += 1;
    }
    let plan = match members.filter(|members| *members > 0.0) {
        Some(members) => format!("Team · {members:.0} members"),
        None => "Team".to_string(),
    };
    Ok(Reading::new(
        Some(plan),
        vec![
            lines::dollar_value(TEAM_SPEND, cents / 100.0),
            lines::count_value(TEAM_REQUESTS, requests, "requests"),
        ],
    ))
}

/// A `cursor.com` dashboard API call with the web session cookie.
fn web(path: &str, session: &Session) -> HttpRequest {
    HttpRequest::get(format!("{WEB}/{path}")).header("Cookie", &session.cookie)
}

/// The billing cycle's usage, the one request a refresh cannot do without.
async fn current_usage(
    context: &FetchContext<'_>,
    token: &str,
) -> Result<Value, SimpleProviderError> {
    let response = http::send(context.http, connect("GetCurrentPeriodUsage", token), APP).await?;
    if matches!(response.status, 401 | 403) {
        return Err(http::expired(EXPIRED));
    }
    if !response.is_success() {
        return Err(http::status_error(&response, APP));
    }
    let body = http::parse(&response, APP)?;
    if body.is_object() {
        Ok(body)
    } else {
        Err(http::decoding(APP))
    }
}

/// An optional lookup's JSON object, or `None` when it fails in any way, so the card still shows
/// the usage it has. Failures are logged by kind only, never with request details.
async fn optional(context: &FetchContext<'_>, request: HttpRequest, what: &str) -> Option<Value> {
    let Ok(response) = http::send(context.http, request, APP).await else {
        tracing::debug!(target: "cursor", "optional {what} lookup could not connect");
        return None;
    };
    if !response.is_success() {
        tracing::debug!(target: "cursor", "optional {what} lookup answered HTTP {}", response.status);
        return None;
    }
    let body = response.json::<Value>().ok().filter(Value::is_object);
    if body.is_none() {
        tracing::debug!(target: "cursor", "optional {what} lookup returned unreadable data");
    }
    body
}

/// A lookup remembered on this card: its last answer and whether that answer is still fresh.
async fn recall(context: &FetchContext<'_>, key: &'static str) -> Option<(Value, bool)> {
    let entry = context.memo.get(key, context.now).await?;
    let fresh = value::time(&entry, "/freshUntil").is_some_and(|until| until > context.now);
    Some((entry.get("value").cloned().unwrap_or(Value::Null), fresh))
}

/// Remember `answer` as fresh for `fresh_for`, and keep it `keep_for` to stand in when a later
/// refresh cannot ask again.
async fn remember(
    context: &FetchContext<'_>,
    key: &'static str,
    answer: Value,
    fresh_for: Duration,
    keep_for: Duration,
) {
    let entry = json!({
        "value": answer,
        "freshUntil": (context.now + fresh_for).timestamp_millis(),
    });
    context
        .memo
        .put(key, entry, Some(context.now + keep_for))
        .await;
}

/// The plan's name from `GetPlanInfo`, remembered for 12 hours. A failed lookup keeps the last name
/// known and is retried after 30 minutes.
async fn plan_name(context: &FetchContext<'_>, token: &str, budget: &mut Budget) -> Option<String> {
    let remembered = recall(context, PLAN_MEMO).await;
    let known = remembered
        .as_ref()
        .and_then(|(name, _)| name.as_str())
        .map(str::to_string);
    if remembered.as_ref().is_some_and(|(_, fresh)| *fresh) {
        return known;
    }
    let fetched = if budget.take(1) {
        optional(context, connect("GetPlanInfo", token), "plan")
            .await
            .and_then(|body| value::text(&body, "/planInfo/planName").map(str::to_string))
    } else {
        None
    };
    let fresh_for = if fetched.is_some() {
        Duration::hours(12)
    } else {
        Duration::minutes(30)
    };
    let name = fetched.or(known);
    remember(
        context,
        PLAN_MEMO,
        json!(name),
        fresh_for,
        Duration::days(7),
    )
    .await;
    name
}

/// The Grok Bot meter: asked on every refresh with a request left, else the last answer (kept for
/// an hour).
async fn grok_bot_line(
    context: &FetchContext<'_>,
    token: &str,
    budget: &mut Budget,
) -> Option<MetricLine> {
    let fetched = if budget.take(1) {
        optional(context, connect("GetSandUsageStatus", token), "Grok Bot").await
    } else {
        None
    };
    let status = match fetched {
        Some(status) => {
            remember(
                context,
                GROK_MEMO,
                status.clone(),
                Duration::zero(),
                Duration::hours(1),
            )
            .await;
            status
        }
        None => recall(context, GROK_MEMO).await?.0,
    };
    grok_bot(&status)
}

/// The credit balance left, from credit grants and the prepaid Stripe balance: asked again once
/// the last answer is 30 minutes old and this refresh has room for both lookups, else the last
/// balance known.
async fn credits_line(
    context: &FetchContext<'_>,
    token: &str,
    session: Option<&Session>,
    budget: &mut Budget,
) -> Option<MetricLine> {
    let remembered = recall(context, CREDITS_MEMO).await;
    let fresh = remembered.as_ref().is_some_and(|(_, fresh)| *fresh);
    let last = remembered.and_then(|(balance, _)| balance.as_f64());
    let cost = 1 + usize::from(session.is_some());
    let balance = if !fresh && budget.take(cost) {
        let grants = optional(
            context,
            connect("GetCreditGrantsBalance", token),
            "credit grants",
        )
        .await;
        let stripe = match session {
            Some(session) => {
                optional(context, web("auth/stripe", session), "prepaid balance").await
            }
            None => None,
        };
        let balance = credits_left(grants.as_ref(), stripe.as_ref());
        if grants.is_some() && (stripe.is_some() || session.is_none()) {
            remember(
                context,
                CREDITS_MEMO,
                json!(balance),
                Duration::minutes(30),
                Duration::hours(6),
            )
            .await;
            balance
        } else {
            last.or(balance)
        }
    } else {
        last
    };
    balance.map(|dollars| lines::dollar_value(CREDITS, dollars))
}

/// `/api/usage-summary`: the Enterprise and team dashboard's meters.
async fn usage_summary(
    context: &FetchContext<'_>,
    session: Option<&Session>,
    budget: &mut Budget,
) -> Option<Value> {
    let session = session?;
    if !budget.take(1) {
        return None;
    }
    optional(context, web("usage-summary", session), "usage summary").await
}

/// `/api/usage`: the included request allowance of request-based plans.
async fn request_usage(
    context: &FetchContext<'_>,
    session: Option<&Session>,
    budget: &mut Budget,
) -> Option<Value> {
    let session = session?;
    if !budget.take(1) {
        return None;
    }
    let path = format!("usage?user={}", session.user);
    optional(context, web(&path, session), "request usage").await
}

/// The `cursor.com` web session the dashboard derives from the app's token.
struct Session {
    user: String,
    cookie: String,
}

/// `WorkosCursorSessionToken=<user id>%3A%3A<token>`, the user id being the part of the token's
/// subject after `|`. `None` for a token that is not a JWT or names no plain user id, so nothing
/// unexpected ever reaches a header or a URL.
fn web_session(token: &str) -> Option<Session> {
    let subject = token_subject(token)?;
    let mut parts = subject.split('|');
    let first = parts.next().unwrap_or_default();
    let user = parts.next().unwrap_or(first);
    let plain = |text: &str| {
        !text.is_empty()
            && text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    };
    (plain(user) && plain(token)).then(|| Session {
        user: user.to_string(),
        cookie: format!("WorkosCursorSessionToken={user}%3A%3A{token}"),
    })
}

/// What the usage answer says about the plan meter, read once so that the fallback choices agree
/// (upstream `CursorPlanUsageFacts`).
struct Facts {
    /// Only an explicit `enabled: false` turns the account off.
    enabled: bool,
    has_plan_usage: bool,
    limit: Option<f64>,
    total_percent: Option<f64>,
    /// A team account by the shape of its spend limits alone.
    team_by_shape: bool,
}

impl Facts {
    fn of(usage: &Value) -> Self {
        let plan = usage.get("planUsage").filter(|plan| plan.is_object());
        let team_by_shape = usage
            .get("spendLimitUsage")
            .filter(|spend| spend.is_object())
            .is_some_and(|spend| {
                value::text(spend, "/limitType")
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("team"))
                    || value::number(spend, "/pooledLimit").unwrap_or(0.0) > 0.0
            });
        Self {
            enabled: value::flag(usage, "/enabled") != Some(false),
            has_plan_usage: plan.is_some(),
            limit: plan.and_then(|plan| value::number(plan, "/limit")),
            total_percent: plan.and_then(|plan| value::number(plan, "/totalPercentUsed")),
            team_by_shape,
        }
    }

    /// A request-based plan reports a plan meter with neither a limit nor a percentage.
    fn counts_requests(&self) -> bool {
        self.enabled && self.has_plan_usage && self.limit.is_none() && self.total_percent.is_none()
    }
}

/// Why the usage answer cannot stand on its own, so that the web dashboard's Enterprise and team
/// endpoints are read instead (upstream `shouldUseRequestBasedFallback`).
fn fallback_reason(facts: &Facts, plan: Option<&str>) -> Option<&'static str> {
    if !facts.enabled || facts.limit.is_some() {
        return None;
    }
    let plan = plan
        .map(|plan| plan.trim().to_lowercase())
        .filter(|plan| !plan.is_empty());
    match plan.as_deref() {
        Some("enterprise") => Some(ENTERPRISE_UNAVAILABLE),
        Some("team") => Some(TEAM_UNAVAILABLE),
        None if facts.total_percent.is_none() => Some(REQUESTS_UNAVAILABLE),
        _ if facts.team_by_shape && facts.has_plan_usage => Some(REQUESTS_UNAVAILABLE),
        _ => None,
    }
}

/// The plan meters of a usage answer (upstream `mapUsage`, less the credits looked up apart).
fn plan_lines(
    usage: &Value,
    facts: &Facts,
    plan: Option<&str>,
) -> Result<Vec<MetricLine>, SimpleProviderError> {
    let plan_usage = usage
        .get("planUsage")
        .filter(|plan_usage| facts.enabled && plan_usage.is_object())
        .ok_or_else(|| http::not_available(NO_SUBSCRIPTION))?;
    if facts.limit.is_none() && facts.total_percent.is_none() {
        return Err(http::decoding(APP));
    }
    let cycle = Cycle::of_usage(usage);
    let used_cents = value::number(plan_usage, "/totalSpend").unwrap_or_else(|| {
        facts.limit.unwrap_or(0.0) - value::number(plan_usage, "/remaining").unwrap_or(0.0)
    });
    let team =
        facts.team_by_shape || plan.is_some_and(|plan| plan.trim().eq_ignore_ascii_case("team"));
    let mut found = Vec::new();
    if team {
        let limit = facts
            .limit
            .ok_or_else(|| http::not_available(REQUESTS_UNAVAILABLE))?;
        found.push(cycle.dollars(TOTAL, (used_cents, limit)));
    } else {
        let computed = facts
            .limit
            .filter(|limit| *limit > 0.0)
            .map_or(0.0, |limit| used_cents / limit * 100.0);
        found.push(cycle.percent(TOTAL, facts.total_percent.unwrap_or(computed)));
    }
    for (pointer, label) in [
        ("/autoPercentUsed", CURSOR_MODELS),
        ("/apiPercentUsed", OTHER_MODELS),
    ] {
        if let Some(percent) = value::number(plan_usage, pointer) {
            found.push(cycle.percent(label, percent));
        }
    }
    found.extend(
        usage
            .get("spendLimitUsage")
            .filter(|spend| spend.is_object())
            .and_then(on_demand_spend),
    );
    Ok(found)
}

/// On-demand spend from the usage answer's spend limits: a meter when there is a limit, else the
/// amount spent. A zero in one field must not hide spend reported in another.
fn on_demand_spend(spend: &Value) -> Option<MetricLine> {
    let first = |pointers: [&str; 2]| {
        pointers
            .iter()
            .find_map(|pointer| value::number(spend, pointer))
    };
    let limit = first(["/individualLimit", "/pooledLimit"]).unwrap_or(0.0);
    let remaining = first(["/individualRemaining", "/pooledRemaining"]).unwrap_or(0.0);
    let reported: Vec<f64> = ["/individualUsed", "/pooledUsed", "/totalSpend"]
        .iter()
        .filter_map(|pointer| value::number(spend, pointer))
        .collect();
    let spent = reported
        .iter()
        .copied()
        .find(|amount| *amount > 0.0)
        .unwrap_or_else(|| {
            let inferred = (limit - remaining).max(0.0);
            if inferred > 0.0 {
                inferred
            } else {
                reported.first().copied().unwrap_or(0.0)
            }
        });
    if limit > 0.0 {
        Some(lines::dollars(
            ON_DEMAND,
            cents(spent),
            cents(limit),
            None,
            None,
        ))
    } else {
        (spent > 0.0).then(|| lines::dollar_value(ON_DEMAND, cents(spent)))
    }
}

/// The Enterprise and team fallback: the request allowance from `/api/usage` and the meters of
/// `/api/usage-summary`, neither enough alone (upstream `CursorUsageSummaryMapper`).
fn summary_lines(summary: Option<&Value>, requests: Option<&Value>) -> Vec<MetricLine> {
    let cycle = Cycle::of_web(summary, requests);
    let mut found = request_lines(requests, &cycle);
    let Some(summary) = summary else {
        return found;
    };
    if found.is_empty() {
        found.extend(summary_total(summary, &cycle));
    }
    for (pointer, label) in [
        ("/individualUsage/plan/autoPercentUsed", CURSOR_MODELS),
        ("/individualUsage/plan/apiPercentUsed", OTHER_MODELS),
    ] {
        if let Some(percent) = value::number(summary, pointer) {
            found.push(cycle.percent(label, percent));
        }
    }
    found.extend(
        on_demand_bucket(summary.pointer("/individualUsage/onDemand"), &cycle)
            .or_else(|| on_demand_bucket(summary.pointer("/teamUsage/onDemand"), &cycle)),
    );
    found
}

/// The included request allowance, as the Total usage meter (the default widget) and its
/// Requests copy.
fn model_requests(requests: Option<&Value>, cycle: &Cycle) -> Vec<MetricLine> {
    requests
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(name, bucket)| {
            let used = value::number(bucket, "/numRequests")
                .or_else(|| value::number(bucket, "/numRequestsTotal"))?;
            if used < 0.0 {
                return None;
            }
            Some(
                match value::number(bucket, "/maxRequestUsage").filter(|limit| *limit > 0.0) {
                    Some(limit) => lines::count(
                        name,
                        used,
                        limit,
                        "requests",
                        cycle.resets_at,
                        Some(cycle.period_ms),
                    ),
                    None => lines::count_value(name, used, "requests"),
                },
            )
        })
        .collect()
}

fn request_lines(requests: Option<&Value>, cycle: &Cycle) -> Vec<MetricLine> {
    let Some(allowance) = requests.and_then(|requests| requests.get("gpt-4")) else {
        return Vec::new();
    };
    let Some(limit) = value::number(allowance, "/maxRequestUsage").filter(|limit| *limit > 0.0)
    else {
        return Vec::new();
    };
    let used = value::number(allowance, "/numRequests")
        .or_else(|| value::number(allowance, "/numRequestsTotal"))
        .unwrap_or(0.0)
        .max(0.0);
    [TOTAL, REQUESTS]
        .into_iter()
        .map(|label| {
            lines::count(
                label,
                used,
                limit,
                "requests",
                cycle.resets_at,
                Some(cycle.period_ms),
            )
        })
        .collect()
}

/// The summary's headline meter: the team pool on a team limit, else the plan percentage, the
/// member's own cap or the team pool.
fn summary_total(summary: &Value, cycle: &Cycle) -> Option<MetricLine> {
    let pooled = dollar_meter(summary.pointer("/teamUsage/pooled"));
    let team_limit =
        value::text(summary, "/limitType").is_some_and(|kind| kind.eq_ignore_ascii_case("team"));
    if team_limit && let Some(meter) = pooled {
        return Some(cycle.dollars(TOTAL, meter));
    }
    if let Some(percent) = value::number(summary, "/individualUsage/plan/totalPercentUsed") {
        return Some(cycle.percent(TOTAL, percent));
    }
    dollar_meter(summary.pointer("/individualUsage/overall"))
        .or(pooled)
        .map(|meter| cycle.dollars(TOTAL, meter))
}

/// An enabled on-demand bucket of the summary: a meter with a limit, else the amount spent.
fn on_demand_bucket(bucket: Option<&Value>, cycle: &Cycle) -> Option<MetricLine> {
    let bucket = bucket.filter(|bucket| enabled_bucket(bucket))?;
    if let Some(meter) = dollar_meter(Some(bucket)) {
        return Some(cycle.dollars(ON_DEMAND, meter));
    }
    let used = value::number(bucket, "/used").filter(|used| *used > 0.0)?;
    Some(lines::dollar_value(ON_DEMAND, cents(used)))
}

/// `(used, limit)` cents of an enabled bucket with a limit; the amount used falls back to the limit
/// less what remains.
fn dollar_meter(bucket: Option<&Value>) -> Option<(f64, f64)> {
    let bucket = bucket.filter(|bucket| enabled_bucket(bucket))?;
    let limit = value::number(bucket, "/limit").filter(|limit| *limit > 0.0)?;
    let inferred = (limit - value::number(bucket, "/remaining").unwrap_or(limit)).max(0.0);
    let used = value::number(bucket, "/used")
        .filter(|used| *used > 0.0)
        .unwrap_or(inferred);
    Some((used.max(0.0), limit))
}

fn enabled_bucket(bucket: &Value) -> bool {
    bucket.is_object() && value::flag(bucket, "/enabled") != Some(false)
}

/// Grok Bot's own weekly allowance (Cursor's "Sand" product). Pooled Enterprise allowances and
/// accounts without an included limit have no personal meter.
fn grok_bot(status: &Value) -> Option<MetricLine> {
    if value::flag(status, "/usesPooledEnterpriseAllowance") == Some(true)
        || value::flag(status, "/hasNonZeroIncludedLimit") == Some(false)
        || value::flag(status, "/includedLimitZero") == Some(true)
    {
        return None;
    }
    let percent = value::number(status, "/usagePercent").filter(|percent| *percent >= 0.0)?;
    let reset = value::time(status, "/nextResetTimestampUtc");
    let period = match (value::time(status, "/currentPeriodStart"), reset) {
        (Some(start), Some(reset)) if reset > start => (reset - start).num_milliseconds(),
        _ => lines::WEEK_MS,
    };
    Some(lines::percent(GROK_BOT, percent, reset, Some(period)))
}

/// Dollars left from credit grants and the prepaid balance (a negative Stripe customer balance),
/// or `None` when the account has neither.
fn credits_left(grants: Option<&Value>, stripe: Option<&Value>) -> Option<f64> {
    let (granted, used) = grants
        .filter(|grants| value::flag(grants, "/hasCreditGrants") == Some(true))
        .map(|grants| {
            (
                value::number(grants, "/totalCents").unwrap_or(0.0),
                value::number(grants, "/usedCents").unwrap_or(0.0),
            )
        })
        .filter(|(granted, used)| *granted > 0.0 && *used >= 0.0)
        .unwrap_or((0.0, 0.0));
    let prepaid = stripe
        .and_then(|stripe| value::number(stripe, "/customerBalance"))
        .filter(|balance| *balance < 0.0)
        .map_or(0.0, f64::abs);
    let total = granted + prepaid;
    (total > 0.0).then(|| cents((total - used).max(0.0)))
}

/// A billing cycle: when it resets and how long it runs.
struct Cycle {
    resets_at: Option<DateTime<Utc>>,
    period_ms: i64,
}

impl Cycle {
    /// From the usage answer's `billingCycleStart` and `billingCycleEnd` (epoch milliseconds).
    fn of_usage(usage: &Value) -> Self {
        let start = value::time(usage, "/billingCycleStart");
        let end = value::time(usage, "/billingCycleEnd");
        match (start, end) {
            (Some(start), Some(end)) if end > start => Self {
                resets_at: Some(end),
                period_ms: (end - start).num_milliseconds(),
            },
            _ => Self {
                resets_at: end,
                period_ms: lines::MONTH_MS,
            },
        }
    }

    /// From the usage summary's bounds, else the request allowance's month.
    fn of_web(summary: Option<&Value>, requests: Option<&Value>) -> Self {
        let bound = |pointer: &str| summary.and_then(|summary| value::time(summary, pointer));
        if let (Some(start), Some(end)) = (bound("/billingCycleStart"), bound("/billingCycleEnd"))
            && end > start
        {
            return Self {
                resets_at: Some(end),
                period_ms: (end - start).num_milliseconds(),
            };
        }
        Self::of_requests(requests)
    }

    /// A 30-day month from the request allowance's `startOfMonth`. A start too close to the end of
    /// representable time has no reset rather than overflowing.
    fn of_requests(requests: Option<&Value>) -> Self {
        let start = requests.and_then(|requests| value::time(requests, "/startOfMonth"));
        Self {
            resets_at: start.and_then(|start| {
                start.checked_add_signed(Duration::milliseconds(lines::MONTH_MS))
            }),
            period_ms: lines::MONTH_MS,
        }
    }

    fn percent(&self, label: &str, used: f64) -> MetricLine {
        lines::percent(label, used, self.resets_at, Some(self.period_ms))
    }

    fn dollars(&self, label: &str, (used, limit): (f64, f64)) -> MetricLine {
        lines::dollars(
            label,
            cents(used),
            cents(limit),
            self.resets_at,
            Some(self.period_ms),
        )
    }
}

/// Integer cents as dollars, snapped to whole cents first.
fn cents(amount: f64) -> f64 {
    amount.round() / 100.0
}

/// A plan name as `GetPlanInfo` gives it, each word capitalized: `pro plan` → `Pro Plan`.
/// The paid plan's current billing period ends when the usage answer's `billingCycleEnd` says; a
/// free plan has no subscription to renew, so it shows no term.
fn plan_term(usage: &Value, label: Option<&str>) -> Option<PlanTerm> {
    if label.is_some_and(|label| label.eq_ignore_ascii_case("free")) {
        return None;
    }
    value::time(usage, "/billingCycleEnd").map(|ends_at| PlanTerm::Stated {
        ends_at,
        checked_at: None,
    })
}

fn plan_label(name: Option<&str>) -> Option<String> {
    let words: Vec<String> = name?
        .split_whitespace()
        .map(|word| {
            let mut characters = word.chars();
            characters
                .next()
                .map(|first| first.to_uppercase().chain(characters).collect())
                .unwrap_or_default()
        })
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// A membership id (`pro_plus`) as the plan's name (`Pro+`).
fn membership_label(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pro_plus" => Some("Pro+".to_string()),
        "free_trial" => Some("Pro Trial".to_string()),
        other => lines::plan_name(other),
    }
}

/// A state value as text. SQLite hands UTF-16 text back with a NUL after every ASCII character,
/// and some values are stored as JSON strings.
fn state_text(raw: String) -> Option<String> {
    let raw = if raw.contains('\0') {
        utf16_text(raw.as_bytes()).unwrap_or(raw)
    } else {
        raw
    };
    let text = raw.trim_matches(|character: char| character == '\0' || character.is_whitespace());
    let text = if text.starts_with('"') {
        serde_json::from_str::<String>(text).unwrap_or_else(|_| text.to_string())
    } else {
        text.to_string()
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn utf16_text(bytes: &[u8]) -> Option<String> {
    let (pairs, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    let units: Vec<u16> = pairs.iter().copied().map(u16::from_le_bytes).collect();
    String::from_utf16(&units).ok()
}

/// The JWT subject of a token: the Cursor account (`google-oauth2|user_…`).
fn token_subject(token: &str) -> Option<String> {
    jwt::claim(token, &["sub"])
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header, owned_context_at};
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use chrono::TimeZone;
    use uc_core::ErrorCategory;

    const SUBJECT: &str = "google-oauth2|user_01TESTUSER";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn at(year: i32, month: u32, day: u32, hour: u32) -> Option<DateTime<Utc>> {
        Some(Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap())
    }

    fn token(subject: &str, expires: DateTime<Utc>) -> String {
        let encode = |part: Value| URL_SAFE_NO_PAD.encode(part.to_string());
        format!(
            "{}.{}.c2lnbmF0dXJl",
            encode(json!({"alg": "HS256", "typ": "JWT"})),
            encode(json!({
                "sub": subject,
                "exp": expires.timestamp(),
                "iss": "https://authentication.cursor.sh",
                "type": "session"
            }))
        )
    }

    fn live_token() -> String {
        token(SUBJECT, now() + Duration::days(40))
    }

    fn secret(token: &str) -> Value {
        json!({"accessToken": token, "membershipType": "pro"})
    }

    fn rpc(method: &str) -> String {
        format!("{DASHBOARD}/{method}")
    }

    const MONTH: i64 = 30 * lines::DAY_MS;

    const USAGE: &str = r#"{
        "billingCycleStart": "1788220800000",
        "billingCycleEnd": "1790812800000",
        "planUsage": {"totalSpend": 1074, "includedSpend": 1074, "remaining": 926, "limit": 2000,
            "autoSpend": 250, "apiSpend": 824, "autoPercentUsed": 12.5, "apiPercentUsed": 41.2,
            "totalPercentUsed": 53.7},
        "spendLimitUsage": {"totalSpend": 425, "individualLimit": 2000, "individualUsed": 425,
            "individualRemaining": 1575, "limitType": "user"},
        "enabled": true,
        "displayMessage": "You've used 54% of your included usage",
        "autoBucketModels": ["auto", "composer-1"]
    }"#;
    const PLAN: &str = r#"{"planInfo":{"planName":"Pro","includedAmountCents":2000,"price":"$20/mo","billingCycleEnd":"1790812800000","planOwner":"PLAN_OWNER_STRIPE"}}"#;
    const GROK: &str = r#"{"currentPeriodStart":"2026-09-24T00:00:00Z","nextResetTimestampUtc":"2026-10-01T00:00:00Z","usagePercent":37.5,"hasAvailableUsage":true,"hasNonZeroIncludedLimit":true}"#;
    const GRANTS: &str = r#"{"hasCreditGrants":true,"creditBalanceCents":"735271","totalCents":"1000000","usedCents":"264729"}"#;
    const STRIPE: &str = r#"{"customerBalance":"-50000"}"#;

    fn pro_account() -> Scripted {
        Scripted::new()
            .on("POST", &rpc("GetCurrentPeriodUsage"), 200, USAGE)
            .on("POST", &rpc("GetPlanInfo"), 200, PLAN)
            .on("POST", &rpc("GetSandUsageStatus"), 200, GROK)
            .on("POST", &rpc("GetCreditGrantsBalance"), 200, GRANTS)
            .on("GET", &format!("{WEB}/auth/stripe"), 200, STRIPE)
    }

    #[tokio::test]
    async fn a_pasted_session_cookie_is_read_as_the_apps_login() {
        let jwt = token("auth0|user_123", now() + Duration::days(30));
        let cookie = format!("user_123%3A%3A{jwt}");
        let http = pro_account();
        let scope = context_at(&http, json!({ "apiKey": cookie }), now());
        let reading = Cursor.fetch(&scope.context()).await.unwrap();
        assert!(!reading.lines.is_empty());
        let requests = http.requests();
        assert_eq!(requests[0].url, rpc("GetCurrentPeriodUsage"));
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some(format!("Bearer {jwt}").as_str())
        );
    }

    #[tokio::test]
    async fn an_expired_pasted_cookie_asks_for_a_fresh_one() {
        let jwt = token("auth0|user_123", now() - Duration::days(1));
        let scope = context_at(
            &Scripted::new(),
            json!({ "apiKey": format!("WorkosCursorSessionToken=user_123::{jwt}") }),
            now(),
        );
        let error = Cursor.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, COOKIE_EXPIRED);
    }

    #[tokio::test]
    async fn an_admin_key_adds_up_the_teams_spend_over_every_page() {
        let page = |members: &str, page: u32| {
            format!(
                r#"{{"teamMemberSpend":[{members}],"subscriptionCycleStart":1788220800000,"totalMembers":3,"totalPages":2,"page":{page}}}"#
            )
        };
        let http = Scripted::new()
            .on(
                "POST",
                TEAM_SPEND_URL,
                200,
                &page(r#"{"spendCents":2450.5,"fastPremiumRequests":1250},{"spendCents":1875.5,"fastPremiumRequests":980}"#, 1),
            );
        let scope = context_at(&http, json!({ "apiKey": "key_admin" }), now());
        let reading = Cursor.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Team · 3 members"));
        assert_eq!(
            reading.lines,
            vec![
                lines::dollar_value(TEAM_SPEND, 86.52),
                lines::count_value(TEAM_REQUESTS, 4460.0, "requests"),
            ]
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some(
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode("key_admin:")
                )
                .as_str()
            )
        );
        let second: Value = serde_json::from_slice(requests[1].body.as_deref().unwrap()).unwrap();
        assert_eq!(second["page"], 2);
    }

    #[tokio::test]
    async fn a_refused_admin_key_says_which_keys_work() {
        let http = Scripted::new().on("POST", TEAM_SPEND_URL, 401, "{}");
        let scope = context_at(&http, json!({ "apiKey": "key_bad" }), now());
        let error = Cursor.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, ADMIN_REFUSED);
    }

    fn start_context(http: &Scripted) -> StartContext {
        StartContext {
            http: http.shared(),
            language: uc_core::loopback::LoginLanguage::English,
            product: APP,
        }
    }

    fn query(url: &str) -> std::collections::HashMap<String, String> {
        url::Url::parse(url)
            .unwrap()
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn a_browser_sign_in_polls_until_cursor_hands_over_the_session() {
        let session = live_token();
        let http = Scripted::new()
            .on("POST", POLL, 404, "Not found")
            .on(
                "POST",
                POLL,
                200,
                &json!({"accessToken": session, "refreshToken": "refresh-1", "authId": SUBJECT})
                    .to_string(),
            )
            .on(
                "GET",
                &format!("{WEB}/auth/me"),
                200,
                r#"{"email":"me@example.com","sub":"google-oauth2|user_01TESTUSER"}"#,
            );
        let pending = Cursor
            .start(Method::GitHub, &start_context(&http))
            .await
            .unwrap();
        let page = query(&pending.url);
        assert!(pending.url.starts_with(LOGIN_PAGE));
        assert_eq!(page["mode"], "login");
        assert_eq!(page["redirectTarget"], "sdk");
        let account = pending.finish.await.result.unwrap();
        assert_eq!(account.identity, SUBJECT);
        assert_eq!(account.label.as_deref(), Some("me@example.com"));
        assert_eq!(
            account.document,
            json!({"accessToken": session, "refreshToken": "refresh-1", "membershipType": null})
        );
        let requests = http.requests();
        let poll: Value = serde_json::from_slice(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(poll["uuid"], page["uuid"].as_str());
        let verifier = poll["verifier"].as_str().unwrap();
        assert_eq!(signin::challenge(verifier), page["challenge"]);
        assert!(!requests[0].url.contains(verifier));
        assert!(
            header(&requests[2], "Cookie").is_some_and(
                |cookie| cookie.starts_with("WorkosCursorSessionToken=user_01TESTUSER")
            )
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_without_the_post_route_is_asked_by_get() {
        let http = Scripted::new()
            .on("POST", POLL, 404, r#"{"error":"Route not found"}"#)
            .on("GET", POLL, 403, r#"{"error":"sign_in_policy_violation"}"#);
        let pending = Cursor
            .start(Method::Google, &start_context(&http))
            .await
            .unwrap();
        let error = pending.finish.await.result.unwrap_err();
        assert_eq!(error.message, POLICY);
        let requests = http.requests();
        assert_eq!(requests[1].method, "GET");
        assert!(requests[1].url.contains("verifier="));
    }

    #[tokio::test]
    async fn a_sign_in_made_here_is_renewed_a_week_into_its_session() {
        let old = token(SUBJECT, now() + Duration::days(50));
        let new = token(SUBJECT, now() + Duration::days(60));
        let http = pro_account().on(
            "POST",
            TOKEN,
            200,
            &json!({"access_token": new, "id_token": "id", "shouldLogout": false}).to_string(),
        );
        let scope = owned_context_at(
            &http,
            json!({"accessToken": old, "refreshToken": "refresh-1", "membershipType": "pro"}),
            now(),
        );
        Cursor.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        let renewal: Value = serde_json::from_slice(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(
            renewal,
            json!({"grant_type": "refresh_token", "client_id": CLIENT_ID, "refresh_token": "refresh-1"})
        );
        let bearer = format!("Bearer {new}");
        assert_eq!(header(&requests[1], "Authorization"), Some(bearer.as_str()));
        assert_eq!(
            scope.renewed().await.unwrap(),
            json!({"accessToken": new, "refreshToken": new, "membershipType": "pro"})
        );
    }

    #[tokio::test]
    async fn a_refused_renewal_keeps_the_current_session_until_it_expires() {
        let old = token(SUBJECT, now() + Duration::days(50));
        let http = pro_account().on("POST", TOKEN, 200, r#"{"shouldLogout":true}"#);
        let scope = owned_context_at(&http, secret(&old), now());
        Cursor.fetch(&scope.context()).await.unwrap();
        let bearer = format!("Bearer {old}");
        assert_eq!(
            header(&http.requests()[1], "Authorization"),
            Some(bearer.as_str())
        );
        assert!(scope.renewed().await.is_none());
        let expired = owned_context_at(
            &Scripted::new(),
            secret(&token(SUBJECT, now() - Duration::minutes(1))),
            now(),
        );
        let error = Cursor.fetch(&expired.context()).await.unwrap_err();
        assert_eq!(error.message, oauth::SIGN_IN_EXPIRED);
    }

    #[tokio::test]
    async fn the_apps_own_login_is_never_renewed() {
        let http = pro_account();
        let scope = context_at(
            &http,
            secret(&token(SUBJECT, now() + Duration::days(50))),
            now(),
        );
        Cursor.fetch(&scope.context()).await.unwrap();
        assert!(http.requests().iter().all(|request| request.url != TOKEN));
    }

    fn urls(requests: &[HttpRequest]) -> Vec<String> {
        requests.iter().map(|request| request.url.clone()).collect()
    }

    fn pro_meters() -> Vec<MetricLine> {
        let cycle_end = at(2026, 10, 1, 0);
        vec![
            lines::percent(TOTAL, 53.7, cycle_end, Some(MONTH)),
            lines::percent(CURSOR_MODELS, 12.5, cycle_end, Some(MONTH)),
            lines::percent(OTHER_MODELS, 41.2, cycle_end, Some(MONTH)),
            lines::dollars(ON_DEMAND, 4.25, 20.0, None, None),
            lines::percent(GROK_BOT, 37.5, cycle_end, Some(lines::WEEK_MS)),
        ]
    }

    /// The paid plan's billing period, from the usage answer's `billingCycleEnd` (1 October 2026).
    fn cycle_term() -> Option<PlanTerm> {
        at(2026, 10, 1, 0).map(|ends_at| PlanTerm::Stated {
            ends_at,
            checked_at: None,
        })
    }

    #[test]
    fn a_free_plan_has_no_term_and_a_paid_one_ends_with_its_billing_cycle() {
        let usage = json!({"billingCycleEnd": "1790812800000"});
        assert_eq!(plan_term(&usage, Some("Free")), None);
        assert_eq!(plan_term(&usage, Some("Pro")), cycle_term());
        assert_eq!(plan_term(&json!({}), Some("Pro")), None);
    }

    #[tokio::test]
    async fn reads_the_billing_cycle_meters_grok_bot_and_the_plan() {
        let http = pro_account();
        let scope = context_at(&http, secret(&live_token()), now());
        let reading = Cursor.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(Some("Pro".into()), pro_meters()).with_plan_term(cycle_term())
        );
        assert_eq!(
            urls(&http.requests()),
            [
                rpc("GetCurrentPeriodUsage"),
                rpc("GetPlanInfo"),
                rpc("GetSandUsageStatus")
            ]
        );
    }

    #[tokio::test]
    async fn the_credit_balance_follows_once_a_refresh_has_room_and_is_then_remembered() {
        let http = pro_account();
        let scope = context_at(&http, secret(&live_token()), now());
        Cursor.fetch(&scope.context()).await.unwrap();
        let second = Cursor.fetch(&scope.context()).await.unwrap();
        let mut expected = pro_meters();
        expected.push(lines::dollar_value(CREDITS, 7852.71));
        assert_eq!(
            second,
            Reading::new(Some("Pro".into()), expected.clone()).with_plan_term(cycle_term())
        );
        let third = Cursor.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            third,
            Reading::new(Some("Pro".into()), expected).with_plan_term(cycle_term())
        );
        assert_eq!(
            urls(&http.requests())[3..],
            [
                rpc("GetCurrentPeriodUsage"),
                rpc("GetSandUsageStatus"),
                rpc("GetCreditGrantsBalance"),
                format!("{WEB}/auth/stripe"),
                rpc("GetCurrentPeriodUsage"),
                rpc("GetSandUsageStatus"),
            ]
        );
    }

    #[tokio::test]
    async fn requests_carry_the_bearer_token_and_the_dashboard_session_cookie() {
        let token = live_token();
        let http = pro_account();
        let scope = context_at(&http, secret(&token), now());
        Cursor.fetch(&scope.context()).await.unwrap();
        Cursor.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        let usage = &requests[0];
        assert_eq!(usage.method, "POST");
        assert_eq!(usage.url, rpc("GetCurrentPeriodUsage"));
        assert_eq!(
            header(usage, "Authorization"),
            Some(format!("Bearer {token}").as_str())
        );
        assert_eq!(header(usage, "Connect-Protocol-Version"), Some("1"));
        assert_eq!(header(usage, "Content-Type"), Some("application/json"));
        assert_eq!(usage.body.as_deref(), Some(b"{}".as_slice()));
        assert_eq!(header(usage, "Cookie"), None);
        let stripe = requests
            .iter()
            .find(|request| request.url.ends_with("/auth/stripe"))
            .unwrap();
        assert_eq!(stripe.method, "GET");
        assert_eq!(
            header(stripe, "Cookie"),
            Some(format!("WorkosCursorSessionToken=user_01TESTUSER%3A%3A{token}").as_str())
        );
        assert_eq!(header(stripe, "Authorization"), None);
    }

    #[tokio::test]
    async fn a_rejected_token_asks_to_open_cursor() {
        for status in [401, 403] {
            let http = Scripted::new().on(
                "POST",
                &rpc("GetCurrentPeriodUsage"),
                status,
                r#"{"code":"unauthenticated","message":"Not logged in"}"#,
            );
            let scope = context_at(&http, secret(&live_token()), now());
            let error = Cursor.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthExpired);
            assert_eq!(error.message, EXPIRED);
            assert_eq!(http.requests().len(), 1, "HTTP {status} is not retried");
        }
    }

    #[tokio::test]
    async fn an_expired_or_missing_token_is_reported_without_any_request() {
        let http = pro_account();
        let expired = token(SUBJECT, now() - Duration::minutes(1));
        for saved in [secret(&expired), json!({"membershipType": "pro"})] {
            let scope = context_at(&http, saved, now());
            let error = Cursor.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthExpired);
            assert_eq!(error.message, EXPIRED);
        }
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn rate_limits_and_server_failures_are_reported() {
        for (status, category, message) in [
            (
                429,
                ErrorCategory::RateLimited,
                "Cursor is rate limiting usage requests. Waiting before retrying.",
            ),
            (
                503,
                ErrorCategory::Http5xx,
                "Cursor answered with HTTP 503.",
            ),
        ] {
            let http = Scripted::new().on(
                "POST",
                &rpc("GetCurrentPeriodUsage"),
                status,
                r#"{"code":"resource_exhausted"}"#,
            );
            let scope = context_at(&http, secret(&live_token()), now());
            let error = Cursor.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category, "HTTP {status}");
            assert_eq!(error.message, message);
            assert_eq!(http.requests().len(), 1, "no lookups after HTTP {status}");
        }
    }

    #[tokio::test]
    async fn an_unreadable_usage_answer_is_a_decoding_error() {
        let http = Scripted::new().on("POST", &rpc("GetCurrentPeriodUsage"), 200, "[1,2]");
        let scope = context_at(&http, secret(&live_token()), now());
        let error = Cursor.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);
    }

    #[tokio::test]
    async fn a_team_account_reads_its_total_in_dollars() {
        let http = Scripted::new()
            .on(
                "POST",
                &rpc("GetCurrentPeriodUsage"),
                200,
                r#"{"billingCycleStart":"1788220800000","billingCycleEnd":"1790812800000",
                    "planUsage":{"totalSpend":10000,"limit":40000,"remaining":30000,"bonusSpend":2500},
                    "spendLimitUsage":{"pooledLimit":100000,"pooledUsed":2500,"pooledRemaining":97500,"limitType":"team"},
                    "enabled":true}"#,
            )
            .on(
                "POST",
                &rpc("GetPlanInfo"),
                200,
                r#"{"planInfo":{"planName":"team"}}"#,
            );
        let scope = context_at(&http, secret(&live_token()), now());
        let reading = Cursor.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Team".into()),
                vec![
                    lines::dollars(TOTAL, 100.0, 400.0, at(2026, 10, 1, 0), Some(MONTH)),
                    lines::dollars(ON_DEMAND, 25.0, 1000.0, None, None),
                ]
            )
            .with_plan_term(cycle_term())
        );
    }

    const ENTERPRISE_SUMMARY: &str = r#"{
        "billingCycleStart": "2026-09-01T00:00:00.000Z",
        "billingCycleEnd": "2026-10-01T00:00:00.000Z",
        "membershipType": "enterprise",
        "limitType": "team",
        "isUnlimited": false,
        "individualUsage": {
            "plan": {"enabled": true, "used": 0, "limit": 0, "autoPercentUsed": 0.36, "apiPercentUsed": 12},
            "onDemand": {"enabled": false}
        },
        "teamUsage": {"onDemand": {"enabled": true, "used": 12345, "limit": 50000, "remaining": 37655}}
    }"#;
    const REQUEST_USAGE: &str = r#"{"gpt-4":{"numRequests":39,"numRequestsTotal":41,"numTokens":0,"maxRequestUsage":500,"maxTokenUsage":null},"startOfMonth":"2026-09-01T00:00:00.000Z"}"#;

    #[tokio::test]
    async fn an_enterprise_account_combines_the_request_allowance_with_the_usage_summary() {
        let user = format!("{WEB}/usage?user=user_01TESTUSER");
        let http = Scripted::new()
            .on(
                "POST",
                &rpc("GetCurrentPeriodUsage"),
                200,
                r#"{"billingCycleStart":"1788220800000","billingCycleEnd":"1790812800000","planUsage":{},"enabled":true}"#,
            )
            .on(
                "POST",
                &rpc("GetPlanInfo"),
                200,
                r#"{"planInfo":{"planName":"Enterprise"}}"#,
            )
            .on("GET", &format!("{WEB}/usage-summary"), 200, ENTERPRISE_SUMMARY)
            .on("GET", &user, 200, REQUEST_USAGE)
            .on("POST", &rpc("GetSandUsageStatus"), 200, GROK);
        let scope = context_at(&http, secret(&live_token()), now());
        let cycle_end = at(2026, 10, 1, 0);
        let mut expected = vec![
            lines::count(TOTAL, 39.0, 500.0, "requests", cycle_end, Some(MONTH)),
            lines::count(REQUESTS, 39.0, 500.0, "requests", cycle_end, Some(MONTH)),
            lines::percent(CURSOR_MODELS, 0.36, cycle_end, Some(MONTH)),
            lines::percent(OTHER_MODELS, 12.0, cycle_end, Some(MONTH)),
            lines::dollars(ON_DEMAND, 123.45, 500.0, cycle_end, Some(MONTH)),
        ];
        let first = Cursor.fetch(&scope.context()).await.unwrap();
        expected.push(lines::count(
            "gpt-4",
            39.0,
            500.0,
            "requests",
            cycle_end,
            Some(MONTH),
        ));
        assert_eq!(
            first,
            Reading::new(Some("Enterprise".into()), expected.clone()).with_plan_term(cycle_term())
        );
        assert_eq!(
            urls(&http.requests()),
            [
                rpc("GetCurrentPeriodUsage"),
                rpc("GetPlanInfo"),
                format!("{WEB}/usage-summary"),
                user
            ]
        );
        let second = Cursor.fetch(&scope.context()).await.unwrap();
        expected.insert(
            expected.len() - 1,
            lines::percent(GROK_BOT, 37.5, cycle_end, Some(lines::WEEK_MS)),
        );
        assert_eq!(
            second,
            Reading::new(Some("Enterprise".into()), expected).with_plan_term(cycle_term())
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 8);
        assert!(
            requests[2..4]
                .iter()
                .all(|request| header(request, "Cookie")
                    .is_some_and(|cookie| cookie
                        .starts_with("WorkosCursorSessionToken=user_01TESTUSER%3A%3A")))
        );
    }

    #[tokio::test]
    async fn an_enterprise_account_without_dashboard_data_is_unavailable() {
        let http = Scripted::new()
            .on(
                "POST",
                &rpc("GetCurrentPeriodUsage"),
                200,
                r#"{"planUsage":{},"enabled":true}"#,
            )
            .on(
                "POST",
                &rpc("GetPlanInfo"),
                200,
                r#"{"planInfo":{"planName":"Enterprise"}}"#,
            )
            .on("GET", &format!("{WEB}/usage-summary"), 500, "{}");
        let scope = context_at(&http, secret(&live_token()), now());
        let error = Cursor.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(error.message, ENTERPRISE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn a_request_based_plan_reads_its_request_allowance() {
        let http = Scripted::new()
            .on(
                "POST",
                &rpc("GetCurrentPeriodUsage"),
                200,
                r#"{"planUsage":{},"enabled":true}"#,
            )
            .on("POST", &rpc("GetPlanInfo"), 200, PLAN)
            .on(
                "GET",
                &format!("{WEB}/usage?user=user_01TESTUSER"),
                200,
                r#"{"gpt-4":{"numRequests":120,"maxRequestUsage":500},"startOfMonth":"2026-09-10T08:00:00.000Z"}"#,
            )
            .on("POST", &rpc("GetSandUsageStatus"), 200, GROK);
        let scope = context_at(&http, secret(&live_token()), now());
        let reading = Cursor.fetch(&scope.context()).await.unwrap();
        let reset = at(2026, 10, 10, 8);
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::count(TOTAL, 120.0, 500.0, "requests", reset, Some(MONTH)),
                    lines::count(REQUESTS, 120.0, 500.0, "requests", reset, Some(MONTH)),
                    lines::percent(GROK_BOT, 37.5, at(2026, 10, 1, 0), Some(lines::WEEK_MS)),
                    lines::count("gpt-4", 120.0, 500.0, "requests", reset, Some(MONTH)),
                ]
            )
        );
        assert_eq!(http.requests().len(), 4);
    }

    #[tokio::test]
    async fn unreadable_plan_metadata_still_reaches_the_request_allowance() {
        let user = format!("{WEB}/usage?user=user_01TESTUSER");
        let http = Scripted::new()
            .on(
                "POST",
                &rpc("GetCurrentPeriodUsage"),
                200,
                r#"{"enabled":true}"#,
            )
            .on(
                "POST",
                &rpc("GetPlanInfo"),
                200,
                r#"{"planInfo":{"planName":42}}"#,
            )
            .on("GET", &format!("{WEB}/usage-summary"), 404, "{}")
            .on(
                "GET",
                &user,
                200,
                r#"{"gpt-4":{"numRequests":100,"maxRequestUsage":500}}"#,
            );
        let scope = context_at(&http, secret(&live_token()), now());
        let reading = Cursor.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::count(TOTAL, 100.0, 500.0, "requests", None, Some(MONTH)),
                    lines::count(REQUESTS, 100.0, 500.0, "requests", None, Some(MONTH)),
                    lines::count("gpt-4", 100.0, 500.0, "requests", None, Some(MONTH)),
                ]
            )
        );
        assert_eq!(
            urls(&http.requests()),
            [
                rpc("GetCurrentPeriodUsage"),
                rpc("GetPlanInfo"),
                format!("{WEB}/usage-summary"),
                user
            ]
        );
    }

    #[tokio::test]
    async fn an_account_without_a_plan_meter_has_no_active_subscription() {
        for usage in [
            r#"{"enabled":true}"#,
            r#"{"enabled":false,"planUsage":{"limit":2000,"totalPercentUsed":10}}"#,
        ] {
            let http = Scripted::new()
                .on("POST", &rpc("GetCurrentPeriodUsage"), 200, usage)
                .on("POST", &rpc("GetPlanInfo"), 200, PLAN);
            let scope = context_at(&http, secret(&live_token()), now());
            let error = Cursor.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::NotAvailable, "{usage}");
            assert_eq!(error.message, NO_SUBSCRIPTION);
            assert_eq!(http.requests().len(), 2, "no lookups after a dead end");
        }
    }

    #[tokio::test]
    async fn a_failed_plan_lookup_falls_back_to_the_membership_and_waits_before_asking_again() {
        let http = Scripted::new()
            .on("POST", &rpc("GetCurrentPeriodUsage"), 200, USAGE)
            .on("POST", &rpc("GetPlanInfo"), 503, "{}")
            .on("POST", &rpc("GetSandUsageStatus"), 404, "{}");
        let token = live_token();
        let scope = context_at(
            &http,
            json!({"accessToken": token, "membershipType": "pro_plus"}),
            now(),
        );
        let first = Cursor.fetch(&scope.context()).await.unwrap();
        assert_eq!(first.plan.as_deref(), Some("Pro+"));
        Cursor.fetch(&scope.context()).await.unwrap();
        let plan_lookups = http
            .requests()
            .iter()
            .filter(|request| request.url == rpc("GetPlanInfo"))
            .count();
        assert_eq!(plan_lookups, 1);
    }

    #[test]
    fn plan_usage_maps_like_the_dashboard() {
        let usage = json!({
            "enabled": true,
            "billingCycleStart": "1781438541000",
            "billingCycleEnd": "1784030541000",
            "planUsage": {"limit": 40000, "totalPercentUsed": 26.346, "totalSpend": 52692},
            "spendLimitUsage": {"individualUsed": 16474, "limitType": "user", "totalSpend": 16474}
        });
        let found = plan_lines(&usage, &Facts::of(&usage), Some("Ultra")).unwrap();
        let reset = Some(DateTime::from_timestamp_millis(1_784_030_541_000).unwrap());
        assert_eq!(
            found,
            vec![
                lines::percent(TOTAL, 26.346, reset, Some(2_592_000_000)),
                lines::dollar_value(ON_DEMAND, 164.74),
            ]
        );
        let masked = json!({
            "planUsage": {"limit": 40000, "totalPercentUsed": 20},
            "spendLimitUsage": {"individualLimit": 5000, "individualRemaining": 4500,
                "individualUsed": 0, "totalSpend": 1200}
        });
        let found = plan_lines(&masked, &Facts::of(&masked), None).unwrap();
        assert_eq!(found[1], lines::dollars(ON_DEMAND, 12.0, 50.0, None, None));
        let percent_only = json!({"planUsage": {"limit": 2000, "remaining": 1500}});
        let found = plan_lines(&percent_only, &Facts::of(&percent_only), None).unwrap();
        assert_eq!(found, vec![lines::percent(TOTAL, 25.0, None, Some(MONTH))]);
    }

    #[test]
    fn fallbacks_follow_the_plan_and_the_spend_limit_shape() {
        let facts = |usage: Value| Facts::of(&usage);
        let unusable = facts(json!({"planUsage": {}}));
        assert_eq!(
            fallback_reason(&unusable, Some("enterprise")),
            Some(ENTERPRISE_UNAVAILABLE)
        );
        assert_eq!(
            fallback_reason(&unusable, Some(" Team ")),
            Some(TEAM_UNAVAILABLE)
        );
        assert_eq!(fallback_reason(&unusable, None), Some(REQUESTS_UNAVAILABLE));
        assert_eq!(fallback_reason(&unusable, Some("Pro")), None);
        let pooled = facts(
            json!({"planUsage": {"totalPercentUsed": 5}, "spendLimitUsage": {"pooledLimit": 1000}}),
        );
        assert_eq!(
            fallback_reason(&pooled, Some("Pro")),
            Some(REQUESTS_UNAVAILABLE)
        );
        let usable = facts(json!({"planUsage": {"limit": 2000}}));
        assert_eq!(fallback_reason(&usable, Some("enterprise")), None);
        let disabled = facts(json!({"enabled": false}));
        assert_eq!(fallback_reason(&disabled, Some("enterprise")), None);
    }

    #[test]
    fn a_request_month_at_the_end_of_time_has_no_reset_instead_of_a_panic() {
        let requests = json!({"startOfMonth": "+262142-12-31T00:00:00"});
        assert!(value::time(&requests, "/startOfMonth").is_some());
        let cycle = Cycle::of_requests(Some(&requests));
        assert_eq!(cycle.resets_at, None);
        assert_eq!(cycle.period_ms, MONTH);
    }

    #[test]
    fn grok_bot_needs_a_personal_allowance() {
        assert_eq!(
            grok_bot(&json!({"usagePercent": 125})),
            Some(lines::percent(GROK_BOT, 100.0, None, Some(lines::WEEK_MS)))
        );
        assert_eq!(
            grok_bot(&json!({"usagePercent": 0})),
            Some(lines::percent(GROK_BOT, 0.0, None, Some(lines::WEEK_MS)))
        );
        for status in [
            json!({"usagePercent": 42, "usesPooledEnterpriseAllowance": true}),
            json!({"usagePercent": 0, "hasNonZeroIncludedLimit": false}),
            json!({"usagePercent": 0, "includedLimitZero": true}),
            json!({"usagePercent": true}),
            json!({"usagePercent": -1}),
            json!({"hasNonZeroIncludedLimit": true}),
        ] {
            assert_eq!(grok_bot(&status), None, "{status}");
        }
    }

    #[test]
    fn credits_add_grants_and_the_prepaid_balance() {
        let grants =
            json!({"hasCreditGrants": true, "totalCents": "1000000", "usedCents": "264729"});
        let stripe = json!({"customerBalance": 991544});
        assert_eq!(credits_left(Some(&grants), Some(&stripe)), Some(7352.71));
        let prepaid = json!({"customerBalance": -991544});
        assert_eq!(credits_left(Some(&grants), Some(&prepaid)), Some(17268.15));
        assert_eq!(credits_left(None, Some(&prepaid)), Some(9915.44));
        assert_eq!(
            credits_left(
                Some(&json!({"hasCreditGrants": true, "totalCents": 5000})),
                None
            ),
            Some(50.0)
        );
        assert_eq!(
            credits_left(Some(&json!({"hasCreditGrants": false})), Some(&stripe)),
            None
        );
        assert_eq!(credits_left(None, None), None);
    }

    #[test]
    fn plan_names_read_like_cursor_names_them() {
        assert_eq!(plan_label(Some("pro plan")).as_deref(), Some("Pro Plan"));
        assert_eq!(plan_label(Some("Pro+")).as_deref(), Some("Pro+"));
        assert_eq!(plan_label(Some("  ")), None);
        assert_eq!(membership_label("pro_plus").as_deref(), Some("Pro+"));
        assert_eq!(membership_label("free_trial").as_deref(), Some("Pro Trial"));
        assert_eq!(
            membership_label("enterprise").as_deref(),
            Some("Enterprise")
        );
    }

    #[test]
    fn the_session_cookie_only_carries_a_plain_user_id() {
        let token = live_token();
        let session = web_session(&token).unwrap();
        assert_eq!(session.user, "user_01TESTUSER");
        assert_eq!(
            session.cookie,
            format!("WorkosCursorSessionToken=user_01TESTUSER%3A%3A{token}")
        );
        assert_eq!(session_user("user_bare").as_deref(), Some("user_bare"));
        assert_eq!(session_user("auth0|user 01;x"), None);
        assert_eq!(session_user("auth0|"), None);
        assert!(web_session("not-a-token").is_none());
    }

    fn session_user(subject: &str) -> Option<String> {
        web_session(&token(subject, now() + Duration::days(1))).map(|session| session.user)
    }

    #[test]
    fn describes_the_dashboard_widgets() {
        let provider = Provider::new("cursor@abc", "Cursor");
        let descriptors = Cursor.descriptors(&provider);
        let rows: Vec<_> = descriptors
            .iter()
            .map(|descriptor| {
                (
                    descriptor.id.as_str(),
                    descriptor.title(),
                    descriptor.metric_label.as_str(),
                    descriptor.limit_resources[0].key.as_str(),
                    descriptor.limit_resources[0].unit.as_str(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "cursor@abc.usage",
                    "Total Usage",
                    TOTAL,
                    "totalUsage",
                    "percent"
                ),
                (
                    "cursor@abc.auto",
                    CURSOR_MODELS,
                    CURSOR_MODELS,
                    "autoUsage",
                    "percent"
                ),
                (
                    "cursor@abc.api",
                    OTHER_MODELS,
                    OTHER_MODELS,
                    "apiUsage",
                    "percent"
                ),
                (
                    "cursor@abc.grokBot",
                    "Grok Bot",
                    GROK_BOT,
                    "grokBot",
                    "percent"
                ),
                (
                    "cursor@abc.onDemand",
                    "Extra Usage",
                    ON_DEMAND,
                    "onDemand",
                    "usd"
                ),
                (
                    "cursor@abc.requests",
                    REQUESTS,
                    REQUESTS,
                    "requests",
                    "requests"
                ),
                ("cursor@abc.credits", CREDITS, CREDITS, "credits", "usd"),
                (
                    "cursor@abc.teamSpend",
                    TEAM_SPEND,
                    TEAM_SPEND,
                    "teamSpend",
                    "usd"
                ),
                (
                    "cursor@abc.teamRequests",
                    TEAM_REQUESTS,
                    TEAM_REQUESTS,
                    "teamPremiumRequests",
                    "count"
                ),
            ]
        );
        assert_eq!(
            descriptors[6].limit_resources[0].kind,
            LimitResourceKind::Balance
        );
        assert_eq!(
            descriptors[5].template.period_duration_ms,
            Some(lines::MONTH_MS)
        );
    }

    fn state_db(
        dir: &std::path::Path,
        values: &[(&str, rusqlite::types::Value)],
    ) -> std::path::PathBuf {
        let path = apps::state_db(&Roots::under(dir), FOLDER);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
            )
            .unwrap();
        for (key, value) in values {
            connection
                .execute(
                    "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
                    rusqlite::params![key, value],
                )
                .unwrap();
        }
        path
    }

    fn text(value: &str) -> rusqlite::types::Value {
        rusqlite::types::Value::Text(value.to_string())
    }

    #[test]
    fn discovers_the_app_login_with_its_email() {
        let dir = tempfile::tempdir().unwrap();
        let token = live_token();
        let path = state_db(
            dir.path(),
            &[
                (ACCESS_TOKEN, text(&token)),
                (REFRESH_TOKEN, text("refresh-token")),
                (CACHED_EMAIL, text("dev@example.com")),
                (MEMBERSHIP, text("pro")),
                ("cursorAuth/openAIKey", text("sk-not-read")),
            ],
        );
        let logins = Cursor.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        let login = &logins[0];
        assert_eq!(login.identity, SUBJECT);
        assert_eq!(login.label.as_deref(), Some("dev@example.com"));
        assert_eq!(login.origin, APP);
        assert_eq!(login.location, path);
        assert_eq!(
            login.secret.value(),
            &json!({"accessToken": token, "membershipType": "pro"})
        );
        assert!(
            Cursor
                .discover(&Roots::under(&dir.path().join("empty")))
                .is_empty()
        );
    }

    #[test]
    fn a_signed_out_app_has_no_login() {
        let dir = tempfile::tempdir().unwrap();
        state_db(dir.path(), &[(CACHED_EMAIL, text("dev@example.com"))]);
        assert!(Cursor.discover(&Roots::under(dir.path())).is_empty());
    }

    #[test]
    fn utf16_values_and_opaque_tokens_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let token = live_token();
        let wide: Vec<u8> = token.encode_utf16().flat_map(u16::to_le_bytes).collect();
        state_db(
            dir.path(),
            &[(ACCESS_TOKEN, rusqlite::types::Value::Blob(wide))],
        );
        let logins = Cursor.discover(&Roots::under(dir.path()));
        assert_eq!(logins[0].identity, SUBJECT);
        assert_eq!(logins[0].secret.str("/accessToken"), Some(token.as_str()));

        let opaque = tempfile::tempdir().unwrap();
        state_db(
            opaque.path(),
            &[
                (ACCESS_TOKEN, text("opaque")),
                (REFRESH_TOKEN, text("refresh")),
            ],
        );
        let logins = Cursor.discover(&Roots::under(opaque.path()));
        assert_eq!(logins[0].identity, sha256_hex("refresh"));
        assert_eq!(logins[0].label, None);

        let emailed = tempfile::tempdir().unwrap();
        state_db(
            emailed.path(),
            &[
                (ACCESS_TOKEN, text("opaque")),
                (REFRESH_TOKEN, text("refresh")),
                (CACHED_EMAIL, text("dev@example.com")),
            ],
        );
        let logins = Cursor.discover(&Roots::under(emailed.path()));
        assert_eq!(logins[0].identity, "dev@example.com");
        assert_eq!(logins[0].label.as_deref(), Some("dev@example.com"));
        assert_eq!(
            logins[0].secret.value(),
            &json!({"accessToken": "opaque", "membershipType": null})
        );
    }

    #[test]
    fn state_values_are_unquoted_and_trimmed() {
        assert_eq!(
            state_text("\"dev@example.com\"".into()).as_deref(),
            Some("dev@example.com")
        );
        assert_eq!(state_text(" pro \n".into()).as_deref(), Some("pro"));
        assert_eq!(state_text("p\0r\0o\0".into()).as_deref(), Some("pro"));
        assert_eq!(state_text("\"\"".into()), None);
    }
}
