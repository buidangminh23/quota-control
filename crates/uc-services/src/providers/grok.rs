//! Grok CLI: the xAI login `grok login` saves in `auth.json` under `$GROK_HOME`, else under `.grok`
//! in the user's home (`%USERPROFILE%\.grok\auth.json` on Windows, `~/.grok/auth.json` on macOS and
//! Linux), read against the Grok Build billing the CLI itself shows.
//!
//! `GET https://cli-chat-proxy.grok.com/v1/billing?format=credits` answers with the weekly shared
//! credit pool, the pay-as-you-go cap and what pay-as-you-go spent (proto3 JSON: amounts wrapped as
//! `{"val": n}`, zero values left out). The plan comes from the same answer when it names one, else
//! from `GET /v1/settings` (then `GET /v1/user?include=subscription`), looked up twice a day, else
//! from the access token's `tier` claim.
//!
//! A renewed login is written back to `auth.json` and its refresh token may rotate, so the saved
//! access token is used as it is and never renewed here: once it has expired, or Grok refuses it, the
//! card asks to run `grok` once, which renews it.
//!
//! The CLI's access token (`key` in `auth.json`) can also be pasted in Quota Control; it is read the
//! same way and lasts about a week, after which the card asks for a fresh copy.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{
    HttpRequest, MetricLine, Provider, ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{apps, http, jwt, lines, value};

pub(crate) struct Grok;

const NAME: &str = "Grok";
const APP: &str = "Grok CLI";
const CREDITS_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
const SETTINGS_URL: &str = "https://cli-chat-proxy.grok.com/v1/settings";
const USER_URL: &str = "https://cli-chat-proxy.grok.com/v1/user?include=subscription";
/// The header the Grok CLI's own billing calls send beside the bearer token.
const TOKEN_AUTH: &str = "xai-grok-cli";

/// `auth.json` entry keys: the xAI OIDC login (`https://auth.x.ai::<client>`) wins over the older
/// session login (`https://accounts.x.ai/sign-in`).
const OIDC_PREFIX: &str = "https://auth.x.ai::";
const SESSION_MARK: &str = "/sign-in";

const WEEKLY_LABEL: &str = "Weekly limit";
const PAY_AS_YOU_GO_LABEL: &str = "Pay as you go";
const ON_DEMAND: &str = "On-Demand";
/// The badge colors upstream uses: green while pay-as-you-go has a cap, neutral grey when it is off.
const CAP_ON_COLOR: &str = "#22c55e";
const CAP_OFF_COLOR: &str = "#a3a3a3";

const PLAN_MEMO: &str = "grok.plan";
/// Where a plan name is looked up when the billing answer names none, and the fields holding it.
const PLAN_SOURCES: [(&str, &[&str]); 2] = [
    (
        SETTINGS_URL,
        &["/subscription_tier_display", "/subscriptionTierDisplay"],
    ),
    (
        USER_URL,
        &[
            "/subscription_tier_display",
            "/subscriptionTierDisplay",
            "/subscriptionTier",
            "/subscription_tier",
            "/subscription/tier",
        ],
    ),
];

#[async_trait]
impl Service for Grok {
    fn id(&self) -> &'static str {
        "grok"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Usage", "https://grok.com/?_s=usage"),
            ProviderLink::new("Status", "https://status.x.ai"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::login(APP).or_api_key(ApiKeyHelp {
            env: &[],
            url: "https://grok.com",
            fields: &[],
        })
    }

