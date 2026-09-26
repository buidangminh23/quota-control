use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use uc_core::{
    ErrorCategory, HttpClient, HttpError, HttpRequest, HttpResponse, MetricLine, ProviderRuntime,
    RefreshContext, fixed_clock,
};
use uc_providers::{
    LocalProvider, ProviderKind, credentials::CredentialStore, mapping::map_response,
};

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 25, 12, 0, 0).unwrap()
}

fn response(status: u16, body: Value) -> HttpResponse {
    HttpResponse {
        status,
        headers: HashMap::new(),
        body: serde_json::to_vec(&body).unwrap(),
    }
}

fn progress<'a>(lines: &'a [MetricLine], label: &str) -> &'a uc_core::ProgressLine {
    lines
        .iter()
        .find_map(|line| match line {
            MetricLine::Progress(line) if line.label == label => Some(line),
            _ => None,
        })
        .unwrap()
}

struct FakeHttp {
    response: HttpResponse,
    requests: Mutex<Vec<HttpRequest>>,
}

#[async_trait]
impl HttpClient for FakeHttp {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        self.requests.lock().unwrap().push(request);
        Ok(self.response.clone())
    }
}

fn fake(status: u16, body: Value) -> Arc<FakeHttp> {
    Arc::new(FakeHttp {
        response: response(status, body),
        requests: Mutex::new(Vec::new()),
    })
}

fn setup(
    kind: ProviderKind,
    credentials: Value,
    http: Arc<dyn HttpClient>,
) -> (tempfile::TempDir, LocalProvider) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("credentials.json");
    std::fs::write(&path, serde_json::to_vec(&credentials).unwrap()).unwrap();
    let provider = LocalProvider::new(kind, CredentialStore::new(kind, path), http)
        .with_clock(fixed_clock(now()));
    (directory, provider)
}

enum ResetReply {
    Response(HttpResponse),
    Timeout,
    Transport,
    Pending,
}

struct ResetHttp {
    usage: HttpResponse,
    reset: ResetReply,
    requests: Mutex<Vec<HttpRequest>>,
}

#[async_trait]
impl HttpClient for ResetHttp {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        assert_eq!(request.method, "GET");
        let url = request.url.clone();
        self.requests.lock().unwrap().push(request);
        match url.as_str() {
            "https://chatgpt.com/backend-api/wham/usage"
            | "https://api.anthropic.com/api/oauth/usage" => Ok(self.usage.clone()),
            "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits" => match &self.reset {
                ResetReply::Response(reply) => Ok(reply.clone()),
                ResetReply::Timeout => Err(HttpError::Timeout),
                ResetReply::Transport => {
                    Err(HttpError::Transport("private-transport-detail".into()))
                }
                ResetReply::Pending => std::future::pending().await,
            },
            _ => panic!("unexpected endpoint: {url}"),
        }
    }
}

fn reset_http(usage: HttpResponse, reset: ResetReply) -> Arc<ResetHttp> {
    Arc::new(ResetHttp {
        usage,
        reset,
        requests: Default::default(),
    })
}

fn reset_usage() -> HttpResponse {
    response(
        200,
        json!({
            "rate_limit":{"primary_window":{"used_percent":23}},
            "rate_limit_reset_credits":{"available_count":7}
        }),
    )
}

fn reset_credentials() -> Value {
    json!({"tokens":{"access_token":"fixture-token","account_id":"fixture-account"}})
}

fn reset_line(snapshot: &uc_core::ProviderSnapshot) -> &uc_core::ValuesLine {
    snapshot
        .lines
        .iter()
        .find_map(|line| match line {
            MetricLine::Values(line) if line.label == "Rate Limit Resets" => Some(line),
            _ => None,
        })
        .unwrap()
}

#[tokio::test]
async fn codex_reset_credit_expiries_enrich_count_using_available_credits_only() {
    let first = now() + chrono::Duration::days(1);
    let second = now() + chrono::Duration::days(2);
    let http = reset_http(
        reset_usage(),
        ResetReply::Response(response(
            200,
            json!({"credits":[
                {"status":"available","expires_at":first.to_rfc3339()},
                {"status":"used","expires_at":"not-a-date"},
                {"status":"available","expires_at":second.timestamp()}
            ]}),
        )),
    );
    let (_directory, provider) = setup(ProviderKind::Codex, reset_credentials(), http.clone());
    let snapshot = provider.refresh(RefreshContext::manual()).await;
    assert!(!snapshot.is_error());
    let line = reset_line(&snapshot);
    assert_eq!(
        line.values,
        vec![uc_core::MetricValue::count(7.0, "available")]
    );
    assert_eq!(line.expiries_at, vec![first, second]);
    assert_eq!(progress(&snapshot.lines, "Session").used, 23.0);
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url,
        "https://chatgpt.com/backend-api/wham/usage"
    );
    assert_eq!(
        requests[1].url,
        "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits"
    );
    assert_eq!(requests[0].headers, requests[1].headers);
    for header in [
        ("Authorization", "Bearer fixture-token"),
        ("ChatGPT-Account-Id", "fixture-account"),
        ("Accept", "application/json"),
    ] {
        assert!(
            requests[1]
                .headers
                .contains(&(header.0.into(), header.1.into()))
        );
    }
    assert!(requests[1].timeout <= std::time::Duration::from_secs(3));
}

