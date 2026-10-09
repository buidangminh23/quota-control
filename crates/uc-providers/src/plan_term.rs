use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, PlanTerm, SharedHttpClient};

use crate::ProviderKind;
use crate::credentials::jwt_claims;

const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const PROFILE_TIMEOUT: Duration = Duration::from_secs(5);
const PROFILE_RECHECK: chrono::Duration = chrono::Duration::minutes(5);
const PROFILE_RETRY: chrono::Duration = chrono::Duration::minutes(1);
pub(crate) const TERM_RECHECK: chrono::Duration = chrono::Duration::hours(1);

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
    pub(crate) checked_at: DateTime<Utc>,
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
    Ok(ProfileSubscription {
        plan,
        checked_at: now,
    })
}

#[derive(Default)]
pub(crate) struct ProfileTerm {
    cached: tokio::sync::Mutex<Option<CachedTerm>>,
}

struct CachedTerm {
    fingerprint: [u8; 32],
    next_check: DateTime<Utc>,
    subscription: Option<ProfileSubscription>,
}

impl ProfileTerm {
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
            Err(retry) => (previous, now + retry),
        };
        *cached = Some(CachedTerm {
            fingerprint,
            next_check,
            subscription: subscription.clone(),
        });
        subscription
    }
}

async fn profile_subscription(
    http: &SharedHttpClient,
    access_token: &str,
    now: DateTime<Utc>,
) -> Result<ProfileSubscription, chrono::Duration> {
    let request = HttpRequest::get(PROFILE_URL)
        .bearer(access_token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .timeout(PROFILE_TIMEOUT);
    let response = tokio::time::timeout(PROFILE_TIMEOUT, http.send(request))
        .await
        .map_err(|_| PROFILE_RETRY)?
        .map_err(|_| PROFILE_RETRY)?;
    if !response.is_success() {
        return Err(if response.status == 429 {
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
        });
    }
    let profile: Value = response.json().map_err(|_| PROFILE_RETRY)?;
    subscription(&profile, now).map_err(|_| PROFILE_RETRY)
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
    fn subscription_start_is_not_a_billing_period() {
        let profile = json!({"organization":{"organization_type":"claude_pro", "subscription_created_at":"2026-07-31T00:00:00Z"}});
        assert_eq!(
            subscription(&profile, now()).unwrap().plan.as_deref(),
            Some("Pro")
        );
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
        let client = std::sync::Arc::new(ProfileHttp { replies: std::sync::Mutex::new([(200, json!({"organization":{"organization_type":"claude_pro"}})), (429, json!({})), (200, json!({"organization":{"organization_type":"claude_free", "subscription_created_at":"2026-10-02T00:00:00Z"}})), (200, json!({"organization":{"organization_type":"claude_max", "rate_limit_tier":"default_claude_max_5x"}}))].into()), calls: Default::default() });
        let http: SharedHttpClient = client.clone();
        let cache = ProfileTerm::default();
        let first = cache.get(&http, "fixture", now()).await.unwrap();
        assert_eq!(first.plan.as_deref(), Some("Pro"));
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
        let renewed = cache
            .get(&http, "fixture", now() + chrono::Duration::minutes(20))
            .await
            .unwrap();
        assert_eq!(renewed.plan.as_deref(), Some("Max 5x"));
        assert_eq!(client.calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn switching_tokens_never_reuses_another_accounts_plan() {
        let cache = ProfileTerm::default();
        cache
            .seed(
                "first",
                &json!({"organization":{"organization_type":"claude_pro"}}),
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
