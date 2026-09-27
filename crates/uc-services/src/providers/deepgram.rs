//! Deepgram: the requests, audio and billable audio hours, agent hours, tokens and text-to-speech
//! characters used by the projects of a Deepgram API key.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `DEEPGRAM_API_KEY` environment variable or from a key saved in Accounts, and is sent as
//! `Authorization: Token <key>`. An optional API URL saved with the key replaces
//! `https://api.deepgram.com/v1` and must use https://. When no Project ID is saved with the key,
//! a refresh lists the key's projects with `GET <api>/projects` and remembers them for 12 hours
//! for the same API URL and key. It then sends `GET <api>/projects/<id>/usage/breakdown` for the
//! saved project or for at most the first three listed ones, adds their rows together and warns
//! when the key has more projects. No audio, generation or other billable request is ever sent.

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{
    endpoint::{self, Policy},
    http, lines,
};

pub(crate) struct Deepgram;

const NAME: &str = "Deepgram";
const BASE: &str = "https://api.deepgram.com/v1";
const PARTIAL: &str =
    "Only the first three Deepgram projects are shown. Enter a Project ID to select one.";

/// The usage fields added up over every row, with the title and unit of their row; `tokens`
/// stands for `tokens_in` plus `tokens_out`.
const FIELDS: [(&str, &str, &str); 6] = [
    ("requests", "Requests", "requests"),
    ("hours", "Audio", "hours"),
    ("total_hours", "Billable Audio", "hours"),
    ("agent_hours", "Agent Hours", "hours"),
    ("tokens", "Tokens", "tokens"),
    ("tts_characters", "Characters", "characters"),
];

