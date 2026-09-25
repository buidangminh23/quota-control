use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uc_accounts::{AccountStore, CredentialMode};
use uc_core::{
    ErrorCategory, HttpClient, HttpError, HttpRequest, HttpResponse, ProviderRuntime,
    RefreshContext,
};
use uc_providers::{
    LocalProvider, ProviderKind,
    accounts::{import_account_from_file, runtimes_with_client},
    credentials::CredentialStore,
    oauth::{OAuthEndpoints, OAuthManager},
};

fn jwt(claims: Value) -> String {
    format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    )
}

fn claude_doc(expiry: i64) -> Value {
    json!({"claudeAiOauth":{"accessToken":"fixture-old","refreshToken":"fixture-refresh","expiresAt":expiry},"oauthAccount":{"accountUuid":"account-a","organizationUuid":"org-a"}})
}

struct LoginHttp {
    requests: Mutex<Vec<HttpRequest>>,
    tokens: Value,
}

#[async_trait]
impl HttpClient for LoginHttp {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let response = if request.method == "POST" {
            self.tokens.clone()
        } else if request.url.ends_with("profile") {
            json!({"account":{"uuid":"account-a"},"organization":{"uuid":"org-a","organization_type":"claude_max","rate_limit_tier":"default_claude_max_5x"}})
        } else {
            json!({"five_hour":{"utilization":15},"rate_limit":{"primary_window":{"used_percent":15}}})
        };
        self.requests.lock().unwrap().push(request);
        tokio::task::yield_now().await;
        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: serde_json::to_vec(&response).unwrap(),
        })
    }
}

fn http(tokens: Value) -> Arc<LoginHttp> {
    Arc::new(LoginHttp {
        requests: Mutex::new(Vec::new()),
        tokens,
    })
}

fn store() -> (tempfile::TempDir, Arc<AccountStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(AccountStore::new(dir.path().join("accounts")));
    (dir, store)
}

