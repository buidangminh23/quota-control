use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_accounts::{AccountError, KeyStore};
use uc_core::{ErrorCategory, HttpRequest, PlanTerm, SharedHttpClient, SimpleProviderError};

use crate::ProviderKind;
use crate::credentials::jwt_claims;

const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const PROFILE_TIMEOUT: Duration = Duration::from_secs(5);
const PROFILE_RECHECK: chrono::Duration = chrono::Duration::minutes(5);
const PROFILE_RETRY: chrono::Duration = chrono::Duration::minutes(1);
pub(crate) const TERM_RECHECK: chrono::Duration = chrono::Duration::hours(1);

#[derive(Clone, Debug)]
pub struct BillingPeriods {
    store: KeyStore,
}

impl BillingPeriods {
    pub fn plan(raw: &str, tier: Option<&str>) -> Option<String> {
        paid_plan(raw, tier)
    }

    pub fn plan_for(provider: &str, raw: &str, tier: Option<&str>) -> Option<String> {
        match provider {
            "claude" => paid_plan(raw, tier),
            "codex" => crate::mapping::codex_plan(raw).filter(|plan| paid_plan_for(provider, plan)),
            _ => None,
        }
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            store: KeyStore::new(root.into()),
        }
    }

    pub fn default_store() -> Self {
        Self::new(
            uc_core::paths::config_dir()
                .join("accounts")
                .join("billing-periods"),
        )
    }

    pub fn enable_browser(&self, organizations: &[String]) -> Result<(), SimpleProviderError> {
        self.enable_browser_for("claude", organizations)
    }

    pub fn enable_browser_for(
        &self,
        provider: &str,
        accounts: &[String],
    ) -> Result<(), SimpleProviderError> {
        if !matches!(provider, "claude" | "codex")
            || accounts.len() > 32
            || accounts
                .iter()
                .any(|org| organization_uuid_value(org).is_none())
        {
            return Err(billing_error(ErrorCategory::Decoding));
        }
        self.store
            .add_login(
                &billing_id_for(provider, "browser-connection"),
                provider,
                "Billing browser",
                "billing",
                &json!({"schemaVersion":1,"browserEnabled":true,"organizations":accounts}),
            )
            .map(|_| ())
            .map_err(|_| billing_error(ErrorCategory::CredentialAccess))
    }

    pub fn browser_enabled(&self) -> bool {
        self.browser_enabled_for("claude")
    }

    pub fn browser_enabled_for(&self, provider: &str) -> bool {
        self.store
            .secret(&billing_id_for(provider, "browser-connection"))
            .ok()
            .is_some_and(|document| {
                document["schemaVersion"] == 1 && document["browserEnabled"] == true
            })
    }

    pub fn browser_organizations(&self) -> Vec<String> {
        self.browser_accounts("claude")
    }

    pub fn browser_accounts(&self, provider: &str) -> Vec<String> {
        self.store
            .secret(&billing_id_for(provider, "browser-connection"))
            .ok()
            .and_then(|document| document["organizations"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|value| organization_uuid_value(value.as_str()?))
            .collect()
    }

    pub fn save(
        &self,
        organization_uuid: &str,
        raw_plan_type: &str,
        tier: Option<&str>,
        billing_json: &Value,
        observed_at: DateTime<Utc>,
    ) -> Result<(), SimpleProviderError> {
        let organization_uuid = organization_uuid_value(organization_uuid)
            .ok_or_else(|| billing_error(ErrorCategory::Decoding))?;
        let plan = paid_plan(raw_plan_type, tier);
        let ends_at = billing_json["status"]
            .as_str()
            .is_some_and(|status| status.trim().eq_ignore_ascii_case("active"))
            .then(|| {
                if billing_json["plan_ending_at"].is_null() {
                    billing_timestamp(&billing_json["next_charge_at"])
                } else {
                    billing_timestamp(&billing_json["plan_ending_at"])
                }
            })
            .flatten()
            .filter(|ends_at| *ends_at > observed_at);
        let (Some(plan), Some(ends_at)) = (plan, ends_at) else {
            return self.invalidate(&organization_uuid);
        };
        let id = billing_id(&organization_uuid);
        let _lock = self
            .store
            .renewal_lock(&id)
            .map_err(|_| billing_error(ErrorCategory::CredentialAccess))?;
        self.store
            .add_login(
                &id,
                "claude",
                "Claude billing",
                "billing",
                &json!({
                    "schemaVersion": 1,
                    "organizationUuid": organization_uuid,
                    "plan": plan,
                    "endsAt": ends_at.to_rfc3339(),
                    "observedAt": observed_at.to_rfc3339(),
                }),
            )
            .map(|_| ())
            .map_err(|_| billing_error(ErrorCategory::CredentialAccess))
    }

    pub fn invalidate(&self, organization_uuid: &str) -> Result<(), SimpleProviderError> {
        self.invalidate_for("claude", organization_uuid)
    }

    pub fn invalidate_for(
        &self,
        provider: &str,
        account_uuid: &str,
    ) -> Result<(), SimpleProviderError> {
        let organization_uuid = organization_uuid_value(account_uuid)
            .ok_or_else(|| billing_error(ErrorCategory::Decoding))?;
        let id = billing_id_for(provider, &organization_uuid);
        let _lock = self
            .store
            .renewal_lock(&id)
            .map_err(|_| billing_error(ErrorCategory::CredentialAccess))?;
        self.remove(&id)
    }

    fn remove(&self, id: &str) -> Result<(), SimpleProviderError> {
        match self.store.remove(id) {
            Ok(()) | Err(AccountError::NotFound) => Ok(()),
            Err(_) => Err(billing_error(ErrorCategory::CredentialAccess)),
        }
    }

    pub fn get(
        &self,
        organization_uuid: &str,
        fresh_profile_plan: Option<&str>,
        now: DateTime<Utc>,
    ) -> Option<PlanTerm> {
        self.get_for("claude", organization_uuid, fresh_profile_plan, now)
    }

    pub fn get_for(
        &self,
        provider: &str,
        account_uuid: &str,
        fresh_profile_plan: Option<&str>,
        now: DateTime<Utc>,
    ) -> Option<PlanTerm> {
        let account_uuid = organization_uuid_value(account_uuid)?;
        let id = billing_id_for(provider, &account_uuid);
        let _lock = self.store.renewal_lock(&id).ok()?;
        let Some(plan) = fresh_profile_plan.filter(|plan| paid_plan_for(provider, plan)) else {
            let _ = self.remove(&id);
            return None;
        };
        let confirmations = self.confirmations(provider, &account_uuid).ok()?;
        if confirmations
            .values()
            .all(|evidence| evidence["plan"].as_str() != Some(plan))
        {
            let _ = self.remove(&id);
        }
        confirmations
            .values()
            .filter(|evidence| evidence["plan"].as_str() == Some(plan))
            .filter_map(|evidence| evidence_term(evidence, now))
            .max_by_key(term_checked_at)
    }

    pub fn peek_for(
        &self,
        provider: &str,
        account_uuid: &str,
        now: DateTime<Utc>,
    ) -> Option<PlanTerm> {
        let account_uuid = organization_uuid_value(account_uuid)?;
        self.confirmations(provider, &account_uuid)
            .ok()?
            .values()
            .filter_map(|evidence| evidence_term(evidence, now))
            .max_by_key(term_checked_at)
    }

    pub fn save_from(
        &self,
        provider: &str,
        account_uuid: &str,
        plan: &str,
        ends_at: Option<DateTime<Utc>>,
        observed_at: DateTime<Utc>,
        source: &str,
    ) -> Result<(), SimpleProviderError> {
        if !matches!(provider, "claude" | "codex") || source.is_empty() || source.len() > 128 {
            return Err(billing_error(ErrorCategory::Decoding));
        }
        let account_uuid = organization_uuid_value(account_uuid)
            .ok_or_else(|| billing_error(ErrorCategory::Decoding))?;
        let id = billing_id_for(provider, &account_uuid);
        let _lock = self
            .store
            .renewal_lock(&id)
            .map_err(|_| billing_error(ErrorCategory::CredentialAccess))?;
        let mut confirmations = self.confirmations(provider, &account_uuid)?;
        if let Some(ends_at) =
            ends_at.filter(|end| *end > observed_at && paid_plan_for(provider, plan))
        {
            confirmations.insert(source.into(), json!({"plan":plan,"endsAt":ends_at.to_rfc3339(),"observedAt":observed_at.to_rfc3339()}));
        } else {
            confirmations.remove(source);
            confirmations.remove("legacy");
        }
        confirmations.retain(|_, evidence| evidence_term(evidence, observed_at).is_some());
        self.write_confirmations(provider, &account_uuid, confirmations)
    }

    pub fn invalidate_source(
        &self,
        provider: &str,
        account_uuid: &str,
        source: &str,
    ) -> Result<(), SimpleProviderError> {
        let account_uuid = organization_uuid_value(account_uuid)
            .ok_or_else(|| billing_error(ErrorCategory::Decoding))?;
        let id = billing_id_for(provider, &account_uuid);
        let _lock = self
            .store
            .renewal_lock(&id)
            .map_err(|_| billing_error(ErrorCategory::CredentialAccess))?;
        let mut confirmations = self.confirmations(provider, &account_uuid)?;
        confirmations.remove(source);
        confirmations.remove("legacy");
        self.write_confirmations(provider, &account_uuid, confirmations)
    }

    fn confirmations(
        &self,
        provider: &str,
        account_uuid: &str,
    ) -> Result<serde_json::Map<String, Value>, SimpleProviderError> {
        let evidence = match self.store.secret(&billing_id_for(provider, account_uuid)) {
            Ok(evidence) => evidence,
            Err(AccountError::NotFound) => return Ok(Default::default()),
            Err(_) => return Err(billing_error(ErrorCategory::CredentialAccess)),
        };
        match evidence["schemaVersion"].as_u64() {
            Some(1)
                if provider == "claude"
                    && evidence["organizationUuid"].as_str() == Some(account_uuid) =>
            {
                Ok(serde_json::Map::from_iter([("legacy".into(), evidence)]))
            }
            Some(2)
                if evidence["provider"] == provider
                    && evidence["accountUuid"].as_str() == Some(account_uuid) =>
            {
                evidence["confirmations"]
                    .as_object()
                    .cloned()
                    .ok_or_else(|| billing_error(ErrorCategory::Decoding))
            }
            _ => Err(billing_error(ErrorCategory::Decoding)),
        }
    }

    fn write_confirmations(
        &self,
        provider: &str,
        account_uuid: &str,
        confirmations: serde_json::Map<String, Value>,
    ) -> Result<(), SimpleProviderError> {
        let id = billing_id_for(provider, account_uuid);
        if confirmations.is_empty() {
            return self.remove(&id);
        }
        self.store.add_login(&id, provider, "Billing", "billing", &json!({
            "schemaVersion":2,"provider":provider,"accountUuid":account_uuid,"confirmations":confirmations
        })).map(|_| ()).map_err(|_| billing_error(ErrorCategory::CredentialAccess))
    }
}

