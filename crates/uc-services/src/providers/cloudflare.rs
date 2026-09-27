//! Cloudflare Workers AI: the neurons an account has used today, against the 10,000 a day that
//! reset at 00:00 UTC.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the API token comes from the
//! `CLOUDFLARE_API_TOKEN` environment variable or from a token saved in Quota Control, with the
//! account's 32-character hexadecimal Account ID entered beside it. A refresh runs no inference and
//! only reads: it sends GraphQL queries as `POST https://api.cloudflare.com/client/v4/graphql` with
//! the token as a bearer token. At most once every 12 hours it first introspects the schema, to
//! check that the Workers AI analytics have the `totalNeurons`, `modelId`, `datetime_geq` and
//! `datetime_lt` fields it relies on; then it asks for today's total neurons and each model's
//! neurons, from 00:00 UTC to the next midnight. The card shows the total as a share of 10,000
//! neurons, the total itself and one row per model, with a warning when the model list reaches
//! its 1,000-row limit and may be cut short. A 401 or 403, a GraphQL error about permissions or an
//! answer without the account asks for the Account Analytics Read permission.
//!
//! Cloudflare documents the introspection at
//! https://developers.cloudflare.com/analytics/graphql-api/features/discovery/introspection/ and
//! the 10,000 daily neurons at https://developers.cloudflare.com/workers-ai/platform/pricing/.

use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use serde_json::{Value, json};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Cloudflare;

const URL: &str = "https://api.cloudflare.com/client/v4/graphql";
const PERMISSION: &str = "Cloudflare refused analytics access. Add Account Analytics Read permission for this account to the API token.";
const SCHEMA: &str =
    "Cloudflare did not expose the Workers AI neuron analytics fields required by this reader.";
const TRUNCATED: &str =
    "Cloudflare model details may be truncated; the daily total includes all models.";

/// A read-only look at the analytics schema, so the usage query never asks for a field that
/// Cloudflare does not have.
const INTROSPECTION: &str = r#"query {
 sumType: __type(name:"AccountAiInferenceAdaptiveGroupsSum") { fields { name } }
 dimensionType: __type(name:"AccountAiInferenceAdaptiveGroupsDimensions") { fields { name } }
 filterType: __type(name:"AccountAiInferenceAdaptiveGroupsFilter_InputObject") { inputFields { name } }
}"#;

/// Sends one GraphQL query and returns its answer, turning a refusal or a GraphQL error into the
/// card error it stands for.
async fn query(
    context: &FetchContext<'_>,
    key: &str,
    text: &str,
) -> Result<Value, SimpleProviderError> {
    let response = http::send(
        context.http,
        HttpRequest::post(URL)
            .bearer(key)
            .json_body(&json!({"query":text})),
        "Cloudflare Workers AI",
    )
    .await?;
    if matches!(response.status, 401 | 403) {
        return Err(http::invalid(PERMISSION));
    }
    if !response.is_success() {
        return Err(http::status_error(&response, "Cloudflare Workers AI"));
    }
    let body: Value = http::parse(&response, "Cloudflare Workers AI")?;
    if let Some(errors) = body
        .get("errors")
        .and_then(Value::as_array)
        .filter(|errors| !errors.is_empty())
    {
        let denied = errors
            .iter()
            .filter_map(|error| value::text(error, "/message"))
            .any(|message| {
                let lowered = message.to_ascii_lowercase();
                [
                    "permission",
                    "not authorized",
                    "unauthorized",
                    "authentication",
                    "forbidden",
                ]
                .iter()
                .any(|word| lowered.contains(word))
            });
        return Err(if denied {
            http::invalid(PERMISSION)
        } else {
            http::not_available(
                "Cloudflare could not provide Workers AI analytics for this account.",
            )
        });
    }
    Ok(body)
}

/// Whether the introspected field list at `path` has a field called `name`.
fn has(schema: &Value, path: &str, name: &str) -> bool {
    schema
        .pointer(path)
        .and_then(Value::as_array)
        .is_some_and(|fields| {
            fields
                .iter()
                .any(|field| value::text(field, "/name") == Some(name))
        })
}