#[async_trait]
impl Service for Deepgram {
    fn id(&self) -> &'static str {
        "deepgram"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["DEEPGRAM_API_KEY"],
            url: "https://console.deepgram.com",
            fields: &[
                ("projectId", "Project ID (optional)"),
                ("baseUrl", "API URL (optional)"),
            ],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        FIELDS
            .iter()
            .map(|(field, title, unit)| {
                let id = match *field {
                    "total_hours" => "totalHours",
                    "agent_hours" => "agentHours",
                    "tts_characters" => "ttsCharacters",
                    other => other,
                };
                WidgetDescriptor::values(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    Some(MetricKind::Count),
                    Some(unit),
                    true,
                    None,
                    false,
                )
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Deepgram API key is missing."))?;
        let base = endpoint::base_url(
            context.secret.str("/baseUrl"),
            Some(BASE),
            Policy::Https,
            NAME,
        )?;
        let fingerprint: String = Sha256::digest(key.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let projects = if let Some(id) = context.secret.str("/projectId") {
            vec![id.to_string()]
        } else {
            let cached = context.memo.get("deepgram.projects", context.now).await;
            if let Some(items) = cached
                .as_ref()
                .filter(|memo| memo["base"] == base && memo["fingerprint"] == fingerprint)
                .and_then(|memo| memo["ids"].as_array())
            {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            } else {
                let body = ask(context, key, format!("{base}/projects")).await?;
                let list = body["projects"]
                    .as_array()
                    .ok_or_else(|| http::decoding(NAME))?;
                let mut ids = Vec::new();
                for project in list {
                    let id = project["project_id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| http::decoding(NAME))?;
                    if !ids.iter().any(|known| known == id) {
                        ids.push(id.to_string());
                    }
                }
                context
                    .memo
                    .put(
                        "deepgram.projects",
                        json!({"base":base,"fingerprint":fingerprint,"ids":ids}),
                        Some(context.now + chrono::Duration::hours(12)),
                    )
                    .await;
                ids
            }
        };
        if projects.is_empty() {
            return Err(http::not_available(
                "No Deepgram projects are available for this key.",
            ));
        }
        let mut totals = [0.0_f64; 6];
        for id in projects.iter().take(3) {
            let mut url = url::Url::parse(&base).map_err(|_| http::decoding(NAME))?;
            url.path_segments_mut()
                .map_err(|_| http::decoding(NAME))?
                .pop_if_empty()
                .extend(["projects", id, "usage", "breakdown"]);
            let body = ask(context, key, url.into()).await?;
            let rows = body["results"]
                .as_array()
                .ok_or_else(|| http::decoding(NAME))?;
            for row in rows {
                if !row.is_object() {
                    return Err(http::decoding(NAME));
                }
                for (index, (field, _, _)) in FIELDS.iter().enumerate() {
                    let amount = if *field == "tokens" {
                        number(row, "tokens_in", true)? + number(row, "tokens_out", true)?
                    } else {
                        number(row, field, matches!(*field, "requests" | "tts_characters"))?
                    };
                    totals[index] += amount;
                    if !totals[index].is_finite() {
                        return Err(http::decoding(NAME));
                    }
                }
            }
        }
        let rows = FIELDS
            .iter()
            .zip(totals)
            .map(|((_, title, unit), total)| lines::count_value(title, total, unit))
            .collect();
        Ok(Reading::new(None, rows).with_warning((projects.len() > 3).then(|| PARTIAL.into())))
    }
}

/// A GET to the Deepgram API with the key in a `Token` authorization header, read as JSON.
async fn ask(
    context: &FetchContext<'_>,
    key: &str,
    url: String,
) -> Result<Value, SimpleProviderError> {
    http::json(
        context.http,
        HttpRequest::get(url)
            .header("Authorization", format!("Token {key}"))
            .header("Accept", "application/json"),
        NAME,
    )
    .await
}

/// The number at `key` of a usage row, which must be finite, not negative and, when `integer`,
/// whole. A missing or null field counts as zero; anything else is a decoding error.
fn number(row: &Value, key: &str, integer: bool) -> Result<f64, SimpleProviderError> {
    let Some(field) = row.get(key).filter(|field| !field.is_null()) else {
        return Ok(0.0);
    };
    field
        .as_f64()
        .filter(|amount| {
            amount.is_finite() && *amount >= 0.0 && (!integer || amount.fract() == 0.0)
        })
        .ok_or_else(|| http::decoding(NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use uc_core::ErrorCategory;

    const USAGE_ROW: &str = r#"{"results":[{"hours":1.5,"total_hours":2.0,"agent_hours":0.5,"tokens_in":10,"tokens_out":5,"tts_characters":100,"requests":3}]}"#;
    const FOUR_PROJECTS: &str = r#"{"projects":[{"project_id":"a"},{"project_id":"b"},{"project_id":"c"},{"project_id":"d"}]}"#;

    #[tokio::test]
    async fn adds_up_every_usage_field_and_lists_the_projects_only_once() {
        let http = Scripted::new()
            .on(
                "GET",
                &format!("{BASE}/projects"),
                200,
                r#"{"projects":[{"project_id":"project-1","name":"Fixture"}]}"#,
            )
            .on(
                "GET",
                &format!("{BASE}/projects/project-1/usage/breakdown"),
                200,
                USAGE_ROW,
            );
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        let result = Deepgram.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            result,
            Reading::new(
                None,
                vec![
                    lines::count_value("Requests", 3.0, "requests"),
                    lines::count_value("Audio", 1.5, "hours"),
                    lines::count_value("Billable Audio", 2.0, "hours"),
                    lines::count_value("Agent Hours", 0.5, "hours"),
                    lines::count_value("Tokens", 15.0, "tokens"),
                    lines::count_value("Characters", 100.0, "characters")
                ]
            )
        );
        assert_eq!(Deepgram.fetch(&scope.context()).await.unwrap(), result);
        let requests = http.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(header(&requests[0], "authorization"), Some("Token fixture"));
        assert_eq!(
            requests[1].url,
            format!("{BASE}/projects/project-1/usage/breakdown")
        );
    }

    #[tokio::test]
    async fn reads_three_of_four_projects_with_a_warning_and_lists_again_for_another_key() {
        let http = Scripted::new()
            .on("GET", &format!("{BASE}/projects"), 200, FOUR_PROJECTS)
            .on(
                "GET",
                &format!("{BASE}/projects/a"),
                200,
                r#"{"results":[]}"#,
            )
            .on(
                "GET",
                &format!("{BASE}/projects/b"),
                200,
                r#"{"results":[]}"#,
            )
            .on(
                "GET",
                &format!("{BASE}/projects/c"),
                200,
                r#"{"results":[]}"#,
            );
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        assert_eq!(
            Deepgram
                .fetch(&scope.context())
                .await
                .unwrap()
                .warning
                .as_deref(),
            Some(PARTIAL)
        );
        assert_eq!(http.requests().len(), 4);
        Deepgram.fetch(&scope.context()).await.unwrap();
        assert_eq!(http.requests().len(), 7);
        let replacement = crate::service::Secret::new(json!({"apiKey":"replacement-fixture"}));
        let mut context = scope.context();
        context.secret = &replacement;
        Deepgram.fetch(&context).await.unwrap();
        assert_eq!(http.requests().len(), 11);
    }

    #[tokio::test]
    async fn refusals_rate_limits_and_unreadable_rows_keep_their_categories() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
            (
                200,
                r#"{"results":[{"requests":0.1}]}"#,
                ErrorCategory::Decoding,
            ),
        ] {
            let http = Scripted::new().on("GET", BASE, status, body);
            let scope = context_at(
                &http,
                json!({"apiKey":"fixture","projectId":"p"}),
                chrono::Utc::now(),
            );
            assert_eq!(
                Deepgram.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
