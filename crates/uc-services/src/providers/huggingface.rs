//! Hugging Face: the inference usage billed this month, the ZeroGPU quota and the request count
//! of a Hugging Face account.
//!
//! The token is the one the Hugging Face CLI saves in `$HF_HOME/token`, else in
//! `~/.cache/huggingface/token`, found at the same place on Windows, macOS and Linux; a token in
//! the `HF_TOKEN` or `HUGGING_FACE_HUB_TOKEN` environment variable or saved in Quota Control works
//! too. Every request carries it as a bearer token. A refresh sends
//! `GET https://huggingface.co/api/settings/billing/usage-v2` from the start of the UTC month to
//! now, then `GET https://huggingface.co/api/spaces/zero-gpu/quota`, whose failure only leaves out
//! the ZeroGPU row, and, unless the plan was asked for this token in the last 12 hours,
//! `GET https://huggingface.co/api/whoami-v2` for a PRO or Free plan. The last two wait two seconds
//! at most. Billable usage is the inference usage past the included amount, against the account's
//! limit when it sets one.

use async_trait::async_trait;
use chrono::{Datelike, Duration, TimeZone, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{http, lines, value};

pub(crate) struct HuggingFace;

const NAME: &str = "Hugging Face";
const BASE: &str = "https://huggingface.co";

#[async_trait]
impl Service for HuggingFace {
    fn id(&self) -> &'static str {
        "huggingface"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::login("Hugging Face CLI").or_api_key(ApiKeyHelp {
            env: &["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN"],
            url: "https://huggingface.co/settings/tokens",
            fields: &[],
        })
    }

    fn key_label(&self) -> &'static str {
        "Hugging Face access token"
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let path = roots
            .dir_from("HF_HOME", roots.home.join(".cache/huggingface"))
            .join("token");
        let Some(token) = std::fs::metadata(&path)
            .ok()
            .filter(|metadata| metadata.is_file() && metadata.len() <= 16384)
            .and_then(|_| std::fs::read_to_string(&path).ok())
            .map(|text| text.trim().to_string())
            .filter(|token| !token.is_empty() && !token.chars().any(char::is_whitespace))
        else {
            return Vec::new();
        };
        vec![Login::new(
            hash(&token),
            "Hugging Face CLI",
            &path,
            Secret::new(json!({"apiKey": token})),
        )]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::bounded_dollars(
                format!("{}.usage", provider.id),
                provider,
                "Billable Usage",
                None,
                100.0,
                None,
                None,
            )
            .exporting_progress("usage", "usd"),
            WidgetDescriptor::percent(
                format!("{}.gpu", provider.id),
                provider,
                "ZeroGPU",
                None,
                None,
            )
            .exporting_progress("gpu", "percent"),
            WidgetDescriptor::values(
                format!("{}.requests", provider.id),
                provider,
                "Requests",
                None,
                Some(MetricKind::Count),
                Some("requests"),
                true,
                None,
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let token = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("Missing Hugging Face access token."))?;
        let start = Utc
            .with_ymd_and_hms(context.now.year(), context.now.month(), 1, 0, 0, 0)
            .single()
            .ok_or_else(|| http::decoding(NAME))?;
        let response = http::send(
            context.http,
            HttpRequest::get(format!(
                "{BASE}/api/settings/billing/usage-v2?startDate={}&endDate={}",
                start.timestamp(),
                context.now.timestamp()
            ))
            .bearer(token),
            NAME,
        )
        .await?;
        if response.status == 403 {
            return Err(http::invalid(
                "The Hugging Face token lacks billing access. Use a classic read token or enable Billing read on a fine-grained token.",
            ));
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let body = http::parse(&response, NAME)?;
        let data = &body["usage"]["inferenceProviders"];
        let number = |field: &str| {
            data[field]
                .as_f64()
                .filter(|amount| amount.is_finite() && *amount >= 0.0)
                .ok_or_else(|| http::decoding(NAME))
        };
        let gross = number("usedNanoUsd")? / 1e9;
        let included = number("includedNanoUsd")? / 1e9;
        let billable = (gross - included).max(0.0);
        let limit = if data["limitNanoUsd"].is_null() {
            0.0
        } else {
            number("limitNanoUsd")? / 1e9
        };
        let mut rows = vec![if limit > 0.0 {
            lines::dollars("Billable Usage", billable, limit, None, None)
        } else {
            lines::dollar_value("Billable Usage", billable)
        }];
        let gpu = http::json(
            context.http,
            HttpRequest::get(format!("{BASE}/api/spaces/zero-gpu/quota"))
                .bearer(token)
                .timeout(std::time::Duration::from_secs(2)),
            NAME,
        )
        .await
        .ok();
        if let Some(gpu) = &gpu
            && let (Some(total), Some(remaining)) = (
                gpu["base"]
                    .as_f64()
                    .filter(|base| base.is_finite() && *base > 0.0),
                gpu["current"]
                    .as_f64()
                    .filter(|current| current.is_finite() && *current >= 0.0),
            )
        {
            rows.push(lines::percent(
                "ZeroGPU",
                (total - remaining).max(0.0) / total * 100.0,
                value::time(gpu, "/resetsAt"),
                None,
            ));
        }
        if !data["numRequests"].is_null() {
            let requests = number("numRequests")?;
            if requests.fract() != 0.0 {
                return Err(http::decoding(NAME));
            }
            rows.push(lines::count_value("Requests", requests, "requests"));
        }
        rows.push(lines::dollar_value("Gross Inference Usage", gross));
        rows.push(lines::dollar_value("Included Inference Amount", included));
        let fingerprint = hash(token);
        let cache = context
            .memo
            .get("identity", context.now)
            .await
            .filter(|saved| saved["fingerprint"] == fingerprint);
        let plan = if let Some(cache) = cache {
            cache["plan"].as_str().map(str::to_string)
        } else {
            let profile = http::json(
                context.http,
                HttpRequest::get(format!("{BASE}/api/whoami-v2"))
                    .bearer(token)
                    .timeout(std::time::Duration::from_secs(2)),
                NAME,
            )
            .await
            .ok();
            let plan = profile
                .and_then(|profile| profile["isPro"].as_bool())
                .map(|pro| if pro { "PRO" } else { "Free" });
            context
                .memo
                .put(
                    "identity",
                    json!({"fingerprint": fingerprint, "plan": plan}),
                    Some(context.now + Duration::hours(12)),
                )
                .await;
            plan.map(str::to_string)
        };
        Ok(Reading::new(plan, rows))
    }
}

