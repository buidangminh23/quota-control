//! Kiro: the login the Kiro IDE saves in `~/.aws/sso/cache/kiro-auth-token.json` (the same path on
//! Windows, macOS and Linux) and the CodeWhisperer profile it signed in to, named in that file for
//! Google and GitHub sign-ins and otherwise read from the IDE's
//! `Kiro/User/globalStorage/kiro.kiroagent/profile.json` under the roaming app data folder
//! (`%APPDATA%` on Windows, `~/Library/Application Support` on macOS, `~/.config` on Linux).
//!
//! Endpoints: `GetUsageLimits`, asked the ways Kiro's clients ask it until one answers:
//! `GET https://codewhisperer.us-east-1.amazonaws.com/getUsageLimits`, the AWS JSON `POST` to the
//! same host (`AmazonCodeWhispererService.GetUsageLimits`), `GET
//! https://q.us-east-1.amazonaws.com/getUsageLimits`, and for a Frankfurt profile the JSON `POST` to
//! `https://q.eu-central-1.amazonaws.com/`. The route that answered is tried first for 12 hours.
//! Kiro rotates its refresh token at every renewal, so the saved access token is used as it is and
//! never renewed here: renewing it would sign the IDE out.
//!
//! A Kiro API key (`ksk_…`, created at app.kiro.dev on Pro plans and above, or found in
//! `KIRO_API_KEY`) is sent as the bearer with `tokentype: API_KEY`, first as the AWS JSON `POST` to
//! `https://q.us-east-1.amazonaws.com/` with only the origin and resource type, then as the
//! CodeWhisperer `GET`; a key names its own account, so no profile ARN is sent.
//!
//! Signing in from Quota Control uses the device sign-in the Kiro CLI uses on a remote machine:
//! Kiro's page opens with Google or GitHub already chosen, and once the user approves, the tokens are
//! saved in Quota Control. That refresh token is the card's own, so the card renews it shortly
//! before the hour-long access token runs out and saves the one Kiro rotates in.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{
    HttpRequest, MetricLine, Provider, ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Memo, Reading, Roots, Secret, Service,
};
use crate::signin::{
    self, Converted, Look, Method, Pending, Poll, Polling, SignIn, SignedIn, StartContext,
};
use crate::support::{http, jwt, lines, oauth, value};

pub(crate) struct Kiro;

const NAME: &str = "Kiro";
const EXPIRED: &str = "The Kiro login expired. Open Kiro once to renew it.";
const KEY_REFUSED: &str = "Kiro refused the API key. Create one at app.kiro.dev (Pro plans and above), or ask your administrator to allow API keys.";
const CODEWHISPERER: &str = "https://codewhisperer.us-east-1.amazonaws.com";
const Q: &str = "https://q.us-east-1.amazonaws.com";
const FRANKFURT: &str = "https://q.eu-central-1.amazonaws.com/";
const FRANKFURT_REGION: &str = "eu-central-1";
const TARGET: &str = "AmazonCodeWhispererService.GetUsageLimits";
const ORIGIN: &str = "AI_EDITOR";
const RESOURCE: &str = "AGENTIC_REQUEST";
const IDE_AGENT: &str = "aws-sdk-js/1.0.0 KiroIDE";
const ROUTE_MEMO: &str = "kiro.route";

/// Kiro's sign-in service for Google and GitHub accounts.
const AUTH: &str = "https://prod.us-east-1.auth.desktop.kiro.dev";
/// The only client identity the sign-in service reads, sent as the Kiro CLI sends it.
const CLIENT_ID: &str = "Kiro-CLI";
/// The hosts of the page a device sign-in opens.
const PAGE_HOSTS: &[&str] = &["kiro.dev", "amazoncognito.com"];
/// How long before its expiry a sign-in made here is renewed.
const RENEW_BEFORE_MINUTES: i64 = 5;

/// The shared profiles Kiro falls back to for a login that names none: Google and GitHub sign-ins,
/// and AWS Builder ID.
const SOCIAL_PROFILE: &str = "arn:aws:codewhisperer:us-east-1:699475941385:profile/EHGA3GRVQMUK";
const BUILDER_ID_PROFILE: &str =
    "arn:aws:codewhisperer:us-east-1:638616132270:profile/AAAACCCCXXXX";

/// A meter a card can show, and the widget that shows it.
#[derive(Clone, Copy)]
struct Meter {
    suffix: &'static str,
    title: &'static str,
    unit: &'static str,
    /// The widget template's limit; template numbers are never shown as data.
    template: f64,
    window: Option<i64>,
}

impl Meter {
    fn line(self, used: f64, limit: f64, resets_at: Option<DateTime<Utc>>) -> MetricLine {
        lines::count(self.title, used, limit, self.unit, resets_at, self.window)
    }

    fn descriptor(self, provider: &Provider) -> WidgetDescriptor {
        WidgetDescriptor::bounded_count(
            format!("{}.{}", provider.id, self.suffix),
            provider,
            self.title,
            None,
            self.template,
            self.unit,
            self.window,
        )
        .exporting_progress(self.suffix, self.unit)
    }
}

/// The month's plan credits.
const CREDITS: Meter = Meter {
    suffix: "credits",
    title: "Credits",
    unit: "credits",
    template: 50.0,
    window: Some(lines::MONTH_MS),
};

/// The free trial and bonus grants still running, which expire rather than reset.
const BONUS: Meter = Meter {
    suffix: "bonusCredits",
    title: "Bonus Credits",
    unit: "credits",
    template: 500.0,
    window: None,
};

/// Credits spent past the plan against the account's overage cap.
const EXTRA: Meter = Meter {
    suffix: "extraUsage",
    title: "Extra Usage",
    unit: "credits",
    template: 1000.0,
    window: Some(lines::MONTH_MS),
};

/// A request allowance, for an account still on Kiro's request-based plans.
const REQUESTS: Meter = Meter {
    suffix: "requests",
    title: "Requests",
    unit: "requests",
    template: 50.0,
    window: Some(lines::MONTH_MS),
};

#[async_trait]
impl Service for Kiro {
    fn id(&self) -> &'static str {
        "kiro"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Usage", "https://app.kiro.dev/account/usage"),
            ProviderLink::new("Plans", "https://kiro.dev/pricing/"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::login(NAME).or_api_key(ApiKeyHelp {
            env: &["KIRO_API_KEY"],
            url: "https://app.kiro.dev",
            fields: &[],
        })
    }