#[tokio::test]
async fn codex_reset_credit_failures_preserve_the_successful_usage_snapshot() {
    let mut cases: Vec<_> = [401, 429, 500]
        .into_iter()
        .map(|status| ResetReply::Response(response(status, json!({"error":"private-response"}))))
        .collect();
    cases.extend([
        ResetReply::Timeout,
        ResetReply::Transport,
        ResetReply::Response(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: b"private-malformed-json".to_vec(),
        }),
    ]);
    for body in [
        json!(null),
        json!([]),
        json!({}),
        json!({"credits":"bad"}),
        json!({"credits":[{"status":"available","expires_at":true}]}),
        json!({"credits":[{"status":"available","expires_at":"2026-09-26T12:00:00Z"},{"status":"available","expires_at":"bad-date"}]}),
    ] {
        cases.push(ResetReply::Response(response(200, body)));
    }
    for reset in cases {
        let http = reset_http(reset_usage(), reset);
        let (_directory, provider) = setup(ProviderKind::Codex, reset_credentials(), http.clone());
        let snapshot = provider.refresh(RefreshContext::manual()).await;
        assert!(!snapshot.is_error());
        assert_eq!(reset_line(&snapshot).values[0].number, 7.0);
        assert!(reset_line(&snapshot).expiries_at.is_empty());
        assert_eq!(progress(&snapshot.lines, "Session").used, 23.0);
        assert_eq!(http.requests.lock().unwrap().len(), 2);
        assert!(
            !serde_json::to_string(&snapshot)
                .unwrap()
                .contains("private-")
        );
    }
}

