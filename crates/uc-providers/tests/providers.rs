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
            "https://api.anthropic.com/api/oauth/profile" => Ok(response(
                200,
                json!({"organization":{"organization_type":"claude_pro"}}),
            )),
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
        let requests = http.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .all(|request| !request.url.ends_with("rate-limit-reset-credits"))
        );
        assert_eq!(
            requests.len(),
            if kind == ProviderKind::Claude { 2 } else { 1 }
        );
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
    let http = fake(
        200,
        json!({"five_hour":{"utilization":4}, "organization":{"organization_type":"claude_max", "rate_limit_tier":"default_claude_max_5x"}}),
    );
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
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/api/oauth/profile"
    );
    assert_eq!(requests[1].url, "https://api.anthropic.com/api/oauth/usage");
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

struct RedeemHttp {
    replies: Mutex<HashMap<String, HttpResponse>>,
    requests: Mutex<Vec<HttpRequest>>,
}

#[async_trait]
impl HttpClient for RedeemHttp {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let reply = self.replies.lock().unwrap().get(&request.url).cloned();
        self.requests.lock().unwrap().push(request);
        reply.ok_or(HttpError::Transport("unexpected endpoint".into()))
    }
}

fn redeem_http(replies: &[(&str, HttpResponse)]) -> Arc<RedeemHttp> {
    Arc::new(RedeemHttp {
        replies: Mutex::new(
            replies
                .iter()
                .map(|(url, reply)| (url.to_string(), reply.clone()))
                .collect(),
        ),
        requests: Default::default(),
    })
}

const CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const CONSUME_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume";

#[tokio::test]
async fn codex_lists_spendable_reset_credits_soonest_first() {
    let later = now() + chrono::Duration::days(9);
    let sooner = now() + chrono::Duration::days(1);
    let http = redeem_http(&[(
        CREDITS_URL,
        response(
            200,
            json!({"credits":[
                {"id":"credit-later","status":"available","expires_at":later.to_rfc3339()},
                {"id":"credit-used","status":"used","expires_at":sooner.to_rfc3339()},
                {"id":"credit-sooner","status":"available","expires_at":sooner.timestamp()},
                {"id":"credit-open","status":"available","expires_at":null}
            ]}),
        ),
    )]);
    let (_directory, provider) = setup(ProviderKind::Codex, reset_credentials(), http.clone());
    let credits = provider.limit_reset_credits().await.unwrap();
    let ids: Vec<_> = credits.iter().map(|credit| credit.id.as_str()).collect();
    assert_eq!(ids, ["credit-sooner", "credit-later", "credit-open"]);
    assert_eq!(credits[0].expires_at, Some(sooner));
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert!(
        requests[0]
            .headers
            .contains(&("ChatGPT-Account-Id".into(), "fixture-account".into()))
    );
}

