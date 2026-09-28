//! GitHub Copilot: a GitHub token that Copilot tooling or the GitHub CLI already saved on this
//! computer, read against the quota the Copilot extensions show. There is no sign-in flow and no
//! browser cookie; a token is used as saved and never renewed.
//!
//! Tokens are read in this order, one card per GitHub user (logins compared without case, and
//! every token a user has kept on the card, so a refused one falls back to the next). GitHub
//! Enterprise hosts are skipped everywhere: api.github.com would refuse their tokens.
//! - The Copilot editor plugins (JetBrains, Neovim, Xcode and other editors built on the Copilot
//!   language server): `apps.json`, then the older `hosts.json`, in
//!   `%LOCALAPPDATA%\github-copilot` on Windows and `~/.config/github-copilot` on macOS and Linux
//!   (`$XDG_CONFIG_HOME/github-copilot` when set).
//! - The GitHub CLI: `hosts.yml` in `$GH_CONFIG_DIR`, else `$XDG_CONFIG_HOME/gh`, else
//!   `%APPDATA%\GitHub CLI` on Windows and `~/.config/gh` on macOS and Linux. The token of each
//!   signed-in user comes from that file, or, when gh uses the credential store, from its
//!   go-keyring item `gh:github.com`: the Windows Credential Manager target
//!   `gh:github.com:<user>`, the macOS login keychain, the Linux Secret Service.
//! - Windows only: the GitHub sign-in of VS Code-style apps (VS Code, VS Code Insiders, VSCodium,
//!   Cursor, Windsurf), the sessions their GitHub Authentication extension keeps in secret storage.
//!
//! Signing in from Quota Control uses Copilot's own GitHub app (`Iv1.b507a08c87ecfe98`, the one
//! the Copilot editor plugins sign in with) through GitHub's device flow: the page
//! github.com/login/device opens, the one-time code is copied to the clipboard to paste there, and
//! the resulting `ghu_` token is saved in Quota Control. GitHub has no way to fill the code in by
//! itself. The Google button is the same flow with `provider=google`, which takes a GitHub account
//! that signs in with Google straight to Google, as VS Code does.
//!
//! Endpoint: `GET https://api.github.com/copilot_internal/user`, GitHub's internal account
//! endpoint, with `Authorization: token <t>` and the Copilot Chat editor headers. Paid plans
//! report `quota_snapshots`: the AI-credit pool as Credits (plus Extra Usage once extra spend is
//! on), chat and completions as unlimited. Free plans report chat and completion allowances, in
//! `quota_snapshots` or, on older answers, as `limited_user_quotas` against `monthly_quotas`.
//! Organization billing is not read.
//!
//! A personal access token pasted in Quota Control is tried on that endpoint first. When GitHub
//! refuses it there, the card reads GitHub's documented billing API instead: `GET /user` for the
//! login, then `GET /users/{login}/settings/billing/premium_request/usage?year=&month=` for this
//! month, whose `usageItems` add up to the premium requests used (`grossQuantity`) and what they
//! cost (`netAmount`, US dollars). The public `email` of that `/user` answer, when the user shows
//! one, names the account. That API takes classic tokens only and covers plans the user pays
//! for; a seat an organization pays for is billed to the organization.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{
    HttpRequest, HttpResponse, LimitResourceKind, LimitResourceSource, MetricKind, MetricLine,
    MetricValue, Provider, ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::signin::{
    self, Converted, DeviceClient, Method, Pending, SignIn, SignedIn, StartContext,
};
use crate::support::{apps, http, keyring, lines, value};

pub(crate) struct Copilot;

const NAME: &str = "Copilot";
/// The app the Accounts screen names, and the origin of logins the Copilot editor plugins saved.
const APP: &str = "GitHub Copilot";
const GH_CLI: &str = "GitHub CLI";
/// The origin of a token signed in to from Quota Control's Accounts screen.
const SIGNED_IN: &str = "Quota Control";
const USER_URL: &str = "https://api.github.com/user";

/// Copilot's own GitHub app and GitHub's device flow, as the Copilot editor plugins sign in.
const DEVICE: DeviceClient = DeviceClient {
    device_url: "https://github.com/login/device/code",
    token_url: "https://github.com/login/oauth/access_token",
    client_id: "Iv1.b507a08c87ecfe98",
    scope: "read:user",
    headers: &[("User-Agent", "QuotaControl")],
};
/// The server named in connection, status and rate-limit errors.
const GITHUB: &str = "GitHub";
const USAGE_URL: &str = "https://api.github.com/copilot_internal/user";
const HOST: &str = "github.com";
/// The go-keyring service the GitHub CLI keeps its github.com tokens under.
const GH_KEYRING: &str = "gh:github.com";
const VSCODE_EXTENSION: &str = "vscode.github-authentication";
const VSCODE_KEY: &str = "github.auth";
/// How many of a user's tokens one refresh tries while GitHub refuses them.
const MAX_TOKENS: usize = 3;
/// The fingerprint of the token that last worked, tried first next time.
const MEMO_TOKEN: &str = "copilot.token";
const MAX_FILE_BYTES: u64 = 256 * 1024;

const CREDITS: &str = "Credits";
const EXTRA_USAGE: &str = "Extra Usage";
const CHAT: &str = "Chat";
const COMPLETIONS: &str = "Completions";
const UNLIMITED: &str = "Unlimited";
const PREMIUM_REQUESTS: &str = "Premium Requests";
const SPEND: &str = "Spend";
const BILLING_URL: &str = "https://api.github.com/users";
const KEY_REFUSED: &str =
    "GitHub refused the token. Paste a classic personal access token with the user scope.";
const NO_PERSONAL_BILLING: &str = "GitHub reports no personal Copilot billing for this token: the Copilot plan is paid by an organization, or the classic token lacks the user scope.";

const UNAVAILABLE: &str = "Copilot usage data is unavailable for this account.";
const FORBIDDEN: &str = "GitHub refused Copilot usage for this login. Check that the account has Copilot, or sign in again.";
const ORG_SEAT: &str = "GitHub reports usage for organization-managed Copilot seats only in the organization's billing.";

#[async_trait]
impl Service for Copilot {
    fn id(&self) -> &'static str {
        "copilot"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://www.githubstatus.com/"),
            ProviderLink::new("Dashboard", "https://github.com/settings/billing"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::login(APP).or_api_key(ApiKeyHelp {
            env: &[],
            url: "https://github.com/settings/tokens",
            fields: &[],
        })
    }