    fn sign_in(&self) -> Option<&'static dyn SignIn> {
        Some(&Kiro)
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let path = roots
            .home
            .join(".aws")
            .join("sso")
            .join("cache")
            .join("kiro-auth-token.json");
        let Some(document) = value::read_json(&path, 256 * 1024) else {
            return Vec::new();
        };
        let Some(access_token) = value::text(&document, "/accessToken") else {
            return Vec::new();
        };
        let saved_profile =
            value::text(&document, "/profileArn").filter(|arn| profile_region(arn).is_some());
        let ide_profile = ide_profile(roots);
        let start = value::text(&document, "/startUrl").map(|url| {
            match value::text(&document, "/region") {
                Some(region) => format!("{url} {region}"),
                None => url.to_string(),
            }
        });
        // Every renewal replaces the refresh token, so it names the account only when nothing
        // steadier does.
        let identity = saved_profile
            .map(str::to_string)
            .or_else(|| value::text(&document, "/clientIdHash").map(str::to_string))
            .or(start)
            .or_else(|| ide_profile.clone())
            .unwrap_or_else(|| {
                digest(value::text(&document, "/refreshToken").unwrap_or(access_token))
            });
        let secret = json!({
            "accessToken": access_token,
            "expiresAt": document.get("expiresAt").cloned().unwrap_or_default(),
            "profileArn": saved_profile.map(str::to_string).or(ide_profile),
            "authMethod": value::text(&document, "/authMethod"),
            "provider": value::text(&document, "/provider"),
        });
        vec![Login::new(identity, NAME, &path, Secret::new(secret))]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [CREDITS, BONUS, EXTRA, REQUESTS]
            .into_iter()
            .map(|meter| meter.descriptor(provider))
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        if let Some(key) = context.secret.key() {
            let session = Session {
                token: key,
                profile: String::new(),
                frankfurt: None,
                external_idp: false,
                api_key: true,
            };
            let body = usage_limits(context, &session).await?;
            return Ok(Reading::new(plan(&body), meters(&body, context.now)));
        }
        let renewed = renew_if_due(context).await?;
        let fresh;
        let secret = match renewed {
            Some(document) => {
                context.keep_renewed(document.clone()).await;
                fresh = Secret::owned(document);
                &fresh
            }
            None => context.secret,
        };
        let session = session_of(secret, context.now)?;
        let body = usage_limits(context, &session).await?;
        Ok(Reading::new(plan(&body), meters(&body, context.now)))
    }
}

#[async_trait]
impl SignIn for Kiro {
    fn methods(&self) -> &'static [Method] {
        &[Method::Google, Method::GitHub]
    }

    async fn start(
        &self,
        method: Method,
        context: &StartContext,
    ) -> Result<Pending, SimpleProviderError> {
        // The service spells GitHub this way.
        let provider = if method == Method::GitHub {
            "Github"
        } else {
            "Google"
        };
        let response = context
            .http
            .send(auth_request(
                "/oauth/device/authorization",
                &json!({ "clientId": CLIENT_ID, "loginProvider": provider }),
            ))
            .await
            .map_err(|_| signin::network(NAME))?;
        let body: Value = response.json().unwrap_or(Value::Null);
        let device_code = value::text(&body, "/deviceCode").map(str::to_string);
        let page = value::text(&body, "/verificationUriComplete")
            .and_then(|raw| signin::page_on(raw, PAGE_HOSTS));
        let (Some(device_code), Some(page), true) = (device_code, page, response.is_success())
        else {
            return Err(SimpleProviderError::new(
                uc_core::ErrorCategory::http(response.status),
                format!("{NAME} did not start a sign-in. Try again later."),
            ));
        };
        let interval = value::number(&body, "/intervalInMilliseconds")
            .unwrap_or(5_000.0)
            .clamp(1_000.0, 60_000.0) as u64;
        let lifetime = value::number(&body, "/expiresInMilliseconds")
            .unwrap_or(600_000.0)
            .max(0.0) as u64;
        let requests = context.http.clone();
        let look = move || -> Look {
            let http = requests.clone();
            let device_code = device_code.clone();
            Box::pin(async move { look_device(&http, &device_code, provider).await })
        };
        Ok(signin::polled(
            page,
            None,
            Polling {
                interval: std::time::Duration::from_millis(interval),
                slow_down: std::time::Duration::from_secs(5),
                lifetime: std::time::Duration::from_millis(lifetime),
            },
            context.http.clone(),
            look,
            signed_in,
        ))
    }
}

/// A request to Kiro's sign-in service, as the Kiro CLI sends it.
fn auth_request(path: &str, body: &Value) -> HttpRequest {
    HttpRequest::post(format!("{AUTH}{path}"))
        .json_body(body)
        .header("Accept", "application/json")
        .header("User-Agent", CLIENT_ID)
        .timeout(signin::REQUEST_TIMEOUT)
}

/// One look at a device sign-in. The service answers every look with HTTP 200 and a status; any
/// other answer is treated as passing, as the Kiro CLI treats it, until the code expires.
async fn look_device(
    http: &uc_core::SharedHttpClient,
    device_code: &str,
    provider: &'static str,
) -> Poll {
    let Ok(response) = http
        .send(auth_request(
            "/oauth/device/poll",
            &json!({ "deviceCode": device_code, "clientId": CLIENT_ID }),
        ))
        .await
    else {
        return Poll::Waiting;
    };
    if !response.is_success() {
        return Poll::Waiting;
    }
    let body: Value = response.json().unwrap_or(Value::Null);
    match value::text(&body, "/status") {
        Some("authorized") => Poll::Approved(json!({ "tokens": body, "provider": provider })),
        Some("expired_token") => Poll::Failed(uc_core::loopback::expired()),
        Some("invalid_token") => Poll::Failed(signin::unfinished(400)),
        _ => Poll::Waiting,
    }
}

/// A device sign-in's tokens in the shape the Kiro IDE saves a social login, named by the account
/// Kiro's usage answer reports.
fn signed_in(http: uc_core::SharedHttpClient, answer: Value) -> Converted {
    Box::pin(async move {
        let tokens = &answer["tokens"];
        let access = value::text(tokens, "/accessToken")
            .ok_or_else(|| signin::invalid("Kiro returned no access token."))?;
        let refresh = value::text(tokens, "/refreshToken").ok_or_else(|| {
            signin::invalid("Kiro did not grant a lasting sign-in. Start a new sign-in.")
        })?;
        let provider = value::text(tokens, "/identityProvider")
            .and_then(social_provider)
            .or_else(|| answer["provider"].as_str())
            .unwrap_or("Google");
        let lifetime = value::number(tokens, "/expiresIn")
            .unwrap_or(3600.0)
            .clamp(60.0, 86_400.0) as i64;
        let document = json!({
            "accessToken": access,
            "refreshToken": refresh,
            "expiresAt": (Utc::now() + Duration::seconds(lifetime)).to_rfc3339(),
            "profileArn": value::text(tokens, "/profileArn").filter(|arn| profile_region(arn).is_some()),
            "authMethod": "social",
            "provider": provider,
        });
        let (identity, label) = account_of(&http, &document).await;
        Ok(SignedIn {
            identity,
            label,
            document,
        })
    })
}

/// The service's name for a sign-in's identity provider.
fn social_provider(wire: &str) -> Option<&'static str> {
    match wire.trim().to_ascii_lowercase().as_str() {
        "google" => Some("Google"),
        "github" => Some("Github"),
        _ => None,
    }
}

/// The account a new sign-in belongs to, as Kiro's usage answer names it (its user id and email);
/// a hash of its refresh token when that answer cannot be had, which still keeps the card apart.
async fn account_of(
    http: &uc_core::SharedHttpClient,
    document: &Value,
) -> (String, Option<String>) {
    let secret = Secret::owned(document.clone());
    let memo = Memo::default();
    let context = FetchContext {
        secret: &secret,
        http,
        now: Utc::now(),
        memo: &memo,
    };
    let answer = match session_of(&secret, context.now) {
        Ok(session) => usage_limits(&context, &session).await.ok(),
        Err(_) => None,
    };
    let text = |pointer: &str| {
        answer
            .as_ref()
            .and_then(|body| value::text(body, pointer))
            .map(str::to_string)
    };
    let email = text("/userInfo/email");
    let identity = text("/userInfo/userId")
        .or_else(|| email.clone())
        .unwrap_or_else(|| digest(value::text(document, "/refreshToken").unwrap_or_default()));
    (identity, email)
}