    fn key_label(&self) -> &'static str {
        "Grok CLI access token (key in ~/.grok/auth.json)"
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let mut homes = Vec::new();
        if let Some(custom) = roots.var("GROK_HOME") {
            homes.push(expand_home(&custom, &roots.home));
        }
        homes.push(roots.home.join(".grok"));
        let Some(path) = apps::first_file(homes.into_iter().map(|home| home.join("auth.json")))
        else {
            return Vec::new();
        };
        let Some(document) = value::read_json(&path, 256 * 1024) else {
            return Vec::new();
        };
        login(&document, &path).into_iter().collect()
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let id = |suffix: &str| format!("{}.{suffix}", provider.id);
        vec![
            WidgetDescriptor::percent(id("weekly"), provider, "Weekly", Some(WEEKLY_LABEL), None)
                .exporting_progress("weekly", "percent"),
            WidgetDescriptor::badge(
                id("payAsYouGo"),
                provider,
                "Extra Usage",
                Some(PAY_AS_YOU_GO_LABEL),
            ),
            WidgetDescriptor::percent(id("onDemand"), provider, ON_DEMAND, None, None)
                .exporting_progress("onDemand", "percent"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let pasted = context.secret.key();
        let token = pasted
            .or_else(|| context.secret.str("/key"))
            .ok_or_else(|| {
                http::invalid("The Grok CLI login has no access token. Run grok login again.")
            })?;
        let expired = || {
            if pasted.is_some() {
                http::expired(PASTED_EXPIRED)
            } else {
                expired()
            }
        };
        let expires =
            jwt::expires_at(token).or_else(|| value::time(context.secret.value(), "/expiresAt"));
        if expires.is_some_and(|expires| expires <= context.now) {
            return Err(expired());
        }
        let response = http::send(context.http, billing_request(CREDITS_URL, token), NAME).await?;
        if matches!(response.status, 401 | 403) {
            return Err(expired());
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let body = http::parse(&response, NAME)?;
        let lines = credits_lines(&body)?;
        let plan = match credits_tier(&body) {
            Some(plan) => Some(plan),
            None => looked_up_plan(context, token)
                .await
                .or_else(|| token_tier(token)),
        };
        Ok(Reading::new(plan, lines))
    }
}

/// The login a CLI-written `auth.json` holds: the OIDC entry before the session one, the entry
/// whose token lasts longest among equals. Only the access token and its expiry reach the secret.
fn login(document: &Value, path: &Path) -> Option<Login> {
    let (_, entry, token) = document
        .as_object()?
        .iter()
        .filter_map(|(key, entry)| Some((key.as_str(), entry, value::text(entry, "/key")?)))
        .min_by(|(a_key, a_entry, a_token), (b_key, b_entry, b_token)| {
            scope_rank(a_key)
                .cmp(&scope_rank(b_key))
                .then_with(|| token_expiry(b_entry, b_token).cmp(&token_expiry(a_entry, a_token)))
                .then_with(|| a_key.cmp(b_key))
        })?;
    let id_token = value::text(entry, "/id_token");
    let claim = |names: &[&str]| {
        jwt::claim(token, names).or_else(|| id_token.and_then(|id| jwt::claim(id, names)))
    };
    let email = value::text(entry, "/email")
        .map(str::to_string)
        .or_else(|| claim(&["email"]));
    let full_name = [
        value::text(entry, "/first_name"),
        value::text(entry, "/last_name"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let identity = value::text(entry, "/user_id")
        .map(str::to_string)
        .or_else(|| email.clone())
        .or_else(|| claim(&["sub"]))
        .unwrap_or_else(|| {
            fingerprint(
                value::text(entry, "/refresh_token")
                    .or_else(|| value::text(entry, "/refresh"))
                    .unwrap_or(token),
            )
        });
    let expires_at = ["expires_at", "expires"]
        .iter()
        .find_map(|name| entry.get(*name).filter(|found| !found.is_null()))
        .cloned()
        .unwrap_or(Value::Null);
    let secret = Secret::new(json!({ "key": token, "expiresAt": expires_at }));
    let label = email.or_else(|| (!full_name.is_empty()).then_some(full_name));
    Some(Login::new(identity, APP, path, secret).with_label(label))
}

fn scope_rank(entry_key: &str) -> u8 {
    if entry_key.starts_with(OIDC_PREFIX) {
        0
    } else if entry_key.contains(SESSION_MARK) {
        1
    } else {
        2
    }
}

/// When an entry's access token stops working: its JWT `exp`, else the entry's own expiry.
fn token_expiry(entry: &Value, token: &str) -> Option<DateTime<Utc>> {
    jwt::expires_at(token)
        .or_else(|| value::time(entry, "/expires_at"))
        .or_else(|| value::time(entry, "/expires"))
}

/// `$GROK_HOME` with a leading `~` meaning the user's home.
fn expand_home(raw: &str, home: &Path) -> PathBuf {
    match raw.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) => match rest.strip_prefix(['/', '\\']) {
            Some(tail) => home.join(tail),
            None => PathBuf::from(raw),
        },
        None => PathBuf::from(raw),
    }
}

fn fingerprint(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A pasted CLI token lasts about a week; the CLI renews its own.
const PASTED_EXPIRED: &str =
    "The pasted Grok token expired. Run grok once, then copy key from ~/.grok/auth.json again.";

fn expired() -> SimpleProviderError {
    http::expired("The Grok CLI login expired. Run grok once to renew it.")
}

fn billing_request(url: &str, token: &str) -> HttpRequest {
    HttpRequest::get(url)
        .bearer(token)
        .header("X-XAI-Token-Auth", TOKEN_AUTH)
        .header("Accept", "application/json")
}

/// A billing window: its start when known, and its end.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Period {
    start: Option<DateTime<Utc>>,
    end: DateTime<Utc>,
}

impl Period {
    fn duration_ms(self) -> Option<i64> {
        self.start
            .map(|start| (self.end - start).num_milliseconds())
            .filter(|milliseconds| *milliseconds > 0)
    }
}

/// The card's rows from the credits answer: the weekly pool's meter, the pay-as-you-go badge and,
/// while pay-as-you-go has a cap, how much of the cap it spent.
fn credits_lines(body: &Value) -> Result<Vec<MetricLine>, SimpleProviderError> {
    let config = body
        .get("config")
        .filter(|config| config.is_object())
        .ok_or_else(|| http::decoding(NAME))?;
    let (kind, current) = current_period(config)?;
    let billing = billing_period(config);
    let window = current.or(billing);
    let published = number(config, &["creditUsagePercent", "credit_usage_percent"])?;
    if published.is_none() && window.is_none() {
        return Err(http::decoding(NAME));
    }
    let weekly = match kind.filter(|kind| !kind.ends_with("UNSPECIFIED")) {
        Some(kind) => kind.ends_with("WEEKLY"),
        None => window
            .and_then(Period::duration_ms)
            .is_some_and(|milliseconds| {
                (4 * lines::DAY_MS..=12 * lines::DAY_MS).contains(&milliseconds)
            }),
    };
    // Proto3 JSON leaves a zero percent out, but only a unified-billing account has the pool: an
    // omitted percent reads 0% there and "No data" anywhere else, never a made-up 0%.
    let unified = flag(config, &["isUnifiedBillingUser", "is_unified_billing_user"]) == Some(true);
    let mut rows = Vec::new();
    if weekly && let Some(used) = published.or(unified.then_some(0.0)) {
        rows.push(lines::percent(
            WEEKLY_LABEL,
            used,
            window.map(|period| period.end),
            Some(
                window
                    .and_then(Period::duration_ms)
                    .unwrap_or(lines::WEEK_MS),
            ),
        ));
    }
    let cap = amount(config, &["onDemandCap", "on_demand_cap"])?
        .unwrap_or(0.0)
        .max(0.0);
    rows.push(pay_as_you_go(cap));
    // The spend only feeds the On-Demand meter, so an unreadable one leaves that meter empty
    // instead of failing the rows the answer does carry.
    if cap > 0.0
        && let Ok(spent) = amount(config, &["onDemandUsed", "on_demand_used"])
    {
        let span = billing.or(window);
        rows.extend(lines::percent_of(
            ON_DEMAND,
            spent.unwrap_or(0.0).max(0.0),
            cap,
            span.map(|period| period.end),
            span.and_then(Period::duration_ms),
        ));
    }
    Ok(rows)
}

/// The current usage period's type (upper-cased) and window. A period whose times are unreadable
/// or run backwards means the answer changed shape.
fn current_period(config: &Value) -> Result<(Option<String>, Option<Period>), SimpleProviderError> {
    let Some(current) = field(config, &["currentPeriod", "current_period"]) else {
        return Ok((None, None));
    };
    if !current.is_object() {
        return Err(http::decoding(NAME));
    }
    let kind = value::text(current, "/type").map(str::to_ascii_uppercase);
    let start = strict_time(current, "start")?;
    let end = strict_time(current, "end")?;
    let period = match (start, end) {
        (Some(start), Some(end)) if end <= start => return Err(http::decoding(NAME)),
        (start, Some(end)) => Some(Period { start, end }),
        (_, None) => None,
    };
    Ok((kind, period))
}

/// The billing cycle, when the answer carries a readable one.
fn billing_period(config: &Value) -> Option<Period> {
    let time = |names: &[&str]| field(config, names).and_then(value::as_time);
    let end = time(&["billingPeriodEnd", "billing_period_end"])?;
    let start = time(&["billingPeriodStart", "billing_period_start"]).filter(|start| *start < end);
    Some(Period { start, end })
}

/// The first of `names` that `object` holds with a non-null value.
fn field<'a>(object: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names
        .iter()
        .find_map(|name| object.get(*name).filter(|found| !found.is_null()))
}

/// A number that may be left out; one that is there but not a number means a changed answer.
fn number(object: &Value, names: &[&str]) -> Result<Option<f64>, SimpleProviderError> {
    match field(object, names) {
        None => Ok(None),
        Some(raw) => value::as_number(raw)
            .map(Some)
            .ok_or_else(|| http::decoding(NAME)),
    }
}

/// An amount in the proto3 wrapper `{"val": n}` (or bare); an empty wrapper is zero.
fn amount(object: &Value, names: &[&str]) -> Result<Option<f64>, SimpleProviderError> {
    let Some(raw) = field(object, names) else {
        return Ok(None);
    };
    let inner = match raw {
        Value::Object(wrapper) => match wrapper.get("val").filter(|val| !val.is_null()) {
            Some(val) => val,
            None => return Ok(Some(0.0)),
        },
        bare => bare,
    };
    value::as_number(inner)
        .map(Some)
        .ok_or_else(|| http::decoding(NAME))
}

fn flag(object: &Value, names: &[&str]) -> Option<bool> {
    names
        .iter()
        .find_map(|name| value::flag(object, &format!("/{name}")))
}

fn strict_time(object: &Value, name: &str) -> Result<Option<DateTime<Utc>>, SimpleProviderError> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(raw) => value::as_time(raw)
            .map(Some)
            .ok_or_else(|| http::decoding(NAME)),
    }
}

