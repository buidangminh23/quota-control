//! ElevenLabs: the character quota and the voice slots of an ElevenLabs subscription. Nothing is
//! read from disk on Windows, macOS or Linux: the key comes from the `ELEVENLABS_API_KEY`
//! environment variable or from a key saved in Quota Control, which may also give an HTTPS API
//! URL to ask instead of the default one.
//!
//! A refresh sends one request, `GET https://api.elevenlabs.io/v1/user/subscription` (or the saved
//! API URL), with the key in the `xi-api-key` header. The answer's `character_count` of
//! `character_limit` fills the Characters meter until `next_character_count_reset_unix`, the voice
//! slots and professional voice slots in use fill a row each when the plan has any, `tier` names the
//! plan. An invoice payment attempt and a credit reset do not state when the paid period ends.
//! A key without the `user_read` permission is reported as such rather than as a
//! refused key.

use async_trait::async_trait;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{
    endpoint::{self, Policy},
    http, lines, value,
};

pub(crate) struct ElevenLabs;

const NAME: &str = "ElevenLabs";
const URL: &str = "https://api.elevenlabs.io/v1/user/subscription";
const PERMISSION: &str = "The ElevenLabs API key needs the user_read permission.";

#[async_trait]
impl Service for ElevenLabs {
    fn id(&self) -> &'static str {
        "elevenlabs"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["ELEVENLABS_API_KEY"],
            url: "https://elevenlabs.io/app/settings/api-keys",
            fields: &[("baseUrl", "API URL (optional)")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::percent(
                format!("{}.characters", provider.id),
                provider,
                "Characters",
                None,
                None,
            )
            .exporting_progress("characters", "percent"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The ElevenLabs API key is missing."))?;
        let url = endpoint::base_url(
            context.secret.str("/baseUrl"),
            Some(URL),
            Policy::Https,
            NAME,
        )?;
        let response = http::send(
            context.http,
            HttpRequest::get(url)
                .header("xi-api-key", key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        if matches!(response.status, 401 | 403) {
            let body = response.json::<serde_json::Value>().unwrap_or_default();
            if ["/detail/code", "/detail/status"].iter().any(|pointer| {
                value::text(&body, pointer).is_some_and(|code| {
                    matches!(code, "missing_permissions" | "insufficient_permissions")
                })
            }) {
                return Err(http::invalid(PERMISSION));
            }
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let body = http::parse(&response, NAME)?;
        let count = |field: &str| {
            body.get(field)
                .and_then(serde_json::Value::as_i64)
                .filter(|number| *number >= 0)
                .ok_or_else(|| http::decoding(NAME))
        };
        let used = count("character_count")? as f64;
        let limit = count("character_limit")? as f64;
        for field in [
            "voice_slots_used",
            "voice_limit",
            "professional_voice_slots_used",
            "professional_voice_limit",
            "next_character_count_reset_unix",
        ] {
            if body.get(field).is_some_and(|entry| !entry.is_null()) {
                count(field)?;
            }
        }
        for field in ["tier", "status"] {
            if body
                .get(field)
                .is_some_and(|entry| !entry.is_null() && !entry.is_string())
            {
                return Err(http::decoding(NAME));
            }
        }
        let reset = value::time(&body, "/next_character_count_reset_unix");
        let mut rows = vec![lines::percent(
            "Characters",
            if limit > 0.0 {
                used / limit * 100.0
            } else {
                0.0
            },
            reset,
            Some(lines::MONTH_MS),
        )];
        for (title, used_field, limit_field) in [
            ("Voice Slots", "voice_slots_used", "voice_limit"),
            (
                "Professional Voices",
                "professional_voice_slots_used",
                "professional_voice_limit",
            ),
        ] {
            if let (Some(used), Some(limit)) =
                (body[used_field].as_i64(), body[limit_field].as_i64())
                && limit > 0
            {
                rows.push(lines::count(
                    title,
                    used as f64,
                    limit as f64,
                    "voices",
                    None,
                    None,
                ));
            }
        }
        Ok(Reading::new(
            value::text(&body, "/tier").and_then(lines::plan_name),
            rows,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use serde_json::json;
    use uc_core::ErrorCategory;

    const CREATOR_SUBSCRIPTION: &str = r#"{"character_count":250,"character_limit":1000,"next_character_count_reset_unix":1791000000,"tier":"creator","voice_slots_used":3,"voice_limit":10,"professional_voice_slots_used":1,"professional_voice_limit":2}"#;

    #[tokio::test]
    async fn characters_voice_slots_and_the_tier_come_from_one_get_with_the_key_header() {
        let http = Scripted::new().on("GET", URL, 200, CREATOR_SUBSCRIPTION);
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        assert_eq!(
            ElevenLabs.fetch(&scope.context()).await.unwrap(),
            Reading::new(
                Some("Creator".into()),
                vec![
                    lines::percent(
                        "Characters",
                        25.0,
                        Some(chrono::Utc.timestamp_opt(1791000000, 0).unwrap()),
                        Some(lines::MONTH_MS)
                    ),
                    lines::count("Voice Slots", 3.0, 10.0, "voices", None, None),
                    lines::count("Professional Voices", 1.0, 2.0, "voices", None, None)
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(header(&requests[0], "xi-api-key"), Some("fixture"));
        assert_eq!(requests[0].url, URL);
    }

    #[tokio::test]
    async fn invoice_payment_attempts_and_credit_resets_do_not_state_a_paid_period() {
        for next_invoice in [
            json!({"amount_due_cents":2200,"next_payment_attempt_unix":1791000000}),
            json!({"amount_due_cents":0,"next_payment_attempt_unix":-1}),
        ] {
            let mut body: serde_json::Value = serde_json::from_str(CREATOR_SUBSCRIPTION).unwrap();
            body["next_invoice"] = next_invoice;
            let http = Scripted::new().on("GET", URL, 200, &body.to_string());
            let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
            let reading = ElevenLabs.fetch(&scope.context()).await.unwrap();
            assert_eq!(reading.plan_term, None);
        }
    }

    #[tokio::test]
    async fn refusals_missing_permissions_and_malformed_answers_keep_their_categories() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (
                403,
                r#"{"detail":{"code":"missing_permissions"}}"#,
                ErrorCategory::AuthInvalid,
            ),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
            (
                200,
                r#"{"character_count":1.5,"character_limit":5}"#,
                ErrorCategory::Decoding,
            ),
            (200, "{", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
            assert_eq!(
                ElevenLabs
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                category
            );
        }
    }

    #[tokio::test]
    async fn a_plain_http_api_url_is_refused_before_any_request() {
        let http = Scripted::new();
        let scope = context_at(
            &http,
            json!({"apiKey":"fixture","baseUrl":"http://example.org"}),
            chrono::Utc::now(),
        );
        assert_eq!(
            ElevenLabs
                .fetch(&scope.context())
                .await
                .unwrap_err()
                .category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
