//! Vertex AI: the peak share of a Google Cloud project's Vertex AI quotas used over the last 24
//! hours, as one meter for the busiest quota and one row per quota.
//!
//! The gcloud application default credentials are read, never written, from `%APPDATA%\gcloud` on
//! Windows and `~/.config/gcloud` on macOS and Linux, or from the folder `CLOUDSDK_CONFIG` names;
//! `GOOGLE_APPLICATION_CREDENTIALS` can name the credentials file itself. Only an
//! `authorized_user` file with an access or refresh token becomes a login, and its refresh token is
//! never kept. The project comes from `GOOGLE_CLOUD_PROJECT`, the file's `project_id`, or the
//! `[core]` project of the active gcloud configuration (`CLOUDSDK_ACTIVE_CONFIG_NAME`, else the
//! `active_config` file, else `default`). An access token and a project ID can instead be saved in
//! Quota Control. A login with only a refresh token is shown as not available, because renewing it
//! would need the shared OAuth client's static client credentials; an expired access token is not
//! sent either.
//!
//! A refresh sends two to four requests,
//! `GET https://monitoring.googleapis.com/v3/projects/<project>/timeSeries` with the token as a
//! bearer token: the quota allocation usage and then the quota limits of
//! `aiplatform.googleapis.com` over the last 24 hours, each read in at most two pages of 1,000
//! series aligned to their hourly maximum. Each quota's peak usage is compared with its matching
//! limit, and a warning says when more pages were left. The approach follows CodexBar's
//! `VertexAIOAuthCredentials.swift` and `VertexAIUsageFetcher.swift`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Duration;
use serde_json::{Value, json};
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{http, lines, value};

pub(crate) struct VertexAi;

const BASE: &str = "https://monitoring.googleapis.com/v3/projects";
const REFRESH: &str = "Vertex AI needs a saved access token. This version cannot renew refresh-only gcloud credentials.";

/// A quota: its metric, its limit's name (empty when the series names none) and its location.
type Key = (String, String, String);

/// The text of a file of at most 64 KiB.
fn small_text(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > 65536 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The project a gcloud login works in: `GOOGLE_CLOUD_PROJECT`, the credentials' `project_id`, or
/// the `[core]` project of the active gcloud configuration.
fn project(roots: &Roots, config: &Path, credentials: &Value) -> Option<String> {
    if let Some(project) = roots
        .var("GOOGLE_CLOUD_PROJECT")
        .or_else(|| value::text(credentials, "/project_id").map(str::to_owned))
    {
        return Some(project);
    }
    let active = roots
        .var("CLOUDSDK_ACTIVE_CONFIG_NAME")
        .or_else(|| small_text(&config.join("active_config")).map(|name| name.trim().to_owned()))
        .unwrap_or_else(|| "default".into());
    if !active
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return None;
    }
    let ini = small_text(
        &config
            .join("configurations")
            .join(format!("config_{active}")),
    )?;
    let mut in_core = false;
    for line in ini.lines().map(str::trim) {
        if line.starts_with('[') {
            in_core = line == "[core]";
        } else if in_core
            && let Some((name, setting)) = line.split_once('=')
            && name.trim() == "project"
            && !setting.trim().is_empty()
        {
            return Some(setting.trim().into());
        }
    }
    None
}