#[async_trait]
impl Service for Cloudflare {
    fn id(&self) -> &'static str {
        "cloudflare"
    }

    fn name(&self) -> &'static str {
        "Cloudflare Workers AI"
    }

    fn key_label(&self) -> &'static str {
        "API token"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["CLOUDFLARE_API_TOKEN"],
            url: "https://dash.cloudflare.com/profile/api-tokens",
            fields: &[("accountId", "Account ID")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::percent(
                format!("{}.daily", provider.id),
                provider,
                "Daily Neurons",
                None,
                None,
            )
            .exporting_progress("daily", "percent"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Cloudflare API token is missing."))?;
        let account = context
            .secret
            .str("/accountId")
            .ok_or_else(|| http::invalid("The Cloudflare account ID is missing."))?;
        if account.len() != 32 || !account.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(http::invalid(
                "The Cloudflare account ID must contain 32 hexadecimal characters.",
            ));
        }
        if context
            .memo
            .get("cloudflare-schema", context.now)
            .await
            .is_none()
        {
            let schema = query(context, key, INTROSPECTION).await?;
            if !has(&schema, "/data/sumType/fields", "totalNeurons")
                || !has(&schema, "/data/dimensionType/fields", "modelId")
                || !has(&schema, "/data/filterType/inputFields", "datetime_geq")
                || !has(&schema, "/data/filterType/inputFields", "datetime_lt")
            {
                return Err(http::not_available(SCHEMA));
            }
            context
                .memo
                .put(
                    "cloudflare-schema",
                    json!(true),
                    Some(context.now + Duration::hours(12)),
                )
                .await;
        }
        let start = Utc.from_utc_datetime(
            &context
                .now
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| http::decoding("Cloudflare Workers AI"))?,
        );
        let end = start + Duration::days(1);
        let filter = format!(
            "{{datetime_geq:\"{}\",datetime_lt:\"{}\"}}",
            start.to_rfc3339(),
            end.to_rfc3339()
        );
        let text = format!(
            "query {{ viewer {{ accounts(filter:{{accountTag:\"{account}\"}}) {{ total:aiInferenceAdaptiveGroups(limit:1,filter:{filter}) {{ sum {{ totalNeurons }} }} models:aiInferenceAdaptiveGroups(limit:1000,filter:{filter}) {{ dimensions {{ modelId }} sum {{ totalNeurons }} }} }} }} }}"
        );
        let body = query(context, key, &text).await?;
        let accounts = body
            .pointer("/data/viewer/accounts")
            .and_then(Value::as_array)
            .ok_or_else(|| http::decoding("Cloudflare Workers AI"))?;
        let analytics = accounts.first().ok_or_else(|| http::invalid(PERMISSION))?;
        let totals = analytics
            .get("total")
            .and_then(Value::as_array)
            .ok_or_else(|| http::decoding("Cloudflare Workers AI"))?;
        let mut total = 0.0;
        for group in totals {
            total += value::number(group, "/sum/totalNeurons")
                .filter(|count| *count >= 0.0)
                .ok_or_else(|| http::decoding("Cloudflare Workers AI"))?;
        }
        if !total.is_finite() {
            return Err(http::decoding("Cloudflare Workers AI"));
        }
        let mut rows = vec![
            lines::percent(
                "Daily Neurons",
                total / 100.0,
                Some(end),
                Some(lines::DAY_MS),
            ),
            lines::count_value("Neurons", total, "neurons"),
        ];
        let models = analytics
            .get("models")
            .and_then(Value::as_array)
            .ok_or_else(|| http::decoding("Cloudflare Workers AI"))?;
        for model in models {
            let label = value::text(model, "/dimensions/modelId")
                .ok_or_else(|| http::decoding("Cloudflare Workers AI"))?;
            let neurons = value::number(model, "/sum/totalNeurons")
                .filter(|count| *count >= 0.0)
                .ok_or_else(|| http::decoding("Cloudflare Workers AI"))?;
            rows.push(lines::count_value(label, neurons, "neurons"));
        }
        Ok(Reading::new(None, rows).with_warning((models.len() >= 1000).then(|| TRUNCATED.into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use uc_core::ErrorCategory;

    const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
    const KNOWN_FIELDS: &str = r#"{"data":{"sumType":{"fields":[{"name":"totalNeurons"}]},"dimensionType":{"fields":[{"name":"modelId"}]},"filterType":{"inputFields":[{"name":"datetime_geq"},{"name":"datetime_lt"}]}}}"#;
    const ONE_MODEL: &str = r#"{"data":{"viewer":{"accounts":[{"total":[{"sum":{"totalNeurons":2500}}],"models":[{"dimensions":{"modelId":"@cf/model"},"sum":{"totalNeurons":2500}}]}]}}}"#;

    #[tokio::test]
    async fn the_schema_is_checked_then_todays_neurons_fill_the_daily_meter_and_model_rows() {
        let http = Scripted::new()
            .on("POST", URL, 200, KNOWN_FIELDS)
            .on("POST", URL, 200, ONE_MODEL);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 12, 30, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test","accountId":ACCOUNT}), now);
        let reading = Cloudflare.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::percent(
                    "Daily Neurons",
                    25.0,
                    Some(Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap()),
                    Some(lines::DAY_MS)
                ),
                lines::count_value("Neurons", 2500.0, "neurons"),
                lines::count_value("@cf/model", 2500.0, "neurons")
            ]
        );
        assert_eq!(http.requests().len(), 2);
        assert_eq!(
            header(&http.requests()[1], "Authorization"),
            Some("Bearer test")
        );
        let body = String::from_utf8(http.requests()[1].body.clone().unwrap()).unwrap();
        assert!(body.contains("2026-09-27T00:00:00+00:00"));
        assert!(!body.contains("mutation"));
        assert!(
            scope
                .context()
                .memo
                .get("cloudflare-schema", now)
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn unknown_schema_never_queries_guessed_fields() {
        let http = Scripted::new().on("POST", URL, 200, r#"{"data":{"sumType":null}}"#);
        let scope = context_at(
            &http,
            json!({"apiKey":"test","accountId":ACCOUNT}),
            Utc::now(),
        );
        assert_eq!(
            Cloudflare
                .fetch(&scope.context())
                .await
                .unwrap_err()
                .category,
            ErrorCategory::NotAvailable
        );
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn aggregate_overflow_is_rejected_and_empty_day_is_zero() {
        for (total, valid) in [
            (json!([]), true),
            (
                json!([{"sum":{"totalNeurons":1e308}},{"sum":{"totalNeurons":1e308}}]),
                false,
            ),
        ] {
            let payload =
                json!({"data":{"viewer":{"accounts":[{"total":total,"models":[]}]}}}).to_string();
            let http = Scripted::new()
                .on("POST", URL, 200, KNOWN_FIELDS)
                .on("POST", URL, 200, &payload);
            let scope = context_at(
                &http,
                json!({"apiKey":"test","accountId":ACCOUNT}),
                Utc::now(),
            );
            let result = Cloudflare.fetch(&scope.context()).await;
            if valid {
                assert_eq!(
                    result.unwrap().lines[1],
                    lines::count_value("Neurons", 0.0, "neurons")
                );
            } else {
                assert_eq!(result.unwrap_err().category, ErrorCategory::Decoding);
            }
        }
    }

    #[tokio::test]
    async fn refused_access_rate_limits_and_unreadable_answers_keep_their_category() {
        for (status, body, category) in [
            (403, "{}", ErrorCategory::AuthInvalid),
            (
                200,
                r#"{"errors":[{"message":"permission denied"}]}"#,
                ErrorCategory::AuthInvalid,
            ),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "not json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("POST", URL, status, body);
            let scope = context_at(
                &http,
                json!({"apiKey":"test","accountId":ACCOUNT}),
                Utc::now(),
            );
            let error = Cloudflare.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            if category == ErrorCategory::AuthInvalid {
                assert_eq!(error.message, PERMISSION);
            }
        }
    }
}