#[tokio::test]
async fn importing_distinct_accounts_and_reimporting_preserves_rows_and_cli_file() {
    let (dir, store) = store();
    let path = dir.path().join("auth.json");
    let client = http(json!({}));
    let mut ids = Vec::new();
    for (account, label) in [
        ("account-one", "Work"),
        ("account-two", "Personal"),
        ("account-one", "Renamed"),
    ] {
        let document = json!({"tokens":{"access_token":jwt(json!({"exp":1,"https://api.openai.com/auth":{"chatgpt_user_id":"fixture-user"}})),"account_id":account}});
        let bytes = serde_json::to_vec(&document).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        ids.push(
            import_account_from_file(
                store.clone(),
                ProviderKind::Codex,
                label.into(),
                path.clone(),
                None,
            )
            .await
            .unwrap()
            .id,
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    assert_eq!(ids[0], ids[2]);
    assert_ne!(ids[0], ids[1]);
    let providers = runtimes_with_client(store.clone(), client.clone()).unwrap();
    assert_eq!(providers.len(), 2);
    for provider in providers {
        assert_eq!(provider.provider().icon, "codex");
        assert!(
            provider
                .widget_descriptors()
                .iter()
                .all(|descriptor| descriptor.id.starts_with(&provider.provider().id))
        );
        assert_eq!(
            provider
                .refresh(RefreshContext::scheduled())
                .await
                .error_category,
            Some(ErrorCategory::AuthExpired)
        );
    }
    assert_eq!(store.list().unwrap().len(), 2);
    assert!(client.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn shared_cli_accounts_never_rotate_refresh_tokens() {
    let (_dir, store) = store();
    let record = store
        .import(
            "claude",
            "Shared",
            "account-a|org-a",
            &claude_doc(1),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let client = http(
        json!({"access_token":"fixture-new","refresh_token":"fixture-next","expires_in":3600}),
    );
    let providers = runtimes_with_client(store.clone(), client.clone()).unwrap();
    assert_eq!(
        providers[0]
            .refresh(RefreshContext::manual())
            .await
            .error_category,
        Some(ErrorCategory::AuthExpired)
    );
    assert_eq!(store.credentials(&record.id).unwrap(), claude_doc(1));
    assert!(client.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn managed_refresh_is_serialized_across_distinct_runtime_instances() {
    let (_dir, store) = store();
    let record = store
        .import(
            "claude",
            "Managed",
            "account-a|org-a",
            &claude_doc(1),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let client = http(
        json!({"access_token":"fixture-new","refresh_token":"fixture-next","expires_in":3600}),
    );
    let first = runtimes_with_client(store.clone(), client.clone())
        .unwrap()
        .remove(0);
    let second = runtimes_with_client(store.clone(), client.clone())
        .unwrap()
        .remove(0);
    let (one, two) = tokio::join!(
        first.refresh(RefreshContext::manual()),
        second.refresh(RefreshContext::manual())
    );
    assert!(!one.is_error(), "{:?}", one.error_category);
    assert!(!two.is_error(), "{:?}", two.error_category);
    let requests = client.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    assert!(
        requests
            .iter()
            .filter(|request| request.method == "GET")
            .all(|request| request
                .headers
                .contains(&("Authorization".into(), "Bearer fixture-new".into())))
    );
    let saved = store.credentials(&record.id).unwrap();
    assert_eq!(saved["claudeAiOauth"]["refreshToken"], "fixture-next");
    assert!(saved["claudeAiOauth"]["expiresAt"].as_i64().unwrap() > Utc::now().timestamp_millis());
}

#[tokio::test]
async fn claude_manual_oauth_validates_state_pkce_and_persists_independent_session() {
    let (_dir, store) = store();
    let client = http(
        json!({"access_token":"fixture-new","refresh_token":"fixture-next","expires_in":3600,"scope":"user:profile user:inference"}),
    );
    let manager = OAuthManager::new(store.clone()).with_http(client.clone());
    let start = manager
        .begin_login(ProviderKind::Claude, "Primary".into())
        .await
        .unwrap();
    let url = url::Url::parse(&start.authorization_url).unwrap();
    let query: HashMap<_, _> = url
        .query_pairs()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    assert_eq!(start.callback_mode, "manual");
    assert_eq!(
        serde_json::to_value(&start).unwrap()["expiresInSeconds"],
        600
    );
    assert_eq!(query["code_challenge_method"], "S256");
    let record = manager
        .complete_login(
            &start.flow_id,
            Some(format!("fixture-code#{}", query["state"])),
        )
        .await
        .unwrap();
    assert_eq!(record.credential_mode, CredentialMode::ManagedOauth);
    {
        let requests = client.requests.lock().unwrap();
        let payload: Value = serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(
            payload["code_verifier"].as_str().unwrap().as_bytes(),
        ));
        assert_eq!(challenge, query["code_challenge"]);
        assert_eq!(payload["state"], query["state"]);
    }
    assert_eq!(
        store.credentials(&record.id).unwrap()["claudeAiOauth"]["subscriptionType"],
        "max"
    );
    assert!(
        manager
            .complete_login(&start.flow_id, Some("fixture-code#wrong".into()))
            .await
            .is_err()
    );
    let invalid = manager
        .begin_login(ProviderKind::Claude, "Bad".into())
        .await
        .unwrap();
    assert!(
        manager
            .complete_login(&invalid.flow_id, Some("fixture-code#wrong".into()))
            .await
            .is_err()
    );
    assert_eq!(client.requests.lock().unwrap().len(), 2);
    assert_eq!(store.list().unwrap().len(), 1);
}

#[tokio::test]
async fn codex_loopback_login_collects_callback_and_creates_account_without_cli_writes() {
    let (_dir, store) = store();
    let client = http(
        json!({"access_token":jwt(json!({"exp":2000000000})),"refresh_token":"fixture-next","id_token":jwt(json!({"sub":"ignored-subject","https://api.openai.com/auth":{"chatgpt_user_id":"user-one","chatgpt_account_id":"account-one"}}))}),
    );
    let endpoints = OAuthEndpoints {
        codex_ports: vec![0],
        ..Default::default()
    };
    let manager = Arc::new(
        OAuthManager::new(store.clone())
            .with_http(client.clone())
            .with_endpoints(endpoints),
    );
    let start = manager
        .begin_login(ProviderKind::Codex, "Primary".into())
        .await
        .unwrap();
    let url = url::Url::parse(&start.authorization_url).unwrap();
    let query: HashMap<_, _> = url
        .query_pairs()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    let redirect = url::Url::parse(&query["redirect_uri"]).unwrap();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", redirect.port().unwrap()))
        .await
        .unwrap();
    stream
        .write_all(
            format!(
                "GET /auth/callback?code=fixture-code&state={} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
                query["state"]
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"));
    let record = manager.complete_login(&start.flow_id, None).await.unwrap();
    assert_eq!(record.credential_mode, CredentialMode::ManagedOauth);
    assert_eq!(
        store.credentials(&record.id).unwrap()["tokens"]["account_id"],
        "account-one"
    );
    let requests = client.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let form: HashMap<_, _> = url::form_urlencoded::parse(requests[0].body.as_ref().unwrap())
        .into_owned()
        .collect();
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["redirect_uri"], query["redirect_uri"]);
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),
        query["code_challenge"]
    );
}

#[tokio::test]
async fn cancelling_while_completing_loopback_releases_listener() {
    let (_dir, store) = store();
    let manager = Arc::new(OAuthManager::new(store).with_endpoints(OAuthEndpoints {
        codex_ports: vec![0],
        ..Default::default()
    }));
    let start = manager
        .begin_login(ProviderKind::Codex, "Cancel".into())
        .await
        .unwrap();
    let url = url::Url::parse(&start.authorization_url).unwrap();
    let redirect = url
        .query_pairs()
        .find(|(key, _)| key == "redirect_uri")
        .unwrap()
        .1
        .into_owned();
    let port = url::Url::parse(&redirect).unwrap().port().unwrap();
    let cloned = manager.clone();
    let id = start.flow_id.clone();
    let completing = tokio::spawn(async move { cloned.complete_login(&id, None).await });
    tokio::task::yield_now().await;
    manager.cancel_login(&start.flow_id).await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), completing)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    tokio::task::yield_now().await;
    assert!(
        tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn unavailable_saved_credentials_do_not_remove_registry_entries() {
    let (_dir, store) = store();
    let record = store
        .import(
            "claude",
            "Unavailable",
            "account-a|org-a",
            &json!({}),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let client = http(json!({}));
    let provider = LocalProvider::new(
        ProviderKind::Claude,
        CredentialStore::for_account(ProviderKind::Claude, store.clone(), record.clone()),
        client,
    )
    .with_account(&record);
    assert!(
        provider
            .refresh(RefreshContext::scheduled())
            .await
            .is_error()
    );
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(provider.provider().id, record.id);
}

#[tokio::test]
async fn identity_is_stable_across_sparse_and_rich_token_claims() {
    let (dir, store) = store();
    let path = dir.path().join("auth.json");
    let variants = [
        json!({"tokens":{"account_id":"workspace","id_token":jwt(json!({"https://api.openai.com/auth":{"chatgpt_account_id":"workspace"}})),"access_token":jwt(json!({"https://api.openai.com/auth":{"user_id":"stable-user"}}))}}),
        json!({"tokens":{"id_token":jwt(json!({"https://api.openai.com/auth":{"chatgpt_account_id":"workspace","chatgpt_user_id":"stable-user"}})),"access_token":"opaque-fixture"}}),
    ];
    let mut ids = Vec::new();
    for document in variants {
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        ids.push(
            import_account_from_file(
                store.clone(),
                ProviderKind::Codex,
                "Stable".into(),
                path.clone(),
                None,
            )
            .await
            .unwrap()
            .id,
        );
    }
    assert_eq!(ids[0], ids[1]);
    std::fs::write(
        &path,
        br#"{"tokens":{"account_id":"workspace","access_token":"opaque-fixture"}}"#,
    )
    .unwrap();
    assert!(
        import_account_from_file(
            store.clone(),
            ProviderKind::Codex,
            "Unknown user".into(),
            path,
            None
        )
        .await
        .is_err()
    );
    assert_eq!(store.list().unwrap().len(), 1);
}

struct PausedRefresh {
    started: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    posts: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl HttpClient for PausedRefresh {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let body = if request.method == "POST" {
            self.posts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.started.notify_one();
            self.resume.notified().await;
            json!({"access_token":"fixture-rotated","refresh_token":"fixture-rotated-refresh","expires_in":3600})
        } else {
            json!({"five_hour":{"utilization":1}})
        };
        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }
}

#[tokio::test]
async fn cancelled_runtime_keeps_rotated_token_transaction_alive() {
    let (_dir, store) = store();
    let record = store
        .import(
            "claude",
            "Cancel refresh",
            "account-a|org-a",
            &claude_doc(1),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let client = Arc::new(PausedRefresh {
        started: Default::default(),
        resume: Default::default(),
        posts: Default::default(),
    });
    let first = runtimes_with_client(store.clone(), client.clone())
        .unwrap()
        .remove(0);
    let started = client.started.notified();
    let task = tokio::spawn(async move { first.refresh(RefreshContext::scheduled()).await });
    started.await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let second = runtimes_with_client(store.clone(), client.clone())
        .unwrap()
        .remove(0);
    let replacement =
        tokio::spawn(async move { second.refresh(RefreshContext::scheduled()).await });
    client.resume.notify_one();
    let snapshot = tokio::time::timeout(std::time::Duration::from_secs(2), replacement)
        .await
        .unwrap()
        .unwrap();
    assert!(!snapshot.is_error());
    assert_eq!(client.posts.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        store.credentials(&record.id).unwrap()["claudeAiOauth"]["refreshToken"],
        "fixture-rotated-refresh"
    );
}
