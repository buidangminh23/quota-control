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
    CliAccount, LocalProvider, ProviderKind, VisibleAccount,
    accounts::{
        account_runtimes_with, cli_account_from, import_account_from_file, runtimes_with_client,
    },
    credentials::CredentialStore,
    oauth::{OAuthEndpoints, OAuthManager},
    visible_accounts,
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

/// Play the browser coming back to the callback: request `target` on the redirect's port.
fn return_to(redirect: &str, target: String) -> tokio::task::JoinHandle<String> {
    let port = url::Url::parse(redirect).unwrap().port().unwrap();
    tokio::spawn(async move {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        reply
    })
}

fn query_of(start: &uc_providers::oauth::LoginStart) -> HashMap<String, String> {
    url::Url::parse(&start.authorization_url)
        .unwrap()
        .query_pairs()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[tokio::test]
async fn claude_loopback_oauth_validates_state_pkce_and_persists_independent_session() {
    let (_dir, store) = store();
    let client = http(
        json!({"access_token":"fixture-new","refresh_token":"fixture-next","expires_in":3600,"scope":"user:profile user:inference"}),
    );
    let manager = OAuthManager::new(store.clone()).with_http(client.clone());
    let start = manager
        .begin_login(ProviderKind::Claude, "Primary".into())
        .await
        .unwrap();
    let query = query_of(&start);
    assert_eq!(
        serde_json::to_value(&start).unwrap()["expiresInSeconds"],
        600
    );
    assert_eq!(query["code_challenge_method"], "S256");
    assert!(query["redirect_uri"].starts_with("http://localhost:"));
    let tab = return_to(
        &query["redirect_uri"],
        format!("/callback?code=fixture-code&state={}", query["state"]),
    );
    let record = manager.complete_login(&start.flow_id).await.unwrap();
    assert!(tab.await.unwrap().starts_with("HTTP/1.1 200"));
    assert_eq!(record.credential_mode, CredentialMode::ManagedOauth);
    {
        let requests = client.requests.lock().unwrap();
        let payload: Value = serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(
            payload["code_verifier"].as_str().unwrap().as_bytes(),
        ));
        assert_eq!(challenge, query["code_challenge"]);
        assert_eq!(payload["state"], query["state"]);
        assert_eq!(payload["redirect_uri"], query["redirect_uri"]);
    }
    assert_eq!(
        store.credentials(&record.id).unwrap()["claudeAiOauth"]["subscriptionType"],
        "max"
    );
    assert!(manager.complete_login(&start.flow_id).await.is_err());
    let invalid = manager
        .begin_login(ProviderKind::Claude, "Bad".into())
        .await
        .unwrap();
    let invalid_query = query_of(&invalid);
    let foreign = return_to(
        &invalid_query["redirect_uri"],
        "/callback?code=fixture-code&state=wrong".into(),
    );
    assert!(foreign.await.unwrap().starts_with("HTTP/1.1 400"));
    manager.cancel_login(&invalid.flow_id).await.unwrap();
    assert!(manager.complete_login(&invalid.flow_id).await.is_err());
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
    let query = query_of(&start);
    let tab = return_to(
        &query["redirect_uri"],
        format!("/auth/callback?code=fixture-code&state={}", query["state"]),
    );
    let record = manager.complete_login(&start.flow_id).await.unwrap();
    let reply = tab.await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"));
    assert!(reply.contains("Codex is connected"));
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
    let port = url::Url::parse(&query_of(&start)["redirect_uri"])
        .unwrap()
        .port()
        .unwrap();
    let cloned = manager.clone();
    let id = start.flow_id.clone();
    let completing = tokio::spawn(async move { cloned.complete_login(&id).await });
    tokio::task::yield_now().await;
    manager.cancel_login(&start.flow_id).await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), completing)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let released = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .is_err()
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(released.is_ok());
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