fn term_checked_at(term: &PlanTerm) -> Option<DateTime<Utc>> {
    match term {
        PlanTerm::Stated { checked_at, .. } => *checked_at,
        _ => None,
    }
}

fn evidence_term(evidence: &Value, now: DateTime<Utc>) -> Option<PlanTerm> {
    let ends_at = billing_timestamp(&evidence["endsAt"])?;
    let checked_at = billing_timestamp(&evidence["observedAt"])?;
    (ends_at > now && checked_at <= now && now - checked_at < TERM_RECHECK).then_some(
        PlanTerm::Stated {
            ends_at,
            checked_at: Some(checked_at),
        },
    )
}

fn paid_plan_for(provider: &str, plan: &str) -> bool {
    match provider {
        "claude" => recognized_paid_plan(plan),
        "codex" => {
            !plan.trim().is_empty()
                && !matches!(
                    plan.trim().to_ascii_lowercase().as_str(),
                    "free" | "unknown"
                )
        }
        _ => false,
    }
}

fn organization_uuid_value(raw: &str) -> Option<String> {
    uuid::Uuid::parse_str(raw.trim())
        .ok()
        .filter(|id| !id.is_nil())
        .map(|id| id.to_string())
}

fn billing_id(organization_uuid: &str) -> String {
    billing_id_for("claude", organization_uuid)
}