    fn sign_in(&self) -> Option<&'static dyn SignIn> {
        Some(&Copilot)
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        logins(roots, &keyring::go_keyring)
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let id = |suffix: &str| format!("{}.{suffix}", provider.id);
        vec![
            WidgetDescriptor::percent(id("premium"), provider, CREDITS, None, None)
                .exporting_limit(
                    "premiumCredits",
                    LimitResourceKind::Consumption,
                    "credits",
                    LimitResourceSource::ProgressOrValue {
                        kind: MetricKind::Count,
                        label: None,
                    },
                    false,
                ),
            WidgetDescriptor::values(
                id("extra"),
                provider,
                EXTRA_USAGE,
                None,
                Some(MetricKind::Count),
                None,
                false,
                None,
                false,
            )
            .exporting_limit(
                "extraUsage",
                LimitResourceKind::Consumption,
                "count",
                LimitResourceSource::Value {
                    kind: MetricKind::Count,
                    label: None,
                },
                false,
            ),
            WidgetDescriptor::percent(id("chat"), provider, CHAT, None, None)
                .exporting_progress("chat", "percent"),
            WidgetDescriptor::percent(id("completions"), provider, COMPLETIONS, None, None)
                .exporting_progress("completions", "percent"),
            WidgetDescriptor::values(
                id("premiumRequests"),
                provider,
                PREMIUM_REQUESTS,
                None,
                Some(MetricKind::Count),
                None,
                true,
                None,
                false,
            )
            .exporting_limit(
                "premiumRequests",
                LimitResourceKind::Consumption,
                "count",
                LimitResourceSource::Value {
                    kind: MetricKind::Count,
                    label: None,
                },
                false,
            ),
            WidgetDescriptor::values(
                id("spend"),
                provider,
                SPEND,
                None,
                Some(MetricKind::Dollars),
                None,
                true,
                None,
                false,
            )
            .exporting_limit(
                "spend",
                LimitResourceKind::Consumption,
                "usd",
                LimitResourceSource::Value {
                    kind: MetricKind::Dollars,
                    label: None,
                },
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        if let Some(key) = context.secret.key() {
            return keyed(context, key).await;
        }
        let mut tokens = saved_tokens(context.secret);
        if tokens.is_empty() {
            return Err(http::invalid(
                "This GitHub Copilot login holds no GitHub token. Sign in to Copilot again.",
            ));
        }
        if let Some(remembered) = context.memo.get(MEMO_TOKEN, context.now).await
            && let Some(index) = tokens
                .iter()
                .position(|(token, _)| remembered.as_str() == Some(fingerprint(token).as_str()))
        {
            tokens[..=index].rotate_right(1);
        }
        let mut refused_by: Option<&str> = None;
        let mut forbidden = false;
        for (token, origin) in tokens.into_iter().take(MAX_TOKENS) {
            let response = http::send(context.http, usage_request(token), GITHUB).await?;
            if let Some(error) = rate_limit_error(&response) {
                return Err(error);
            }
            match response.status {
                401 | 403 => {
                    forbidden |= response.status == 403;
                    refused_by.get_or_insert(origin);
                }
                status if (200..300).contains(&status) => {
                    context
                        .memo
                        .put(
                            MEMO_TOKEN,
                            Value::String(fingerprint(token)),
                            Some(context.now + Duration::hours(12)),
                        )
                        .await;
                    return reading(&http::parse(&response, GITHUB)?);
                }
                _ => return Err(http::status_error(&response, GITHUB)),
            }
        }
        Err(if forbidden {
            http::invalid(FORBIDDEN)
        } else {
            expired(refused_by.unwrap_or(APP))
        })
    }
}

/// A pasted token: the Copilot quota when GitHub answers it there, else this month's premium
/// requests from the billing API.
async fn keyed(context: &FetchContext<'_>, key: &str) -> Result<Reading, SimpleProviderError> {
    let response = http::send(context.http, usage_request(key), GITHUB).await?;
    if let Some(error) = rate_limit_error(&response) {
        return Err(error);
    }
    if response.is_success() {
        return reading(&http::parse(&response, GITHUB)?);
    }
    if !matches!(response.status, 401 | 403 | 404) {
        return Err(http::status_error(&response, GITHUB));
    }
    premium_request_usage(context, key).await
}

fn api_request(url: &str, key: &str) -> HttpRequest {
    HttpRequest::get(url)
        .bearer(key)
        .header("Accept", "application/vnd.github+json")
}

/// This month's premium requests and what they cost, from GitHub's billing API.
async fn premium_request_usage(
    context: &FetchContext<'_>,
    key: &str,
) -> Result<Reading, SimpleProviderError> {
    let response = http::send(context.http, api_request(USER_URL, key), GITHUB).await?;
    if let Some(error) = rate_limit_error(&response) {
        return Err(error);
    }
    if matches!(response.status, 401 | 403) {
        return Err(http::invalid(KEY_REFUSED));
    }
    if !response.is_success() {
        return Err(http::status_error(&response, GITHUB));
    }
    let user = http::parse(&response, GITHUB)?;
    let login = value::text(&user, "/login")
        .filter(|login| {
            login
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        .ok_or_else(|| http::decoding(GITHUB))?;
    let url = format!(
        "{BILLING_URL}/{login}/settings/billing/premium_request/usage?year={}&month={}",
        context.now.year(),
        context.now.month()
    );
    let response = http::send(context.http, api_request(&url, key), GITHUB).await?;
    if let Some(error) = rate_limit_error(&response) {
        return Err(error);
    }
    if matches!(response.status, 401 | 403 | 404) {
        return Err(http::invalid(NO_PERSONAL_BILLING));
    }
    if !response.is_success() {
        return Err(http::status_error(&response, GITHUB));
    }
    let body = http::parse(&response, GITHUB)?;
    let items = body
        .get("usageItems")
        .and_then(Value::as_array)
        .ok_or_else(|| http::decoding(GITHUB))?;
    let sum = |field: &str| -> f64 {
        items
            .iter()
            .filter_map(|item| value::number(item, &format!("/{field}")))
            .sum()
    };
    Ok(Reading::new(
        Some("Copilot".to_string()),
        vec![
            lines::count_value(PREMIUM_REQUESTS, sum("grossQuantity").max(0.0), "requests"),
            lines::dollar_value(SPEND, sum("netAmount").max(0.0)),
        ],
    )
    .with_account(value::text(&user, "/email")))
}

/// The tokens a login holds, as `(token, origin)` in the order they were found.
fn saved_tokens(secret: &Secret) -> Vec<(&str, &str)> {
    secret
        .value()
        .get("tokens")
        .and_then(Value::as_array)
        .map(|tokens| {
            tokens
                .iter()
                .filter_map(|entry| {
                    Some((
                        value::text(entry, "/token")?,
                        value::text(entry, "/origin").unwrap_or(APP),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn usage_request(token: &str) -> HttpRequest {
    HttpRequest::get(USAGE_URL)
        .header("Authorization", format!("token {token}"))
        .header("Accept", "application/json")
        .header("Editor-Version", "vscode/1.96.2")
        .header("Editor-Plugin-Version", "copilot-chat/0.26.7")
        .header("User-Agent", "GitHubCopilotChat/0.26.7")
        .header("X-Github-Api-Version", "2025-04-01")
}

/// GitHub answers an exhausted rate limit with 429, or with 403 and `x-ratelimit-remaining: 0` or
/// `retry-after`; either way the card waits instead of asking to sign in again.
fn rate_limit_error(response: &HttpResponse) -> Option<SimpleProviderError> {
    let limited = response.status == 429
        || (response.status == 403
            && (response.header("x-ratelimit-remaining").map(str::trim) == Some("0")
                || response.header("retry-after").is_some()));
    limited.then(|| {
        let too_many = HttpResponse {
            status: 429,
            headers: HashMap::new(),
            body: Vec::new(),
        };
        http::status_error(&too_many, GITHUB)
    })
}

#[async_trait]
impl SignIn for Copilot {
    fn methods(&self) -> &'static [Method] {
        &[Method::GitHub, Method::Google]
    }

    async fn start(
        &self,
        method: Method,
        context: &StartContext,
    ) -> Result<Pending, SimpleProviderError> {
        let mut pending = signin::start_device(&DEVICE, context, signed_in).await?;
        if method == Method::Google
            && let Ok(mut page) = url::Url::parse(&pending.url)
        {
            page.query_pairs_mut().append_pair("provider", "google");
            pending.url = page.to_string();
        }
        Ok(pending)
    }
}

/// The GitHub account a device sign-in's token belongs to, as a card the editor plugins' login of
/// the same user would share.
fn signed_in(http: uc_core::SharedHttpClient, tokens: Value) -> Converted {
    Box::pin(async move {
        let token = tokens
            .get("access_token")
            .and_then(Value::as_str)
            .and_then(usable)
            .ok_or_else(|| http::invalid("GitHub returned no usable token."))?;
        let user = http::json(
            &http,
            HttpRequest::get(USER_URL)
                .header("Authorization", format!("token {token}"))
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "QuotaControl"),
            GITHUB,
        )
        .await?;
        let login = value::text(&user, "/login")
            .ok_or_else(|| http::invalid("GitHub did not say which account signed in."))?;
        Ok(SignedIn {
            identity: login.to_lowercase(),
            label: Some(login.to_string()),
            document: json!({ "tokens": [{ "token": token, "origin": SIGNED_IN }] }),
        })
    })
}

/// What to do when GitHub refused every token: sign in again where the first one came from.
fn expired(origin: &str) -> SimpleProviderError {
    http::expired(match origin {
        APP => "The GitHub Copilot login expired. Sign in to Copilot in your editor again.".into(),
        GH_CLI => "The GitHub CLI login expired. Run gh auth login to renew it.".into(),
        SIGNED_IN => "The GitHub sign-in expired. Sign in to Copilot again in Accounts.".into(),
        app => format!("The {app} login expired. Open {app} and sign in to GitHub again."),
    })
}

/// A token's short SHA-256: which token worked is remembered without keeping the token.
fn fingerprint(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One `quota_snapshots` bucket.
#[derive(Clone, Copy)]
enum Bucket {
    /// A metered allowance: the percent of it used.
    Used(f64),
    /// Unlimited: GitHub's `unlimited` flag or its `-1` sentinel.
    Unlimited,
    /// Missing, or a zero-entitlement placeholder with no allowance to show a share of (Credits on
    /// a free plan, every bucket of an organization-managed seat).
    Empty,
}

fn bucket(bucket: Option<&Value>) -> Bucket {
    let Some(bucket) = bucket.filter(|bucket| bucket.is_object()) else {
        return Bucket::Empty;
    };
    let entitlement = value::number(bucket, "/entitlement");
    let remaining = value::number(bucket, "/remaining");
    if value::flag(bucket, "/unlimited") == Some(true)
        || entitlement == Some(-1.0)
        || remaining == Some(-1.0)
    {
        return Bucket::Unlimited;
    }
    if entitlement == Some(0.0) {
        return Bucket::Empty;
    }
    match (
        value::number(bucket, "/percent_remaining"),
        entitlement,
        remaining,
    ) {
        (Some(left), _, _) => Bucket::Used(100.0 - left),
        (None, Some(entitlement), Some(remaining)) if entitlement > 0.0 => {
            Bucket::Used(100.0 - remaining / entitlement * 100.0)
        }
        _ => Bucket::Empty,
    }
}

/// The plan and meters `/copilot_internal/user` reports.
fn reading(body: &Value) -> Result<Reading, SimpleProviderError> {
    if !body.is_object() {
        return Err(http::decoding(GITHUB));
    }
    let plan = plan(body);
    let resets = resets_at(body);
    let premium = body.pointer("/quota_snapshots/premium_interactions");
    let buckets = [
        (CREDITS, bucket(premium)),
        (CHAT, bucket(body.pointer("/quota_snapshots/chat"))),
        (
            COMPLETIONS,
            bucket(body.pointer("/quota_snapshots/completions")),
        ),
    ];
    // A metered credit pool is a paid plan, whose chat and completions GitHub reports as
    // unlimited; a free plan meters those two and has no credits.
    if buckets
        .iter()
        .any(|(_, bucket)| matches!(bucket, Bucket::Used(_)))
    {
        let paid = matches!(buckets[0].1, Bucket::Used(_));
        let mut meters = Vec::new();
        for (label, bucket) in buckets {
            match bucket {
                Bucket::Used(used) => {
                    meters.push(lines::percent(label, used, resets, Some(lines::MONTH_MS)));
                    if label == CREDITS {
                        meters.extend(extra_usage(premium));
                    }
                }
                Bucket::Unlimited if paid => meters.push(lines::badge(label, UNLIMITED)),
                Bucket::Unlimited | Bucket::Empty => {}
            }
        }
        return Ok(Reading::new(plan, meters));
    }
    let legacy: Vec<MetricLine> = [(CHAT, "chat"), (COMPLETIONS, "completions")]
        .into_iter()
        .filter_map(|(label, key)| limited_meter(label, body, key, resets))
        .collect();
    if !legacy.is_empty() {
        return Ok(Reading::new(plan, legacy));
    }
    // An organization-managed seat: token-based billing and no allowance at all. Copilot Free is
    // billed by tokens too, but always has chat and completion allowances.
    let org_seat = !is_free(body)
        && (value::flag(body, "/token_based_billing") == Some(true)
            || premium.and_then(|premium| value::flag(premium, "/token_based_billing"))
                == Some(true));
    if org_seat {
        let personal: Vec<MetricLine> = personal_credits(premium).into_iter().collect();
        let warning = personal.is_empty().then(|| ORG_SEAT.to_string());
        return Ok(Reading::new(plan, personal).with_warning(warning));
    }
    let unlimited: Vec<MetricLine> = buckets
        .iter()
        .filter(|(_, bucket)| matches!(bucket, Bucket::Unlimited))
        .map(|(label, _)| lines::badge(label, UNLIMITED))
        .collect();
    if !unlimited.is_empty() {
        return Ok(Reading::new(plan, unlimited));
    }
    Err(http::not_available(UNAVAILABLE))
}

/// Premium use beyond the included credits, once extra spend is on (`overage_permitted`); a real
/// zero is shown. GitHub states no cap here, so it is a count rather than a meter.
fn extra_usage(premium: Option<&Value>) -> Option<MetricLine> {
    let premium = premium?;
    if value::flag(premium, "/overage_permitted") != Some(true) {
        return None;
    }
    let overage = value::number(premium, "/overage_count")
        .unwrap_or(0.0)
        .max(0.0);
    Some(lines::values(
        EXTRA_USAGE,
        vec![MetricValue::new(overage, MetricKind::Count)],
    ))
}

/// Older free-plan answers: what is left of a monthly allowance (`limited_user_quotas`) against the
/// allowance (`monthly_quotas`).
fn limited_meter(
    label: &str,
    body: &Value,
    key: &str,
    resets: Option<DateTime<Utc>>,
) -> Option<MetricLine> {
    let total =
        value::number(body, &format!("/monthly_quotas/{key}")).filter(|total| *total > 0.0)?;
    let remaining = value::number(body, &format!("/limited_user_quotas/{key}"))?;
    lines::percent_of(
        label,
        (total - remaining).max(0.0),
        total,
        resets,
        Some(lines::MONTH_MS),
    )
}

/// The seat's own credit use on an organization-managed seat, which has no allotment to show a
/// share of.
fn personal_credits(premium: Option<&Value>) -> Option<MetricLine> {
    let used = value::number(premium?, "/credits_used").filter(|used| *used > 0.0)?;
    Some(lines::values(
        CREDITS,
        vec![MetricValue::new(used, MetricKind::Count)],
    ))
}

/// `copilot_plan` as a name (`individual_pro` → `Individual Pro`), `access_type_sku` when it is
/// missing. The Copilot Free SKU reads `Free`, since GitHub reports free and paid individual
/// accounts alike as `individual`.
fn plan(body: &Value) -> Option<String> {
    if is_free(body) {
        return Some("Free".into());
    }
    value::text(body, "/copilot_plan")
        .or_else(|| value::text(body, "/access_type_sku"))
        .and_then(lines::plan_name)
}

/// Whether the account is on Copilot Free.
fn is_free(body: &Value) -> bool {
    value::text(body, "/access_type_sku")
        .is_some_and(|sku| sku.eq_ignore_ascii_case("free_limited_copilot"))
}

fn resets_at(body: &Value) -> Option<DateTime<Utc>> {
    [
        "/quota_reset_date_utc",
        "/quota_reset_date",
        "/limited_user_reset_date",
    ]
    .iter()
    .find_map(|pointer| value::time(body, pointer))
}

/// A GitHub token one app saved, and the user it belongs to when the app says.
struct Found {
    login: Option<String>,
    token: String,
    origin: String,
    location: PathBuf,
}

/// Every github.com login under `roots`, one per GitHub user. `keyring` reads a go-keyring item
/// (`service`, `user`) from the credential store.
fn logins(roots: &Roots, keyring: &dyn Fn(&str, &str) -> Option<String>) -> Vec<Login> {
    let mut found = editor_tokens(roots);
    found.extend(gh_tokens(roots, keyring));
    found.extend(vscode_tokens(roots));
    group(found)
}

struct Account {
    identity: String,
    label: Option<String>,
    origin: String,
    location: PathBuf,
    tokens: Vec<(String, String)>,
}

/// The found tokens as logins, one per GitHub user, keeping each user's tokens in the order found.
/// A token saved without its user joins the user another app saved the same token for; with no
/// user anywhere it stands alone under the token's SHA-256.
fn group(found: Vec<Found>) -> Vec<Login> {
    let mut owners: HashMap<String, String> = HashMap::new();
    for entry in &found {
        if let Some(login) = &entry.login {
            owners
                .entry(entry.token.clone())
                .or_insert_with(|| login.clone());
        }
    }
    let mut accounts: Vec<Account> = Vec::new();
    for entry in found {
        let login = entry.login.or_else(|| owners.get(&entry.token).cloned());
        let identity = login
            .as_deref()
            .map_or_else(|| token_hash(&entry.token), str::to_lowercase);
        let index = match accounts
            .iter()
            .position(|account| account.identity == identity)
        {
            Some(index) => index,
            None => {
                accounts.push(Account {
                    identity,
                    label: None,
                    origin: entry.origin.clone(),
                    location: entry.location.clone(),
                    tokens: Vec::new(),
                });
                accounts.len() - 1
            }
        };
        let account = &mut accounts[index];
        if account.label.is_none() {
            account.label = login;
        }
        if !account
            .tokens
            .iter()
            .any(|(token, _)| *token == entry.token)
        {
            account.tokens.push((entry.token, entry.origin));
        }
    }
    accounts
        .into_iter()
        .map(|account| {
            let tokens: Vec<Value> = account
                .tokens
                .into_iter()
                .map(|(token, origin)| json!({"token": token, "origin": origin}))
                .collect();
            Login::new(
                account.identity,
                account.origin,
                &account.location,
                Secret::new(json!({ "tokens": tokens })),
            )
            .with_label(account.label)
        })
        .collect()
}

fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A token fit for an `Authorization` header: non-empty printable ASCII without spaces.
fn usable(token: &str) -> Option<String> {
    let token = token.trim();
    (!token.is_empty() && token.chars().all(|character| character.is_ascii_graphic()))
        .then(|| token.to_string())
}

/// Where the Copilot editor plugins keep their logins: `$XDG_CONFIG_HOME/github-copilot` when set,
/// `%LOCALAPPDATA%\github-copilot` on Windows, and `~/.config/github-copilot`.
fn editor_dirs(roots: &Roots) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(config) = roots.var("XDG_CONFIG_HOME") {
        candidates.push(PathBuf::from(config).join("github-copilot"));
    }
    if cfg!(windows) {
        candidates.push(roots.local_data.join("github-copilot"));
    }
    candidates.push(roots.home.join(".config").join("github-copilot"));
    let mut dirs: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        if !dirs.contains(&candidate) {
            dirs.push(candidate);
        }
    }
    dirs
}

/// The Copilot editor plugins' logins: `apps.json` (keys `github.com:<app id>`), then the older
/// `hosts.json` (key `github.com`), each entry `{user, oauth_token}`.
fn editor_tokens(roots: &Roots) -> Vec<Found> {
    let mut found = Vec::new();
    for dir in editor_dirs(roots) {
        for file in ["apps.json", "hosts.json"] {
            let path = dir.join(file);
            let Some(document) = value::read_json(&path, MAX_FILE_BYTES) else {
                continue;
            };
            let Some(entries) = document.as_object() else {
                continue;
            };
            for (host, entry) in entries {
                if host != HOST && !host.starts_with("github.com:") {
                    continue;
                }
                let Some(token) = value::text(entry, "/oauth_token").and_then(usable) else {
                    continue;
                };
                found.push(Found {
                    login: value::text(entry, "/user").map(str::to_string),
                    token,
                    origin: APP.into(),
                    location: path.clone(),
                });
            }
        }
    }
    found
}

/// The GitHub CLI's config folder, found the way gh finds it: `$GH_CONFIG_DIR`, else
/// `$XDG_CONFIG_HOME/gh`, else `%APPDATA%\GitHub CLI` on Windows and `~/.config/gh` elsewhere.
fn gh_dir(roots: &Roots) -> PathBuf {
    if let Some(dir) = roots.var("GH_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(config) = roots.var("XDG_CONFIG_HOME") {
        return PathBuf::from(config).join("gh");
    }
    if cfg!(windows) {
        roots.app_data.join("GitHub CLI")
    } else {
        roots.home.join(".config").join("gh")
    }
}

/// Every github.com user the GitHub CLI is signed in to, the active one first, with the token gh
/// keeps in `hosts.yml` or else in its go-keyring item for that user.
fn gh_tokens(roots: &Roots, keyring: &dyn Fn(&str, &str) -> Option<String>) -> Vec<Found> {
    let path = gh_dir(roots).join("hosts.yml");
    let Some(text) = read_text(&path) else {
        return Vec::new();
    };
    let hosts = GhHosts::parse(&text);
    if !hosts.present {
        return Vec::new();
    }
    let mut users = hosts.users.clone();
    if let Some(active) = &hosts.active {
        let entry = match users
            .iter()
            .position(|(name, _)| name.eq_ignore_ascii_case(active))
        {
            Some(index) => users.remove(index),
            None => (active.clone(), None),
        };
        users.insert(0, entry);
    }
    if users.is_empty() {
        return hosts
            .active_token
            .clone()
            .or_else(|| keyring(GH_KEYRING, ""))
            .as_deref()
            .and_then(usable)
            .map(|token| Found {
                login: None,
                token,
                origin: GH_CLI.into(),
                location: path.clone(),
            })
            .into_iter()
            .collect();
    }
    // gh also keeps the active user's token under an empty account name (older versions only
    // there). It is read only while gh knows a single user: a macOS keychain lookup with an empty
    // account may match any user's item.
    let single = users.len() == 1;
    users
        .into_iter()
        .filter_map(|(name, in_users)| {
            let active = hosts
                .active
                .as_deref()
                .is_some_and(|active| active.eq_ignore_ascii_case(&name));
            let in_file = in_users.or_else(|| {
                if active {
                    hosts.active_token.clone()
                } else {
                    None
                }
            });
            let (token, location) = match in_file {
                Some(token) => (token, path.clone()),
                None => {
                    let token = keyring(GH_KEYRING, &name).or_else(|| {
                        if active && single {
                            keyring(GH_KEYRING, "")
                        } else {
                            None
                        }
                    })?;
                    (token, PathBuf::from(format!("{GH_KEYRING}:{name}")))
                }
            };
            Some(Found {
                token: usable(&token)?,
                login: Some(name),
                origin: GH_CLI.into(),
                location,
            })
        })
        .collect()
}

fn read_text(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// What the GitHub CLI's `hosts.yml` says about github.com. It holds tokens, so it prints only in
/// tests.
#[derive(Default, PartialEq)]
#[cfg_attr(test, derive(Debug))]
struct GhHosts {
    /// Whether the file has a `github.com` block at all.
    present: bool,
    /// The active user (`user`) and the token kept for it in the file (`oauth_token`).
    active: Option<String>,
    active_token: Option<String>,
    /// Every signed-in user (`users`) in file order, with the token kept for it in the file.
    users: Vec<(String, Option<String>)>,
}

impl GhHosts {
    /// Reads the indented `key: value` mapping gh writes, following each key's path from the top
    /// level, so a GitHub Enterprise block or the nested `users` map cannot be mistaken for the
    /// github.com keys. It is a line reader for that file, not a YAML parser.
    fn parse(text: &str) -> Self {
        let mut hosts = Self::default();
        let mut parents: Vec<(usize, String)> = Vec::new();
        for line in text.trim_start_matches('\u{feff}').lines() {
            let content = line.trim_end();
            let trimmed = content.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let indent = content.len() - trimmed.len();
            let Some((key, scalar)) = key_value(trimmed) else {
                continue;
            };
            while parents.last().is_some_and(|(depth, _)| *depth >= indent) {
                parents.pop();
            }
            let path: Vec<&str> = parents
                .iter()
                .map(|(_, parent)| parent.as_str())
                .chain(std::iter::once(key.as_str()))
                .collect();
            match (path.as_slice(), scalar.as_deref()) {
                ([HOST], _) => hosts.present = true,
                ([HOST, "user"], Some(user)) => hosts.active = Some(user.to_string()),
                ([HOST, "oauth_token"], Some(token)) => {
                    hosts.active_token = Some(token.to_string())
                }
                ([HOST, "users", name], _) => hosts.users.push((name.to_string(), None)),
                ([HOST, "users", name, "oauth_token"], Some(token)) => {
                    if let Some(user) = hosts.users.iter_mut().find(|user| user.0 == **name) {
                        user.1 = Some(token.to_string());
                    }
                }
                _ => {}
            }
            if scalar.is_none() {
                parents.push((indent, key));
            }
        }
        hosts
    }
}

/// `key: value`, or `key:` opening a nested mapping, with quotes removed; `None` for a line that is
/// not a mapping entry. An empty, `null` or `~` value counts as no value.
fn key_value(line: &str) -> Option<(String, Option<String>)> {
    let (key, rest) = match line.strip_suffix(':') {
        Some(key) => (key, ""),
        None => line.split_once(": ")?,
    };
    let key = unquote(key.trim());
    if key.is_empty() {
        return None;
    }
    let value = unquote(rest.split(" #").next().unwrap_or_default().trim());
    let value = (!matches!(value, "" | "null" | "~")).then(|| value.to_string());
    Some((key.to_string(), value))
}

fn unquote(text: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    text
}

/// Windows only: the GitHub sign-in of VS Code-style apps, the sessions
/// (`[{accessToken, account: {label}}]`) their GitHub Authentication extension keeps in secret
/// storage. Elsewhere that storage stays locked behind the login keychain.
fn vscode_tokens(roots: &Roots) -> Vec<Found> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let mut found = Vec::new();
    for app in apps::VSCODE_APPS {
        let Some(stored) = apps::secret(roots, app, VSCODE_EXTENSION, VSCODE_KEY) else {
            continue;
        };
        let Ok(Value::Array(sessions)) = serde_json::from_str::<Value>(&stored) else {
            continue;
        };
        for session in &sessions {
            let Some(token) = value::text(session, "/accessToken").and_then(usable) else {
                continue;
            };
            found.push(Found {
                login: value::text(session, "/account/label").map(str::to_string),
                token,
                origin: app_name(app).into(),
                location: apps::state_db(roots, app),
            });
        }
    }
    found
}

/// The name an app folder stands for on the Accounts screen.
fn app_name(folder: &str) -> &str {
    match folder {
        "Code" => "VS Code",
        "Code - Insiders" => "VS Code Insiders",
        other => other,
    }
}

#[cfg(test)]
mod sign_in_tests {
    use super::*;
    use crate::testing::{Scripted, header};

    #[tokio::test]
    async fn a_device_token_becomes_the_github_users_card() {
        let http = Scripted::new().on("GET", USER_URL, 200, r#"{"login":"OctoCat","id":583231}"#);
        let account = signed_in(
            http.shared(),
            json!({"access_token": "ghu_abc123", "token_type": "bearer"}),
        )
        .await
        .unwrap();
        assert_eq!(account.identity, "octocat");
        assert_eq!(account.label.as_deref(), Some("OctoCat"));
        assert_eq!(
            account.document,
            json!({"tokens": [{"token": "ghu_abc123", "origin": SIGNED_IN}]})
        );
        let request = &http.requests()[0];
        assert_eq!(header(request, "Authorization"), Some("token ghu_abc123"));
        let secret = Secret::owned(account.document);
        assert_eq!(saved_tokens(&secret), vec![("ghu_abc123", SIGNED_IN)]);
        assert!(
            expired(SIGNED_IN)
                .message
                .contains("Sign in to Copilot again in Accounts")
        );
    }

    #[tokio::test]
    async fn the_google_button_asks_github_to_go_straight_to_google() {
        let http = Scripted::new().on(
            "POST",
            DEVICE.device_url,
            200,
            r#"{"device_code":"dc","user_code":"WDJB-MJHT","verification_uri":"https://github.com/login/device","interval":5,"expires_in":900}"#,
        );
        let context = StartContext {
            http: http.shared(),
            language: uc_core::loopback::LoginLanguage::English,
            product: NAME,
        };
        let google = Copilot.start(Method::Google, &context).await.unwrap();
        assert_eq!(
            google.url,
            "https://github.com/login/device?provider=google"
        );
        assert_eq!(google.user_code.as_deref(), Some("WDJB-MJHT"));
        let github = Copilot.start(Method::GitHub, &context).await.unwrap();
        assert_eq!(github.url, "https://github.com/login/device");
        let body = String::from_utf8(http.requests()[0].body.clone().unwrap()).unwrap();
        assert!(body.contains("client_id=Iv1.b507a08c87ecfe98"));
        assert!(body.contains("scope=read%3Auser"));
    }

    #[tokio::test]
    async fn a_token_github_does_not_recognize_is_not_saved() {
        let http = Scripted::new().on("GET", USER_URL, 401, "{}");
        let refused = signed_in(http.shared(), json!({"access_token": "ghu_x"})).await;
        assert!(refused.is_err());
        let empty = signed_in(http.shared(), json!({"access_token": "has space"})).await;
        assert!(empty.is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use std::cell::RefCell;
    use uc_core::ErrorCategory;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn secret(tokens: &[(&str, &str)]) -> Value {
        let tokens: Vec<Value> = tokens
            .iter()
            .map(|(token, origin)| json!({"token": token, "origin": origin}))
            .collect();
        json!({ "tokens": tokens })
    }

    fn at(year: i32, month: u32, day: u32) -> Option<DateTime<Utc>> {
        Some(Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap())
    }

    fn count(label: &str, number: f64) -> MetricLine {
        lines::values(label, vec![MetricValue::new(number, MetricKind::Count)])
    }

    /// A Copilot Pro+ answer: a metered credit pool with extra spend on, chat unlimited by flag,
    /// completions by the `-1` sentinel, and a stale free-plan allowance that must be ignored.
    const PAID: &str = r#"{
        "login": "octocat",
        "access_type_sku": "plus_monthly_subscriber_quota",
        "copilot_plan": "individual_pro",
        "chat_enabled": true,
        "quota_reset_date": "2026-10-01",
        "quota_reset_date_utc": "2026-10-01T00:00:00.000Z",
        "limited_user_quotas": {"chat": 100},
        "monthly_quotas": {"chat": 500},
        "quota_snapshots": {
            "chat": {"entitlement": 0, "overage_count": 0, "overage_permitted": false, "percent_remaining": 100.0, "quota_id": "chat", "quota_remaining": 0.0, "remaining": 0, "unlimited": true},
            "completions": {"entitlement": -1, "overage_count": 0, "overage_permitted": false, "quota_id": "completions", "remaining": -1},
            "premium_interactions": {"entitlement": 1500, "overage_count": 36, "overage_permitted": true, "percent_remaining": 41.0, "quota_id": "premium_interactions", "quota_remaining": 615.0, "remaining": 615, "unlimited": false}
        }
    }"#;

    /// A Copilot Free answer as GitHub sends it today.
    const FREE: &str = r#"{
        "access_type_sku": "free_limited_copilot",
        "copilot_plan": "individual",
        "token_based_billing": true,
        "quota_reset_date": "2026-10-27",
        "quota_snapshots": {
            "chat": {"entitlement": 200, "remaining": 182, "percent_remaining": 91.0, "overage_permitted": false, "token_based_billing": true},
            "completions": {"entitlement": 2000, "remaining": 1990, "percent_remaining": 99.5, "overage_permitted": false, "token_based_billing": true},
            "premium_interactions": {"entitlement": 0, "remaining": 0, "percent_remaining": 0.0, "overage_permitted": false, "token_based_billing": true}
        }
    }"#;

    /// An organization-managed Copilot Business seat: zero-entitlement placeholders only.
    const ORG: &str = r#"{
        "access_type_sku": "copilot_for_business_seat",
        "copilot_plan": "business",
        "token_based_billing": true,
        "quota_snapshots": {
            "chat": {"entitlement": 0, "remaining": 0, "unlimited": true, "overage_permitted": false, "overage_count": 0},
            "completions": {"entitlement": 0, "remaining": 0, "unlimited": true, "overage_permitted": false, "overage_count": 0},
            "premium_interactions": {"entitlement": 0, "remaining": 0, "unlimited": true, "overage_permitted": true, "overage_count": 0, "credits_used": 2111}
        }
    }"#;

    async fn read(body: &str) -> Result<Reading, SimpleProviderError> {
        let http = Scripted::new().on("GET", USAGE_URL, 200, body);
        let scope = context_at(&http, secret(&[("gho_saved", APP)]), now());
        Copilot.fetch(&scope.context()).await
    }

    const BILLING: &str = "https://api.github.com/users/octocat/settings/billing/premium_request/usage?year=2026&month=9";

    #[tokio::test]
    async fn a_pasted_token_github_answers_on_the_quota_endpoint_reads_the_quota() {
        let http = Scripted::new().on("GET", USAGE_URL, 200, PAID);
        let scope = context_at(&http, json!({"apiKey": "ghp_token"}), now());
        let reading = Copilot.fetch(&scope.context()).await.unwrap();
        assert!(reading.lines.iter().any(|line| line.label() == CREDITS));
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_refused_pasted_token_reads_this_months_premium_requests_from_billing() {
        let http = Scripted::new()
            .on("GET", USAGE_URL, 404, "{}")
            .on(
                "GET",
                USER_URL,
                200,
                r#"{"login":"octocat","email":"octocat@example.com"}"#,
            )
            .on(
                "GET",
                BILLING,
                200,
                r#"{"timePeriod":{"year":2026,"month":9},"user":"octocat","usageItems":[
                    {"product":"Copilot","sku":"Copilot Premium Request","model":"Claude",
                     "unitType":"requests","pricePerUnit":0.04,"grossQuantity":120,
                     "grossAmount":4.8,"discountQuantity":100,"discountAmount":4.0,
                     "netQuantity":20,"netAmount":0.8},
                    {"product":"Copilot","sku":"Copilot Premium Request","model":"GPT",
                     "unitType":"requests","pricePerUnit":0.04,"grossQuantity":30,
                     "grossAmount":1.2,"discountQuantity":30,"discountAmount":1.2,
                     "netQuantity":0,"netAmount":0}]}"#,
            );
        let scope = context_at(&http, json!({"apiKey": "ghp_token"}), now());
        let reading = Copilot.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::count_value(PREMIUM_REQUESTS, 150.0, "requests"),
                lines::dollar_value(SPEND, 0.8),
            ]
        );
        assert_eq!(reading.account.as_deref(), Some("octocat@example.com"));
        let requests = http.requests();
        assert_eq!(
            header(&requests[2], "Authorization"),
            Some("Bearer ghp_token")
        );
    }

    #[tokio::test]
    async fn an_organization_paid_seat_explains_why_the_token_shows_nothing() {
        let http = Scripted::new()
            .on("GET", USAGE_URL, 403, "{}")
            .on("GET", USER_URL, 200, r#"{"login":"octocat"}"#)
            .on("GET", BILLING, 404, "{}");
        let scope = context_at(&http, json!({"apiKey": "ghp_token"}), now());
        let error = Copilot.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, NO_PERSONAL_BILLING);
    }

    #[tokio::test]
    async fn a_token_github_does_not_accept_at_all_asks_for_a_classic_token() {
        let http = Scripted::new()
            .on("GET", USAGE_URL, 401, "{}")
            .on("GET", USER_URL, 401, "{}");
        let scope = context_at(&http, json!({"apiKey": "bad"}), now());
        let error = Copilot.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, KEY_REFUSED);
    }

    #[tokio::test]
    async fn reads_the_credit_pool_of_a_paid_plan() {
        let http = Scripted::new().on("GET", USAGE_URL, 200, PAID);
        let scope = context_at(&http, secret(&[("gho_editor", APP)]), now());
        let reading = Copilot.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Individual Pro".into()),
                vec![
                    lines::percent(CREDITS, 59.0, at(2026, 10, 1), Some(lines::MONTH_MS)),
                    count(EXTRA_USAGE, 36.0),
                    lines::badge(CHAT, UNLIMITED),
                    lines::badge(COMPLETIONS, UNLIMITED),
                ],
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, USAGE_URL);
        assert!(request.body.is_none());
        for (name, value) in [
            ("Authorization", "token gho_editor"),
            ("Accept", "application/json"),
            ("Editor-Version", "vscode/1.96.2"),
            ("Editor-Plugin-Version", "copilot-chat/0.26.7"),
            ("User-Agent", "GitHubCopilotChat/0.26.7"),
            ("X-Github-Api-Version", "2025-04-01"),
        ] {
            assert_eq!(header(request, name), Some(value), "{name}");
        }
    }

    #[tokio::test]
    async fn extra_usage_needs_extra_spend_turned_on() {
        let body = PAID.replace(
            r#""overage_permitted": true"#,
            r#""overage_permitted": false"#,
        );
        let reading = read(&body).await.unwrap();
        let labels: Vec<&str> = reading.lines.iter().map(MetricLine::label).collect();
        assert_eq!(labels, [CREDITS, CHAT, COMPLETIONS]);
    }

    #[tokio::test]
    async fn reads_chat_and_completions_of_the_free_plan() {
        let reading = read(FREE).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Free".into()),
                vec![
                    lines::percent(CHAT, 9.0, at(2026, 10, 27), Some(lines::MONTH_MS)),
                    lines::percent(COMPLETIONS, 0.5, at(2026, 10, 27), Some(lines::MONTH_MS)),
                ],
            )
        );
    }