/// A sign-in made in Quota Control, renewed shortly before it expires: its refresh token is the
/// card's own, so renewing it signs nothing else out. Another app's login is never renewed.
async fn renew_if_due(context: &FetchContext<'_>) -> Result<Option<Value>, SimpleProviderError> {
    let secret = context.secret;
    if !secret.is_owned() {
        return Ok(None);
    }
    let expires = value::time(secret.value(), "/expiresAt");
    if expires
        .is_some_and(|expires| expires > context.now + Duration::minutes(RENEW_BEFORE_MINUTES))
    {
        return Ok(None);
    }
    let refresh = secret
        .str("/refreshToken")
        .ok_or_else(|| http::expired(oauth::SIGN_IN_EXPIRED))?;
    let response = http::send(
        context.http,
        auth_request("/refreshToken", &json!({ "refreshToken": refresh })),
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
    let access = value::text(&body, "/accessToken").ok_or_else(|| http::decoding(NAME))?;
    let lifetime = value::number(&body, "/expiresIn")
        .unwrap_or(3600.0)
        .clamp(60.0, 86_400.0) as i64;
    let mut document = secret.value().clone();
    document["accessToken"] = json!(access);
    document["expiresAt"] = json!((context.now + Duration::seconds(lifetime)).to_rfc3339());
    if let Some(rotated) = value::text(&body, "/refreshToken") {
        document["refreshToken"] = json!(rotated);
    }
    if let Some(profile) =
        value::text(&body, "/profileArn").filter(|arn| profile_region(arn).is_some())
    {
        document["profileArn"] = json!(profile);
    }
    Ok(Some(document))
}

/// What one fetch sends for `secret`, unless its token has expired.
fn session_of(secret: &Secret, now: DateTime<Utc>) -> Result<Session<'_>, SimpleProviderError> {
    let expired = || {
        http::expired(if secret.is_owned() {
            oauth::SIGN_IN_EXPIRED
        } else {
            EXPIRED
        })
    };
    let token = secret.str("/accessToken").ok_or_else(expired)?;
    // Kiro's own tokens are opaque; a Microsoft Entra ID token may carry its expiry itself.
    let expires = value::time(secret.value(), "/expiresAt").or_else(|| jwt::expires_at(token));
    if expires.is_some_and(|expires| expires <= now) {
        return Err(expired());
    }
    let saved = secret.str("/profileArn");
    Ok(Session {
        token,
        profile: saved
            .and_then(normalize_arn)
            .unwrap_or_else(|| default_profile(secret).to_string()),
        frankfurt: saved.filter(|arn| profile_region(arn) == Some(FRANKFURT_REGION)),
        external_idp: secret.str("/authMethod").is_some_and(is_external_idp),
        api_key: false,
    })
}

/// What one fetch sends.
struct Session<'a> {
    token: &'a str,
    /// The profile ARN in us-east-1, where the us-east-1 hosts look every profile up.
    profile: String,
    /// The saved profile ARN when the profile lives in Frankfurt, for that region's own host.
    frankfurt: Option<&'a str>,
    /// Microsoft Entra ID sign-ins, whose token the service reads only when it is marked.
    external_idp: bool,
    /// A Kiro API key, which the service reads only when it is marked.
    api_key: bool,
}

/// One way to call `GetUsageLimits`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    CodeWhispererGet,
    CodeWhispererPost,
    QGet,
    FrankfurtPost,
    /// The AWS JSON `POST` to the Q host without a profile, the way API-key clients ask.
    KeyPost,
}

impl Route {
    fn name(self) -> &'static str {
        match self {
            Self::CodeWhispererGet => "codewhisperer-get",
            Self::CodeWhispererPost => "codewhisperer-post",
            Self::QGet => "q-get",
            Self::FrankfurtPost => "q-eu-central-1-post",
            Self::KeyPost => "q-key-post",
        }
    }

    fn request(self, session: &Session<'_>) -> HttpRequest {
        let request = match self {
            Self::CodeWhispererGet => HttpRequest::get(format!(
                "{CODEWHISPERER}/getUsageLimits?{}",
                query(&[
                    ("isEmailRequired", "true"),
                    ("origin", ORIGIN),
                    ("resourceType", RESOURCE),
                ])
            ))
            .header("x-amz-user-agent", IDE_AGENT)
            .header("User-Agent", IDE_AGENT),
            Self::CodeWhispererPost => json_call(
                &format!("{CODEWHISPERER}/"),
                &json!({
                    "origin": ORIGIN,
                    "profileArn": session.profile,
                    "resourceType": RESOURCE,
                }),
            ),
            Self::QGet => HttpRequest::get(format!(
                "{Q}/getUsageLimits?{}",
                query(&[
                    ("origin", ORIGIN),
                    ("profileArn", session.profile.as_str()),
                    ("resourceType", RESOURCE),
                ])
            )),
            Self::FrankfurtPost => json_call(
                FRANKFURT,
                &json!({ "profileArn": session.frankfurt.unwrap_or_default() }),
            ),
            Self::KeyPost => json_call(
                &format!("{Q}/"),
                &json!({ "origin": ORIGIN, "resourceType": RESOURCE }),
            ),
        }
        .bearer(session.token)
        .header("Accept", "application/json");
        if session.api_key {
            request.header("tokentype", "API_KEY")
        } else if session.external_idp {
            request.header("TokenType", "EXTERNAL_IDP")
        } else {
            request
        }
    }
}

/// An AWS JSON 1.0 call of `GetUsageLimits`.
fn json_call(url: &str, body: &Value) -> HttpRequest {
    HttpRequest::post(url)
        .header("Content-Type", "application/x-amz-json-1.0")
        .header("x-amz-target", TARGET)
        .body(serde_json::to_vec(body).unwrap_or_default())
}

fn query(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// `GetUsageLimits` through the first route that answers, starting with the one that answered last
/// time. A refused token moves on, since another route may take it, and so does an answer without
/// a `usageBreakdownList`, which is not usage data; a rate limit stops at once.
async fn usage_limits(
    context: &FetchContext<'_>,
    session: &Session<'_>,
) -> Result<Value, SimpleProviderError> {
    let mut routes = if session.api_key {
        vec![Route::KeyPost, Route::CodeWhispererGet]
    } else {
        vec![
            Route::CodeWhispererGet,
            Route::CodeWhispererPost,
            Route::QGet,
        ]
    };
    if session.frankfurt.is_some() {
        routes.push(Route::FrankfurtPost);
    }
    if let Some(remembered) = context.memo.get(ROUTE_MEMO, context.now).await
        && let Some(index) = routes
            .iter()
            .position(|route| remembered.as_str() == Some(route.name()))
    {
        routes[..=index].rotate_right(1);
    }
    let mut refused = false;
    let mut failure = None;
    for route in routes {
        let response = match http::send(context.http, route.request(session), NAME).await {
            Ok(response) => response,
            Err(error) => {
                failure = Some(error);
                continue;
            }
        };
        match response.status {
            401 | 403 => refused = true,
            429 => return Err(http::status_error(&response, NAME)),
            _ if response.is_success() => match http::parse(&response, NAME) {
                Ok(body) if body.get("usageBreakdownList").is_some_and(Value::is_array) => {
                    context
                        .memo
                        .put(
                            ROUTE_MEMO,
                            json!(route.name()),
                            Some(context.now + Duration::hours(12)),
                        )
                        .await;
                    return Ok(body);
                }
                Ok(_) => failure = Some(http::decoding(NAME)),
                Err(error) => failure = Some(error),
            },
            _ => failure = Some(http::status_error(&response, NAME)),
        }
    }
    if refused {
        return Err(if session.api_key {
            http::invalid(KEY_REFUSED)
        } else {
            http::expired(EXPIRED)
        });
    }
    Err(failure.unwrap_or_else(|| http::decoding(NAME)))
}

/// The card's meters: plan credits, bonus credits and overage from the `CREDIT` breakdown, or the
/// first request allowance of an account still on request-based plans.
fn meters(body: &Value, now: DateTime<Utc>) -> Vec<MetricLine> {
    let mut rows = aggregate_meters(body, now);
    for entry in body
        .get("usageBreakdownList")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(kind) =
            value::text(entry, "/resourceType").filter(|kind| !kind.eq_ignore_ascii_case("CREDIT"))
        else {
            continue;
        };
        let Some(used) = amount(entry, "currentUsage").filter(|used| *used >= 0.0) else {
            continue;
        };
        let label = value::text(entry, "/displayName")
            .map(str::to_string)
            .or_else(|| lines::plan_name(kind))
            .unwrap_or_else(|| kind.to_string());
        let unit = value::text(entry, "/unit").unwrap_or("requests");
        let reset = value::time(entry, "/nextDateReset")
            .or_else(|| value::time(body, "/nextDateReset"))
            .or_else(|| value::time(body, "/resetDate"));
        rows.push(
            match amount(entry, "usageLimit").filter(|limit| *limit > 0.0) {
                Some(limit) => {
                    lines::count(&label, used, limit, unit, reset, Some(lines::MONTH_MS))
                }
                None => lines::count_value(&label, used, unit),
            },
        );
    }
    rows
}