fn billing_id_for(provider: &str, account_uuid: &str) -> String {
    let hash: String = Sha256::digest(account_uuid.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{provider}@{hash}")
}

pub(crate) fn codex_billing_account(document: &Value) -> Option<String> {
    let explicit = document
        .pointer("/tokens/account_id")
        .filter(|value| !value.is_null());
    let parse = |value: &Value| organization_uuid_value(value.as_str()?);
    let mut account = match explicit {
        Some(value) => Some(parse(value)?),
        None => None,
    };
    for field in ["id_token", "access_token"] {
        let Some(claims) = document["tokens"][field].as_str().and_then(jwt_claims) else {
            continue;
        };
        let Some(claimed) = claims
            .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
            .filter(|value| !value.is_null())
        else {
            continue;
        };
        let claimed = parse(claimed)?;
        if account.as_ref().is_some_and(|account| account != &claimed) {
            return None;
        }
        account = Some(claimed);
    }
    account
}

fn paid_plan(raw: &str, tier: Option<&str>) -> Option<String> {
    let raw = raw.trim().to_ascii_lowercase();
    let raw = raw.strip_prefix("claude_").unwrap_or(&raw);
    matches!(raw, "pro" | "max" | "team").then(|| crate::mapping::claude_plan(raw, tier))
}

fn recognized_paid_plan(plan: &str) -> bool {
    let mut parts = plan.split_whitespace();
    matches!(parts.next(), Some("Pro" | "Max" | "Team"))
        && parts.next().is_none_or(|multiplier| {
            multiplier
                .strip_suffix('x')
                .is_some_and(|number| number.parse::<u32>().is_ok())
        })
        && parts.next().is_none()
}

fn billing_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.as_str()?.trim())
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

fn billing_error(category: ErrorCategory) -> SimpleProviderError {
    SimpleProviderError::new(category, "Billing confirmation is unavailable.")
}

pub(crate) fn from_document(kind: ProviderKind, document: &Value) -> Option<PlanTerm> {
    if kind != ProviderKind::Codex {
        return None;
    }
    let claims = document["tokens"]["id_token"]
        .as_str()
        .and_then(jwt_claims)?;
    let auth = &claims["https://api.openai.com/auth"];
    if auth["chatgpt_plan_type"]
        .as_str()
        .is_some_and(|plan| plan.eq_ignore_ascii_case("free"))
    {
        return None;
    }
    Some(PlanTerm::Stated {
        ends_at: timestamp(&auth["chatgpt_subscription_active_until"])?,
        checked_at: timestamp(&auth["chatgpt_subscription_last_checked"]),
    })
}

pub(crate) fn codex_plan(document: &Value) -> Option<String> {
    let claims = document["tokens"]["id_token"]
        .as_str()
        .and_then(jwt_claims)?;
    claims["https://api.openai.com/auth"]["chatgpt_plan_type"]
        .as_str()
        .and_then(crate::mapping::codex_plan)
}

pub(crate) fn confirmed_term(
    term: Option<PlanTerm>,
    stored_plan: Option<&str>,
    live_plan: Option<&str>,
    now: DateTime<Utc>,
) -> Option<PlanTerm> {
    if !stored_plan.zip(live_plan).is_some_and(|(stored, live)| {
        !live.trim().is_empty() && !live.eq_ignore_ascii_case("free") && stored == live
    }) {
        return None;
    }
    match term {
        Some(PlanTerm::Stated {
            ends_at,
            checked_at: Some(checked_at),
        }) if ends_at > now && checked_at <= now && now - checked_at < TERM_RECHECK => {
            Some(PlanTerm::Stated {
                ends_at,
                checked_at: Some(checked_at),
            })
        }
        _ => None,
    }
}

fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    let text = value.as_str()?.trim();
    if let Ok(date) = DateTime::parse_from_rfc3339(text) {
        return Some(date.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|date| date.and_utc())
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProfileSubscription {
    pub(crate) plan: Option<String>,
    pub(crate) organization_uuid: Option<String>,
    billing_eligible: bool,
    pub(crate) checked_at: DateTime<Utc>,
}

impl ProfileSubscription {
    pub(crate) fn confirmed_term(
        &self,
        periods: Option<&BillingPeriods>,
        now: DateTime<Utc>,
    ) -> Option<PlanTerm> {
        let periods = periods?;
        let organization_uuid = self.organization_uuid.as_deref()?;
        if !self.billing_eligible {
            let _ = periods.invalidate(organization_uuid);
            return None;
        }
        if self.checked_at > now || now - self.checked_at >= PROFILE_RECHECK {
            return None;
        }
        periods.get(organization_uuid, self.plan.as_deref(), now)
    }
}

fn subscription(profile: &Value, now: DateTime<Utc>) -> Result<ProfileSubscription, ()> {
    let organization = profile["organization"].as_object().ok_or(())?;
    let plan = organization
        .get("organization_type")
        .filter(|value| !value.is_null())
        .map(|value| {
            let raw = value
                .as_str()
                .filter(|raw| !raw.trim().is_empty())
                .ok_or(())?;
            let raw = raw.strip_prefix("claude_").unwrap_or(raw);
            Ok::<_, ()>(crate::mapping::claude_plan(
                raw,
                organization.get("rate_limit_tier").and_then(Value::as_str),
            ))
        })
        .transpose()?;
    let organization_uuid = organization
        .get("uuid")
        .and_then(Value::as_str)
        .and_then(organization_uuid_value);
    let billing_eligible = plan.as_deref().is_some_and(recognized_paid_plan)
        && !organization
            .get("subscription_status")
            .and_then(Value::as_str)
            .is_some_and(|status| {
                matches!(
                    status.trim().to_ascii_lowercase().as_str(),
                    "inactive" | "canceled" | "cancelled" | "expired"
                )
            });
    Ok(ProfileSubscription {
        plan,
        organization_uuid,
        billing_eligible,
        checked_at: now,
    })
}

#[derive(Default)]
pub(crate) struct ProfileTerm {
    cached: tokio::sync::Mutex<Option<CachedTerm>>,
    billing_periods: Option<BillingPeriods>,
}

struct CachedTerm {
    fingerprint: [u8; 32],
    next_check: DateTime<Utc>,
    subscription: Option<ProfileSubscription>,
}

impl ProfileTerm {
    pub(crate) fn with_billing_periods(mut self, periods: BillingPeriods) -> Self {
        self.billing_periods = Some(periods);
        self
    }

    async fn invalidate_period(&self, organization_uuid: Option<String>) {
        if let (Some(periods), Some(organization_uuid)) = (&self.billing_periods, organization_uuid)
        {
            let periods = periods.clone();
            let _ = uc_core::load_blocking(move || periods.invalidate(&organization_uuid)).await;
        }
    }

    pub(crate) async fn invalidate_billing(&self) {
        let mut cached = self.cached.lock().await;
        let organization_uuid = cached
            .take()
            .and_then(|entry| entry.subscription)
            .and_then(|profile| profile.organization_uuid);
        self.invalidate_period(organization_uuid).await;
    }

    pub(crate) async fn seed(&self, access_token: &str, profile: &Value, now: DateTime<Utc>) {
        if let Ok(subscription) = subscription(profile, now) {
            *self.cached.lock().await = Some(CachedTerm {
                fingerprint: Sha256::digest(access_token.as_bytes()).into(),
                next_check: now + PROFILE_RECHECK,
                subscription: Some(subscription),
            });
        }
    }

    pub(crate) async fn peek(&self) -> Option<ProfileSubscription> {
        self.cached
            .lock()
            .await
            .as_ref()
            .and_then(|entry| entry.subscription.clone())
    }

    pub(crate) async fn get(
        &self,
        http: &SharedHttpClient,
        access_token: &str,
        now: DateTime<Utc>,
    ) -> Option<ProfileSubscription> {
        let fingerprint: [u8; 32] = Sha256::digest(access_token.as_bytes()).into();
        let mut cached = self.cached.lock().await;
        if cached
            .as_ref()
            .is_some_and(|entry| entry.fingerprint != fingerprint)
        {
            *cached = None;
        }
        if let Some(entry) = cached.as_ref().filter(|entry| now < entry.next_check) {
            return entry.subscription.clone();
        }
        let previous = cached.take().and_then(|entry| entry.subscription);
        let (subscription, next_check) = match profile_subscription(http, access_token, now).await {
            Ok(subscription) => (Some(subscription), now + PROFILE_RECHECK),
            Err(failure) => {
                let previous = if failure.authentication_rejected {
                    self.invalidate_period(previous.and_then(|profile| profile.organization_uuid))
                        .await;
                    None
                } else {
                    previous
                };
                (previous, now + failure.retry_after)
            }
        };
        *cached = Some(CachedTerm {
            fingerprint,
            next_check,
            subscription: subscription.clone(),
        });
        subscription
    }
}

struct ProfileFailure {
    retry_after: chrono::Duration,
    authentication_rejected: bool,
}

impl ProfileFailure {
    fn retry() -> Self {
        Self {
            retry_after: PROFILE_RETRY,
            authentication_rejected: false,
        }
    }
}

async fn profile_subscription(
    http: &SharedHttpClient,
    access_token: &str,
    now: DateTime<Utc>,
) -> Result<ProfileSubscription, ProfileFailure> {
    let request = HttpRequest::get(PROFILE_URL)
        .bearer(access_token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .timeout(PROFILE_TIMEOUT);
    let response = tokio::time::timeout(PROFILE_TIMEOUT, http.send(request))
        .await
        .map_err(|_| ProfileFailure::retry())?
        .map_err(|_| ProfileFailure::retry())?;
    if !response.is_success() {
        let retry_after = if response.status == 429 {
            chrono::Duration::seconds(
                response
                    .header("retry-after")
                    .and_then(|value| value.parse::<i64>().ok())
                    .filter(|seconds| *seconds > 0)
                    .unwrap_or(300)
                    .min(86400),
            )
        } else {
            PROFILE_RETRY
        };
        return Err(ProfileFailure {
            retry_after,
            authentication_rejected: matches!(response.status, 401 | 403),
        });
    }
    let profile: Value = response.json().map_err(|_| ProfileFailure::retry())?;
    subscription(&profile, now).map_err(|_| ProfileFailure::retry())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use chrono::TimeZone;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 9, 1, 0, 0).unwrap()
    }

    #[test]
    fn a_free_login_never_retains_a_paid_date() {
        let claims = json!({"https://api.openai.com/auth":{"chatgpt_plan_type":"free", "chatgpt_subscription_active_until":"2026-11-01T00:00:00Z"}});
        let document = json!({"tokens":{"id_token": format!("header.{}.signature", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap()))}});
        assert_eq!(from_document(ProviderKind::Codex, &document), None);
    }

    #[test]
    fn a_live_paid_profile_does_not_state_a_billing_term_from_a_historical_start() {
        let profile = json!({"organization":{"organization_type":"claude_pro", "subscription_created_at":"2026-07-31T00:00:00Z"}});
        let current = subscription(&profile, now()).unwrap();
        assert_eq!(current.plan.as_deref(), Some("Pro"));
        assert_eq!(current.checked_at, now());
        assert_eq!(current.confirmed_term(None, now()), None);
        assert_eq!(
            from_document(
                ProviderKind::Claude,
                &json!({"oauthAccount":{"subscriptionCreatedAt":"2026-07-31T00:00:00Z"}})
            ),
            None
        );
        assert_eq!(
            subscription(&json!({"organization":{}}), now())
                .unwrap()
                .plan,
            None
        );
        assert!(subscription(&json!({}), now()).is_err());
    }

    #[test]
    fn historical_starts_never_establish_the_current_cycle_after_a_resubscription() {
        let resubscribed_at = Utc.with_ymd_and_hms(2026, 10, 3, 7, 3, 48).unwrap();
        let observed_at = resubscribed_at + chrono::Duration::days(1);
        for plan in ["claude_pro", "claude_max", "claude_team"] {
            for started in [
                "2026-07-31T03:40:09Z",
                "2026-07-31T03:40:09.000000Z",
                "2026-07-31T10:40:09+07:00",
                "2026-07-31T03:40:09",
                "2026-07-31T03:40:09.000",
            ] {
                let profile = json!({"organization":{"organization_type":plan, "subscription_created_at":started, "subscription_status":"active", "cancel_at_period_end":true}});
                let current = subscription(&profile, observed_at).unwrap();
                assert_eq!(current.checked_at, observed_at);
                assert_eq!(
                    current.confirmed_term(None, observed_at),
                    None,
                    "{plan} {started}"
                );
            }
        }
        for organization in [
            json!({"organization_type":"claude_free", "subscription_created_at":"2026-07-31T00:00:00Z"}),
            json!({"organization_type":"claude_unknown", "subscription_created_at":"2026-07-31T00:00:00Z"}),
            json!({"subscription_created_at":"2026-07-31T00:00:00Z"}),
            json!({"organization_type":"claude_pro"}),
            json!({"organization_type":"claude_pro", "subscription_created_at":null}),
            json!({"organization_type":"claude_pro", "subscription_created_at":"July"}),
            json!({"organization_type":"claude_pro", "subscription_created_at":"2026-11-01T00:00:00Z"}),
        ] {
            assert_eq!(
                subscription(&json!({"organization":organization}), now())
                    .unwrap()
                    .confirmed_term(None, now()),
                None
            );
        }
        for status in ["inactive", "canceled", "cancelled", "expired", " CANCELED "] {
            let profile = json!({"organization":{"organization_type":"claude_max", "subscription_created_at":"2026-07-31T00:00:00Z", "subscription_status":status}});
            assert_eq!(
                subscription(&profile, now())
                    .unwrap()
                    .confirmed_term(None, now()),
                None
            );
        }
    }

    #[test]
    fn refreshing_a_profile_does_not_reconfirm_a_separately_acquired_billing_term() {
        let directory = tempfile::tempdir().unwrap();
        let periods = BillingPeriods::new(directory.path());
        let organization_uuid = "00000000-0000-4000-8000-000000000001";
        let profile = json!({"organization":{"uuid":organization_uuid, "organization_type":"claude_max", "subscription_created_at":"2026-07-31T00:00:00Z"}});
        let current = subscription(&profile, now()).unwrap();
        let term = PlanTerm::Stated {
            ends_at: now() + chrono::Duration::days(20),
            checked_at: Some(now()),
        };
        periods.save(organization_uuid, "claude_max", None, &json!({"status":"active", "next_charge_at":(now() + chrono::Duration::days(20)).to_rfc3339()}), now()).unwrap();
        assert_eq!(current.confirmed_term(Some(&periods), now()), Some(term));
        let later = now() + TERM_RECHECK;
        let refreshed = subscription(&profile, later).unwrap();
        assert_eq!(refreshed.plan.as_deref(), Some("Max"));
        assert_eq!(refreshed.checked_at, later);
        assert_eq!(refreshed.confirmed_term(Some(&periods), later), None);
    }

    #[test]
    fn billing_dates_are_exact_encrypted_and_bound_to_the_current_paid_organization() {
        let directory = tempfile::tempdir().unwrap();
        let periods = BillingPeriods::new(directory.path());
        let org = "00000000-0000-4000-8000-000000000001";
        let other = "00000000-0000-4000-8000-000000000002";
        let end = now() + chrono::Duration::days(24);
        periods.save(org, "claude_max", Some("default_claude_max_20x"),
            &json!({"status":"active", "next_charge_at":end.to_rfc3339(), "subscription_created_at":"2026-07-31T00:00:00Z", "sessionKey":"must-not-be-stored"}), now()).unwrap();
        let term = PlanTerm::Stated {
            ends_at: end,
            checked_at: Some(now()),
        };
        assert_eq!(periods.get(org, Some("Max 20x"), now()), Some(term));
        assert_eq!(periods.get(other, Some("Max 20x"), now()), None);
        let evidence = periods.store.secret(&billing_id(org)).unwrap();
        assert_eq!(evidence.as_object().unwrap().len(), 5);
        assert!(evidence.get("sessionKey").is_none());
        assert!(evidence.get("subscription_created_at").is_none());
        assert_eq!(periods.get(org, Some("Max 5x"), now()), None);
        assert!(periods.store.secret(&billing_id(org)).is_err());
    }

    #[test]
    fn billing_requires_active_explicit_future_dates_and_never_rolls_an_old_cycle() {
        let directory = tempfile::tempdir().unwrap();
        let periods = BillingPeriods::new(directory.path());
        let org = "00000000-0000-4000-8000-000000000001";
        let end = now() + chrono::Duration::days(2);
        let active = json!({"status":"active", "next_charge_at":end.to_rfc3339()});
        for invalid in [
            json!({"status":"active", "next_charge_date":"2026-11-03"}),
            json!({"status":"inactive", "next_charge_at":end.to_rfc3339()}),
            json!({"status":"canceled", "next_charge_at":end.to_rfc3339()}),
            json!({"status":"active", "next_charge_at":"bad-date"}),
            json!({"status":"active", "next_charge_at":now().to_rfc3339()}),
        ] {
            periods
                .save(org, "claude_pro", None, &active, now())
                .unwrap();
            periods
                .save(org, "claude_pro", None, &invalid, now())
                .unwrap();
            assert_eq!(periods.get(org, Some("Pro"), now()), None);
        }
        periods
            .save(org, "claude_pro", None, &active, now())
            .unwrap();
        assert_eq!(
            periods.get(org, Some("Pro"), now() - chrono::Duration::seconds(1)),
            None
        );
        assert_eq!(periods.get(org, Some("Pro"), now() + TERM_RECHECK), None);
        assert_eq!(periods.get(org, Some("Pro"), end), None);
        assert_eq!(periods.get(org, Some("Free"), now()), None);
        assert!(
            periods
                .save("not-an-organization", "claude_pro", None, &active, now())
                .is_err()
        );
        assert!(
            periods
                .save(
                    "00000000-0000-0000-0000-000000000000",
                    "claude_pro",
                    None,
                    &active,
                    now()
                )
                .is_err()
        );
    }

    #[test]
    fn ending_an_active_subscription_uses_its_explicit_end_instead_of_a_later_charge() {
        let directory = tempfile::tempdir().unwrap();
        let periods = BillingPeriods::new(directory.path());
        let org = "00000000-0000-4000-8000-000000000001";
        let end = now() + chrono::Duration::days(2);
        periods
            .save(
                org,
                "claude_pro",
                None,
                &json!({
                    "status":"active", "plan_ending_at":end.to_rfc3339(),
                    "next_charge_at":(end + chrono::Duration::days(30)).to_rfc3339()
                }),
                now(),
            )
            .unwrap();
        assert_eq!(
            periods.get(org, Some("Pro"), now()),
            Some(PlanTerm::Stated {
                ends_at: end,
                checked_at: Some(now())
            })
        );
    }

    #[test]
    fn billing_sources_and_providers_keep_independent_original_confirmations() {
        let directory = tempfile::tempdir().unwrap();
        let periods = BillingPeriods::new(directory.path());
        let account = "00000000-0000-4000-8000-000000000001";
        let end = now() + chrono::Duration::days(24);
        let newer = now() + chrono::Duration::minutes(5);
        periods
            .save_from("codex", account, "Pro 5x", Some(end), now(), "browser")
            .unwrap();
        periods
            .save_from(
                "codex",
                account,
                "Pro 5x",
                Some(end + chrono::Duration::days(1)),
                newer,
                "session:fixture",
            )
            .unwrap();
        periods
            .save_from("claude", account, "Max 20x", Some(end), now(), "browser")
            .unwrap();
        assert_eq!(
            periods.get_for("codex", account, Some("Pro 5x"), newer),
            Some(PlanTerm::Stated {
                ends_at: end + chrono::Duration::days(1),
                checked_at: Some(newer)
            })
        );
        periods
            .invalidate_source("codex", account, "session:fixture")
            .unwrap();
        assert_eq!(
            periods.get_for("codex", account, Some("Pro 5x"), newer),
            Some(PlanTerm::Stated {
                ends_at: end,
                checked_at: Some(now())
            })
        );
        assert_eq!(
            periods.get(account, Some("Max 20x"), newer),
            Some(PlanTerm::Stated {
                ends_at: end,
                checked_at: Some(now())
            })
        );
        periods
            .enable_browser_for("codex", &[account.into()])
            .unwrap();
        assert!(periods.browser_enabled_for("codex"));
        assert!(!periods.browser_enabled());
        assert_eq!(periods.browser_accounts("codex"), [account]);
        assert!(periods.browser_organizations().is_empty());
        let saved = periods
            .store
            .secret(&billing_id_for("codex", account))
            .unwrap();
        assert_eq!(saved.as_object().unwrap().len(), 4);
        assert_eq!(
            saved["confirmations"]["browser"].as_object().unwrap().len(),
            3
        );
        assert_eq!(periods.get_for("codex", account, Some("Free"), newer), None);
        assert!(
            periods
                .store
                .secret(&billing_id_for("codex", account))
                .is_err()
        );
        assert!(periods.get(account, Some("Max 20x"), newer).is_some());
    }

    #[test]
    fn migrating_legacy_billing_does_not_change_its_verification_time() {
        let directory = tempfile::tempdir().unwrap();
        let periods = BillingPeriods::new(directory.path());
        let org = "00000000-0000-4000-8000-000000000001";
        let end = now() + chrono::Duration::days(24);
        periods
            .save(
                org,
                "claude_pro",
                None,
                &json!({"status":"active","next_charge_at":end.to_rfc3339()}),
                now(),
            )
            .unwrap();
        let newer = now() + chrono::Duration::minutes(5);
        periods
            .save_from("claude", org, "Pro", Some(end), newer, "browser")
            .unwrap();
        let saved = periods.store.secret(&billing_id(org)).unwrap();
        assert_eq!(saved["schemaVersion"], 2);
        assert_eq!(
            saved["confirmations"]["legacy"]["observedAt"],
            now().to_rfc3339()
        );
        periods
            .invalidate_source("claude", org, "unrelated-session")
            .unwrap();
        assert_eq!(
            periods.get(org, Some("Pro"), newer),
            Some(PlanTerm::Stated {
                ends_at: end,
                checked_at: Some(newer)
            })
        );
        periods
            .save_from("claude", org, "Pro", None, newer, "browser")
            .unwrap();
        assert_eq!(periods.get(org, Some("Pro"), newer), None);
    }

    #[test]
    fn codex_billing_account_requires_consistent_non_nil_credential_identity() {
        let account = "00000000-0000-4000-8000-000000000001";
        let other = "00000000-0000-4000-8000-000000000002";
        let document = |explicit: Value, claimed: Value| {
            let claims = json!({"https://api.openai.com/auth":{"chatgpt_account_id":claimed}});
            json!({"tokens":{"account_id":explicit,"id_token":format!("header.{}.signature", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap()))}})
        };
        for (explicit, claimed) in [
            (json!(account), json!(account)),
            (json!(account), Value::Null),
            (Value::Null, json!(account)),
        ] {
            assert_eq!(
                codex_billing_account(&document(explicit, claimed)).as_deref(),
                Some(account)
            );
        }
        for (explicit, claimed) in [
            (json!(account), json!(other)),
            (json!("bad-id"), json!(account)),
            (json!(account), json!("bad-id")),
            (Value::Null, Value::Null),
            (
                json!("00000000-0000-0000-0000-000000000000"),
                json!(account),
            ),
            (json!(123), json!(account)),
        ] {
            assert_eq!(codex_billing_account(&document(explicit, claimed)), None);
        }
        let mut mixed = document(json!(account), json!(account));
        let claims = json!({"https://api.openai.com/auth":{"chatgpt_account_id":other}});
        mixed["tokens"]["access_token"] = json!(format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        ));
        assert_eq!(codex_billing_account(&mixed), None);
        mixed["tokens"]["id_token"] = Value::Null;
        mixed["tokens"]["account_id"] = Value::Null;
        assert_eq!(codex_billing_account(&mixed).as_deref(), Some(other));
    }

    #[test]
    fn a_date_requires_a_current_matching_paid_confirmation() {
        let term = |age| {
            Some(PlanTerm::Stated {
                ends_at: now() + chrono::Duration::days(2),
                checked_at: Some(now() - age),
            })
        };
        assert!(
            confirmed_term(
                term(chrono::Duration::minutes(5)),
                Some("Plus"),
                Some("Plus"),
                now()
            )
            .is_some()
        );
        assert_eq!(
            confirmed_term(
                term(chrono::Duration::minutes(5)),
                Some("Plus"),
                None,
                now()
            ),
            None
        );
        assert_eq!(
            confirmed_term(
                term(chrono::Duration::minutes(5)),
                None,
                Some("Plus"),
                now()
            ),
            None
        );
        for (stored, live, age) in [
            ("Plus", "Free", chrono::Duration::minutes(1)),
            ("Plus", "Pro 20x", chrono::Duration::minutes(1)),
            ("Plus", "Plus", TERM_RECHECK),
        ] {
            assert_eq!(
                confirmed_term(term(age), Some(stored), Some(live), now()),
                None
            );
        }
        assert_eq!(
            confirmed_term(
                Some(PlanTerm::Stated {
                    ends_at: now(),
                    checked_at: Some(now())
                }),
                Some("Plus"),
                Some("Plus"),
                now()
            ),
            None
        );
    }

    struct ProfileHttp {
        replies: std::sync::Mutex<std::collections::VecDeque<(u16, Value)>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl uc_core::HttpClient for ProfileHttp {
        async fn send(
            &self,
            request: HttpRequest,
        ) -> Result<uc_core::HttpResponse, uc_core::HttpError> {
            assert_eq!(request.url, PROFILE_URL);
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (status, body) = self.replies.lock().unwrap().pop_front().unwrap();
            Ok(uc_core::HttpResponse {
                status,
                headers: std::collections::HashMap::from([("retry-after".into(), "600".into())]),
                body: serde_json::to_vec(&body).unwrap(),
            })
        }
    }

    #[tokio::test]
    async fn plans_downgrade_and_renew_without_relogin_and_back_off_on_failure() {
        let client = std::sync::Arc::new(ProfileHttp { replies: std::sync::Mutex::new([(200, json!({"organization":{"organization_type":"claude_pro", "subscription_created_at":"2026-07-31T00:00:00Z"}})), (429, json!({})), (200, json!({"organization":{"organization_type":"claude_free", "subscription_created_at":"2026-10-02T00:00:00Z"}})), (200, json!({"organization":{"organization_type":"claude_max", "rate_limit_tier":"default_claude_max_5x", "subscription_created_at":"2026-10-01T00:00:00Z"}}))].into()), calls: Default::default() });
        let http: SharedHttpClient = client.clone();
        let cache = ProfileTerm::default();
        let first = cache.get(&http, "fixture", now()).await.unwrap();
        assert_eq!(first.plan.as_deref(), Some("Pro"));
        assert_eq!(first.confirmed_term(None, now()), None);
        assert_eq!(first.confirmed_term(None, now() + PROFILE_RECHECK), None);
        assert_eq!(
            cache
                .get(&http, "fixture", now() + chrono::Duration::minutes(4))
                .await,
            Some(first.clone())
        );
        assert_eq!(
            cache
                .get(&http, "fixture", now() + chrono::Duration::minutes(5))
                .await,
            Some(first.clone())
        );
        assert_eq!(
            cache
                .get(&http, "fixture", now() + chrono::Duration::minutes(14))
                .await,
            Some(first)
        );
        let free = cache
            .get(&http, "fixture", now() + chrono::Duration::minutes(15))
            .await
            .unwrap();
        assert_eq!(free.plan.as_deref(), Some("Free"));
        assert_eq!(
            free.confirmed_term(None, now() + chrono::Duration::minutes(15)),
            None
        );
        let renewed = cache
            .get(&http, "fixture", now() + chrono::Duration::minutes(20))
            .await
            .unwrap();
        assert_eq!(renewed.plan.as_deref(), Some("Max 5x"));
        assert_eq!(
            renewed.confirmed_term(None, now() + chrono::Duration::minutes(20)),
            None
        );
        assert_eq!(client.calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn failed_profile_refreshes_do_not_reconfirm_metadata_and_retry_after_one_minute() {
        let client = std::sync::Arc::new(ProfileHttp {
            replies: std::sync::Mutex::new([
                (500, json!({})),
                (200, json!({"organization":{"organization_type":"claude_pro", "subscription_created_at":"2026-10-01T00:00:00Z"}})),
            ].into()),
            calls: Default::default(),
        });
        let http: SharedHttpClient = client.clone();
        let cache = ProfileTerm::default();
        cache
            .seed(
                "fixture",
                &json!({"organization":{"organization_type":"claude_pro", "subscription_created_at":"2026-07-31T00:00:00Z"}}),
                now(),
            )
            .await;
        for offset in [
            PROFILE_RECHECK,
            PROFILE_RECHECK + chrono::Duration::seconds(30),
        ] {
            let cached = cache.get(&http, "fixture", now() + offset).await.unwrap();
            assert_eq!(cached.checked_at, now());
            assert_eq!(cached.confirmed_term(None, now() + offset), None);
        }
        assert_eq!(client.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let checked_at = now() + PROFILE_RECHECK + PROFILE_RETRY;
        let renewed = cache.get(&http, "fixture", checked_at).await.unwrap();
        assert_eq!(renewed.checked_at, checked_at);
        assert_eq!(renewed.confirmed_term(None, checked_at), None);
        assert_eq!(client.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn switching_tokens_never_reuses_another_accounts_plan() {
        let cache = ProfileTerm::default();
        cache
            .seed(
                "first",
                &json!({"organization":{"organization_type":"claude_pro", "subscription_created_at":"2026-07-31T00:00:00Z"}}),
                now(),
            )
            .await;
        let http: SharedHttpClient = std::sync::Arc::new(ProfileHttp {
            replies: std::sync::Mutex::new([(500, json!({}))].into()),
            calls: Default::default(),
        });
        assert_eq!(cache.get(&http, "second", now()).await, None);
    }
}