/// A token's SHA-256 in hex: the login's identity, and what ties the remembered plan to a token.
fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use uc_core::ErrorCategory;

    const USAGE: &str = r#"{"usage":{"inferenceProviders":{"usedNanoUsd":5000000000,"includedNanoUsd":2000000000,"limitNanoUsd":10000000000,"numRequests":4}}}"#;

    #[test]
    fn the_cli_token_file_is_found_trimmed_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        assert!(HuggingFace.discover(&roots).is_empty());
        let path = roots.home.join(".cache/huggingface/token");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "hf_fixture\n").unwrap();
        let found = HuggingFace.discover(&roots);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].identity, hash("hf_fixture"));
        assert_eq!(found[0].secret.key(), Some("hf_fixture"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "hf_fixture\n");
    }

    #[tokio::test]
    async fn billable_usage_is_usage_past_the_included_amount_and_the_plan_is_asked_once() {
        let http = Scripted::new()
            .on(
                "GET",
                &format!("{BASE}/api/settings/billing/usage-v2"),
                200,
                USAGE,
            )
            .on(
                "GET",
                &format!("{BASE}/api/spaces/zero-gpu/quota"),
                200,
                r#"{"base":100,"current":60}"#,
            )
            .on(
                "GET",
                &format!("{BASE}/api/whoami-v2"),
                200,
                r#"{"isPro":true}"#,
            );
        let scope = context_at(&http, json!({"apiKey": "fixture"}), Utc::now());
        let reading = HuggingFace.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, Some("PRO".into()));
        assert_eq!(
            reading.lines[0],
            lines::dollars("Billable Usage", 3.0, 10.0, None, None)
        );
        assert_eq!(
            reading.lines[1],
            lines::percent("ZeroGPU", 40.0, None, None)
        );
        assert_eq!(
            header(&http.requests()[0], "authorization"),
            Some("Bearer fixture")
        );
        HuggingFace.fetch(&scope.context()).await.unwrap();
        assert_eq!(http.requests().len(), 5);
    }

    #[tokio::test]
    async fn a_failed_usage_request_stops_the_refresh_with_its_category() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthInvalid),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", BASE, status, body);
            let scope = context_at(&http, json!({"apiKey": "fixture"}), Utc::now());
            assert_eq!(
                HuggingFace
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                category
            );
            assert_eq!(http.requests().len(), 1);
        }
    }
}