#[async_trait]
impl Service for VertexAi {
    fn id(&self) -> &'static str {
        "vertexai"
    }

    fn name(&self) -> &'static str {
        "Vertex AI"
    }

    fn key_label(&self) -> &'static str {
        "Google OAuth access token"
    }

    fn connection(&self) -> Connection {
        Connection::login("gcloud ADC").or_api_key(ApiKeyHelp {
            env: &[],
            url: "https://console.cloud.google.com/iam-admin/iam",
            fields: &[("projectId", "Google Cloud project ID")],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let config = roots.dir_from(
            "CLOUDSDK_CONFIG",
            if cfg!(windows) {
                roots.app_data.join("gcloud")
            } else {
                roots.home.join(".config/gcloud")
            },
        );
        let path = roots
            .var("GOOGLE_APPLICATION_CREDENTIALS")
            .map(PathBuf::from)
            .unwrap_or_else(|| config.join("application_default_credentials.json"));
        let Some(credentials) = value::read_json(&path, 262144) else {
            return vec![];
        };
        if value::text(&credentials, "/type") != Some("authorized_user") {
            return vec![];
        }
        let token = value::text(&credentials, "/access_token");
        let refresh = value::text(&credentials, "/refresh_token").is_some();
        if token.is_none() && !refresh {
            return vec![];
        }
        let project_id = project(roots, &config, &credentials);
        let email = value::text(&credentials, "/client_email")
            .or_else(|| value::text(&credentials, "/account"));
        let identity = format!(
            "{}:{}",
            email.unwrap_or("gcloud"),
            project_id.as_deref().unwrap_or("unknown")
        );
        vec![
            Login::new(
                identity,
                "gcloud ADC",
                &path,
                Secret::new(json!({
                    "access_token": token,
                    "refresh_only": refresh,
                    "projectId": project_id,
                    "expiry": credentials.get("expiry").or_else(|| credentials.get("expiry_date"))
                })),
            )
            .with_label(email.map(str::to_owned)),
        ]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::percent(
                format!("{}.quota", provider.id),
                provider,
                "Quota Usage (24h Peak)",
                None,
                None,
            )
            .exporting_progress("quota", "percent"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let token = context
            .secret
            .key()
            .or_else(|| context.secret.str("/access_token"))
            .ok_or_else(|| http::not_available(REFRESH))?;
        if value::time(context.secret.value(), "/expiry")
            .is_some_and(|expiry| expiry <= context.now)
        {
            return Err(http::expired(
                "The Vertex AI access token expired. Save a fresh access token.",
            ));
        }
        let project = context
            .secret
            .str("/projectId")
            .ok_or_else(|| http::invalid("The Google Cloud project ID is missing."))?;
        if !project
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(http::invalid(
                "The Google Cloud project ID contains invalid characters.",
            ));
        }
        let (usage, more_usage) = series(
            context,
            token,
            project,
            "serviceruntime.googleapis.com/quota/allocation/usage",
        )
        .await?;
        let (limits, more_limits) = series(
            context,
            token,
            project,
            "serviceruntime.googleapis.com/quota/limit",
        )
        .await?;
        let mut matched = Vec::new();
        for (key, used) in usage {
            let limit = limits.get(&key).copied().or_else(|| {
                if !key.1.is_empty() {
                    return None;
                }
                let mut found = limits
                    .iter()
                    .filter(|(candidate, _)| candidate.0 == key.0 && candidate.2 == key.2);
                let first = found.next().map(|(_, limit)| *limit);
                if found.next().is_none() { first } else { None }
            });
            if let Some(limit) = limit.filter(|limit| *limit > 0.0) {
                matched.push((key, used, limit));
            }
        }
        matched.sort_by(|left, right| {
            (right.1 / right.2)
                .total_cmp(&(left.1 / left.2))
                .then(left.0.cmp(&right.0))
        });
        let Some(first) = matched.first() else {
            return Err(http::not_available(
                "Google Monitoring returned no matching Vertex AI quota usage and limits.",
            ));
        };
        let mut rows = vec![lines::percent(
            "Quota Usage (24h Peak)",
            first.1 / first.2 * 100.0,
            None,
            None,
        )];
        for ((metric, limit, location), used, total) in matched {
            rows.push(lines::count(
                &format!("{metric} · {limit} · {location}"),
                used,
                total,
                "units",
                None,
                None,
            ));
        }
        Ok(
            Reading::new(None, rows).with_warning((more_usage || more_limits).then(|| {
                "Vertex AI monitoring returned more pages than this refresh can include.".into()
            })),
        )
    }
}

/// The peak of each quota's `metric` over the last 24 hours, read in at most two pages, and
/// whether Google Monitoring had more pages left.
async fn series(
    context: &FetchContext<'_>,
    token: &str,
    project: &str,
    metric: &str,
) -> Result<(BTreeMap<Key, f64>, bool), SimpleProviderError> {
    let filter = format!(
        "metric.type=\"{metric}\" AND resource.type=\"consumer_quota\" AND resource.label.service=\"aiplatform.googleapis.com\""
    );
    let mut page: Option<String> = None;
    let mut result: BTreeMap<Key, f64> = BTreeMap::new();
    for _ in 0..2 {
        let mut url = url::Url::parse(&format!("{BASE}/{project}/timeSeries"))
            .map_err(|_| http::decoding("Vertex AI"))?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("filter", &filter)
                .append_pair(
                    "interval.startTime",
                    &(context.now - Duration::hours(24)).to_rfc3339(),
                )
                .append_pair("interval.endTime", &context.now.to_rfc3339())
                .append_pair("aggregation.alignmentPeriod", "3600s")
                .append_pair("aggregation.perSeriesAligner", "ALIGN_MAX")
                .append_pair("view", "FULL")
                .append_pair("pageSize", "1000");
            if let Some(page_token) = &page {
                query.append_pair("pageToken", page_token);
            }
        }
        let body = http::json(
            context.http,
            HttpRequest::get(url.as_str()).bearer(token),
            "Vertex AI",
        )
        .await?;
        if !body.is_object() {
            return Err(http::decoding("Vertex AI"));
        }
        if let Some(entries) = body.get("timeSeries") {
            let entries = entries
                .as_array()
                .ok_or_else(|| http::decoding("Vertex AI"))?;
            for entry in entries {
                let Some(metric) = value::text(entry, "/metric/labels/quota_metric")
                    .or_else(|| value::text(entry, "/resource/labels/quota_id"))
                else {
                    continue;
                };
                let key = (
                    metric.to_owned(),
                    value::text(entry, "/metric/labels/limit_name")
                        .unwrap_or("")
                        .to_owned(),
                    value::text(entry, "/resource/labels/location")
                        .unwrap_or("global")
                        .to_owned(),
                );
                let peak = entry
                    .get("points")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|point| {
                        value::number(point, "/value/doubleValue")
                            .or_else(|| value::number(point, "/value/int64Value"))
                    })
                    .filter(|number| *number >= 0.0)
                    .max_by(f64::total_cmp);
                if let Some(peak) = peak {
                    result
                        .entry(key)
                        .and_modify(|old| *old = old.max(peak))
                        .or_insert(peak);
                }
            }
        }
        page = value::text(&body, "/nextPageToken").map(str::to_owned);
        if page.is_none() {
            break;
        }
    }
    Ok((result, page.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use uc_core::ErrorCategory;

    /// A Monitoring answer with one `requests` · `minute` · `us` series whose only point is `peak`.
    fn answer_with_one_series(peak: i32) -> String {
        json!({
            "timeSeries": [{
                "metric": {"labels": {"quota_metric": "requests", "limit_name": "minute"}},
                "resource": {"labels": {"location": "us"}},
                "points": [{"value": {"int64Value": peak.to_string()}}]
            }]
        })
        .to_string()
    }

    #[tokio::test]
    async fn each_quota_peak_is_read_against_its_limit_with_the_token_as_a_bearer() {
        let url = format!("{BASE}/project-one/timeSeries");
        let http = Scripted::new()
            .on("GET", &url, 200, &answer_with_one_series(25))
            .on("GET", &url, 200, &answer_with_one_series(100));
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 0, 0, 0).unwrap();
        let scope = context_at(
            &http,
            json!({"apiKey":"test","projectId":"project-one"}),
            now,
        );
        let reading = VertexAi.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::percent("Quota Usage (24h Peak)", 25.0, None, None),
                lines::count("requests · minute · us", 25.0, 100.0, "units", None, None)
            ]
        );
        assert_eq!(http.requests().len(), 2);
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer test")
        );
        assert!(http.requests()[0].url.contains("ALIGN_MAX"));
    }

    #[tokio::test]
    async fn refresh_only_and_expired_send_nothing() {
        for secret in [
            json!({"refresh_only":true}),
            json!({"access_token":"test","expiry":1,"projectId":"p"}),
        ] {
            let http = Scripted::new();
            let scope = context_at(&http, secret, Utc::now());
            assert!(VertexAi.fetch(&scope.context()).await.is_err());
            assert!(http.requests().is_empty());
        }
    }

    #[tokio::test]
    async fn pagination_stops_at_four_requests_with_notice() {
        let mut usage: Value = serde_json::from_str(&answer_with_one_series(25)).unwrap();
        usage["nextPageToken"] = json!("next");
        let mut limit: Value = serde_json::from_str(&answer_with_one_series(100)).unwrap();
        limit["nextPageToken"] = json!("more");
        let http = Scripted::new()
            .on("GET", BASE, 200, &usage.to_string())
            .on("GET", BASE, 200, &usage.to_string())
            .on("GET", BASE, 200, &limit.to_string())
            .on("GET", BASE, 200, &limit.to_string());
        let scope = context_at(&http, json!({"apiKey":"test", "projectId":"p"}), Utc::now());
        let reading = VertexAi.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines[0],
            lines::percent("Quota Usage (24h Peak)", 25.0, None, None)
        );
        assert!(reading.warning.is_some());
        assert_eq!(http.requests().len(), 4);
        assert!(http.requests()[1].url.contains("pageToken=next"));
    }

    #[tokio::test]
    async fn refused_rate_limited_and_unreadable_answers_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", BASE, status, "[]");
            let scope = context_at(&http, json!({"apiKey":"test","projectId":"p"}), Utc::now());
            assert_eq!(
                VertexAi.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }

    #[test]
    fn discovery_reads_active_config_without_writes() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("gcloud");
        std::fs::create_dir_all(config.join("configurations")).unwrap();
        std::fs::write(config.join("active_config"), "work").unwrap();
        std::fs::write(
            config.join("configurations/config_work"),
            "[core]\nproject = project-one\n",
        )
        .unwrap();
        let path = config.join("application_default_credentials.json");
        let bytes = r#"{"type":"authorized_user","refresh_token":"fixture","client_id":"dynamic","client_secret":"fixture"}"#;
        std::fs::write(&path, bytes).unwrap();
        let roots = Roots::under(dir.path()).with_var("CLOUDSDK_CONFIG", config.to_str().unwrap());
        let found = VertexAi.discover(&roots);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].secret.str("/projectId"), Some("project-one"));
        assert!(found[0].secret.str("/refresh_token").is_none());
        assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
    }
}