#[tokio::test]
async fn codex_spends_the_chosen_credit_with_its_request_id() {
    let http = redeem_http(&[(
        CONSUME_URL,
        response(
            200,
            json!({"code":"reset","credit":{"id":"credit-sooner","reset_type":"weekly"}}),
        ),
    )]);
    let (_directory, provider) = setup(ProviderKind::Codex, reset_credentials(), http.clone());
    let reply = provider
        .redeem_limit_reset("credit-sooner", "request-1")
        .await
        .unwrap();
    assert_eq!(reply.code, "reset");
    assert_eq!(reply.reset_type.as_deref(), Some("weekly"));
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    let body: Value = serde_json::from_slice(requests[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(
        body,
        json!({"credit_id":"credit-sooner","redeem_request_id":"request-1"})
    );
    for header in [
        ("Authorization", "Bearer fixture-token"),
        ("ChatGPT-Account-Id", "fixture-account"),
        ("Content-Type", "application/json"),
    ] {
        assert!(
            requests[0]
                .headers
                .contains(&(header.0.into(), header.1.into()))
        );
    }
}

#[tokio::test]
async fn codex_reports_refusals_and_failures_without_leaking_the_response() {
    let http = redeem_http(&[(
        CONSUME_URL,
        response(
            200,
            json!({"code":"nothing_to_reset","detail":"private-detail"}),
        ),
    )]);
    let (_directory, provider) = setup(ProviderKind::Codex, reset_credentials(), http);
    let reply = provider
        .redeem_limit_reset("credit", "request")
        .await
        .unwrap();
    assert_eq!(reply.code, "nothing_to_reset");
    assert_eq!(reply.reset_type, None);

    for (status, category) in [
        (403, ErrorCategory::AuthExpired),
        (429, ErrorCategory::RateLimited),
        (500, ErrorCategory::Http5xx),
    ] {
        let http = redeem_http(&[(
            CONSUME_URL,
            response(status, json!({"error":"private-body"})),
        )]);
        let (_directory, provider) = setup(ProviderKind::Codex, reset_credentials(), http);
        let error = provider
            .redeem_limit_reset("credit", "request")
            .await
            .unwrap_err();
        assert_eq!(error.category, category);
        assert!(!error.message.contains("private-"));
    }

    let offline = redeem_http(&[]);
    let (_directory, provider) = setup(ProviderKind::Codex, reset_credentials(), offline);
    assert_eq!(
        provider
            .redeem_limit_reset("credit", "request")
            .await
            .unwrap_err()
            .category,
        ErrorCategory::Network
    );
}

#[tokio::test]
async fn claude_has_no_limit_resets_and_sends_nothing() {
    let http = redeem_http(&[]);
    let (_directory, provider) = setup(
        ProviderKind::Claude,
        json!({"claudeAiOauth":{"accessToken":"fixture-token"}}),
        http.clone(),
    );
    assert_eq!(
        provider.limit_reset_credits().await.unwrap_err().category,
        ErrorCategory::NotAvailable
    );
    assert_eq!(
        provider
            .redeem_limit_reset("credit", "request")
            .await
            .unwrap_err()
            .category,
        ErrorCategory::NotAvailable
    );
    assert!(http.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn codex_cards_carry_the_paid_through_date_their_login_states() {
    let claims = json!({"https://api.openai.com/auth":{
        "chatgpt_plan_type":"plus",
        "chatgpt_subscription_active_until":"2026-10-17T01:56:39+00:00",
        "chatgpt_subscription_last_checked":"2026-09-25T11:56:24+00:00"
    }});
    let id_token = format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    let http = fake(
        200,
        json!({"plan_type":"plus", "rate_limit":{"primary_window":{"used_percent":23}}}),
    );
    let (_directory, provider) = setup(
        ProviderKind::Codex,
        json!({"tokens":{"access_token":"fixture-token","account_id":"fixture-account","id_token":id_token}}),
        http.clone(),
    );
    let snapshot = provider.refresh(RefreshContext::manual()).await;
    assert!(!snapshot.is_error(), "{:?}", snapshot.error_category);
    assert_eq!(
        snapshot.plan_term,
        Some(uc_core::PlanTerm::Stated {
            ends_at: Utc.with_ymd_and_hms(2026, 10, 17, 1, 56, 39).unwrap(),
            checked_at: Some(Utc.with_ymd_and_hms(2026, 9, 25, 11, 56, 24).unwrap()),
        })
    );
    assert_eq!(http.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn claude_cards_from_a_credentials_file_refresh_the_profile_and_never_invent_a_term() {
    let http = fake(200, json!({"five_hour":{"utilization":15}}));
    let (_directory, provider) = setup(
        ProviderKind::Claude,
        json!({"claudeAiOauth":{"accessToken":"fixture-token"}}),
        http.clone(),
    );
    let snapshot = provider.refresh(RefreshContext::manual()).await;
    assert!(!snapshot.is_error(), "{:?}", snapshot.error_category);
    assert_eq!(snapshot.plan_term, None);
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/api/oauth/profile"
    );
    assert_eq!(requests[1].url, "https://api.anthropic.com/api/oauth/usage");
}

struct SubscriptionHttp {
    profiles: Mutex<std::collections::VecDeque<Value>>,
    profile_calls: std::sync::atomic::AtomicUsize,
    usage_calls: std::sync::atomic::AtomicUsize,
    usage_status: u16,
}

#[async_trait]
impl HttpClient for SubscriptionHttp {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        if request.url.ends_with("/profile") {
            self.profile_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Ok(response(
                200,
                self.profiles.lock().unwrap().pop_front().unwrap(),
            ));
        }
        assert!(request.url.ends_with("/usage"));
        self.usage_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut reply = response(self.usage_status, json!({"five_hour":{"utilization":15}}));
        reply.headers.insert("retry-after".into(), "3600".into());
        Ok(reply)
    }
}

#[tokio::test]
async fn claude_plan_downgrades_and_renews_while_usage_remains_rate_limited() {
    let client = Arc::new(SubscriptionHttp {
        profiles: Mutex::new([
            json!({"organization":{"organization_type":"claude_pro", "subscription_created_at":"2026-09-15T00:00:00Z"}}),
            json!({"organization":{"organization_type":"claude_free","subscription_status":"canceled","subscription_created_at":"2026-09-15T00:00:00Z"}}),
            json!({"organization":{"organization_type":"claude_pro","subscription_status":"active", "subscription_created_at":"2026-09-20T00:00:00Z"}}),
        ].into()),
        profile_calls: Default::default(),
        usage_calls: Default::default(),
        usage_status: 429,
    });
    let clock = Arc::new(Mutex::new(now()));
    let read_clock = clock.clone();
    let (directory, provider) = setup(
        ProviderKind::Claude,
        json!({"claudeAiOauth":{"accessToken":"fixture","subscriptionType":"pro"}}),
        client.clone(),
    );
    let provider = provider.with_clock(Arc::new(move || *read_clock.lock().unwrap()));
    for (offset, plan) in [(0, "Pro"), (5, "Free"), (10, "Pro")] {
        let checked = now() + chrono::Duration::minutes(offset);
        *clock.lock().unwrap() = checked;
        let snapshot = provider.refresh(RefreshContext::scheduled()).await;
        assert_eq!(snapshot.error_category, Some(ErrorCategory::RateLimited));
        assert_eq!(snapshot.plan.as_deref(), Some(plan));
        assert_eq!(snapshot.plan_checked_at, Some(checked));
        assert_eq!(snapshot.plan_term, None);
    }
    assert_eq!(
        client
            .profile_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        3
    );
    assert_eq!(
        client.usage_calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    std::fs::write(directory.path().join("credentials.json"), b"{}").unwrap();
    let invalid = provider.refresh(RefreshContext::manual()).await;
    assert_eq!(invalid.error_category, Some(ErrorCategory::AuthInvalid));
    assert_eq!(invalid.plan, None);
    assert_eq!(invalid.plan_checked_at, None);
    assert_eq!(invalid.plan_term, None);
}

#[tokio::test]
async fn claude_resubscription_and_upgrades_do_not_make_historical_start_a_billing_anchor() {
    let client = Arc::new(SubscriptionHttp {
        profiles: Mutex::new([
            json!({"organization":{"organization_type":"claude_max", "rate_limit_tier":"default_claude_max_5x", "subscription_created_at":"2026-07-31T00:00:00Z", "subscription_status":"active"}}),
            json!({"organization":{"organization_type":"claude_max", "rate_limit_tier":"default_claude_max_20x", "subscription_created_at":"2026-07-31T00:00:00Z", "subscription_status":"active", "cancel_at_period_end":true}}),
            json!({"organization":{"organization_type":"claude_max", "rate_limit_tier":"default_claude_max_20x"}}),
        ].into()),
        profile_calls: Default::default(),
        usage_calls: Default::default(),
        usage_status: 200,
    });
    let resubscribed_at = Utc.with_ymd_and_hms(2026, 10, 3, 7, 3, 48).unwrap();
    let clock = Arc::new(Mutex::new(resubscribed_at));
    let read_clock = clock.clone();
    let (_directory, provider) = setup(
        ProviderKind::Claude,
        json!({"claudeAiOauth":{"accessToken":"fixture","subscriptionType":"pro"}, "oauthAccount":{"subscriptionCreatedAt":"2026-07-05T00:00:00Z"}}),
        client.clone(),
    );
    let provider = provider.with_clock(Arc::new(move || *read_clock.lock().unwrap()));
    for (offset, confirmation_offset, plan) in [
        (0, 0, "Max 5x"),
        (4, 0, "Max 5x"),
        (5, 5, "Max 20x"),
        (10, 10, "Max 20x"),
    ] {
        *clock.lock().unwrap() = resubscribed_at + chrono::Duration::minutes(offset);
        let checked_at = resubscribed_at + chrono::Duration::minutes(confirmation_offset);
        let snapshot = provider.refresh(RefreshContext::scheduled()).await;
        assert_eq!(snapshot.error_category, None);
        assert_eq!(snapshot.plan.as_deref(), Some(plan));
        assert_eq!(snapshot.plan_checked_at, Some(checked_at));
        assert_eq!(snapshot.plan_term, None);
        assert_eq!(progress(&snapshot.lines, "Session").used, 15.0);
    }
    assert_eq!(
        client
            .profile_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        3
    );
    assert_eq!(
        client.usage_calls.load(std::sync::atomic::Ordering::SeqCst),
        4
    );
}

#[tokio::test]
async fn claude_billing_is_read_independently_of_usage_and_removed_on_a_live_downgrade() {
    let org = "00000000-0000-4000-8000-000000000001";
    let client = Arc::new(SubscriptionHttp {
        profiles: Mutex::new([
            json!({"organization":{"uuid":org,"organization_type":"claude_max","rate_limit_tier":"default_claude_max_20x","subscription_created_at":"2026-07-31T00:00:00Z"}}),
            json!({"organization":{"uuid":org,"organization_type":"claude_free"}}),
        ].into()),
        profile_calls: Default::default(), usage_calls: Default::default(), usage_status: 429,
    });
    let clock = Arc::new(Mutex::new(now()));
    let read_clock = clock.clone();
    let (directory, provider) = setup(
        ProviderKind::Claude,
        json!({"claudeAiOauth":{"accessToken":"fixture","subscriptionType":"pro"}}),
        client,
    );
    let periods = uc_providers::BillingPeriods::new(directory.path().join("billing"));
    let end = Utc.with_ymd_and_hms(2026, 11, 3, 7, 3, 48).unwrap();
    periods
        .save(
            org,
            "claude_max",
            Some("default_claude_max_20x"),
            &json!({"status":"active","next_charge_at":end.to_rfc3339()}),
            now(),
        )
        .unwrap();
    let provider = provider
        .with_billing_periods(periods.clone())
        .with_clock(Arc::new(move || *read_clock.lock().unwrap()));
    let first = provider.refresh(RefreshContext::manual()).await;
    assert_eq!(first.error_category, Some(ErrorCategory::RateLimited));
    assert_eq!(first.plan.as_deref(), Some("Max 20x"));
    assert_eq!(
        first.plan_term,
        Some(uc_core::PlanTerm::Stated {
            ends_at: end,
            checked_at: Some(now())
        })
    );
    let later = now() + chrono::Duration::minutes(1);
    *clock.lock().unwrap() = later;
    let next_end = Utc.with_ymd_and_hms(2026, 12, 3, 7, 3, 48).unwrap();
    periods
        .save(
            org,
            "claude_max",
            Some("default_claude_max_20x"),
            &json!({"status":"active","next_charge_at":next_end.to_rfc3339()}),
            later,
        )
        .unwrap();
    let renewed = provider.refresh(RefreshContext::scheduled()).await;
    assert_eq!(
        renewed.plan_term,
        Some(uc_core::PlanTerm::Stated {
            ends_at: next_end,
            checked_at: Some(later)
        })
    );
    *clock.lock().unwrap() = now() + chrono::Duration::minutes(5);
    let free = provider.refresh(RefreshContext::scheduled()).await;
    assert_eq!(free.plan.as_deref(), Some("Free"));
    assert_eq!(free.plan_term, None);
    assert_eq!(
        periods.get(org, Some("Max 20x"), *clock.lock().unwrap()),
        None
    );
}

struct CodexSubscriptionHttp {
    tokens: Value,
    token_status: u16,
    plan: &'static str,
    renewals: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl HttpClient for CodexSubscriptionHttp {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        if request.url.ends_with("/oauth/token") {
            self.renewals
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Ok(response(self.token_status, self.tokens.clone()));
        }
        assert!(request.url.ends_with("/usage"));
        Ok(response(
            200,
            json!({"plan_type":self.plan, "rate_limit":{"primary_window":{"used_percent":23}}}),
        ))
    }
}

fn billing_codex_document(account: &str, plan: &str) -> Value {
    let claims = json!({"https://api.openai.com/auth":{
        "chatgpt_account_id":account,
        "chatgpt_user_id":"fixture-user",
        "chatgpt_plan_type":plan
    }});
    json!({"tokens":{
        "access_token":"fixture",
        "account_id":account,
        "id_token":format!("header.{}.signature", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap()))
    }})
}

#[tokio::test]
async fn codex_browser_billing_matches_the_current_account_and_live_plan() {
    let account = "00000000-0000-4000-8000-000000000021";
    let other = "00000000-0000-4000-8000-000000000022";
    for (status, live_plan, source_account, expected_term) in [
        (200, "prolite", account, true),
        (429, "prolite", account, true),
        (403, "prolite", account, false),
        (200, "free", account, false),
        (200, "prolite", other, false),
    ] {
        let (directory, provider) = setup(
            ProviderKind::Codex,
            billing_codex_document(account, "prolite"),
            fake(
                status,
                json!({"plan_type":live_plan,"rate_limit":{"primary_window":{"used_percent":23}}}),
            ),
        );
        let periods = uc_providers::BillingPeriods::new(directory.path().join("billing"));
        let end = now() + chrono::Duration::days(30);
        periods
            .save_from(
                "codex",
                source_account,
                "Pro 5x",
                Some(end),
                now(),
                "browser",
            )
            .unwrap();
        let snapshot = provider
            .with_billing_periods(periods.clone())
            .refresh(RefreshContext::scheduled())
            .await;
        assert_eq!(
            snapshot.plan_term,
            expected_term.then_some(uc_core::PlanTerm::Stated {
                ends_at: end,
                checked_at: Some(now())
            })
        );
        if status == 429 {
            assert_eq!(snapshot.error_category, Some(ErrorCategory::RateLimited));
            assert_eq!(snapshot.plan_checked_at, Some(now()));
            assert_eq!(snapshot.plan.as_deref(), Some("Pro 5x"));
        }
        if live_plan == "free" {
            assert!(
                periods
                    .get_for("codex", account, Some("Pro 5x"), now())
                    .is_none()
            );
        }
    }
}

#[tokio::test]
async fn codex_cached_billing_keeps_its_original_confirmation_time_during_quota_failures() {
    let account = "00000000-0000-4000-8000-000000000031";
    let (directory, provider) = setup(
        ProviderKind::Codex,
        billing_codex_document(account, "plus"),
        fake(429, json!({})),
    );
    let periods = uc_providers::BillingPeriods::new(directory.path().join("billing"));
    let end = now() + chrono::Duration::days(30);
    periods
        .save_from("codex", account, "Plus", Some(end), now(), "browser")
        .unwrap();
    let later = now() + chrono::Duration::minutes(15);
    let snapshot = provider
        .with_billing_periods(periods)
        .with_clock(fixed_clock(later))
        .refresh(RefreshContext::scheduled())
        .await;
    assert_eq!(snapshot.refreshed_at, later);
    assert_eq!(snapshot.plan_checked_at, Some(now()));
    assert_eq!(
        snapshot.plan_term,
        Some(uc_core::PlanTerm::Stated {
            ends_at: end,
            checked_at: Some(now())
        })
    );
}

fn subscription_token(
    plan: &str,
    checked: chrono::DateTime<Utc>,
    ends: chrono::DateTime<Utc>,
) -> String {
    let claims = json!({"https://api.openai.com/auth":{"chatgpt_account_id":"account-one", "chatgpt_user_id":"user-one", "chatgpt_plan_type":plan, "chatgpt_subscription_last_checked":checked.to_rfc3339(), "chatgpt_subscription_active_until":ends.to_rfc3339()}});
    format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    )
}

#[tokio::test]
async fn a_fresh_signed_codex_period_survives_a_revoked_browser_source_during_quota_failure() {
    let account = "00000000-0000-4000-8000-000000000041";
    let end = now() + chrono::Duration::days(30);
    let claims = json!({"https://api.openai.com/auth":{
        "chatgpt_account_id":account,"chatgpt_plan_type":"plus",
        "chatgpt_subscription_last_checked":now().to_rfc3339(),
        "chatgpt_subscription_active_until":end.to_rfc3339()
    }});
    let mut document = billing_codex_document(account, "plus");
    document["tokens"]["id_token"] = Value::String(format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    ));
    let (directory, provider) = setup(ProviderKind::Codex, document, fake(429, json!({})));
    let periods = uc_providers::BillingPeriods::new(directory.path().join("billing"));
    periods
        .save_from("codex", account, "Plus", Some(end), now(), "browser")
        .unwrap();
    periods
        .invalidate_source("codex", account, "browser")
        .unwrap();
    let snapshot = provider
        .with_billing_periods(periods)
        .refresh(RefreshContext::scheduled())
        .await;
    assert_eq!(snapshot.error_category, Some(ErrorCategory::RateLimited));
    assert_eq!(snapshot.plan.as_deref(), Some("Plus"));
    assert_eq!(snapshot.plan_checked_at, Some(now()));
    assert_eq!(
        snapshot.plan_term,
        Some(uc_core::PlanTerm::Stated {
            ends_at: end,
            checked_at: Some(now())
        })
    );
}

#[tokio::test]
async fn managed_codex_renews_stale_subscription_metadata_without_repeated_rotation() {
    for token_status in [200, 503] {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(uc_accounts::AccountStore::new(
            directory.path().join("accounts"),
        ));
        let end = now() + chrono::Duration::days(30);
        let document = json!({"tokens":{"access_token":"fixture-old", "refresh_token":"fixture-refresh", "account_id":"account-one", "id_token":subscription_token("plus", now() - chrono::Duration::days(20), now() - chrono::Duration::days(1))}});
        let record = store
            .import(
                "codex",
                "Managed",
                "account-one|user-one",
                &document,
                uc_accounts::CredentialMode::ManagedOauth,
            )
            .unwrap();
        let client = Arc::new(CodexSubscriptionHttp {
            tokens: json!({"access_token":"fixture-new", "refresh_token":"fixture-next", "expires_in":3600, "id_token":subscription_token("plus", now(), end)}),
            token_status,
            plan: "plus",
            renewals: Default::default(),
        });
        let provider = LocalProvider::new(
            ProviderKind::Codex,
            CredentialStore::for_account(ProviderKind::Codex, store.clone(), record.clone()),
            client.clone(),
        )
        .with_clock(fixed_clock(now()));
        for _ in 0..3 {
            let snapshot = provider.refresh(RefreshContext::scheduled()).await;
            assert_eq!(snapshot.error_category, None);
            assert_eq!(snapshot.plan.as_deref(), Some("Plus"));
            assert_eq!(snapshot.plan_checked_at, Some(now()));
            assert_eq!(
                snapshot.plan_term,
                if token_status == 200 {
                    Some(uc_core::PlanTerm::Stated {
                        ends_at: end,
                        checked_at: Some(now()),
                    })
                } else {
                    None
                }
            );
        }
        assert_eq!(client.renewals.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            store.credentials(&record.id).unwrap()["tokens"]["access_token"],
            if token_status == 200 {
                "fixture-new"
            } else {
                "fixture-old"
            }
        );
    }
}
