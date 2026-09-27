//! llmman: the memory a llmman server's loaded models take, and how many models it has loaded
//! and stored, with their sizes in bytes.
//!
//! Nothing is read from disk on Windows, macOS or Linux, no command is run and nothing is renewed:
//! the server address and an optional API key are saved in Accounts. Without a key the address
//! must use `https://` or plain `http://` to this computer; with one, plain `http://` may also
//! reach a private network. A trailing `/v1` is dropped. A refresh sends two `GET`s in this order,
//! each with `Accept: application/json` and, when a key is saved, `Authorization: Bearer <key>`:
//! - `/llmman/node` for the server's memory and the size of each loaded and stored model; a failed
//!   answer stops the refresh there;
//! - `/api/version` for the server's `version`, shown when it answers with one and left out
//!   otherwise.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{
    HttpRequest, MetricKind, MetricLine, MetricValue, Provider, SimpleProviderError,
    WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{endpoint, http, lines};

pub(crate) struct LlmMan;

const NAME: &str = "llmman";

#[async_trait]
impl Service for LlmMan {
    fn id(&self) -> &'static str {
        "llmman"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "API key (optional for local daemon)"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "",
            fields: &[("baseUrl", "Server address")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut descriptors = vec![
            WidgetDescriptor::bounded_count(
                format!("{}.memory", provider.id),
                provider,
                "Memory",
                None,
                1.0,
                "bytes",
                None,
            )
            .exporting_progress("memory", "bytes"),
        ];
        for (s, title) in [("loaded", "Loaded"), ("stored", "Stored")] {
            descriptors.push(WidgetDescriptor::values(
                format!("{}.{s}", provider.id),
                provider,
                title,
                None,
                Some(MetricKind::Count),
                None,
                true,
                None,
                false,
            ));
        }
        descriptors
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let policy = if context.secret.key().is_some() {
            endpoint::Policy::HttpsOrPrivateNetworkHttp
        } else {
            endpoint::Policy::HttpsOrLoopbackHttp
        };
        let base = endpoint::base_url(context.secret.str("/baseUrl"), None, policy, NAME)?;
        let base = base.strip_suffix("/v1").unwrap_or(&base);
        let request = |path: &str| {
            let plain =
                HttpRequest::get(format!("{base}{path}")).header("Accept", "application/json");
            if let Some(key) = context.secret.key() {
                plain.header("Authorization", format!("Bearer {key}"))
            } else {
                plain
            }
        };
        let body = http::json(context.http, request("/llmman/node"), NAME).await?;
        let mut found = parse(&body)?;
        if let Ok(version) = http::json(context.http, request("/api/version"), NAME).await
            && let Some(version) = version["version"].as_str().filter(|text| {
                !text.is_empty()
                    && text.chars().count() <= 120
                    && !text.chars().any(char::is_control)
            })
        {
            found.push(lines::text("Version", version));
        }
        Ok(Reading::new(None, found))
    }
}

/// A byte count: a whole number from 0 to 9007199254740991 (2^53 - 1).
fn bytes(value: &Value) -> Result<f64, SimpleProviderError> {
    value
        .as_f64()
        .filter(|number| {
            number.is_finite()
                && *number >= 0.0
                && number.fract() == 0.0
                && *number <= 9007199254740991.0
        })
        .ok_or_else(|| http::decoding(NAME))
}

/// Each model's name and size in bytes, largest first, then by name.
fn models(value: &Value) -> Result<Vec<(&str, f64)>, SimpleProviderError> {
    let sizes = value.as_object().ok_or_else(|| http::decoding(NAME))?;
    let mut listed: Vec<_> = sizes
        .iter()
        .map(|(name, size)| Ok((name.as_str(), bytes(size)?)))
        .collect::<Result<_, SimpleProviderError>>()?;
    listed.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(right.0)));
    Ok(listed)
}

