//! The plan's paid period for the card header. ChatGPT's login states when the period ends;
//! Anthropic only says when the subscription began, so the card estimates a monthly renewal from
//! that day.

use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use uc_core::{HttpRequest, PlanTerm, SharedHttpClient};

use crate::ProviderKind;
use crate::credentials::jwt_claims;

const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const PROFILE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an answer from Anthropic's profile stands; the start date changes only with the plan.
const PROFILE_RECHECK: chrono::Duration = chrono::Duration::hours(24);
/// How soon a card asks again after the profile could not be read.
const PROFILE_RETRY: chrono::Duration = chrono::Duration::hours(1);

/// The paid period a login document states, if any.
pub(crate) fn from_document(kind: ProviderKind, document: &Value) -> Option<PlanTerm> {
    match kind {
        ProviderKind::Codex => {
            let claims = document["tokens"]["id_token"]
                .as_str()
                .and_then(jwt_claims)?;
            let auth = &claims["https://api.openai.com/auth"];
            Some(PlanTerm::Stated {
                ends_at: timestamp(&auth["chatgpt_subscription_active_until"])?,
                checked_at: timestamp(&auth["chatgpt_subscription_last_checked"]),
            })
        }
        ProviderKind::Claude => monthly_from(&document["oauthAccount"]["subscriptionCreatedAt"]),
    }
}

fn monthly_from(started: &Value) -> Option<PlanTerm> {
    Some(PlanTerm::MonthlyFrom {
        started_at: timestamp(started)?,
    })
}

/// An RFC 3339 time; one without an offset is UTC, the way Claude Code reads it.
fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    let text = value.as_str()?.trim();
    if let Ok(date) = DateTime::parse_from_rfc3339(text) {
        return Some(date.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|date| date.and_utc())
}

/// Anthropic's profile answer for one card, asked at most once a day (once an hour after a
/// failure), because the profile endpoint is rate limited and the start date rarely changes.
#[derive(Default)]
pub(crate) struct ProfileTerm {
    cached: tokio::sync::Mutex<Option<CachedTerm>>,
}

struct CachedTerm {
    next_check: DateTime<Utc>,
    term: Option<PlanTerm>,
}

impl ProfileTerm {
    /// The subscription start from the live profile; a failed lookup keeps the last answer.
    pub(crate) async fn get(
        &self,
        http: &SharedHttpClient,
        access_token: &str,
        now: DateTime<Utc>,
    ) -> Option<PlanTerm> {
        let mut cached = self.cached.lock().await;
        if let Some(entry) = cached.as_ref().filter(|entry| now < entry.next_check) {
            return entry.term.clone();
        }
        let previous = cached.take().and_then(|entry| entry.term);
        let (term, next_check) = match profile_term(http, access_token).await {
            Ok(term) => (term, now + PROFILE_RECHECK),
            Err(()) => (previous, now + PROFILE_RETRY),
        };
        *cached = Some(CachedTerm {
            next_check,
            term: term.clone(),
        });
        term
    }
}