#[tokio::test]
async fn managed_renewal_waits_for_another_process_and_reuses_its_session() {
    let (dir, store) = store();
    let record = store
        .import(
            "claude",
            "Managed",
            "account-a|org-a",
            &claude_doc(1),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let other_process = AccountStore::new(dir.path().join("accounts"));
    let held = other_process.renewal_lock(&record.id).unwrap();
    let client = http(
        json!({"access_token":"fixture-new","refresh_token":"fixture-next","expires_in":3600}),
    );
    let provider = runtimes_with_client(store.clone(), client.clone())
        .unwrap()
        .remove(0);
    let refresh = tokio::spawn(async move { provider.refresh(RefreshContext::manual()).await });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(client.requests.lock().unwrap().is_empty());
    let mut renewed = claude_doc(Utc::now().timestamp_millis() + 3_600_000);
    renewed["claudeAiOauth"]["accessToken"] = json!("fixture-other");
    renewed["claudeAiOauth"]["refreshToken"] = json!("fixture-other-refresh");
    other_process
        .update_credentials(&record.id, &renewed)
        .unwrap();
    drop(held);
    let snapshot = refresh.await.unwrap();
    assert!(!snapshot.is_error(), "{:?}", snapshot.error_category);
    let requests = client.requests.lock().unwrap();
    assert!(requests.iter().all(|request| request.method == "GET"));
    assert!(requests.iter().all(|request| {
        request
            .headers
            .contains(&("Authorization".into(), "Bearer fixture-other".into()))
    }));
    assert_eq!(store.credentials(&record.id).unwrap(), renewed);
}

fn write_codex_login(dir: &std::path::Path, account: &str) -> std::path::PathBuf {
    let path = dir.join("auth.json");
    let document = json!({"tokens":{"access_token":jwt(json!({"exp":2000000000,"https://api.openai.com/auth":{"chatgpt_user_id":"fixture-user"}})),"account_id":account}});
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    path
}

#[tokio::test]
async fn cli_logins_are_read_live_and_a_switched_account_is_refused() {
    let (dir, store) = store();
    let path = write_codex_login(dir.path(), "account-one");
    let login = cli_account_from(ProviderKind::Codex, path.clone(), None).unwrap();
    let client = http(json!({}));
    let runtimes =
        account_runtimes_with(store.clone(), std::slice::from_ref(&login), client.clone()).unwrap();
    assert_eq!(runtimes.len(), 1);
    assert_eq!(runtimes[0].provider().id, login.id);
    assert_eq!(runtimes[0].provider().display_name, "Codex");
    let live = runtimes[0].refresh(RefreshContext::scheduled()).await;
    assert_eq!(live.error_category, None);
    assert_eq!(client.requests.lock().unwrap().len(), 1);
    let switched_bytes = std::fs::read(write_codex_login(dir.path(), "account-two")).unwrap();
    let switched = runtimes[0].refresh(RefreshContext::scheduled()).await;
    assert_eq!(switched.error_category, Some(ErrorCategory::NotAvailable));
    assert_eq!(client.requests.lock().unwrap().len(), 1);
    assert_eq!(std::fs::read(&path).unwrap(), switched_bytes);
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn a_claude_cli_without_a_token_is_not_a_login() {
    let (dir, store) = store();
    let credentials = dir.path().join(".credentials.json");
    let profile = dir.path().join(".claude.json");
    std::fs::write(
        &profile,
        serde_json::to_vec(
            &json!({"oauthAccount":{"accountUuid":"account-a","organizationUuid":"org-a"}}),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        &credentials,
        serde_json::to_vec(
            &json!({"claudeAiOauth":{"accessToken":"","refreshToken":"","expiresAt":0}}),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        cli_account_from(
            ProviderKind::Claude,
            credentials.clone(),
            Some(profile.clone())
        )
        .is_err()
    );
    std::fs::write(
        &credentials,
        serde_json::to_vec(&claude_doc(4_000_000_000_000)).unwrap(),
    )
    .unwrap();
    let login = cli_account_from(ProviderKind::Claude, credentials, Some(profile)).unwrap();
    let imported = store
        .import(
            "claude",
            "Same",
            "account-a|org-a",
            &claude_doc(1),
            CredentialMode::SharedCli,
        )
        .unwrap();
    assert_eq!(login.id, imported.id);
}

#[tokio::test]
async fn cli_login_switch_during_unauthorized_request_never_retries_as_another_account() {
    struct SwitchingHttp {
        path: std::path::PathBuf,
        replacement: Value,
        requests: Mutex<Vec<HttpRequest>>,
    }

    #[async_trait]
    impl HttpClient for SwitchingHttp {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            let first = requests.len() == 1;
            if first {
                std::fs::write(&self.path, serde_json::to_vec(&self.replacement).unwrap()).unwrap();
            }
            Ok(HttpResponse {
                status: if first { 401 } else { 200 },
                headers: HashMap::new(),
                body: serde_json::to_vec(
                    &json!({"rate_limit":{"primary_window":{"used_percent":75}}}),
                )
                .unwrap(),
            })
        }
    }

    let (dir, store) = store();
    let path = write_codex_login(dir.path(), "account-one");
    let login = cli_account_from(ProviderKind::Codex, path.clone(), None).unwrap();
    let replacement = json!({"tokens":{
        "access_token": jwt(json!({"exp":2000000000,"https://api.openai.com/auth":{"chatgpt_user_id":"another-user"}})),
        "account_id":"account-two"
    }});
    let client = Arc::new(SwitchingHttp {
        path: path.clone(),
        replacement: replacement.clone(),
        requests: Mutex::new(Vec::new()),
    });
    let runtimes = account_runtimes_with(store, &[login], client.clone()).unwrap();
    let result = runtimes[0].refresh(RefreshContext::scheduled()).await;

    assert_eq!(result.error_category, Some(ErrorCategory::NotAvailable));
    assert_eq!(client.requests.lock().unwrap().len(), 1);
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(path).unwrap()).unwrap(),
        replacement
    );
}

#[test]
fn a_browser_session_wins_over_the_cli_and_the_cli_wins_over_an_imported_copy() {
    let (dir, store) = store();
    let managed = store
        .import(
            "codex",
            "Work",
            "account-one|fixture-user",
            &json!({"tokens":{}}),
            CredentialMode::ManagedOauth,
        )
        .unwrap();
    let copied = store
        .import(
            "codex",
            "Old",
            "account-two|fixture-user",
            &json!({"tokens":{}}),
            CredentialMode::SharedCli,
        )
        .unwrap();
    let records = store.list().unwrap();
    let login = |id: &str| CliAccount {
        kind: ProviderKind::Codex,
        id: id.into(),
        updated_at: Utc::now(),
        path: dir.path().join("auth.json"),
        profile: None,
    };
    let logins = vec![login(&managed.id), login(&copied.id), login("codex@other")];
    let shown: Vec<(String, bool)> = visible_accounts(&records, &logins)
        .into_iter()
        .map(|account| match account {
            VisibleAccount::Stored(record) => (record.id.clone(), false),
            VisibleAccount::Cli(login) => (login.id.clone(), true),
        })
        .collect();
    assert_eq!(shown.len(), 3);
    assert!(shown.contains(&(managed.id.clone(), false)));
    assert!(shown.contains(&(copied.id.clone(), true)));
    assert!(shown.contains(&("codex@other".to_string(), true)));
}

struct CliProfileHttp {
    requests: Mutex<Vec<HttpRequest>>,
    profile_status: u16,
    malformed_profile: bool,
    rotation: Mutex<Option<std::path::PathBuf>>,
}

#[async_trait]
impl HttpClient for CliProfileHttp {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let profile = request.url.ends_with("/profile");
        let foreign = request
            .headers
            .iter()
            .any(|(key, value)| key == "Authorization" && value == "Bearer fixture-foreign");
        self.requests.lock().unwrap().push(request);
        if profile && self.profile_status == 0 {
            return Err(HttpError::Timeout);
        }
        let mut status = if profile { self.profile_status } else { 200 };
        if !profile && let Some(path) = self.rotation.lock().unwrap().take() {
            write_claude_token(&path, "fixture-renewed");
            status = 401;
        }
        let body = if profile {
            if self.malformed_profile {
                json!({})
            } else {
                json!({"account":{"uuid": if foreign { "account-b" } else { "account-a" }},"organization":{"uuid":"org-a"}})
            }
        } else {
            json!({"five_hour":{"utilization":15}})
        };
        Ok(HttpResponse {
            status,
            headers: HashMap::from([("retry-after".into(), "60".into())]),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }
}

fn write_claude_token(path: &std::path::Path, token: &str) {
    let mut document = claude_doc(4_000_000_000_000);
    document["claudeAiOauth"]["accessToken"] = json!(token);
    std::fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
}

fn claude_cli_fixture(
    directory: &std::path::Path,
    token: &str,
    client: Arc<CliProfileHttp>,
) -> (LocalProvider, std::path::PathBuf) {
    let path = directory.join(".credentials.json");
    let profile = directory.join(".claude.json");
    write_claude_token(&path, token);
    std::fs::write(&profile, serde_json::to_vec(&claude_doc(1)).unwrap()).unwrap();
    let login = cli_account_from(ProviderKind::Claude, path.clone(), Some(profile)).unwrap();
    (
        LocalProvider::new(
            ProviderKind::Claude,
            CredentialStore::new(ProviderKind::Claude, path.clone()),
            client,
        )
        .with_cli_login(&login),
        path,
    )
}

fn cli_profile_http(status: u16, malformed: bool) -> Arc<CliProfileHttp> {
    Arc::new(CliProfileHttp {
        requests: Mutex::new(Vec::new()),
        profile_status: status,
        malformed_profile: malformed,
        rotation: Mutex::new(None),
    })
}

#[tokio::test]
async fn claude_cli_token_must_match_the_separate_metadata_identity() {
    let directory = tempfile::tempdir().unwrap();
    let client = cli_profile_http(200, false);
    let (runtime, _) = claude_cli_fixture(directory.path(), "fixture-foreign", client.clone());
    let snapshot = runtime.refresh(RefreshContext::scheduled()).await;
    assert_eq!(snapshot.error_category, Some(ErrorCategory::NotAvailable));
    let requests = client.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.ends_with("/profile"));
}

#[tokio::test]
async fn claude_cli_verification_is_cached_only_for_the_current_token() {
    let directory = tempfile::tempdir().unwrap();
    let client = cli_profile_http(200, false);
    let (runtime, path) = claude_cli_fixture(directory.path(), "fixture-old", client.clone());
    for token in [
        "fixture-old",
        "fixture-old",
        "fixture-renewed",
        "fixture-old",
    ] {
        write_claude_token(&path, token);
        assert_eq!(
            runtime
                .refresh(RefreshContext::scheduled())
                .await
                .error_category,
            None
        );
    }
    let requests = client.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.ends_with("/profile"))
            .count(),
        3
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.ends_with("/usage"))
            .count(),
        4
    );
}