fn aggregate_meters(body: &Value, now: DateTime<Utc>) -> Vec<MetricLine> {
    let entries: &[Value] = body
        .get("usageBreakdownList")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let reset = |entry: &Value| {
        value::time(entry, "/nextDateReset")
            .or_else(|| value::time(body, "/nextDateReset"))
            .or_else(|| value::time(body, "/resetDate"))
    };
    let credit = entries.iter().find(|entry| {
        value::text(entry, "/resourceType").is_some_and(|kind| kind.eq_ignore_ascii_case("CREDIT"))
    });
    let Some(credit) = credit else {
        return entries
            .iter()
            .find_map(|entry| allowance(REQUESTS, entry, reset(entry)))
            .into_iter()
            .collect();
    };
    let resets_at = reset(credit);
    [
        allowance(CREDITS, credit, resets_at),
        bonus(credit, now),
        overage(body, credit, resets_at),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// A plan allowance. `currentUsage` also counts overage, so the plan's share is what is left once
/// overage is taken out, and it never passes the allowance.
fn allowance(meter: Meter, entry: &Value, resets_at: Option<DateTime<Utc>>) -> Option<MetricLine> {
    let limit = amount(entry, "usageLimit").filter(|limit| *limit > 0.0)?;
    let used = amount(entry, "currentUsage").unwrap_or(0.0)
        - amount(entry, "currentOverages").unwrap_or(0.0);
    Some(meter.line(used.clamp(0.0, limit), limit, resets_at))
}

/// The free trial and the bonus grants still running, added together, expiring with the first.
fn bonus(entry: &Value, now: DateTime<Utc>) -> Option<MetricLine> {
    let trial = entry.get("freeTrialInfo").map(|trial| {
        (
            trial,
            value::text(trial, "/freeTrialStatus"),
            value::time(trial, "/freeTrialExpiry"),
        )
    });
    let bonuses = entry
        .get("bonuses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|grant| {
            let expiry = ["/expiresAt", "/expiryDate", "/expiry"]
                .iter()
                .find_map(|pointer| value::time(grant, pointer));
            (grant, value::text(grant, "/status"), expiry)
        });
    let grants: Vec<(f64, f64, Option<DateTime<Utc>>)> = trial
        .into_iter()
        .chain(bonuses)
        .filter(|&(_, status, expiry)| {
            status.is_none_or(|status| !status.eq_ignore_ascii_case("EXPIRED"))
                && expiry.is_none_or(|expiry| expiry > now)
        })
        .filter_map(|(grant, _, expiry)| {
            let limit = amount(grant, "usageLimit").filter(|limit| *limit > 0.0)?;
            let used = amount(grant, "currentUsage")
                .unwrap_or(0.0)
                .clamp(0.0, limit);
            Some((used, limit, expiry))
        })
        .collect();
    let limit: f64 = grants.iter().map(|(_, limit, _)| limit).sum();
    (limit > 0.0).then(|| {
        let used: f64 = grants.iter().map(|(used, _, _)| used).sum();
        let expires = grants.iter().filter_map(|(_, _, expiry)| *expiry).min();
        BONUS.line(used, limit, expires)
    })
}

/// Credits spent past the plan against the account's overage cap, while overage is on.
fn overage(body: &Value, entry: &Value, resets_at: Option<DateTime<Utc>>) -> Option<MetricLine> {
    let enabled = value::text(body, "/overageConfiguration/overageStatus")
        .is_some_and(|status| status.eq_ignore_ascii_case("ENABLED"));
    let cap = amount(entry, "overageCap").filter(|cap| enabled && *cap > 0.0)?;
    Some(EXTRA.line(
        amount(entry, "currentOverages").unwrap_or(0.0),
        cap,
        resets_at,
    ))
}

/// A quantity, from the precise `<name>WithPrecision` field AWS sends beside the rounded one.
fn amount(item: &Value, name: &str) -> Option<f64> {
    value::number(item, &format!("/{name}WithPrecision"))
        .or_else(|| value::number(item, &format!("/{name}")))
}

/// The plan's name: `KIRO POWER` → `Power`, `Kiro Enterprise` → `Enterprise`, else the
/// subscription type (`Q_DEVELOPER_STANDALONE_PRO_PLUS` → `Pro Plus`).
fn plan(body: &Value) -> Option<String> {
    let titled = value::text(body, "/subscriptionInfo/subscriptionTitle").and_then(|title| {
        let tier = match title.split_once(char::is_whitespace) {
            Some((brand, rest)) if brand.eq_ignore_ascii_case("kiro") => rest,
            _ => title,
        };
        lines::plan_name(tier)
    });
    titled.or_else(|| {
        let kind = value::text(body, "/subscriptionInfo/type")?;
        lines::plan_name(kind.strip_prefix("Q_DEVELOPER_STANDALONE_").unwrap_or(kind))
    })
}

/// The profile the IDE signed in to, as its Kiro agent extension records it.
fn ide_profile(roots: &Roots) -> Option<String> {
    let path = roots
        .app_data
        .join("Kiro")
        .join("User")
        .join("globalStorage")
        .join("kiro.kiroagent")
        .join("profile.json");
    let document = value::read_json(&path, 64 * 1024)?;
    value::text(&document, "/arn")
        .filter(|arn| profile_region(arn).is_some())
        .map(str::to_string)
}

/// The region of a CodeWhisperer profile ARN (`arn:aws:codewhisperer:<region>:<account>:profile/
/// <id>`), or `None` for anything else.
fn profile_region(arn: &str) -> Option<&str> {
    if arn
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return None;
    }
    let mut parts = arn.splitn(6, ':');
    let (
        Some("arn"),
        Some("aws"),
        Some("codewhisperer"),
        Some(region),
        Some(account),
        Some(resource),
    ) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    )
    else {
        return None;
    };
    let profile = resource.strip_prefix("profile/")?;
    (!region.is_empty() && !account.is_empty() && !profile.is_empty()).then_some(region)
}

/// `arn` with its region set to us-east-1, the region the us-east-1 hosts look every profile up
/// in, whatever region the profile lives in.
fn normalize_arn(arn: &str) -> Option<String> {
    let region = profile_region(arn)?;
    let rest = &arn["arn:aws:codewhisperer:".len() + region.len()..];
    Some(format!("arn:aws:codewhisperer:us-east-1{rest}"))
}

