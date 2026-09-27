//! GitKraken AI: the personal and shared-pool AI credit allowances of a GitKraken account.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the access token comes from the
//! `GITKRAKEN_API_TOKEN` environment variable or from a token saved in Quota Control, with an
//! optional organization ID saved beside it. A refresh sends one request,
//! `GET https://api.gitkraken.dev/v1/ai-tasks/usage`, with the token as a bearer token, Quota
//! Control's `Client-Name`, `Client-Version` and `User-Agent`, and the organization ID as
//! `gk-org-id` when one is saved. Each allowance is a weekly meter renewing at `resetsOn`; an
//! unlimited or zero allowance shows the credits used instead, with a notice. When the answer
//! gives `sharedUsed`, the shared pool is also split between this user and the rest of the
//! organization.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines};

pub(crate) struct GitKraken;

const NAME: &str = "GitKraken AI";
const URL: &str = "https://api.gitkraken.dev/v1/ai-tasks/usage";

#[async_trait]
impl Service for GitKraken {
    fn id(&self) -> &'static str {
        "gitkraken"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["GITKRAKEN_API_TOKEN"],
            url: "https://gitkraken.dev/account#ai-usage",
            fields: &[("orgId", "API organization ID")],
        })
    }

    fn key_label(&self) -> &'static str {
        "GitKraken access token"
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("personal", "Personal"), ("pool", "Shared Pool")]
            .iter()
            .map(|(id, title)| {
                WidgetDescriptor::percent(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    None,
                )
                .exporting_progress(id, "percent")
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let printable = |text: &str, max: usize| {
            !text.is_empty()
                && text.len() <= max
                && text.bytes().all(|byte| (33..=126).contains(&byte))
        };
        let key = context
            .secret
            .key()
            .filter(|key| printable(key, 16384))
            .ok_or_else(|| {
                http::invalid("Invalid GitKraken token. Paste only the value after Bearer.")
            })?;
        let mut request = HttpRequest::get(URL)
            .bearer(key)
            .header("Client-Name", "QuotaControl")
            .header("Client-Version", env!("CARGO_PKG_VERSION"))
            .header("User-Agent", "QuotaControl");
        if let Some(org) = context
            .secret
            .str("/orgId")
            .map(str::trim)
            .filter(|org| !org.is_empty())
        {
            if !printable(org, 256) {
                return Err(http::invalid(
                    "Invalid GitKraken organization ID. Enter a single ID without whitespace.",
                ));
            }
            request = request.header("gk-org-id", org);
        }
        let body = http::json(context.http, request, NAME).await?;
        if !body["error"].is_null() {
            return Err(http::decoding(NAME));
        }
        let data = &body["data"];
        let personal = quota(data).ok_or_else(|| http::decoding(NAME))?;
        let reset = data["resetsOn"]
            .as_str()
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .map(|at| at.with_timezone(&chrono::Utc))
            .ok_or_else(|| http::decoding(NAME))?;
        let pool = quota(&data["organization"]);
        let mut rows = Vec::new();
        let mut notices = Vec::new();
        for (label, (used, limit)) in std::iter::once(("Personal", personal))
            .chain(pool.map(|allowance| ("Shared Pool", allowance)))
        {
            if let Some(row) =
                lines::percent_of(label, used, limit, Some(reset), Some(lines::WEEK_MS))
            {
                rows.push(row);
            } else {
                rows.push(lines::count_value(label, used, "credits"));
                notices.push(format!(
                    "{label}: {}",
                    if limit < 0.0 {
                        "Unlimited"
                    } else {
                        "No allowance"
                    }
                ));
            }
        }
        if let Some((pool_used, _)) = pool
            && let Some(shared) = data["sharedUsed"]
                .as_f64()
                .filter(|shared| shared.is_finite() && *shared >= 0.0 && *shared <= pool_used)
        {
            rows.push(lines::count_value("Your Shared Usage", shared, "credits"));
            rows.push(lines::count_value(
                "Rest of Organization",
                pool_used - shared,
                "credits",
            ));
        }
        Ok(
            Reading::new(None, rows)
                .with_warning((!notices.is_empty()).then(|| notices.join(". "))),
        )
    }
}

/// An allowance's `used` and `limit`, where a limit of -1 stands for unlimited. `None` when
/// either is missing or out of range, or when the share used would not be a finite percentage.
fn quota(allowance: &Value) -> Option<(f64, f64)> {
    let used = allowance["used"]
        .as_f64()
        .filter(|used| used.is_finite() && *used >= 0.0)?;
    let limit = allowance["limit"]
        .as_f64()
        .filter(|limit| limit.is_finite() && (*limit >= 0.0 || *limit == -1.0))?;
    if limit > 0.0 && !(used / limit * 100.0).is_finite() {
        return None;
    }
    Some((used, limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const SHARED_POOL: &str = r#"{"data":{"used":20,"limit":100,"resetsOn":"2026-10-01T00:00:00Z","organization":{"used":60,"limit":200},"sharedUsed":10}}"#;

    #[tokio::test]
    async fn the_shared_pool_is_split_between_this_user_and_the_rest_of_the_organization() {
        let http = Scripted::new().on("GET", URL, 200, SHARED_POOL);
        let scope = context_at(
            &http,
            json!({"apiKey": "fixture", "orgId": "org-1"}),
            chrono::Utc::now(),
        );
        let reading = GitKraken.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines.len(), 4);
        assert_eq!(
            reading.lines[0],
            lines::percent(
                "Personal",
                20.0,
                chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
                    .ok()
                    .map(|at| at.with_timezone(&chrono::Utc)),
                Some(lines::WEEK_MS)
            )
        );
        assert_eq!(
            reading.lines[3],
            lines::count_value("Rest of Organization", 50.0, "credits")
        );
        assert_eq!(header(&http.requests()[0], "gk-org-id"), Some("org-1"));
        assert_eq!(
            header(&http.requests()[0], "authorization"),
            Some("Bearer fixture")
        );
    }

    #[tokio::test]
    async fn unlimited_allowances_show_a_count_and_failures_and_bad_org_ids_are_errors() {
        let http = Scripted::new().on(
            "GET",
            URL,
            200,
            r#"{"data":{"used":12,"limit":-1,"resetsOn":"2026-10-01T00:00:00Z"}}"#,
        );
        let scope = context_at(&http, json!({"apiKey": "fixture"}), chrono::Utc::now());
        let reading = GitKraken.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines[0],
            lines::count_value("Personal", 12.0, "credits")
        );
        assert!(reading.warning.unwrap().contains("Unlimited"));
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey": "fixture"}), chrono::Utc::now());
            assert_eq!(
                GitKraken
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                category
            );
        }
        let http = Scripted::new();
        let scope = context_at(
            &http,
            json!({"apiKey": "fixture", "orgId": "one\r\ntwo"}),
            chrono::Utc::now(),
        );
        assert!(GitKraken.fetch(&scope.context()).await.is_err());
        assert!(http.requests().is_empty());
    }
}