#[tokio::test]
async fn codex_reset_credit_request_is_bounded_when_transport_never_completes() {
    let http = reset_http(reset_usage(), ResetReply::Pending);
    let (_directory, provider) = setup(ProviderKind::Codex, reset_credentials(), http.clone());
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        provider.refresh(RefreshContext::manual()),
    )
    .await
    .expect("optional request must not block usage indefinitely");
    assert!(!snapshot.is_error());
    assert_eq!(reset_line(&snapshot).values[0].number, 7.0);
    assert!(reset_line(&snapshot).expiries_at.is_empty());
    assert_eq!(http.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn reset_credit_lookup_is_skipped_without_successful_codex_count() {
    for (kind, usage) in [
        (ProviderKind::Codex, response(401, json!({}))),
        (ProviderKind::Codex, response(429, json!({}))),
        (ProviderKind::Codex, response(500, json!({}))),
        (
            ProviderKind::Codex,
            response(200, json!({"rate_limit":"invalid"})),
        ),
        (
            ProviderKind::Codex,
            response(
                200,
                json!({"rate_limit":{"primary_window":{"used_percent":1}}}),
            ),
        ),
        (
            ProviderKind::Claude,
            response(
                200,
                json!({"five_hour":{"utilization":1},"rate_limit_reset_credits":{"available_count":4}}),
            ),
        ),
    ] {
        let http = reset_http(usage, ResetReply::Pending);
        let credentials = if kind == ProviderKind::Codex {
            reset_credentials()
        } else {
            json!({"claudeAiOauth":{"accessToken":"fixture-token"}})
        };
        let (_directory, provider) = setup(kind, credentials, http.clone());
        provider.refresh(RefreshContext::manual()).await;
        assert_eq!(http.requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn claude_maps_real_units_and_preserves_null_vs_zero() {
    let response = response(
        200,
        json!({
            "five_hour": {"utilization": 0, "resets_at": null},
            "seven_day": {"utilization": 98, "resets_at": "2026-09-29T03:00:00Z"},
            "seven_day_sonnet": null,
            "limits": [{"kind":"weekly_scoped","scope":{"model":{"display_name":"Fable"}},"percent":12}],
            "extra_usage": {"is_enabled":true,"used_credits":1234,"monthly_limit":10000}
        }),
    );
    let mapped = map_response(ProviderKind::Claude, &response, now()).unwrap();
    assert_eq!(progress(&mapped.lines, "Session").used, 0.0);
    assert_eq!(progress(&mapped.lines, "Session").resets_at, None);
    assert_eq!(
        progress(&mapped.lines, "Weekly").period_duration_ms,
        Some(604_800_000)
    );
    assert_eq!(progress(&mapped.lines, "Fable").used, 12.0);
    assert!(!mapped.lines.iter().any(|line| line.label() == "Sonnet"));
    assert_eq!(progress(&mapped.lines, "Extra usage spent").used, 12.34);
    assert_eq!(progress(&mapped.lines, "Extra usage spent").limit, 100.0);
}

#[test]
fn codex_sole_weekly_primary_does_not_become_session() {
    let response = response(
        200,
        json!({
            "plan_type":"prolite",
            "rate_limit":{"primary_window":{"used_percent":84,"limit_window_seconds":604800,"reset_after_seconds":3600}},
            "additional_rate_limits":[null,{"limit_name":"GPT-Codex-Spark","rate_limit":{"primary_window":{"used_percent":3,"limit_window_seconds":18000}}}],
            "credits":{"balance":"14.9"},
            "rate_limit_reset_credits":{"available_count":0}
        }),
    );
    let mapped = map_response(ProviderKind::Codex, &response, now()).unwrap();
    assert_eq!(mapped.plan.as_deref(), Some("Pro 5x"));
    assert!(!mapped.lines.iter().any(|line| line.label() == "Session"));
    assert_eq!(
        progress(&mapped.lines, "Weekly").resets_at,
        Some(now() + chrono::Duration::hours(1))
    );
    assert_eq!(progress(&mapped.lines, "Spark").used, 3.0);
    let json = serde_json::to_value(&mapped.lines).unwrap();
    assert_eq!(json[2]["values"][0]["number"], 0.56);
    assert_eq!(json[2]["values"][1]["number"], 14.0);
    assert_eq!(json[3]["values"][0]["number"], 0.0);
}

#[test]
fn empty_usage_never_fabricates_a_limit() {
    for kind in [ProviderKind::Claude, ProviderKind::Codex] {
        let mapped = map_response(kind, &response(200, json!({})), now()).unwrap();
        assert_eq!(mapped.lines, vec![MetricLine::no_usage_data()]);
    }
}

#[test]
fn malformed_usage_is_a_decoding_error_without_response_body() {
    for (kind, body) in [
        (ProviderKind::Claude, json!([])),
        (
            ProviderKind::Claude,
            json!({"five_hour":{"utilization":true}}),
        ),
        (
            ProviderKind::Claude,
            json!({"five_hour":{"utilization":1,"resets_at":"private-malformed-data"}}),
        ),
        (
            ProviderKind::Codex,
            json!({"rate_limit":"private-malformed-data"}),
        ),
        (ProviderKind::Codex, json!({"credits":{"balance":"NaN"}})),
        (
            ProviderKind::Codex,
            json!({"rate_limit":{"primary_window":{"used_percent":-1}}}),
        ),
    ] {
        let error = map_response(kind, &response(200, body), now())
            .err()
            .unwrap();
        assert_eq!(error.category, ErrorCategory::Decoding);
        assert!(!error.message.contains("private-malformed-data"));
    }
}

#[test]
fn status_errors_are_classified_without_leaking_response() {
    for (status, category) in [
        (401, ErrorCategory::AuthExpired),
        (403, ErrorCategory::AuthInvalid),
        (429, ErrorCategory::RateLimited),
        (500, ErrorCategory::Http5xx),
        (404, ErrorCategory::Http4xx),
    ] {
        let error = map_response(
            ProviderKind::Codex,
            &response(status, json!({"error":"private-response"})),
            now(),
        )
        .err()
        .unwrap();
        assert_eq!(error.category, category);
        assert!(!error.message.contains("private-response"));
    }
}

#[tokio::test]
async fn claude_request_uses_oauth_beta_and_keeps_credentials_unchanged() {
    let http = fake(200, json!({"five_hour":{"utilization":4}}));
    let (directory, provider) = setup(
        ProviderKind::Claude,
        json!({"claudeAiOauth":{"accessToken":"fixture-access","refreshToken":"fixture-refresh","expiresAt":2000000000000_i64,"subscriptionType":"max","rateLimitTier":"default_claude_max_5x"}}),
        http.clone(),
    );
    let path = directory.path().join("credentials.json");
    let original = std::fs::read(&path).unwrap();
    assert!(provider.has_local_credentials().await);
    let snapshot = provider.refresh(RefreshContext::manual()).await;
    assert!(!snapshot.is_error());
    assert_eq!(snapshot.plan.as_deref(), Some("Max 5x"));
    assert_eq!(std::fs::read(path).unwrap(), original);
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].url, "https://api.anthropic.com/api/oauth/usage");
    assert!(
        requests[0]
            .headers
            .contains(&("anthropic-beta".into(), "oauth-2025-04-20".into()))
    );
    assert!(
        requests[0]
            .headers
            .contains(&("Authorization".into(), "Bearer fixture-access".into()))
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("fixture-access"));
    assert!(!serialized.contains("fixture-refresh"));
}

#[tokio::test]
async fn expired_and_scope_limited_logins_never_send_http() {
    for (credentials, category) in [
        (
            json!({"claudeAiOauth":{"accessToken":"fixture","expiresAt":1}}),
            ErrorCategory::AuthExpired,
        ),
        (
            json!({"claudeAiOauth":{"accessToken":"fixture","scopes":["user:inference"]}}),
            ErrorCategory::NotAvailable,
        ),
        (
            json!({"claudeAiOauth":{"accessToken":"bad\r\nheader"}}),
            ErrorCategory::AuthInvalid,
        ),
    ] {
        let http = fake(200, json!({}));
        let (_directory, provider) = setup(ProviderKind::Claude, credentials, http.clone());
        assert_eq!(
            provider
                .refresh(RefreshContext::scheduled())
                .await
                .error_category,
            Some(category)
        );
        assert!(http.requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn codex_account_header_and_auth_file_reload() {
    let http = fake(
        200,
        json!({"rate_limit":{"primary_window":{"used_percent":23}}}),
    );
    let (directory, provider) = setup(
        ProviderKind::Codex,
        json!({"tokens":{"access_token":"fixture-first","account_id":"fixture-account"}}),
        http.clone(),
    );
    provider.refresh(RefreshContext::manual()).await;
    std::fs::write(
        directory.path().join("credentials.json"),
        br#"{"tokens":{"access_token":"fixture-second","account_id":"fixture-next"}}"#,
    )
    .unwrap();
    provider.refresh(RefreshContext::manual()).await;
    let requests = http.requests.lock().unwrap();
    assert!(
        requests[0]
            .headers
            .contains(&("ChatGPT-Account-Id".into(), "fixture-account".into()))
    );
    assert!(
        requests[1]
            .headers
            .contains(&("Authorization".into(), "Bearer fixture-second".into()))
    );
    assert!(
        requests[1]
            .headers
            .contains(&("ChatGPT-Account-Id".into(), "fixture-next".into()))
    );
}

#[tokio::test]
async fn codex_expired_jwt_and_api_key_return_truthful_errors() {
    let jwt = format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(br#"{"exp":1}"#)
    );
    for (credentials, category) in [
        (
            json!({"tokens":{"access_token":jwt}}),
            ErrorCategory::AuthExpired,
        ),
        (
            json!({"OPENAI_API_KEY":"fixture-api-key"}),
            ErrorCategory::NotAvailable,
        ),
    ] {
        let http = fake(200, json!({}));
        let (_directory, provider) = setup(ProviderKind::Codex, credentials, http.clone());
        assert_eq!(
            provider
                .refresh(RefreshContext::scheduled())
                .await
                .error_category,
            Some(category)
        );
        assert!(http.requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn rate_limit_honors_cooldown_even_for_manual_refresh() {
    let http = fake(429, json!({}));
    let (_directory, provider) = setup(
        ProviderKind::Codex,
        json!({"tokens":{"access_token":"fixture-token"}}),
        http.clone(),
    );
    assert_eq!(
        provider
            .refresh(RefreshContext::manual())
            .await
            .error_category,
        Some(ErrorCategory::RateLimited)
    );
    assert_eq!(
        provider
            .refresh(RefreshContext::manual())
            .await
            .error_category,
        Some(ErrorCategory::RateLimited)
    );
    assert_eq!(http.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn missing_unreadable_and_malformed_credentials_are_distinct() {
    let directory = tempfile::tempdir().unwrap();
    let missing = CredentialStore::new(ProviderKind::Codex, directory.path().join("missing.json"));
    assert_eq!(
        missing.load().await.err().unwrap().category,
        ErrorCategory::NotLoggedIn
    );
    let unreadable = CredentialStore::new(ProviderKind::Codex, directory.path());
    assert_eq!(
        unreadable.load().await.err().unwrap().category,
        ErrorCategory::CredentialAccess
    );
    let path = directory.path().join("invalid.json");
    std::fs::write(&path, b"malformed-private-content").unwrap();
    let invalid = CredentialStore::new(ProviderKind::Codex, path);
    let error = invalid.load().await.err().unwrap();
    assert_eq!(error.category, ErrorCategory::AuthInvalid);
    assert!(!error.message.contains("malformed-private-content"));
}