#[tokio::test]
async fn claude_cli_verifies_a_rotated_token_before_retrying_usage() {
    let directory = tempfile::tempdir().unwrap();
    let client = cli_profile_http(200, false);
    let (runtime, path) = claude_cli_fixture(directory.path(), "fixture-old", client.clone());
    *client.rotation.lock().unwrap() = Some(path);
    assert_eq!(
        runtime
            .refresh(RefreshContext::scheduled())
            .await
            .error_category,
        None
    );
    let requests = client.requests.lock().unwrap();
    let endpoints: Vec<_> = requests
        .iter()
        .map(|request| request.url.rsplit('/').next().unwrap())
        .collect();
    assert_eq!(endpoints, ["profile", "usage", "profile", "usage"]);
}

#[tokio::test]
async fn claude_cli_does_not_fetch_usage_when_profile_verification_fails() {
    for (status, malformed) in [
        (0, false),
        (401, false),
        (503, false),
        (200, true),
        (429, false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let client = cli_profile_http(status, malformed);
        let (runtime, _) = claude_cli_fixture(directory.path(), "fixture-old", client.clone());
        assert!(
            runtime
                .refresh(RefreshContext::scheduled())
                .await
                .error_category
                .is_some()
        );
        assert!(
            runtime
                .refresh(RefreshContext::scheduled())
                .await
                .error_category
                .is_some()
        );
        let requests = client.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .all(|request| request.url.ends_with("/profile"))
        );
        assert_eq!(requests.len(), if status == 429 { 1 } else { 2 });
    }
}

#[tokio::test]
async fn claude_cli_invalid_session_does_not_attempt_profile_verification() {
    for (expiry, scopes, category) in [
        (1, json!(["user:profile"]), ErrorCategory::AuthExpired),
        (
            4_000_000_000_000,
            json!(["user:inference"]),
            ErrorCategory::NotAvailable,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let client = cli_profile_http(200, false);
        let (runtime, path) = claude_cli_fixture(directory.path(), "fixture-old", client.clone());
        let mut document = claude_doc(expiry);
        document["claudeAiOauth"]["scopes"] = scopes;
        std::fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(
            runtime
                .refresh(RefreshContext::scheduled())
                .await
                .error_category,
            Some(category)
        );
        assert!(client.requests.lock().unwrap().is_empty());
    }
}