    #[tokio::test]
    async fn reads_the_older_free_plan_allowances() {
        let reading = read(
            r#"{"copilot_plan":"individual","limited_user_quotas":{"chat":25,"completions":1500},
                "monthly_quotas":{"chat":50,"completions":2000},"limited_user_reset_date":"2026-10-15"}"#,
        )
        .await
        .unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Individual".into()),
                vec![
                    lines::percent(CHAT, 50.0, at(2026, 10, 15), Some(lines::MONTH_MS)),
                    lines::percent(COMPLETIONS, 25.0, at(2026, 10, 15), Some(lines::MONTH_MS)),
                ],
            )
        );
    }

    #[tokio::test]
    async fn an_organization_seat_shows_its_own_credits_or_explains_why_it_is_empty() {
        let reading = read(ORG).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(Some("Business".into()), vec![count(CREDITS, 2111.0)])
        );
        let reading = read(&ORG.replace(r#", "credits_used": 2111"#, ""))
            .await
            .unwrap();
        assert_eq!(
            reading,
            Reading::new(Some("Business".into()), Vec::new()).with_warning(Some(ORG_SEAT.into()))
        );
    }

    #[tokio::test]
    async fn an_all_unlimited_plan_says_so() {
        let reading = read(
            r#"{"copilot_plan":"enterprise","quota_snapshots":{
                "premium_interactions":{"entitlement":-1,"remaining":-1},
                "chat":{"unlimited":true},"completions":{"unlimited":true}}}"#,
        )
        .await
        .unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Enterprise".into()),
                vec![
                    lines::badge(CREDITS, UNLIMITED),
                    lines::badge(CHAT, UNLIMITED),
                    lines::badge(COMPLETIONS, UNLIMITED),
                ],
            )
        );
    }

    #[tokio::test]
    async fn an_answer_without_usage_is_unavailable_or_unreadable() {
        for body in [
            r#"{"copilot_plan":"pro"}"#,
            r#"{"access_type_sku":"free_limited_copilot","copilot_plan":"individual","token_based_billing":true}"#,
        ] {
            let error = read(body).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::NotAvailable, "{body}");
            assert_eq!(error.message, UNAVAILABLE);
        }
        for body in ["[]", "<html>"] {
            let error = read(body).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
        }
    }

    #[tokio::test]
    async fn a_refused_login_asks_to_sign_in_again_where_it_came_from() {
        for (origin, message) in [
            (
                GH_CLI,
                "The GitHub CLI login expired. Run gh auth login to renew it.",
            ),
            (
                APP,
                "The GitHub Copilot login expired. Sign in to Copilot in your editor again.",
            ),
            (
                "VS Code",
                "The VS Code login expired. Open VS Code and sign in to GitHub again.",
            ),
        ] {
            let http = Scripted::new().on(
                "GET",
                USAGE_URL,
                401,
                r#"{"message":"Bad credentials","status":"401"}"#,
            );
            let scope = context_at(&http, secret(&[("gho_revoked", origin)]), now());
            let error = Copilot.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthExpired);
            assert_eq!(error.message, message);
        }
    }

    #[tokio::test]
    async fn a_refused_token_falls_back_to_the_next_and_remembers_it() {
        let http = Scripted::new()
            .on("GET", USAGE_URL, 401, r#"{"message":"Bad credentials"}"#)
            .on("GET", USAGE_URL, 200, FREE);
        let scope = context_at(
            &http,
            secret(&[("gho_old", APP), ("gho_new", GH_CLI)]),
            now(),
        );
        let reading = Copilot.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Free"));
        Copilot.fetch(&scope.context()).await.unwrap();
        let used: Vec<String> = http
            .requests()
            .iter()
            .map(|request| header(request, "Authorization").unwrap().to_string())
            .collect();
        assert_eq!(used, ["token gho_old", "token gho_new", "token gho_new"]);
    }

    #[tokio::test]
    async fn at_most_three_tokens_are_tried() {
        let http = Scripted::new().on("GET", USAGE_URL, 401, "{}");
        let scope = context_at(
            &http,
            secret(&[
                ("a1", "VS Code"),
                ("a2", APP),
                ("a3", GH_CLI),
                ("a4", GH_CLI),
            ]),
            now(),
        );
        let error = Copilot.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "The VS Code login expired. Open VS Code and sign in to GitHub again."
        );
        assert_eq!(http.requests().len(), 3);
    }

    #[tokio::test]
    async fn a_forbidden_login_is_reported_as_invalid() {
        let http = Scripted::new().on(
            "GET",
            USAGE_URL,
            403,
            r#"{"message":"Resource not accessible by integration"}"#,
        );
        let scope = context_at(&http, secret(&[("ghu_app", APP)]), now());
        let error = Copilot.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(error.message, FORBIDDEN);
    }

    #[tokio::test]
    async fn rate_limits_wait_without_trying_other_tokens() {
        let limited = [
            Scripted::new().on("GET", USAGE_URL, 429, "{}"),
            Scripted::new().on_with_headers(
                "GET",
                USAGE_URL,
                403,
                &[("X-RateLimit-Remaining", "0")],
                r#"{"message":"API rate limit exceeded"}"#,
            ),
            Scripted::new().on_with_headers(
                "GET",
                USAGE_URL,
                403,
                &[("Retry-After", "60")],
                r#"{"message":"You have exceeded a secondary rate limit"}"#,
            ),
        ];
        for http in limited {
            let scope = context_at(&http, secret(&[("gho_a", APP), ("gho_b", GH_CLI)]), now());
            let error = Copilot.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::RateLimited);
            assert_eq!(http.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn a_server_error_is_not_retried_with_other_tokens() {
        let http = Scripted::new().on("GET", USAGE_URL, 502, "bad gateway");
        let scope = context_at(&http, secret(&[("gho_a", APP), ("gho_b", GH_CLI)]), now());
        let error = Copilot.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(http.requests().len(), 1);
    }

    #[test]
    fn descriptors_match_the_lines_and_the_limits_contract() {
        let provider = Provider::new("copilot@abc", NAME);
        let descriptors = Copilot.descriptors(&provider);
        let ids: Vec<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "copilot@abc.premium",
                "copilot@abc.extra",
                "copilot@abc.chat",
                "copilot@abc.completions",
                "copilot@abc.premiumRequests",
                "copilot@abc.spend"
            ]
        );
        let labels: Vec<&str> = descriptors
            .iter()
            .map(|d| d.metric_label.as_str())
            .collect();
        assert_eq!(
            labels,
            [
                CREDITS,
                EXTRA_USAGE,
                CHAT,
                COMPLETIONS,
                PREMIUM_REQUESTS,
                SPEND
            ]
        );
        let keys: Vec<&str> = descriptors
            .iter()
            .map(|d| d.limit_resources[0].key.as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "premiumCredits",
                "extraUsage",
                "chat",
                "completions",
                "premiumRequests",
                "spend"
            ]
        );
        assert_eq!(
            descriptors[1].template.selection_kind,
            Some(MetricKind::Count)
        );
    }

    fn editor_dir(roots: &Roots) -> PathBuf {
        if cfg!(windows) {
            roots.local_data.join("github-copilot")
        } else {
            roots.home.join(".config").join("github-copilot")
        }
    }

    fn cli_dir(roots: &Roots) -> PathBuf {
        if cfg!(windows) {
            roots.app_data.join("GitHub CLI")
        } else {
            roots.home.join(".config").join("gh")
        }
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn tokens_of(login: &Login) -> Vec<(String, String)> {
        saved_tokens(&login.secret)
            .into_iter()
            .map(|(token, origin)| (token.to_string(), origin.to_string()))
            .collect()
    }

    fn pair(token: &str, origin: &str) -> (String, String) {
        (token.to_string(), origin.to_string())
    }

    #[test]
    fn discovers_one_card_per_github_user_from_the_editor_and_cli_files() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        let editor = editor_dir(&roots);
        write(
            &editor.join("apps.json"),
            r#"{"github.com:Iv1.b507a08c87ecfe98":{"user":"octocat","oauth_token":"ghu_editor","githubAppId":"Iv1.b507a08c87ecfe98"},
                "ghe.example.com:Iv1.b507a08c87ecfe98":{"user":"enterprise","oauth_token":"ghu_enterprise"}}"#,
        );
        write(
            &editor.join("hosts.json"),
            r#"{"github.com":{"user":"Hubot","oauth_token":"gho_hosts"}}"#,
        );
        let hosts = cli_dir(&roots).join("hosts.yml");
        write(
            &hosts,
            "github.com:\n    users:\n        OctoCat:\n            oauth_token: gho_cli_octocat\n        Monalisa:\n            oauth_token: gho_cli_monalisa\n    git_protocol: https\n    user: Monalisa\n    oauth_token: gho_cli_monalisa\nghe.example.com:\n    user: enterprise\n    oauth_token: gho_enterprise\n",
        );
        let logins = Copilot.discover(&roots);
        let summary: Vec<(&str, Option<&str>, &str, &Path)> = logins
            .iter()
            .map(|login| {
                (
                    login.identity.as_str(),
                    login.label.as_deref(),
                    login.origin.as_str(),
                    login.location.as_path(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "octocat",
                    Some("octocat"),
                    APP,
                    editor.join("apps.json").as_path()
                ),
                (
                    "hubot",
                    Some("Hubot"),
                    APP,
                    editor.join("hosts.json").as_path()
                ),
                ("monalisa", Some("Monalisa"), GH_CLI, hosts.as_path()),
            ]
        );
        assert_eq!(
            tokens_of(&logins[0]),
            [pair("ghu_editor", APP), pair("gho_cli_octocat", GH_CLI)]
        );
        assert_eq!(tokens_of(&logins[1]), [pair("gho_hosts", APP)]);
        assert_eq!(tokens_of(&logins[2]), [pair("gho_cli_monalisa", GH_CLI)]);
        assert!(
            Copilot
                .discover(&Roots::under(&dir.path().join("empty")))
                .is_empty()
        );
    }

    #[test]
    fn cli_tokens_come_from_the_credential_store_per_user() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        write(
            &cli_dir(&roots).join("hosts.yml"),
            "github.com:\n    users:\n        octocat:\n        monalisa:\n    git_protocol: https\n    user: monalisa\n",
        );
        let asked = RefCell::new(Vec::new());
        let keyring = |service: &str, user: &str| {
            asked.borrow_mut().push(format!("{service}:{user}"));
            match user {
                "octocat" => Some("gho_k_octocat".to_string()),
                "monalisa" => Some("gho_k_monalisa".to_string()),
                _ => Some("gho_active".to_string()),
            }
        };
        let found = logins(&roots, &keyring);
        let summary: Vec<(&str, PathBuf)> = found
            .iter()
            .map(|login| (login.identity.as_str(), login.location.clone()))
            .collect();
        assert_eq!(
            summary,
            [
                ("monalisa", PathBuf::from("gh:github.com:monalisa")),
                ("octocat", PathBuf::from("gh:github.com:octocat")),
            ]
        );
        assert_eq!(tokens_of(&found[0]), [pair("gho_k_monalisa", GH_CLI)]);
        assert_eq!(tokens_of(&found[1]), [pair("gho_k_octocat", GH_CLI)]);
        assert_eq!(
            *asked.borrow(),
            ["gh:github.com:monalisa", "gh:github.com:octocat"]
        );
    }

    #[test]
    fn an_older_cli_keeps_the_active_token_under_an_empty_account() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        write(
            &cli_dir(&roots).join("hosts.yml"),
            "github.com:\n    user: octocat\n    git_protocol: https\n",
        );
        let keyring = |_: &str, user: &str| user.is_empty().then(|| "gho_active".to_string());
        let found = logins(&roots, &keyring);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].identity, "octocat");
        assert_eq!(tokens_of(&found[0]), [pair("gho_active", GH_CLI)]);
    }

    #[test]
    fn a_token_saved_without_its_user_joins_the_user_saved_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let xdg = dir.path().join("xdg");
        let gh = dir.path().join("gh-config");
        let roots = Roots::under(dir.path())
            .with_var("XDG_CONFIG_HOME", &xdg.to_string_lossy())
            .with_var("GH_CONFIG_DIR", &gh.to_string_lossy());
        write(
            &xdg.join("github-copilot").join("hosts.json"),
            r#"{"github.com":{"oauth_token":"gho_shared"}}"#,
        );
        write(
            &xdg.join("github-copilot").join("apps.json"),
            r#"{"github.com:Iv1.b507a08c87ecfe98":{"oauth_token":"ghu_lonely"}}"#,
        );
        write(
            &gh.join("hosts.yml"),
            "github.com:\n    oauth_token: gho_shared\n    user: octocat\n",
        );
        let found = logins(&roots, &|_: &str, _: &str| -> Option<String> { None });
        let summary: Vec<(String, Option<&str>)> = found
            .iter()
            .map(|login| (login.identity.clone(), login.label.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [
                (token_hash("ghu_lonely"), None),
                ("octocat".to_string(), Some("octocat")),
            ]
        );
        assert_eq!(tokens_of(&found[1]), [pair("gho_shared", APP)]);
        assert_eq!(found[1].origin, APP);
    }

    #[test]
    fn hosts_yml_keys_are_read_by_their_path() {
        let hosts = GhHosts::parse(
            "\u{feff}ghe.corp.example:\r\n    user: enterprise\r\n    oauth_token: gho_ent\r\ngithub.com:\r\n    users:\r\n        octocat:\r\n            oauth_token: 'gho_quoted'\r\n        user:\r\n    # a comment\r\n    user: \"octocat\"\r\n    git_protocol: https # trailing\r\n",
        );
        assert_eq!(
            hosts,
            GhHosts {
                present: true,
                active: Some("octocat".into()),
                active_token: None,
                users: vec![
                    ("octocat".into(), Some("gho_quoted".into())),
                    ("user".into(), None)
                ],
            }
        );
        assert!(!GhHosts::parse("ghe.corp.example:\n    user: enterprise\n").present);
        assert_eq!(
            key_value("oauth_token: null"),
            Some(("oauth_token".into(), None))
        );
        assert_eq!(key_value("- item"), None);
    }

    #[cfg(windows)]
    fn write_vscode_sessions(roots: &Roots, app: &str, sessions: &str) {
        use base64::Engine;
        use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};

        let master = [9u8; 32];
        let input = CRYPT_INTEGER_BLOB {
            cbData: master.len() as u32,
            pbData: master.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let protected = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut output,
            )
        };
        assert_ne!(protected, 0);
        let mut wrapped = b"DPAPI".to_vec();
        wrapped.extend_from_slice(unsafe {
            std::slice::from_raw_parts(output.pbData, output.cbData as usize)
        });
        unsafe { LocalFree(output.pbData.cast()) };
        let folder = roots.app_data.join(app);
        write(
            &folder.join("Local State"),
            &json!({"os_crypt": {"encrypted_key": base64::engine::general_purpose::STANDARD.encode(wrapped)}})
                .to_string(),
        );
        let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &master).unwrap());
        let nonce = [5u8; 12];
        let mut sealed = sessions.as_bytes().to_vec();
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut sealed,
        )
        .unwrap();
        let mut blob = b"v10".to_vec();
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&sealed);
        let database = apps::state_db(roots, app);
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ItemTable VALUES (?1, ?2)",
                rusqlite::params![
                    format!(
                        "secret://{{\"extensionId\":\"{VSCODE_EXTENSION}\",\"key\":\"{VSCODE_KEY}\"}}"
                    ),
                    json!({"type": "Buffer", "data": blob}).to_string(),
                ],
            )
            .unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn discovers_the_github_sign_in_of_vs_code_apps() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        write_vscode_sessions(
            &roots,
            "Code",
            r#"[{"id":"s1","accessToken":"gho_vscode","account":{"label":"octocat","id":"583231"},"scopes":["read:user","user:email"]},
                {"id":"s2","accessToken":"gho_vscode_repo","account":{"label":"OctoCat","id":"583231"},"scopes":["repo","workflow"]},
                {"id":"s3","accessToken":"gho_monalisa","account":{"label":"monalisa","id":"2"},"scopes":["user:email"]}]"#,
        );
        let logins = Copilot.discover(&roots);
        let summary: Vec<(&str, Option<&str>, &str)> = logins
            .iter()
            .map(|login| {
                (
                    login.identity.as_str(),
                    login.label.as_deref(),
                    login.origin.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("octocat", Some("octocat"), "VS Code"),
                ("monalisa", Some("monalisa"), "VS Code"),
            ]
        );
        assert_eq!(logins[0].location, apps::state_db(&roots, "Code"));
        assert_eq!(
            tokens_of(&logins[0]),
            [
                pair("gho_vscode", "VS Code"),
                pair("gho_vscode_repo", "VS Code")
            ]
        );
    }
}