/// The shared profile Kiro picks for a login that names none, by how the user signed in.
fn default_profile(secret: &Secret) -> &'static str {
    let social = secret
        .str("/authMethod")
        .is_some_and(|method| method.eq_ignore_ascii_case("social"))
        || secret.str("/provider").is_some_and(|provider| {
            ["google", "github"]
                .iter()
                .any(|name| provider.eq_ignore_ascii_case(name))
        });
    if social {
        SOCIAL_PROFILE
    } else {
        BUILDER_ID_PROFILE
    }
}

fn is_external_idp(method: &str) -> bool {
    method
        .replace('-', "_")
        .eq_ignore_ascii_case("external_idp")
}

fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod sign_in_tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header, owned_context_at};
    use chrono::TimeZone;

    const USAGE: &str = "https://codewhisperer.us-east-1.amazonaws.com/getUsageLimits";
    const ANSWER: &str = r#"{"subscriptionInfo":{"subscriptionTitle":"KIRO FREE"},
        "usageBreakdownList":[{"resourceType":"CREDIT","displayName":"Credit","currentUsage":12,
            "usageLimit":50}],
        "userInfo":{"email":"person@example.com","userId":"user-1"}}"#;

    fn body(request: &HttpRequest) -> Value {
        serde_json::from_slice(request.body.as_deref().unwrap_or_default()).unwrap()
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn start_context(http: &Scripted) -> StartContext {
        StartContext {
            http: http.shared(),
            language: uc_core::loopback::LoginLanguage::English,
            product: NAME,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_github_device_sign_in_saves_a_social_login_named_by_its_user() {
        let http = Scripted::new()
            .on(
                "POST",
                &format!("{AUTH}/oauth/device/authorization"),
                200,
                r#"{"deviceCode":"dc-1","userCode":"ABCD-EFGH",
                    "verificationUri":"https://app.kiro.dev/device",
                    "verificationUriComplete":"https://app.kiro.dev/device?code=ABCD-EFGH",
                    "expiresInMilliseconds":600000,"intervalInMilliseconds":5000}"#,
            )
            .on(
                "POST",
                &format!("{AUTH}/oauth/device/poll"),
                200,
                r#"{"status":"authorization_pending"}"#,
            )
            .on(
                "POST",
                &format!("{AUTH}/oauth/device/poll"),
                200,
                r#"{"status":"authorized","accessToken":"aoa-1","refreshToken":"aor-1",
                    "expiresIn":3600,"identityProvider":"github"}"#,
            )
            .on("GET", USAGE, 200, ANSWER);
        let pending = Kiro
            .start(Method::GitHub, &start_context(&http))
            .await
            .unwrap();
        assert_eq!(pending.url, "https://app.kiro.dev/device?code=ABCD-EFGH");
        assert!(pending.user_code.is_none());
        let account = pending.finish.await.result.unwrap();
        assert_eq!(account.identity, "user-1");
        assert_eq!(account.label.as_deref(), Some("person@example.com"));
        assert_eq!(account.document["provider"], "Github");
        assert_eq!(account.document["authMethod"], "social");
        assert_eq!(account.document["refreshToken"], "aor-1");
        let requests = http.requests();
        assert_eq!(
            body(&requests[0]),
            json!({"clientId": "Kiro-CLI", "loginProvider": "Github"})
        );
        assert_eq!(header(&requests[0], "User-Agent"), Some("Kiro-CLI"));
        assert_eq!(
            body(&requests[1]),
            json!({"deviceCode": "dc-1", "clientId": "Kiro-CLI"})
        );
        assert_eq!(header(&requests[3], "Authorization"), Some("Bearer aoa-1"));
    }

    #[tokio::test]
    async fn a_device_page_off_kiro_is_not_opened() {
        let http = Scripted::new().on(
            "POST",
            &format!("{AUTH}/oauth/device/authorization"),
            200,
            r#"{"deviceCode":"dc-1","verificationUriComplete":"https://example.com/device"}"#,
        );
        let error = Kiro
            .start(Method::Google, &start_context(&http))
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.message,
            "Kiro did not start a sign-in. Try again later."
        );
    }

    #[tokio::test]
    async fn a_sign_in_made_here_is_renewed_before_it_expires_and_the_rotated_token_kept() {
        let http = Scripted::new()
            .on(
                "POST",
                &format!("{AUTH}/refreshToken"),
                200,
                r#"{"accessToken":"aoa-2","refreshToken":"aor-2","expiresIn":3600}"#,
            )
            .on("GET", USAGE, 200, ANSWER);
        let scope = owned_context_at(
            &http,
            json!({
                "accessToken": "aoa-1",
                "refreshToken": "aor-1",
                "expiresAt": (now() + Duration::minutes(2)).to_rfc3339(),
                "authMethod": "social",
                "provider": "Google"
            }),
            now(),
        );
        let reading = Kiro.fetch(&scope.context()).await.unwrap();
        assert!(!reading.lines.is_empty());
        let requests = http.requests();
        assert_eq!(body(&requests[0]), json!({"refreshToken": "aor-1"}));
        assert_eq!(header(&requests[1], "Authorization"), Some("Bearer aoa-2"));
        let renewed = scope.renewed().await.unwrap();
        assert_eq!(renewed["accessToken"], "aoa-2");
        assert_eq!(renewed["refreshToken"], "aor-2");
        assert_eq!(
            value::time(&renewed, "/expiresAt"),
            Some(now() + Duration::hours(1))
        );
    }

    #[tokio::test]
    async fn a_fresh_sign_in_is_used_as_saved() {
        let http = Scripted::new().on("GET", USAGE, 200, ANSWER);
        let scope = owned_context_at(
            &http,
            json!({
                "accessToken": "aoa-1",
                "refreshToken": "aor-1",
                "expiresAt": (now() + Duration::minutes(30)).to_rfc3339(),
                "authMethod": "social"
            }),
            now(),
        );
        Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(http.requests().len(), 1);
        assert!(scope.renewed().await.is_none());
    }

    #[tokio::test]
    async fn a_refused_renewal_asks_to_sign_in_again_in_accounts() {
        let http = Scripted::new().on(
            "POST",
            &format!("{AUTH}/refreshToken"),
            400,
            r#"{"error":"invalid_grant"}"#,
        );
        let scope = owned_context_at(
            &http,
            json!({"accessToken": "a", "refreshToken": "r", "expiresAt": now().to_rfc3339()}),
            now(),
        );
        let error = Kiro.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, oauth::SIGN_IN_REVOKED);
        assert!(scope.renewed().await.is_none());
    }

    #[tokio::test]
    async fn the_apps_own_expired_login_is_never_renewed() {
        let http = Scripted::new();
        let scope = context_at(
            &http,
            json!({"accessToken": "a", "refreshToken": "r", "expiresAt": now().to_rfc3339()}),
            now(),
        );
        let error = Kiro.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, EXPIRED);
        assert!(http.requests().is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use std::path::{Path, PathBuf};
    use uc_core::ErrorCategory;

    const PROFILE: &str = "arn:aws:codewhisperer:us-east-1:123456789012:profile/TESTPROFILE";
    const FRANKFURT_PROFILE: &str =
        "arn:aws:codewhisperer:eu-central-1:123456789012:profile/EUPROFILE";

    /// Shaped like a live answer for a Kiro Power account past its plan credits and into overage
    /// (identifiers scrubbed), with quarter values and an October reset.
    const POWER: &str = r#"{"daysUntilReset":4,"limits":[],"nextDateReset":1.7908128E9,
        "overageConfiguration":{"__type":"com.amazon.aws.codewhisperer#OverageConfiguration",
            "overageStatus":"ENABLED"},
        "subscriptionInfo":{"overageCapability":"OVERAGE_CAPABLE","subscriptionTitle":"KIRO POWER",
            "type":"Q_DEVELOPER_STANDALONE_POWER"},
        "usageBreakdownList":[{"bonuses":[],"currency":"USD","currentOverages":3603,
            "currentOveragesWithPrecision":3603.25,"currentUsage":13603,
            "currentUsageWithPrecision":13603.25,"displayName":"Credit",
            "nextDateReset":1.7908128E9,"overageCap":10000,"overageCapWithPrecision":10000.0,
            "overageCharges":144.13,"overageCredits":[],"overageRate":0.04,
            "resourceType":"CREDIT","unit":"INVOCATIONS","usageLimit":10000,
            "usageLimitWithPrecision":10000.0}],
        "userInfo":{"email":"person@example.com","userId":"user-1"}}"#;

    /// A free account in its trial, with one running and one expired bonus grant.
    const FREE: &str = r#"{"daysUntilReset":4,"nextDateReset":1.7908128E9,
        "overageConfiguration":{"overageStatus":"DISABLED"},
        "subscriptionInfo":{"subscriptionTitle":"KIRO FREE","type":"Q_DEVELOPER_STANDALONE_FREE"},
        "usageBreakdownList":[{"resourceType":"CREDIT","displayName":"Credit","currency":"USD",
            "currentUsage":12,"currentUsageWithPrecision":12.5,"usageLimit":50,
            "usageLimitWithPrecision":50.0,"currentOverages":0,"currentOveragesWithPrecision":0.0,
            "overageCap":0,"overageCapWithPrecision":0.0,
            "freeTrialInfo":{"freeTrialStatus":"ACTIVE","freeTrialExpiry":1.7916768E9,
                "currentUsage":120,"currentUsageWithPrecision":120.25,"usageLimit":500,
                "usageLimitWithPrecision":500.0},
            "bonuses":[
                {"bonusCode":"WELCOME","displayName":"Welcome bonus","status":"ACTIVE",
                 "expiresAt":1.7911584E9,"currentUsage":2,"currentUsageWithPrecision":2.5,
                 "usageLimit":10,"usageLimitWithPrecision":10.0},
                {"bonusCode":"SPRING","displayName":"Spring bonus","status":"EXPIRED",
                 "expiresAt":1.7898624E9,"currentUsage":20,"usageLimit":20}]}],
        "userInfo":{"email":"person@example.com","userId":"user-1"}}"#;

    /// A meter as (label, used, limit, unit, reset, window).
    type Row = (String, f64, f64, String, Option<DateTime<Utc>>, Option<i64>);

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn october(day: u32) -> Option<DateTime<Utc>> {
        Some(Utc.with_ymd_and_hms(2026, 10, day, 0, 0, 0).unwrap())
    }

    #[tokio::test]
    async fn an_api_key_is_sent_as_a_marked_bearer_without_a_profile() {
        let http = Scripted::new().on("POST", "https://q.us-east-1.amazonaws.com/", 200, POWER);
        let scope = context_at(&http, json!({"apiKey": "ksk_test"}), now());
        let reading = Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Power"));
        let request = &http.requests()[0];
        assert_eq!(header(request, "Authorization"), Some("Bearer ksk_test"));
        assert_eq!(header(request, "tokentype"), Some("API_KEY"));
        let body: Value = serde_json::from_slice(request.body.as_deref().unwrap()).unwrap();
        assert_eq!(body, json!({"origin": ORIGIN, "resourceType": RESOURCE}));
    }

    #[tokio::test]
    async fn a_refused_api_key_says_where_keys_come_from() {
        let http = Scripted::new()
            .on("POST", "https://q.us-east-1.amazonaws.com/", 403, "{}")
            .on(
                "GET",
                "https://codewhisperer.us-east-1.amazonaws.com/getUsageLimits?isEmailRequired=true&origin=AI_EDITOR&resourceType=AGENTIC_REQUEST",
                401,
                "{}",
            );
        let scope = context_at(&http, json!({"apiKey": "ksk_bad"}), now());
        let error = Kiro.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, KEY_REFUSED);
        assert_eq!(http.requests().len(), 2);
    }

    fn secret(expires: DateTime<Utc>) -> Value {
        json!({
            "accessToken": "saved-token",
            "expiresAt": expires.to_rfc3339(),
            "profileArn": PROFILE,
            "authMethod": "IdC",
            "provider": "BuilderId"
        })
    }

    fn fresh() -> Value {
        secret(now() + Duration::minutes(30))
    }

    fn cw_get() -> String {
        format!("{CODEWHISPERER}/getUsageLimits")
    }

    fn cw_post() -> String {
        format!("{CODEWHISPERER}/")
    }

    fn q_get() -> String {
        format!("{Q}/getUsageLimits")
    }

    fn rows(found: &[MetricLine]) -> Vec<Row> {
        found
            .iter()
            .map(|line| match line {
                MetricLine::Progress(line) => (
                    line.label.clone(),
                    line.used,
                    line.limit,
                    line.format.count_suffix().unwrap_or_default().to_string(),
                    line.resets_at,
                    line.period_duration_ms,
                ),
                other => panic!("unexpected line {other:?}"),
            })
            .collect()
    }

    fn body_of(request: &HttpRequest) -> Value {
        serde_json::from_slice(request.body.as_ref().unwrap()).unwrap()
    }

    /// The method of every request sent so far, in order.
    fn methods(http: &Scripted) -> Vec<String> {
        http.requests()
            .into_iter()
            .map(|request| request.method)
            .collect()
    }

    fn token_file(home: &Path) -> PathBuf {
        home.join(".aws")
            .join("sso")
            .join("cache")
            .join("kiro-auth-token.json")
    }

    fn write(path: &Path, content: &Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content.to_string()).unwrap();
    }

    #[tokio::test]
    async fn reads_plan_credits_and_extra_usage() {
        let http = Scripted::new().on("GET", &cw_get(), 200, POWER);
        let scope = context_at(&http, fresh(), now());
        let reading = Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Power"));
        assert_eq!(
            rows(&reading.lines),
            vec![
                (
                    "Credits".into(),
                    10000.0,
                    10000.0,
                    "credits".into(),
                    october(1),
                    Some(lines::MONTH_MS)
                ),
                (
                    "Extra Usage".into(),
                    3603.25,
                    10000.0,
                    "credits".into(),
                    october(1),
                    Some(lines::MONTH_MS)
                ),
            ]
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(
            requests[0].url,
            "https://codewhisperer.us-east-1.amazonaws.com/getUsageLimits?isEmailRequired=true&origin=AI_EDITOR&resourceType=AGENTIC_REQUEST"
        );
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some("Bearer saved-token")
        );
        assert_eq!(header(&requests[0], "x-amz-user-agent"), Some(IDE_AGENT));
        assert_eq!(header(&requests[0], "User-Agent"), Some(IDE_AGENT));
        assert_eq!(header(&requests[0], "Accept"), Some("application/json"));
        assert_eq!(header(&requests[0], "TokenType"), None);
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn adds_the_trial_and_running_bonuses_up_as_bonus_credits() {
        let http = Scripted::new().on("GET", &cw_get(), 200, FREE);
        let scope = context_at(&http, fresh(), now());
        let reading = Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Free"));
        assert_eq!(
            rows(&reading.lines),
            vec![
                (
                    "Credits".into(),
                    12.5,
                    50.0,
                    "credits".into(),
                    october(1),
                    Some(lines::MONTH_MS)
                ),
                (
                    "Bonus Credits".into(),
                    122.75,
                    510.0,
                    "credits".into(),
                    october(5),
                    None
                ),
            ]
        );
    }

    #[tokio::test]
    async fn falls_back_to_the_json_call_and_starts_there_next_time() {
        let http = Scripted::new()
            .on("GET", &cw_get(), 400, r#"{"__type":"ValidationException"}"#)
            .on("POST", &cw_post(), 200, FREE);
        let scope = context_at(&http, fresh(), now());
        let reading = Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Free"));
        Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(methods(&http), ["GET", "POST", "POST"]);
        let requests = http.requests();
        let post = &requests[1];
        assert_eq!(post.url, "https://codewhisperer.us-east-1.amazonaws.com/");
        assert_eq!(header(post, "Authorization"), Some("Bearer saved-token"));
        assert_eq!(
            header(post, "x-amz-target"),
            Some("AmazonCodeWhispererService.GetUsageLimits")
        );
        assert_eq!(
            header(post, "Content-Type"),
            Some("application/x-amz-json-1.0")
        );
        assert_eq!(
            body_of(post),
            json!({"origin": "AI_EDITOR", "profileArn": PROFILE, "resourceType": "AGENTIC_REQUEST"})
        );
    }

    #[tokio::test]
    async fn falls_back_to_the_q_host_with_the_profile_in_the_query() {
        let http = Scripted::new()
            .on("GET", &cw_get(), 500, "{}")
            .on("POST", &cw_post(), 404, "{}")
            .on("GET", &q_get(), 200, POWER);
        let scope = context_at(&http, fresh(), now());
        let reading = Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines.len(), 2);
        let requests = http.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests[2].url,
            "https://q.us-east-1.amazonaws.com/getUsageLimits?origin=AI_EDITOR&profileArn=arn%3Aaws%3Acodewhisperer%3Aus-east-1%3A123456789012%3Aprofile%2FTESTPROFILE&resourceType=AGENTIC_REQUEST"
        );
        assert_eq!(
            header(&requests[2], "Authorization"),
            Some("Bearer saved-token")
        );
    }

    #[tokio::test]
    async fn a_frankfurt_profile_is_also_asked_on_its_own_host() {
        let mut saved = fresh();
        saved["profileArn"] = json!(FRANKFURT_PROFILE);
        let http = Scripted::new()
            .on("GET", &cw_get(), 403, "{}")
            .on("POST", &cw_post(), 403, "{}")
            .on("GET", &q_get(), 403, "{}")
            .on("POST", FRANKFURT, 200, POWER);
        let scope = context_at(&http, saved, now());
        let reading = Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Power"));
        let requests = http.requests();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            body_of(&requests[1])["profileArn"],
            "arn:aws:codewhisperer:us-east-1:123456789012:profile/EUPROFILE"
        );
        assert_eq!(requests[3].method, "POST");
        assert_eq!(requests[3].url, "https://q.eu-central-1.amazonaws.com/");
        assert_eq!(
            header(&requests[3], "x-amz-target"),
            Some("AmazonCodeWhispererService.GetUsageLimits")
        );
        assert_eq!(
            body_of(&requests[3]),
            json!({"profileArn": FRANKFURT_PROFILE})
        );
    }

    #[tokio::test]
    async fn a_token_every_route_refuses_asks_to_open_kiro() {
        let http = Scripted::new()
            .on("GET", &cw_get(), 401, "{}")
            .on("POST", &cw_post(), 403, "{}")
            .on("GET", &q_get(), 401, "{}");
        let scope = context_at(&http, fresh(), now());
        let error = Kiro.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "The Kiro login expired. Open Kiro once to renew it."
        );
        assert_eq!(http.requests().len(), 3);
    }

    #[tokio::test]
    async fn an_expired_saved_token_is_never_sent() {
        let http = Scripted::new().on("GET", &cw_get(), 200, POWER);
        let scope = context_at(&http, secret(now() - Duration::minutes(1)), now());
        let error = Kiro.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "The Kiro login expired. Open Kiro once to renew it."
        );
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn an_expired_entra_token_is_known_by_its_own_expiry() {
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = json!({"exp": (now() - Duration::minutes(1)).timestamp()}).to_string();
        let token = format!("h.{}.s", URL_SAFE_NO_PAD.encode(payload));
        let http = Scripted::new().on("GET", &cw_get(), 200, POWER);
        let saved =
            json!({"accessToken": token, "authMethod": "external_idp", "profileArn": PROFILE});
        let scope = context_at(&http, saved, now());
        let error = Kiro.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn a_rate_limit_stops_at_the_first_route() {
        let http =
            Scripted::new()
                .on("GET", &cw_get(), 429, "{}")
                .on("POST", &cw_post(), 200, POWER);
        let scope = context_at(&http, fresh(), now());
        let error = Kiro.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn an_outage_on_every_route_reports_the_last_status() {
        let http = Scripted::new()
            .on("GET", &cw_get(), 500, "{}")
            .on("POST", &cw_post(), 502, "{}")
            .on("GET", &q_get(), 503, "{}");
        let scope = context_at(&http, fresh(), now());
        let error = Kiro.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "Kiro answered with HTTP 503.");
    }

    #[tokio::test]
    async fn an_answer_without_a_usage_breakdown_moves_on_and_is_not_remembered() {
        let http = Scripted::new()
            .on(
                "GET",
                &cw_get(),
                200,
                r#"{"daysUntilReset":4,"limits":[],"nextDateReset":1.7908128E9}"#,
            )
            .on("POST", &cw_post(), 200, FREE);
        let scope = context_at(&http, fresh(), now());
        let reading = Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Free"));
        assert_eq!(reading.lines.len(), 2);
        Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(methods(&http), ["GET", "POST", "POST"]);
    }

    #[tokio::test]
    async fn unreadable_answers_on_every_route_report_a_decoding_error() {
        let http = Scripted::new()
            .on("GET", &cw_get(), 200, "<html>Sign in</html>")
            .on("POST", &cw_post(), 200, r#"{"message":"OK"}"#)
            .on("GET", &q_get(), 200, r#"{"usageBreakdownList":"none"}"#);
        let scope = context_at(&http, fresh(), now());
        let error = Kiro.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);
        assert_eq!(
            error.message,
            "Kiro returned usage data this version cannot read."
        );
        assert_eq!(http.requests().len(), 3);
    }

    #[tokio::test]
    async fn a_remembered_route_that_stops_answering_falls_back_to_the_others() {
        let http = Scripted::new()
            .on("GET", &cw_get(), 400, r#"{"__type":"ValidationException"}"#)
            .on("GET", &cw_get(), 200, POWER)
            .on("POST", &cw_post(), 200, FREE)
            .on(
                "POST",
                &cw_post(),
                403,
                r#"{"__type":"AccessDeniedException"}"#,
            );
        let scope = context_at(&http, fresh(), now());
        let first = Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(first.plan.as_deref(), Some("Free"));
        let second = Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(second.plan.as_deref(), Some("Power"));
        let third = Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(third.plan.as_deref(), Some("Power"));
        assert_eq!(methods(&http), ["GET", "POST", "POST", "GET", "GET"]);
    }

    #[tokio::test]
    async fn a_social_login_without_a_profile_uses_the_shared_social_profile() {
        let http =
            Scripted::new()
                .on("GET", &cw_get(), 400, "{}")
                .on("POST", &cw_post(), 200, FREE);
        let saved =
            json!({"accessToken": "saved-token", "authMethod": "social", "provider": "Google"});
        let scope = context_at(&http, saved, now());
        Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(body_of(&http.requests()[1])["profileArn"], SOCIAL_PROFILE);
    }

    #[tokio::test]
    async fn external_identity_provider_logins_mark_their_token() {
        let http = Scripted::new().on("GET", &cw_get(), 200, FREE);
        let mut saved = fresh();
        saved["authMethod"] = json!("external_idp");
        let scope = context_at(&http, saved, now());
        Kiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            header(&http.requests()[0], "TokenType"),
            Some("EXTERNAL_IDP")
        );
    }

    #[test]
    fn disabled_overage_shows_no_extra_usage() {
        let body: Value =
            serde_json::from_str(&POWER.replace("\"ENABLED\"", "\"DISABLED\"")).unwrap();
        let labels: Vec<String> = meters(&body, now())
            .iter()
            .map(|line| line.label().to_string())
            .collect();
        assert_eq!(labels, ["Credits"]);
    }

    #[test]
    fn request_based_accounts_show_their_first_allowance_as_requests() {
        let body = json!({"nextDateReset": 1790812800, "usageBreakdownList": [
            {"resourceType": "SPEC", "currentUsage": 3, "usageLimit": 0},
            {"resourceType": "VIBE", "currentUsage": 12, "usageLimit": 50}
        ]});
        assert_eq!(
            rows(&aggregate_meters(&body, now())),
            vec![(
                "Requests".into(),
                12.0,
                50.0,
                "requests".into(),
                october(1),
                Some(lines::MONTH_MS)
            )]
        );
        let all = meters(&body, now());
        assert_eq!(all.len(), 3);
        assert_eq!(all[1], lines::count_value("Spec", 3.0, "requests"));
        assert_eq!(
            all[2],
            lines::count(
                "Vibe",
                12.0,
                50.0,
                "requests",
                october(1),
                Some(lines::MONTH_MS)
            )
        );
    }

    #[test]
    fn every_meter_has_a_widget_of_the_same_label() {
        let provider = Provider::new("kiro@abc", "Kiro");
        let descriptors = Kiro.descriptors(&provider);
        let ids: Vec<&str> = descriptors
            .iter()
            .map(|descriptor| descriptor.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "kiro@abc.credits",
                "kiro@abc.bonusCredits",
                "kiro@abc.extraUsage",
                "kiro@abc.requests"
            ]
        );
        let labels: Vec<&str> = descriptors
            .iter()
            .map(|descriptor| descriptor.metric_label.as_str())
            .collect();
        assert_eq!(
            labels,
            ["Credits", "Bonus Credits", "Extra Usage", "Requests"]
        );
    }

    #[test]
    fn plans_read_like_kiro_names_them() {
        let plan_of = |info: Value| plan(&json!({ "subscriptionInfo": info }));
        assert_eq!(
            plan_of(json!({"subscriptionTitle": "KIRO PRO+"})).as_deref(),
            Some("Pro+")
        );
        assert_eq!(
            plan_of(json!({"subscriptionTitle": "Kiro Enterprise"})).as_deref(),
            Some("Enterprise")
        );
        assert_eq!(
            plan_of(json!({"subscriptionTitle": "Q Developer Pro"})).as_deref(),
            Some("Q Developer Pro")
        );
        assert_eq!(
            plan_of(json!({"type": "Q_DEVELOPER_STANDALONE_PRO_PLUS"})).as_deref(),
            Some("Pro Plus")
        );
        assert_eq!(plan(&json!({})), None);
    }

    #[test]
    fn profile_arns_are_checked_and_moved_to_us_east_1() {
        assert_eq!(
            normalize_arn(FRANKFURT_PROFILE).as_deref(),
            Some("arn:aws:codewhisperer:us-east-1:123456789012:profile/EUPROFILE")
        );
        assert_eq!(normalize_arn(PROFILE).as_deref(), Some(PROFILE));
        assert_eq!(profile_region(FRANKFURT_PROFILE), Some("eu-central-1"));
        for invalid in [
            "",
            "not-an-arn",
            "arn:aws-cn:codewhisperer:cn-north-1:123456789012:profile/A",
            "arn:aws:s3:us-east-1:123456789012:profile/A",
            "arn:aws:codewhisperer::123456789012:profile/A",
            "arn:aws:codewhisperer:us-east-1::profile/A",
            "arn:aws:codewhisperer:us-east-1:123456789012:profile/",
            "arn:aws:codewhisperer:us-east-1:123456789012:other/A",
            "arn:aws:codewhisperer:us-east-1:123456789012:profile/A B",
        ] {
            assert_eq!(normalize_arn(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn discovers_a_social_login_by_its_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = token_file(dir.path());
        write(
            &path,
            &json!({
                "accessToken": "aoa-access",
                "refreshToken": "aor-refresh",
                "expiresAt": "2026-09-27T11:00:00.000Z",
                "authMethod": "social",
                "provider": "Google",
                "profileArn": SOCIAL_PROFILE
            }),
        );
        let logins = Kiro.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        let login = &logins[0];
        assert_eq!(login.identity, SOCIAL_PROFILE);
        assert_eq!(login.origin, "Kiro");
        assert_eq!(login.location, path);
        assert_eq!(login.label, None);
        assert_eq!(login.secret.str("/accessToken"), Some("aoa-access"));
        assert_eq!(login.secret.str("/profileArn"), Some(SOCIAL_PROFILE));
        assert_eq!(login.secret.str("/refreshToken"), None);
        assert_eq!(
            value::time(login.secret.value(), "/expiresAt"),
            Some(Utc.with_ymd_and_hms(2026, 9, 27, 11, 0, 0).unwrap())
        );
        let empty = tempfile::tempdir().unwrap();
        assert!(Kiro.discover(&Roots::under(empty.path())).is_empty());
    }

    #[test]
    fn an_identity_center_login_takes_its_profile_from_the_ide() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        write(
            &token_file(dir.path()),
            &json!({
                "accessToken": "aoa-access",
                "refreshToken": "aor-refresh",
                "expiresAt": "2026-09-27T11:00:00Z",
                "authMethod": "IdC",
                "provider": "Enterprise",
                "clientIdHash": "0123abcd",
                "region": "eu-central-1",
                "startUrl": "https://d-1234567890.awsapps.com/start"
            }),
        );
        write(
            &roots
                .app_data
                .join("Kiro")
                .join("User")
                .join("globalStorage")
                .join("kiro.kiroagent")
                .join("profile.json"),
            &json!({"arn": FRANKFURT_PROFILE, "name": "Org profile"}),
        );
        let logins = Kiro.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, "0123abcd");
        assert_eq!(logins[0].secret.str("/profileArn"), Some(FRANKFURT_PROFILE));
        assert_eq!(logins[0].secret.str("/authMethod"), Some("IdC"));
    }

    #[test]
    fn a_login_without_an_account_id_is_named_by_a_digest_of_its_refresh_token() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &token_file(dir.path()),
            &json!({"accessToken": "aoa-access", "refreshToken": "aor-refresh"}),
        );
        let logins = Kiro.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        let expected: String = Sha256::digest(b"aor-refresh")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(logins[0].identity, expected);
        assert_eq!(logins[0].identity.len(), 64);
        assert_eq!(logins[0].secret.str("/profileArn"), None);
    }

    #[test]
    fn a_token_file_without_an_access_token_is_not_a_login() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &token_file(dir.path()),
            &json!({"refreshToken": "aor-refresh"}),
        );
        assert!(Kiro.discover(&Roots::under(dir.path())).is_empty());
    }
}