/// The Memory meter (when the server's memory is above zero), the Loaded and Stored rows, then a
/// line for each loaded model, largest first. A model whose name is blank, longer than 120
/// characters or holds a control character gets no line of its own.
fn parse(body: &Value) -> Result<Vec<MetricLine>, SimpleProviderError> {
    let memory = bytes(&body["memory"])?;
    let loaded = models(&body["loaded"])?;
    let stored = models(&body["stored"])?;
    let total = |sizes: &Vec<(&str, f64)>| -> Result<f64, SimpleProviderError> {
        let sum: f64 = sizes.iter().map(|(_, size)| size).sum();
        if sum.is_finite() && sum <= 9007199254740991.0 {
            Ok(sum)
        } else {
            Err(http::decoding(NAME))
        }
    };
    let used = total(&loaded)?;
    let disk = total(&stored)?;
    let mut found = vec![];
    if memory > 0.0 {
        found.push(lines::count("Memory", used, memory, "bytes", None, None));
    }
    found.push(lines::values(
        "Loaded",
        vec![
            MetricValue::count(loaded.len() as f64, "models"),
            MetricValue::count(used, "bytes"),
        ],
    ));
    found.push(lines::values(
        "Stored",
        vec![
            MetricValue::count(stored.len() as f64, "models"),
            MetricValue::count(disk, "bytes"),
        ],
    ));
    for (name, size) in loaded {
        if name.trim().is_empty()
            || name.chars().count() > 120
            || name.chars().any(char::is_control)
        {
            continue;
        }
        let label = if ["Memory", "Loaded", "Stored"].contains(&name) {
            format!("Model: {name}")
        } else {
            name.to_string()
        };
        found.push(if memory > 0.0 {
            lines::count(&label, size, memory, "bytes", None, None)
        } else {
            lines::count_value(&label, size, "bytes")
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const NODE: &str = r#"{"memory":8000000000,"loaded":{"qwen":2000000000},"stored":{"qwen":2000000000,"gemma":1000000000}}"#;

    fn now() -> DateTime<Utc> {
        "2026-09-27T00:00:00Z".parse().unwrap()
    }

    #[tokio::test]
    async fn reads_memory_and_model_sizes_from_a_local_daemon_without_a_key() {
        let http = Scripted::new()
            .on("GET", "http://localhost:8000/llmman/node", 200, NODE)
            .on(
                "GET",
                "http://localhost:8000/api/version",
                404,
                "old daemon",
            );
        let scope = context_at(&http, json!({"baseUrl":"http://localhost:8000/v1"}), now());
        let reading = LlmMan.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(
            reading.lines,
            vec![
                lines::count("Memory", 2e9, 8e9, "bytes", None, None),
                lines::values(
                    "Loaded",
                    vec![
                        MetricValue::count(1.0, "models"),
                        MetricValue::count(2e9, "bytes")
                    ]
                ),
                lines::values(
                    "Stored",
                    vec![
                        MetricValue::count(2.0, "models"),
                        MetricValue::count(3e9, "bytes")
                    ]
                ),
                lines::count("qwen", 2e9, 8e9, "bytes", None, None)
            ]
        );
        assert_eq!(http.requests().len(), 2);
        assert_eq!(header(&http.requests()[0], "Authorization"), None);
    }

    #[tokio::test]
    async fn a_saved_key_is_sent_as_a_bearer_token_and_failures_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (503, ErrorCategory::http(503)),
        ] {
            let http = Scripted::new().on(
                "GET",
                "http://192.168.1.10/llmman/node",
                status,
                "secret-echo",
            );
            let scope = context_at(
                &http,
                json!({"apiKey":"fixture","baseUrl":"http://192.168.1.10"}),
                now(),
            );
            let error = LlmMan.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert_eq!(
                header(&http.requests()[0], "Authorization"),
                Some("Bearer fixture")
            );
            assert!(!error.message.contains("secret-echo"));
        }
    }

    #[test]
    fn malformed_sizes_fail_and_zero_memory_shows_plain_model_sizes() {
        assert!(parse(&json!({"memory":1,"loaded":{"bad":-1},"stored":{}})).is_err());
        assert!(parse(&json!({})).is_err());
        let found = parse(&json!({"memory":0,"loaded":{"a":5},"stored":{}})).unwrap();
        assert_eq!(found[2], lines::count_value("a", 5.0, "bytes"));
    }
}
