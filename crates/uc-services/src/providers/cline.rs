//! Cline: the Session (5-hour), Weekly and Monthly usage limits of a Cline plan, as the share of
//! each window used. The login Cline keeps in `providers.json` is found the same way on Windows,
//! macOS and Linux: `CLINE_PROVIDER_SETTINGS_PATH` when it is set, else `settings/providers.json`
//! under `CLINE_DATA_DIR`, else `data/settings/providers.json` under `CLINE_DIR` or `~/.cline`.
//! Its WorkOS access token (`auth.accessToken`) is taken before a saved API key (`apiKey` or
//! `auth.apiKey`). A Cline API key can also come from `CLINE_API_KEY` or `CLINEPASS_API_KEY` or be
//! saved in Quota Control.
//!
//! A refresh sends one request, `GET https://api.cline.bot/api/v1/users/me/plan/usage-limits`,
//! with the key or token as a bearer token. A login token is used as saved and never renewed here:
//! an expired one is not sent, and an expired or refused login asks for Cline to be opened once.

use async_trait::async_trait;
use serde_json::json;
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{http, jwt, lines, value};

pub(crate) struct Cline;

const NAME: &str = "Cline";
const URL: &str = "https://api.cline.bot/api/v1/users/me/plan/usage-limits";
const EXPIRED: &str = "The Cline login expired. Open Cline once to renew it.";

/// Each window: its `type` in the answer, the widget id, the row title and the window's length.
const WINDOWS: [(&str, &str, &str, i64); 3] = [
    ("five_hour", "session", "Session", 5 * lines::HOUR_MS),
    ("weekly", "weekly", "Weekly", lines::WEEK_MS),
    ("monthly", "monthly", "Monthly", lines::MONTH_MS),
];

#[async_trait]
impl Service for Cline {
    fn id(&self) -> &'static str {
        "cline"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::login("Cline").or_api_key(ApiKeyHelp {
            env: &["CLINE_API_KEY", "CLINEPASS_API_KEY"],
            url: "https://app.cline.bot",
            fields: &[],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let env_path = |name: &str| {
            roots.var(name).map(|raw| {
                if raw == "~" {
                    roots.home.clone()
                } else if let Some(tail) = raw.strip_prefix("~/") {
                    roots.home.join(tail)
                } else {
                    std::path::PathBuf::from(raw)
                }
            })
        };
        let path = env_path("CLINE_PROVIDER_SETTINGS_PATH").unwrap_or_else(|| {
            env_path("CLINE_DATA_DIR")
                .unwrap_or_else(|| {
                    env_path("CLINE_DIR")
                        .unwrap_or_else(|| roots.home.join(".cline"))
                        .join("data")
                })
                .join("settings/providers.json")
        });
        let Some(body) = value::read_json(&path, 1024 * 1024) else {
            return Vec::new();
        };
        let settings = &body["providers"]["cline"]["settings"];
        let (token, oauth) = if let Some(v) = value::text(settings, "/auth/accessToken") {
            (
                if v.starts_with("workos:") {
                    v.to_string()
                } else {
                    format!("workos:{v}")
                },
                true,
            )
        } else if let Some(key) =
            value::text(settings, "/apiKey").or_else(|| value::text(settings, "/auth/apiKey"))
        {
            (key.into(), false)
        } else {
            return Vec::new();
        };
        let identity: String = Sha256::digest(token.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        vec![Login::new(
            identity,
            "Cline",
            &path,
            Secret::new(json!({"apiKey":token,"oauth":oauth})),
        )]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        WINDOWS
            .iter()
            .map(|(_, id, title, _)| {
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
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Cline API key is missing."))?;
        let oauth = context.secret.value()["oauth"] == true;
        if oauth
            && jwt::expires_at(key.strip_prefix("workos:").unwrap_or(key))
                .is_some_and(|expiry| expiry <= context.now)
        {
            return Err(http::expired(EXPIRED));
        }
        let response = http::send(
            context.http,
            HttpRequest::get(URL)
                .bearer(key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        if oauth && matches!(response.status, 401 | 403) {
            return Err(http::expired(EXPIRED));
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let body = http::parse(&response, NAME)?;
        if body["success"] != true {
            return Err(http::decoding(NAME));
        }
        let limits = body["data"]["limits"]
            .as_array()
            .ok_or_else(|| http::decoding(NAME))?;
        let mut rows = Vec::new();
        for (kind, _, title, period) in WINDOWS {
            let mut last = None;
            for item in limits {
                let item_kind = item["type"].as_str().ok_or_else(|| http::decoding(NAME))?;
                if item_kind != kind {
                    continue;
                }
                let used = item["percentUsed"]
                    .as_f64()
                    .filter(|percent| percent.is_finite())
                    .ok_or_else(|| http::decoding(NAME))?;
                let reset = if item["resetsAt"].is_null() {
                    None
                } else {
                    Some(value::time(item, "/resetsAt").ok_or_else(|| http::decoding(NAME))?)
                };
                last = Some(lines::percent(title, used, reset, Some(period)));
            }
            rows.extend(last);
        }
        Ok(Reading::new(None, rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use uc_core::ErrorCategory;

    const KEY_AND_LOGIN: &str = r#"{"providers":{"cline":{"settings":{"apiKey":"old-key","auth":{"accessToken":"access-fixture"}}}}}"#;
    const THREE_WINDOWS: &str = r#"{"success":true,"data":{"limits":[{"type":"five_hour","percentUsed":25,"resetsAt":"2026-10-01T00:00:00Z"},{"type":"weekly","percentUsed":12.5},{"type":"monthly","percentUsed":2}]}}"#;

    #[test]
    fn discovery_prefers_the_login_token_and_gives_it_the_workos_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        assert!(Cline.discover(&roots).is_empty());
        let path = roots.home.join(".cline/data/settings/providers.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, KEY_AND_LOGIN).unwrap();
        let entries = Cline.discover(&roots);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].identity,
            Sha256::digest(b"workos:access-fixture")
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert_eq!(entries[0].secret.key(), Some("workos:access-fixture"));
        assert!(entries[0].label.is_none());
    }

    #[tokio::test]
    async fn each_window_becomes_a_row_from_one_bearer_authorized_get() {
        let http = Scripted::new().on("GET", URL, 200, THREE_WINDOWS);
        let scope = context_at(
            &http,
            json!({"apiKey":"workos:fixture","oauth":true}),
            chrono::Utc::now(),
        );
        assert_eq!(
            Cline.fetch(&scope.context()).await.unwrap(),
            Reading::new(
                None,
                vec![
                    lines::percent(
                        "Session",
                        25.0,
                        Some("2026-10-01T00:00:00Z".parse().unwrap()),
                        Some(5 * lines::HOUR_MS)
                    ),
                    lines::percent("Weekly", 12.5, None, Some(lines::WEEK_MS)),
                    lines::percent("Monthly", 2.0, None, Some(lines::MONTH_MS))
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(
            header(&requests[0], "authorization"),
            Some("Bearer workos:fixture")
        );
    }

    #[tokio::test]
    async fn failures_are_reported_after_one_request_without_renewing_the_login() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
            (
                200,
                r#"{"success":true,"data":{"limits":[{"type":"weekly","percentUsed":"bad"}]}}"#,
                ErrorCategory::Decoding,
            ),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(
                &http,
                json!({"apiKey":"fixture","oauth":true}),
                chrono::Utc::now(),
            );
            assert_eq!(
                Cline.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
            assert_eq!(http.requests().len(), 1);
        }
    }
}