fn pay_as_you_go(cap: f64) -> MetricLine {
    let badge = if cap > 0.0 {
        MetricLine::badge(PAY_AS_YOU_GO_LABEL, format!("{} cap", units(cap))).color(CAP_ON_COLOR)
    } else {
        MetricLine::badge(PAY_AS_YOU_GO_LABEL, "Disabled").color(CAP_OFF_COLOR)
    };
    badge.into()
}

/// An amount as upstream prints it: whole numbers without a fraction.
fn units(amount: f64) -> String {
    match whole(amount) {
        Some(whole) => whole.to_string(),
        None => amount.to_string(),
    }
}

fn whole(number: f64) -> Option<i64> {
    let rounded = number.round();
    ((number - rounded).abs() < 1e-9 && rounded.abs() < 1e15).then_some(rounded as i64)
}

/// The plan the credits answer names, on the config or beside it.
fn credits_tier(body: &Value) -> Option<String> {
    [
        "/config/subscriptionTier",
        "/config/subscription_tier",
        "/subscriptionTier",
        "/subscription_tier",
    ]
    .iter()
    .filter_map(|pointer| value::text(body, pointer))
    .find_map(plan_label)
}

/// The plan from the settings or user endpoint, remembered for 12 hours (an hour when neither
/// answered). A failed lookup never fails the card.
async fn looked_up_plan(context: &FetchContext<'_>, token: &str) -> Option<String> {
    if let Some(memo) = context.memo.get(PLAN_MEMO, context.now).await {
        return memo["plan"].as_str().map(str::to_string);
    }
    let mut answered = false;
    let mut plan = None;
    for (url, pointers) in PLAN_SOURCES {
        let Some(body) = optional_json(context, url, token).await else {
            continue;
        };
        answered = true;
        plan = pointers
            .iter()
            .filter_map(|pointer| value::text(&body, pointer))
            .find_map(plan_label);
        if plan.is_some() {
            break;
        }
    }
    let keep = if answered {
        Duration::hours(12)
    } else {
        Duration::hours(1)
    };
    context
        .memo
        .put(PLAN_MEMO, json!({ "plan": plan }), Some(context.now + keep))
        .await;
    plan
}