/// `Ok(None)` when the profile answers without a subscription start (no paid plan).
async fn profile_term(http: &SharedHttpClient, access_token: &str) -> Result<Option<PlanTerm>, ()> {
    let request = HttpRequest::get(PROFILE_URL)
        .bearer(access_token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .timeout(PROFILE_TIMEOUT);
    let response = tokio::time::timeout(PROFILE_TIMEOUT, http.send(request))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
    if !response.is_success() {
        return Err(());
    }
    let profile: Value = response.json().map_err(|_| ())?;
    Ok(monthly_from(
        &profile["organization"]["subscription_created_at"],
    ))
}

#[cfg(test)]
mod tests {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn id_token(auth: Value) -> String {
        let claims = json!({ "https://api.openai.com/auth": auth });
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        )
    }

    #[test]
    fn a_chatgpt_login_states_its_paid_through_date() {
        let document = json!({"tokens": {"id_token": id_token(json!({
            "chatgpt_plan_type": "prolite",
            "chatgpt_subscription_active_until": "2026-10-17T01:56:39+00:00",
            "chatgpt_subscription_last_checked": "2026-09-25T13:56:24.432661+00:00"
        }))}});
        let Some(PlanTerm::Stated {
            ends_at,
            checked_at,
        }) = from_document(ProviderKind::Codex, &document)
        else {
            panic!("expected a stated term");
        };
        assert_eq!(
            ends_at,
            Utc.with_ymd_and_hms(2026, 10, 17, 1, 56, 39).unwrap()
        );
        assert_eq!(
            checked_at.map(|date| date.timestamp()),
            Some(
                Utc.with_ymd_and_hms(2026, 9, 25, 13, 56, 24)
                    .unwrap()
                    .timestamp()
            )
        );
    }

    #[test]
    fn a_free_or_unreadable_chatgpt_login_states_nothing() {
        for document in [
            json!({"tokens": {"id_token": id_token(json!({"chatgpt_plan_type": "free"}))}}),
            json!({"tokens": {"id_token": id_token(json!({"chatgpt_subscription_active_until": null}))}}),
            json!({"tokens": {"id_token": id_token(json!({"chatgpt_subscription_active_until": "soon"}))}}),
            json!({"tokens": {"id_token": "not-a-jwt"}}),
            json!({"tokens": {"access_token": "fixture"}}),
        ] {
            assert_eq!(from_document(ProviderKind::Codex, &document), None);
        }
    }

    #[test]
    fn claude_counts_from_the_subscription_start_with_or_without_an_offset() {
        let expected = Utc.with_ymd_and_hms(2026, 7, 31, 3, 40, 9).unwrap();
        for started in [
            "2026-07-31T03:40:09Z",
            "2026-07-31T03:40:09.000000Z",
            "2026-07-31T10:40:09+07:00",
            "2026-07-31T03:40:09",
            "2026-07-31T03:40:09.000",
        ] {
            let document = json!({"oauthAccount": {"subscriptionCreatedAt": started}});
            assert_eq!(
                from_document(ProviderKind::Claude, &document),
                Some(PlanTerm::MonthlyFrom {
                    started_at: expected
                }),
                "{started}"
            );
        }
        for document in [
            json!({"oauthAccount": {"subscriptionCreatedAt": null}}),
            json!({"oauthAccount": {"subscriptionCreatedAt": "July"}}),
            json!({"oauthAccount": {}}),
            json!({}),
        ] {
            assert_eq!(from_document(ProviderKind::Claude, &document), None);
        }
    }

    struct ProfileHttp {
        replies: std::sync::Mutex<std::collections::VecDeque<Option<(u16, Value)>>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl uc_core::HttpClient for ProfileHttp {
        async fn send(
            &self,
            request: HttpRequest,
        ) -> Result<uc_core::HttpResponse, uc_core::HttpError> {
            assert_eq!(request.url, PROFILE_URL);
            assert!(request.timeout <= PROFILE_TIMEOUT);
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let reply = self.replies.lock().unwrap().pop_front();
            match reply.expect("the card asked the profile more often than expected") {
                Some((status, body)) => Ok(uc_core::HttpResponse {
                    status,
                    headers: Default::default(),
                    body: serde_json::to_vec(&body).unwrap(),
                }),
                None => Err(uc_core::HttpError::Timeout),
            }
        }
    }

    #[tokio::test]
    async fn the_profile_is_asked_once_a_day_and_a_failure_keeps_the_last_answer() {
        let started = |date: &str| json!({"account": {"uuid": "a"}, "organization": {"uuid": "o", "subscription_created_at": date}});
        let fake = std::sync::Arc::new(ProfileHttp {
            replies: std::sync::Mutex::new(
                [
                    Some((200, started("2026-07-31T03:40:09Z"))),
                    Some((429, json!({}))),
                    None,
                    Some((
                        200,
                        json!({"account": {"uuid": "a"}, "organization": {"uuid": "o"}}),
                    )),
                ]
                .into(),
            ),
            calls: Default::default(),
        });
        let http: SharedHttpClient = fake.clone();
        let term = ProfileTerm::default();
        let first = Utc.with_ymd_and_hms(2026, 9, 27, 3, 0, 0).unwrap();
        let expected = Some(PlanTerm::MonthlyFrom {
            started_at: Utc.with_ymd_and_hms(2026, 7, 31, 3, 40, 9).unwrap(),
        });
        let hours = chrono::Duration::hours;
        for (offset, answer, calls) in [
            (hours(0), expected.clone(), 1),
            (hours(23), expected.clone(), 1),
            (hours(25), expected.clone(), 2),
            (
                hours(25) + chrono::Duration::minutes(30),
                expected.clone(),
                2,
            ),
            (
                hours(26) + chrono::Duration::minutes(1),
                expected.clone(),
                3,
            ),
            (hours(27) + chrono::Duration::minutes(2), None, 4),
            (hours(50), None, 4),
        ] {
            assert_eq!(
                term.get(&http, "fixture", first + offset).await,
                answer,
                "{offset}"
            );
            assert_eq!(
                fake.calls.load(std::sync::atomic::Ordering::SeqCst),
                calls,
                "{offset}"
            );
        }
    }
}