async fn optional_json(context: &FetchContext<'_>, url: &str, token: &str) -> Option<Value> {
    let response = http::send(context.http, billing_request(url, token), NAME)
        .await
        .ok()?;
    if !response.is_success() {
        return None;
    }
    http::parse(&response, NAME).ok()
}

/// The plan the access token's `tier` claim stands for, as the Grok CLI numbers its tiers (a
/// number, or the same number as text).
fn token_tier(token: &str) -> Option<String> {
    let claims = jwt::claims(token)?;
    let tier = claims.get("tier")?;
    let Some(number) = value::as_number(tier) else {
        return tier.as_str().and_then(plan_label);
    };
    let name = match whole(number)? {
        0 => "Free",
        1 => "SuperGrok",
        2 => "X Basic",
        3 => "X Premium",
        4 => "X Premium Plus",
        5 => "SuperGrok Heavy",
        6 => "SuperGrok Lite",
        _ => return None,
    };
    Some(name.to_string())
}

/// A tier as Grok spells it (`SUPERGROK_HEAVY`, `supergrok_heavy`, `SuperGrok Heavy`,
/// `X Premium+`) turned into its display name.
fn plan_label(raw: &str) -> Option<String> {
    const ENUM_PREFIX: &str = "SUBSCRIPTION_TIER_";
    let raw = raw.trim();
    let bare = match raw.get(..ENUM_PREFIX.len()) {
        Some(head) if head.eq_ignore_ascii_case(ENUM_PREFIX) => &raw[ENUM_PREFIX.len()..],
        _ => raw,
    };
    // `+` is part of a tier's name (`X Premium+` is X Premium Plus), so it must not be dropped.
    let compact: String = bare
        .replace('+', "plus")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    let known = match compact.as_str() {
        "" | "none" | "null" | "unknown" | "unspecified" | "invalid" => return None,
        "free" => "Free",
        "supergrok" => "SuperGrok",
        "supergrokheavy" | "heavy" => "SuperGrok Heavy",
        "supergroklite" => "SuperGrok Lite",
        "xbasic" => "X Basic",
        "xpremium" => "X Premium",
        "xpremiumplus" => "X Premium Plus",
        _ if !bare.contains('_') && bare.chars().any(char::is_lowercase) => {
            return Some(bare.to_string());
        }
        _ => return lines::plan_name(bare).map(|plan| plan.replace("Supergrok", "SuperGrok")),
    };
    Some(known.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use chrono::TimeZone;
    use uc_core::{BadgeLine, ErrorCategory, ProgressFormat, ProgressLine};

    const CREDITS: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
    const SETTINGS: &str = "https://cli-chat-proxy.grok.com/v1/settings";
    const USER: &str = "https://cli-chat-proxy.grok.com/v1/user?include=subscription";
    const START: &str = "2026-09-23T21:36:52.140114+00:00";
    const END: &str = "2026-09-30T21:36:52.140114+00:00";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn signed(payload: Value) -> String {
        format!(
            "{}.{}.signature",
            URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256"}"#),
            URL_SAFE_NO_PAD.encode(payload.to_string())
        )
    }

    fn access_token(extra: Value) -> String {
        let mut payload = json!({
            "sub": "0f9c2d4e-user",
            "exp": (now() + Duration::minutes(30)).timestamp(),
        });
        payload
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        signed(payload)
    }

    fn secret(token: &str) -> Value {
        json!({ "key": token, "expiresAt": "2026-09-27T10:30:00Z" })
    }

    fn credits(config: Value) -> String {
        json!({ "config": config }).to_string()
    }

    /// The answer upstream captured live from the credits endpoint, moved to this week.
    fn captured() -> Value {
        json!({
            "creditUsagePercent": 99.0,
            "currentPeriod": { "type": "USAGE_PERIOD_TYPE_WEEKLY", "start": START, "end": END },
            "onDemandCap": { "val": 0 },
            "onDemandUsed": { "val": 0 },
            "isUnifiedBillingUser": true,
            "prepaidBalance": { "val": 0 },
            "topUpMethod": "TOP_UP_METHOD_SAVED_PAYMENT_METHOD",
            "billingPeriodStart": START,
            "billingPeriodEnd": END
        })
    }

    fn weekly(used: f64) -> MetricLine {
        MetricLine::Progress(ProgressLine {
            label: "Weekly limit".into(),
            used,
            limit: 100.0,
            format: ProgressFormat::Percent,
            resets_at: Some(at(END)),
            period_duration_ms: Some(7 * 24 * 3_600_000),
            color_hex: None,
        })
    }

    fn badge(text: &str, color: &str) -> MetricLine {
        MetricLine::Badge(BadgeLine {
            label: "Pay as you go".into(),
            text: text.into(),
            color_hex: Some(color.into()),
            subtitle: None,
        })
    }

    #[tokio::test]
    async fn reads_the_weekly_pool_the_cap_badge_and_the_plan() {
        let token = access_token(json!({}));
        let http = Scripted::new()
            .on("GET", CREDITS, 200, &credits(captured()))
            .on(
                "GET",
                SETTINGS,
                200,
                r#"{"subscription_tier_display":"SuperGrok Heavy"}"#,
            );
        let scope = context_at(&http, secret(&token), now());
        let reading = Grok.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("SuperGrok Heavy"));
        assert_eq!(
            reading.lines,
            vec![weekly(99.0), badge("Disabled", "#a3a3a3")]
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        for (request, url) in requests.iter().zip([CREDITS, SETTINGS]) {
            assert_eq!(request.method, "GET");
            assert_eq!(request.url, url);
            assert_eq!(request.body, None);
            assert_eq!(
                header(request, "Authorization"),
                Some(format!("Bearer {token}").as_str())
            );
            assert_eq!(header(request, "X-XAI-Token-Auth"), Some("xai-grok-cli"));
            assert_eq!(header(request, "Accept"), Some("application/json"));
        }
    }

    #[tokio::test]
    async fn a_pasted_cli_token_is_read_like_the_login_and_asks_for_a_fresh_copy_when_refused() {
        let token = access_token(json!({}));
        let http = Scripted::new()
            .on("GET", CREDITS, 200, &credits(captured()))
            .on(
                "GET",
                SETTINGS,
                200,
                r#"{"subscription_tier_display":"SuperGrok"}"#,
            );
        let scope = context_at(&http, json!({ "apiKey": token }), now());
        let reading = Grok.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("SuperGrok"));
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some(format!("Bearer {token}").as_str())
        );
        let refused = Scripted::new().on("GET", CREDITS, 401, "{}");
        let scope = context_at(&refused, json!({ "apiKey": token }), now());
        let error = Grok.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, PASTED_EXPIRED);
    }

    #[tokio::test]
    async fn the_plan_lookup_is_remembered_between_refreshes() {
        let http = Scripted::new()
            .on("GET", CREDITS, 200, &credits(captured()))
            .on(
                "GET",
                SETTINGS,
                200,
                r#"{"subscription_tier_display":"SuperGrok"}"#,
            );
        let scope = context_at(&http, secret(&access_token(json!({}))), now());
        for _ in 0..2 {
            let reading = Grok.fetch(&scope.context()).await.unwrap();
            assert_eq!(reading.plan.as_deref(), Some("SuperGrok"));
        }
        let urls: Vec<_> = http
            .requests()
            .into_iter()
            .map(|request| request.url)
            .collect();
        assert_eq!(urls, vec![CREDITS, SETTINGS, CREDITS]);
    }

    #[tokio::test]
    async fn a_pay_as_you_go_cap_shows_its_spend_and_an_omitted_percent_reads_zero() {
        let mut config = captured();
        let object = config.as_object_mut().unwrap();
        object.remove("creditUsagePercent");
        object.insert("onDemandCap".into(), json!({ "val": 1000 }));
        object.insert("onDemandUsed".into(), json!({ "val": 250 }));
        let body = json!({ "config": config, "subscriptionTier": "SUPERGROK_HEAVY" });
        let http = Scripted::new().on("GET", CREDITS, 200, &body.to_string());
        let scope = context_at(&http, secret(&access_token(json!({}))), now());
        let reading = Grok.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("SuperGrok Heavy"));
        assert_eq!(
            reading.lines,
            vec![
                weekly(0.0),
                badge("1000 cap", "#22c55e"),
                MetricLine::Progress(ProgressLine {
                    label: "On-Demand".into(),
                    used: 25.0,
                    limit: 100.0,
                    format: ProgressFormat::Percent,
                    resets_at: Some(at(END)),
                    period_duration_ms: Some(7 * 24 * 3_600_000),
                    color_hex: None,
                }),
            ]
        );
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_monthly_account_has_no_weekly_pool_and_its_plan_comes_from_the_user_endpoint() {
        let config = json!({
            "creditUsagePercent": 42,
            "currentPeriod": {
                "type": "USAGE_PERIOD_TYPE_MONTHLY",
                "start": "2026-09-01T00:00:00Z",
                "end": "2026-10-01T00:00:00Z"
            }
        });
        let http = Scripted::new()
            .on("GET", CREDITS, 200, &credits(config))
            .on("GET", SETTINGS, 404, "{}")
            .on("GET", USER, 200, r#"{"subscriptionTier":"SUPERGROK"}"#);
        let scope = context_at(&http, secret(&access_token(json!({}))), now());
        let reading = Grok.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("SuperGrok"));
        assert_eq!(reading.lines, vec![badge("Disabled", "#a3a3a3")]);
    }

    #[tokio::test]
    async fn an_omitted_percent_outside_unified_billing_is_no_data_not_zero() {
        let config = json!({
            "currentPeriod": { "type": "USAGE_PERIOD_TYPE_WEEKLY", "start": START, "end": END },
            "onDemandCap": { "val": 0 },
            "onDemandUsed": { "val": 0 },
            "billingPeriodEnd": END
        });
        let http = Scripted::new()
            .on("GET", CREDITS, 200, &credits(config))
            .on("GET", SETTINGS, 500, "")
            .on("GET", USER, 500, "");
        let token = access_token(json!({ "tier": 6 }));
        let scope = context_at(&http, secret(&token), now());
        let reading = Grok.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines, vec![badge("Disabled", "#a3a3a3")]);
        assert_eq!(reading.plan.as_deref(), Some("SuperGrok Lite"));
    }

    #[tokio::test]
    async fn a_refused_token_asks_to_run_grok_once() {
        for status in [401, 403] {
            let http = Scripted::new().on("GET", CREDITS, status, "{}");
            let scope = context_at(&http, secret(&access_token(json!({}))), now());
            let error = Grok.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthExpired);
            assert_eq!(
                error.message,
                "The Grok CLI login expired. Run grok once to renew it."
            );
            assert_eq!(http.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn an_expired_saved_token_is_never_renewed_or_sent() {
        let stale =
            signed(json!({ "sub": "u", "exp": (now() - Duration::minutes(1)).timestamp() }));
        let opaque = json!({ "key": "opaque-token", "expiresAt": "2026-09-27T09:00:00Z" });
        for saved in [secret(&stale), opaque] {
            let http = Scripted::new();
            let scope = context_at(&http, saved, now());
            let error = Grok.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthExpired);
            assert!(http.requests().is_empty());
        }
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_are_reported_by_status() {
        for (status, category, message) in [
            (
                429,
                ErrorCategory::RateLimited,
                "Grok is rate limiting usage requests. Waiting before retrying.",
            ),
            (503, ErrorCategory::Http5xx, "Grok answered with HTTP 503."),
        ] {
            let http = Scripted::new().on("GET", CREDITS, status, "");
            let scope = context_at(&http, secret(&access_token(json!({}))), now());
            let error = Grok.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert_eq!(error.message, message);
            assert_eq!(http.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn an_unreadable_spend_leaves_the_on_demand_meter_empty_not_the_card() {
        let mut config = captured();
        let object = config.as_object_mut().unwrap();
        object.insert("onDemandCap".into(), json!({ "val": 2500 }));
        object.insert("onDemandUsed".into(), json!({ "val": "n/a" }));
        let body = json!({ "config": config, "subscriptionTier": "SUPERGROK" });
        let http = Scripted::new().on("GET", CREDITS, 200, &body.to_string());
        let scope = context_at(&http, secret(&access_token(json!({}))), now());
        let reading = Grok.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("SuperGrok"));
        assert_eq!(
            reading.lines,
            vec![weekly(99.0), badge("2500 cap", "#22c55e")]
        );
    }

    #[test]
    fn the_token_tier_claim_reads_as_a_number_or_numeric_text() {
        for (tier, expected) in [
            (json!(5), Some("SuperGrok Heavy")),
            (json!("5"), Some("SuperGrok Heavy")),
            (json!(0), Some("Free")),
            (json!("supergrok_lite"), Some("SuperGrok Lite")),
            (json!(9), None),
            (json!(1.5), None),
        ] {
            let token = signed(json!({ "sub": "u", "tier": tier }));
            assert_eq!(token_tier(&token).as_deref(), expected, "{tier}");
        }
        assert_eq!(token_tier(&signed(json!({ "sub": "u" }))), None);
        assert_eq!(token_tier("opaque-token"), None);
    }

    #[tokio::test]
    async fn answers_in_another_shape_are_decoding_errors() {
        let period = json!({ "type": "USAGE_PERIOD_TYPE_WEEKLY", "start": START, "end": END });
        let backwards = json!({ "type": "USAGE_PERIOD_TYPE_WEEKLY", "start": END, "end": START });
        let unreadable = json!({ "type": "USAGE_PERIOD_TYPE_WEEKLY", "end": "soon" });
        let bodies = [
            "not json".to_string(),
            "{}".to_string(),
            credits(json!({})),
            credits(json!({ "onDemandCap": { "val": 0 } })),
            credits(json!({ "creditUsagePercent": "high", "currentPeriod": period })),
            credits(json!({ "onDemandCap": { "val": "lots" }, "currentPeriod": period })),
            credits(json!({ "creditUsagePercent": 5, "currentPeriod": backwards })),
            credits(json!({ "creditUsagePercent": 5, "currentPeriod": unreadable })),
        ];
        for body in bodies {
            let http = Scripted::new().on("GET", CREDITS, 200, &body);
            let scope = context_at(&http, secret(&access_token(json!({}))), now());
            let error = Grok.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
        }
    }

    #[test]
    fn tiers_read_as_display_names() {
        for (raw, expected) in [
            ("SuperGrok Heavy", Some("SuperGrok Heavy")),
            ("SUPERGROK_HEAVY", Some("SuperGrok Heavy")),
            ("supergrok_lite", Some("SuperGrok Lite")),
            ("SUBSCRIPTION_TIER_X_PREMIUM_PLUS", Some("X Premium Plus")),
            ("X Premium+", Some("X Premium Plus")),
            ("X Premium", Some("X Premium")),
            ("SuperGrok+", Some("SuperGrok+")),
            ("SUPERGROK_PRO", Some("SuperGrok Pro")),
            ("free", Some("Free")),
            ("none", None),
            ("  ", None),
        ] {
            assert_eq!(plan_label(raw).as_deref(), expected, "{raw}");
        }
    }

    fn write_auth(home: &Path, auth: &Value) -> PathBuf {
        std::fs::create_dir_all(home).unwrap();
        let path = home.join("auth.json");
        std::fs::write(&path, serde_json::to_string_pretty(auth).unwrap()).unwrap();
        path
    }

    #[test]
    fn discovers_the_oidc_login_with_its_account() {
        let dir = tempfile::tempdir().unwrap();
        let token = access_token(json!({ "email": "me@example.com" }));
        let path = write_auth(
            &dir.path().join(".grok"),
            &json!({
                "https://accounts.x.ai/sign-in": {
                    "key": "session-token",
                    "email": "old@example.com",
                    "auth_mode": "session"
                },
                "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828": {
                    "key": token,
                    "refresh_token": "refresh-token",
                    "auth_mode": "oidc",
                    "user_id": "0f9c2d4e-user",
                    "email": "me@example.com",
                    "first_name": "Minh",
                    "expires_at": "2026-09-27T10:30:00Z"
                }
            }),
        );
        let logins = Grok.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, "0f9c2d4e-user");
        assert_eq!(logins[0].label.as_deref(), Some("me@example.com"));
        assert_eq!(logins[0].origin, "Grok CLI");
        assert_eq!(logins[0].location, path);
        assert_eq!(
            logins[0].secret.value(),
            &json!({ "key": token, "expiresAt": "2026-09-27T10:30:00Z" })
        );
        assert!(
            Grok.discover(&Roots::under(&dir.path().join("empty")))
                .is_empty()
        );
    }

    #[test]
    fn grok_home_wins_and_tokens_name_an_account_without_profile_fields() {
        let dir = tempfile::tempdir().unwrap();
        let custom = dir.path().join("grok-home");
        write_auth(
            &dir.path().join(".grok"),
            &json!({ "https://auth.x.ai::client": { "key": "default-token" } }),
        );
        let token = signed(json!({ "sub": "sub-7", "email": "jwt@example.com" }));
        write_auth(
            &custom,
            &json!({ "https://auth.x.ai::client": { "key": token } }),
        );
        for grok_home in [custom.to_str().unwrap(), "~/grok-home"] {
            let roots = Roots::under(dir.path()).with_var("GROK_HOME", grok_home);
            let logins = Grok.discover(&roots);
            assert_eq!(logins.len(), 1);
            assert_eq!(logins[0].identity, "jwt@example.com");
            assert_eq!(logins[0].label.as_deref(), Some("jwt@example.com"));
            assert_eq!(logins[0].location, custom.join("auth.json"));
            assert_eq!(logins[0].secret.str("/key"), Some(token.as_str()));
        }
    }

    #[test]
    fn every_row_has_its_widget() {
        let provider = Provider::new("grok@abc", "Grok");
        let descriptors = Grok.descriptors(&provider);
        let rows: Vec<_> = descriptors
            .iter()
            .map(|descriptor| {
                (
                    descriptor.id.as_str(),
                    descriptor.title(),
                    descriptor.metric_label.as_str(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                ("grok@abc.weekly", "Weekly", "Weekly limit"),
                ("grok@abc.payAsYouGo", "Extra Usage", "Pay as you go"),
                ("grok@abc.onDemand", "On-Demand", "On-Demand"),
            ]
        );
        assert_eq!(descriptors[0].limit_resources[0].key, "weekly");
        assert!(descriptors[1].limit_resources.is_empty());
        assert_eq!(descriptors[2].limit_resources[0].key, "onDemand");
    }

    #[test]
    fn a_login_without_an_account_is_named_by_its_refresh_token_hash() {
        let dir = tempfile::tempdir().unwrap();
        write_auth(
            &dir.path().join(".grok"),
            &json!({
                "https://auth.x.ai::client": { "key": "opaque", "refresh_token": "refresh-token" },
                "https://auth.x.ai::other": { "refresh_token": "no-access-token" }
            }),
        );
        let logins = Grok.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(
            logins[0].identity,
            "0eb17643d4e9261163783a420859c92c7d212fa9624106a12b510afbec266120"
        );
        assert_eq!(logins[0].label, None);
        write_auth(
            &dir.path().join(".grok"),
            &json!({ "https://auth.x.ai::client": { "refresh_token": "r" } }),
        );
        assert!(Grok.discover(&Roots::under(dir.path())).is_empty());
    }
}
