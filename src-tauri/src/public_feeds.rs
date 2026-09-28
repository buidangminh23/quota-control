//! Public data shown beside the user's own numbers: Codex reset announcements from
//! codex-resets.com (which follows @thsottiaux on X), model benchmark scores from Epoch AI and
//! human-preference leaderboards from Arena and 3D Arena. Only these fixed addresses are fetched:
//! the popup names a feed, never a URL. Each body is validated, cached on disk with its ETag and
//! refreshed on its own schedule, so the tabs work offline and one slow source never blocks another.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use uc_core::{HttpRequest, ReqwestHttpClient, SharedHttpClient};

const TIMEOUT: Duration = Duration::from_secs(20);
const MANUAL_REFRESH_GAP_SECONDS: i64 = 60;
/// The reset history is read page by page (100 per page); more than this is left unread.
const MAX_RESET_PAGES: usize = 5;
/// A `Retry-After` longer than this is trusted only this far.
const RETRY_AFTER_CAP_SECONDS: i64 = 6 * 3600;
const ARENA_ROOT: &str =
    "https://raw.githubusercontent.com/oolong-tea-2026/arena-ai-leaderboards/main/data";
/// The Arena leaderboards the popup knows how to label; any other name in the index is ignored.
pub const ARENA_BOARDS: &[&str] = &[
    "text",
    "code",
    "vision",
    "document",
    "search",
    "agent",
    "text-to-image",
    "image-edit",
    "text-to-video",
    "image-to-video",
    "video-edit",
];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum FeedName {
    CodexResetStatus,
    CodexResets,
    EpochScores,
    EpochBenchmarks,
    Arena,
    Arena3d,
}

impl FeedName {
    pub const ALL: [FeedName; 6] = [
        FeedName::CodexResetStatus,
        FeedName::CodexResets,
        FeedName::EpochScores,
        FeedName::EpochBenchmarks,
        FeedName::Arena,
        FeedName::Arena3d,
    ];

    fn file(self) -> &'static str {
        match self {
            FeedName::CodexResetStatus => "codex-reset-status.json",
            FeedName::CodexResets => "codex-resets.json",
            FeedName::EpochScores => "epoch-scores.json",
            FeedName::EpochBenchmarks => "epoch-benchmarks.json",
            FeedName::Arena => "arena.json",
            FeedName::Arena3d => "arena-3d.json",
        }
    }

    fn url(self) -> &'static str {
        match self {
            FeedName::CodexResetStatus => "https://codex-resets.com/api/v1/status",
            FeedName::CodexResets => "https://codex-resets.com/api/v1/resets?limit=100",
            FeedName::EpochScores => "https://epoch.ai/data/eci_scores.csv",
            FeedName::EpochBenchmarks => "https://epoch.ai/data/eci_benchmarks.csv",
            FeedName::Arena => {
                "https://raw.githubusercontent.com/oolong-tea-2026/arena-ai-leaderboards/main/data/latest.json"
            }
            FeedName::Arena3d => "https://3d-arena-3d-arena.hf.space/api/leaderboard",
        }
    }

    /// How long a successful fetch stays current.
    pub fn interval(self) -> chrono::Duration {
        match self {
            FeedName::CodexResetStatus => chrono::Duration::minutes(5),
            FeedName::CodexResets => chrono::Duration::minutes(30),
            FeedName::Arena => chrono::Duration::hours(6),
            FeedName::EpochScores | FeedName::EpochBenchmarks | FeedName::Arena3d => {
                chrono::Duration::hours(12)
            }
        }
    }

    /// After a failure the next attempt waits this long, so an outage is not hammered.
    fn retry(self) -> chrono::Duration {
        match self {
            FeedName::CodexResetStatus => chrono::Duration::minutes(5),
            _ => chrono::Duration::minutes(30),
        }
    }

    fn max_bytes(self) -> usize {
        match self {
            FeedName::CodexResetStatus | FeedName::Arena3d => 256 * 1024,
            FeedName::CodexResets | FeedName::EpochScores => 2 * 1024 * 1024,
            FeedName::EpochBenchmarks => 8 * 1024 * 1024,
            FeedName::Arena => 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct StoredFeed {
    etag: Option<String>,
    body: Option<String>,
    fetched_at: Option<DateTime<Utc>>,
    checked_at: Option<DateTime<Utc>>,
    error: Option<String>,
    /// The source asked (`Retry-After`) not to be called again before this time.
    #[serde(default)]
    retry_after: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FeedSnapshot {
    pub name: FeedName,
    /// The last body that passed validation, or `None` before the first success.
    pub body: Option<String>,
    pub fetched_at: Option<String>,
    pub checked_at: Option<String>,
    /// Why the latest attempt failed, while the cached body is still shown.
    pub error: Option<String>,
    /// The body is older than its refresh interval or the last attempt failed.
    pub stale: bool,
}

pub struct PublicFeeds {
    root: PathBuf,
    http: SharedHttpClient,
    clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    feeds: Mutex<HashMap<FeedName, StoredFeed>>,
}

impl PublicFeeds {
    pub fn new(root: PathBuf) -> Self {
        let feeds = FeedName::ALL
            .iter()
            .map(|name| {
                let stored = std::fs::read(root.join(name.file()))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<StoredFeed>(&bytes).ok())
                    .filter(|stored| stored.body.as_deref().is_none_or(|body| valid(*name, body)))
                    .unwrap_or_default();
                (*name, stored)
            })
            .collect();
        Self {
            root,
            http: ReqwestHttpClient::shared(),
            clock: Arc::new(Utc::now),
            feeds: Mutex::new(feeds),
        }
    }

    pub fn with_http(mut self, http: SharedHttpClient) -> Self {
        self.http = http;
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    pub async fn snapshot(&self, name: FeedName) -> FeedSnapshot {
        let feeds = self.feeds.lock().await;
        snapshot(
            name,
            feeds.get(&name).cloned().unwrap_or_default(),
            (self.clock)(),
        )
    }

    /// Whether the feed is due: never checked, past its interval, or past its retry wait, and
    /// past any `Retry-After` the source asked for.
    pub async fn due(&self, name: FeedName) -> bool {
        let feeds = self.feeds.lock().await;
        let stored = feeds.get(&name).cloned().unwrap_or_default();
        let now = (self.clock)();
        let wait = if stored.error.is_some() {
            name.retry()
        } else {
            name.interval()
        };
        let held = stored.retry_after.is_some_and(|until| now < until);
        !held
            && stored
                .checked_at
                .is_none_or(|checked| now.signed_duration_since(checked) >= wait)
    }

    /// Fetch the feed now. A manual refresh within a minute of the last check is answered from
    /// the cache. Returns the snapshot and whether its body changed.
    pub async fn refresh(&self, name: FeedName, manual: bool) -> (FeedSnapshot, bool) {
        let previous = {
            let feeds = self.feeds.lock().await;
            feeds.get(&name).cloned().unwrap_or_default()
        };
        let now = (self.clock)();
        if manual
            && previous.checked_at.is_some_and(|checked| {
                now.signed_duration_since(checked).num_seconds() < MANUAL_REFRESH_GAP_SECONDS
            })
        {
            return (snapshot(name, previous, now), false);
        }
        let mut next = previous.clone();
        next.checked_at = Some(now);
        next.retry_after = None;
        match self.fetch(name, previous.etag.as_deref()).await {
            Ok(Fetched::Unchanged) => next.error = None,
            Ok(Fetched::Body { body, etag }) => {
                next.body = Some(body);
                next.etag = etag;
                next.fetched_at = Some((self.clock)());
                next.error = None;
            }
            Err(failure) => {
                next.error = Some(failure.message);
                next.retry_after = failure.retry_after.map(|wait| now + wait);
            }
        }
        let changed = next.body != previous.body;
        if let Ok(bytes) = serde_json::to_vec(&next) {
            std::fs::create_dir_all(&self.root).ok();
            if let Err(error) = uc_core::paths::write_atomic(&self.root.join(name.file()), &bytes) {
                tracing::warn!("Saving the {name:?} feed failed: {error}");
            }
        }
        self.feeds.lock().await.insert(name, next.clone());
        (snapshot(name, next, (self.clock)()), changed)
    }

    async fn fetch(&self, name: FeedName, etag: Option<&str>) -> Result<Fetched, FetchFailure> {
        if name == FeedName::Arena {
            return self
                .fetch_arena()
                .await
                .map(|body| Fetched::Body { body, etag: None })
                .map_err(FetchFailure::from);
        }
        let mut request = HttpRequest::get(name.url()).timeout(TIMEOUT);
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }
        let response = self.get(request, name.max_bytes()).await?;
        if response.status == 304 && etag.is_some() {
            return Ok(Fetched::Unchanged);
        }
        let mut body = successful_text(&response, name.max_bytes())?;
        if name == FeedName::CodexResets {
            body = self.follow_reset_pages(body).await?;
        }
        if !valid(name, &body) {
            return Err("The source answered with data in an unexpected shape".into());
        }
        Ok(Fetched::Body {
            body,
            etag: response.header("etag").map(str::to_owned),
        })
    }

    /// The reset list is paginated by an opaque cursor. The first page's text is kept verbatim
    /// when it is the only one, so its ETag still describes the stored body; further pages are
    /// merged into one list under the first page's `meta`.
    async fn follow_reset_pages(&self, first: String) -> Result<String, FetchFailure> {
        let invalid = || FetchFailure::from("The source answered with data that is not JSON");
        let mut page: Value = serde_json::from_str(&first).map_err(|_| invalid())?;
        let meta = page["meta"].clone();
        let mut rows = page["data"].as_array().cloned().unwrap_or_default();
        let mut pages = 1;
        loop {
            let cursor = page["pagination"]["next_cursor"]
                .as_str()
                .filter(|cursor| is_cursor(cursor))
                .map(str::to_owned);
            let Some(cursor) = cursor.filter(|_| page["pagination"]["has_more"] == true) else {
                break;
            };
            if pages >= MAX_RESET_PAGES {
                break;
            }
            let url = format!("{}&cursor={cursor}", FeedName::CodexResets.url());
            let response = self
                .get(HttpRequest::get(url), FeedName::CodexResets.max_bytes())
                .await?;
            let text = successful_text(&response, FeedName::CodexResets.max_bytes())?;
            page = serde_json::from_str(&text).map_err(|_| invalid())?;
            rows.extend(page["data"].as_array().cloned().unwrap_or_default());
            pages += 1;
        }
        if pages == 1 {
            return Ok(first);
        }
        Ok(serde_json::json!({
            "data": rows,
            "pagination": { "has_more": false, "next_cursor": Value::Null },
            "meta": meta,
        })
        .to_string())
    }

    async fn fetch_arena(&self) -> Result<String, String> {
        let index = self
            .get(HttpRequest::get(FeedName::Arena.url()), 4096)
            .await?;
        let index: Value = serde_json::from_str(&successful_text(&index, 4096)?)
            .map_err(|_| "The Arena index is not valid JSON".to_string())?;
        let path = index["path"].as_str().unwrap_or_default();
        if !is_day(path) {
            return Err("The Arena index names an unexpected folder".into());
        }
        let mut boards = serde_json::Map::new();
        for board in ARENA_BOARDS {
            let url = format!("{ARENA_ROOT}/{path}/{board}.json");
            let Ok(response) = self.get(HttpRequest::get(url), 256 * 1024).await else {
                continue;
            };
            let Ok(text) = successful_text(&response, 256 * 1024) else {
                continue;
            };
            if let Ok(value) = serde_json::from_str::<Value>(&text)
                && value["models"].is_array()
            {
                boards.insert((*board).to_owned(), value);
            }
        }
        if boards.is_empty() {
            return Err("No Arena leaderboard could be read".into());
        }
        let body = serde_json::json!({ "date": path, "boards": boards });
        Ok(body.to_string())
    }

    async fn get(
        &self,
        request: HttpRequest,
        max_bytes: usize,
    ) -> Result<uc_core::HttpResponse, String> {
        let request = request.timeout(TIMEOUT).max_response_bytes(max_bytes);
        match tokio::time::timeout(TIMEOUT, self.http.send(request)).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err("The request timed out.".into()),
        }
    }
}

enum Fetched {
    Unchanged,
    Body { body: String, etag: Option<String> },
}

/// Why a fetch failed, and how long the source asked to be left alone (`Retry-After`), if it did.
struct FetchFailure {
    message: String,
    retry_after: Option<chrono::Duration>,
}

impl From<String> for FetchFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            retry_after: None,
        }
    }
}

impl From<&str> for FetchFailure {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}

impl From<FetchFailure> for String {
    fn from(failure: FetchFailure) -> Self {
        failure.message
    }
}

/// `Retry-After` in seconds (the date form is ignored), clamped to a sane range.
fn retry_after(response: &uc_core::HttpResponse) -> Option<chrono::Duration> {
    let seconds = response.header("retry-after")?.trim().parse::<i64>().ok()?;
    Some(chrono::Duration::seconds(
        seconds.clamp(1, RETRY_AFTER_CAP_SECONDS),
    ))
}

fn successful_text(
    response: &uc_core::HttpResponse,
    max_bytes: usize,
) -> Result<String, FetchFailure> {
    if !response.is_success() {
        return Err(FetchFailure {
            message: format!("HTTP {}", response.status),
            retry_after: retry_after(response),
        });
    }
    if response.body.len() > max_bytes {
        return Err("The source answered with more data than expected".into());
    }
    String::from_utf8(response.body.clone())
        .map_err(|_| "The source answered with data that is not text".into())
}

fn is_day(value: &str) -> bool {
    value.len() == 10 && chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}

/// A pagination cursor as the API defines it: `[A-Za-z0-9_-]{1,1024}`, so it can only ever be a
/// query value, never a path or another query parameter.
fn is_cursor(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn valid(name: FeedName, body: &str) -> bool {
    match name {
        FeedName::CodexResetStatus => serde_json::from_str::<Value>(body)
            .is_ok_and(|value| value["data"]["stats"].is_object()),
        FeedName::CodexResets => {
            serde_json::from_str::<Value>(body).is_ok_and(|value| value["data"].is_array())
        }
        FeedName::EpochScores => body
            .trim_start_matches('\u{feff}')
            .starts_with("Model,Display name,eci,"),
        FeedName::EpochBenchmarks => body
            .trim_start_matches('\u{feff}')
            .starts_with("model_id,benchmark_id,performance,benchmark,"),
        FeedName::Arena => serde_json::from_str::<Value>(body).is_ok_and(|value| {
            value["boards"]
                .as_object()
                .is_some_and(|boards| !boards.is_empty())
        }),
        FeedName::Arena3d => serde_json::from_str::<Value>(body).is_ok_and(|value| {
            value.as_array().is_some_and(|rows| {
                !rows.is_empty()
                    && rows
                        .iter()
                        .all(|row| row["name"].is_string() && row["score"].is_number())
            })
        }),
    }
}

fn snapshot(name: FeedName, stored: StoredFeed, now: DateTime<Utc>) -> FeedSnapshot {
    let stale = stored.error.is_some()
        || stored
            .fetched_at
            .is_none_or(|fetched| now.signed_duration_since(fetched) >= name.interval() * 3);
    FeedSnapshot {
        name,
        body: stored.body,
        fetched_at: stored.fetched_at.map(|at| at.to_rfc3339()),
        checked_at: stored.checked_at.map(|at| at.to_rfc3339()),
        error: stored.error,
        stale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, Ordering};
    use uc_core::{HttpClient, HttpError, HttpResponse};

    const STATUS: &str = r#"{"data":{"latest_reset":null,"scheduled_reset":null,"active_watch":null,"stats":{"total":54,"last_reset_at":null,"days_since_last":null,"avg_interval_days":7}},"meta":{"api_version":"v1","generated_at":"2026-09-26T09:12:55Z"}}"#;

    struct Script {
        responses: std::sync::Mutex<VecDeque<(String, Result<HttpResponse, HttpError>)>>,
        seen: std::sync::Mutex<Vec<HttpRequest>>,
    }

    impl Script {
        fn new(responses: Vec<(impl Into<String>, Result<HttpResponse, HttpError>)>) -> Arc<Self> {
            Arc::new(Self {
                responses: std::sync::Mutex::new(
                    responses
                        .into_iter()
                        .map(|(url, response)| (url.into(), response))
                        .collect(),
                ),
                seen: std::sync::Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait::async_trait]
    impl HttpClient for Script {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            let (url, response) = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("an expected request");
            assert_eq!(request.url, url);
            let max_bytes = if url == FeedName::Arena.url() {
                4096
            } else if url.starts_with(ARENA_ROOT) {
                256 * 1024
            } else {
                FeedName::ALL
                    .into_iter()
                    .find(|name| url.starts_with(name.url()))
                    .expect("a known feed")
                    .max_bytes()
            };
            assert_eq!(request.max_response_bytes, Some(max_bytes));
            self.seen.lock().unwrap().push(request);
            response
        }
    }

    fn ok(body: &str, etag: Option<&str>) -> Result<HttpResponse, HttpError> {
        let mut headers = HashMap::new();
        if let Some(etag) = etag {
            headers.insert("etag".to_owned(), etag.to_owned());
        }
        Ok(HttpResponse {
            status: 200,
            headers,
            body: body.as_bytes().to_vec(),
        })
    }

    fn status(code: u16) -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse {
            status: code,
            headers: HashMap::new(),
            body: Vec::new(),
        })
    }

    fn status_with(code: u16, header: &str, value: &str) -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse {
            status: code,
            headers: HashMap::from([(header.to_owned(), value.to_owned())]),
            body: Vec::new(),
        })
    }

    fn page(rows: &[&str], next_cursor: Option<&str>) -> String {
        let data = rows
            .iter()
            .map(|id| format!(r#"{{"id":"{id}","reset_type":"regular","announced_at":"2026-09-12T08:09:17.000Z","text":"Reset all propagated.","source":{{"type":"x_post","author":"thsottiaux","url":"https://x.com/thsottiaux/status/{id}"}}}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let pagination = match next_cursor {
            Some(cursor) => format!(r#"{{"has_more":true,"next_cursor":"{cursor}"}}"#),
            None => r#"{"has_more":false,"next_cursor":null}"#.to_owned(),
        };
        format!(
            r#"{{"data":[{data}],"pagination":{pagination},"meta":{{"api_version":"v1","generated_at":"2026-09-26T09:12:55Z"}}}}"#
        )
    }

    fn feeds(root: &std::path::Path, http: Arc<Script>, seconds: Arc<AtomicI64>) -> PublicFeeds {
        PublicFeeds::new(root.to_path_buf())
            .with_http(http)
            .with_clock(Arc::new(move || {
                DateTime::from_timestamp(1_790_400_000 + seconds.load(Ordering::SeqCst), 0).unwrap()
            }))
    }

    #[tokio::test]
    async fn every_direct_feed_sends_its_transport_budget() {
        for name in FeedName::ALL
            .into_iter()
            .filter(|name| *name != FeedName::Arena)
        {
            let root = tempfile::tempdir().unwrap();
            let http = Script::new(vec![(name.url(), status(503))]);
            let store = feeds(root.path(), http.clone(), Arc::new(AtomicI64::new(0)));
            let (snapshot, changed) = store.refresh(name, false).await;
            assert!(!changed);
            assert_eq!(snapshot.error.as_deref(), Some("HTTP 503"));
            assert_eq!(http.seen.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn caches_bodies_revalidates_with_etag_and_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::CodexResetStatus.url();
        let http = Script::new(vec![(url, ok(STATUS, Some("\"v1\""))), (url, status(304))]);
        let store = feeds(root.path(), http.clone(), seconds.clone());
        assert!(store.due(FeedName::CodexResetStatus).await);
        let (first, changed) = store.refresh(FeedName::CodexResetStatus, false).await;
        assert!(changed && !first.stale && first.error.is_none());
        assert_eq!(first.body.as_deref(), Some(STATUS));
        assert!(!store.due(FeedName::CodexResetStatus).await);
        seconds.store(300, Ordering::SeqCst);
        assert!(store.due(FeedName::CodexResetStatus).await);
        let (second, changed) = store.refresh(FeedName::CodexResetStatus, false).await;
        assert!(!changed);
        assert_eq!(second.body.as_deref(), Some(STATUS));
        let revalidation = http.seen.lock().unwrap()[1].headers.clone();
        assert_eq!(
            revalidation,
            vec![("If-None-Match".to_owned(), "\"v1\"".to_owned())]
        );
        let reopened = feeds(root.path(), Script::new(Vec::<(String, _)>::new()), seconds);
        assert_eq!(
            reopened
                .snapshot(FeedName::CodexResetStatus)
                .await
                .body
                .as_deref(),
            Some(STATUS)
        );
    }

    #[tokio::test]
    async fn transport_size_failure_preserves_cached_body_and_etag() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::CodexResetStatus.url();
        let http = Script::new(vec![
            (url, ok(STATUS, Some("cached"))),
            (
                url,
                Err(HttpError::Transport(
                    "The response exceeded its size limit.".into(),
                )),
            ),
            (url, status(304)),
        ]);
        let store = feeds(root.path(), http.clone(), seconds.clone());
        let (first, _) = store.refresh(FeedName::CodexResetStatus, false).await;
        seconds.store(300, Ordering::SeqCst);
        let (failed, changed) = store.refresh(FeedName::CodexResetStatus, false).await;
        assert!(!changed && failed.stale && failed.error.is_some());
        assert_eq!(failed.body, first.body);
        assert_eq!(failed.fetched_at, first.fetched_at);
        assert!(!store.due(FeedName::CodexResetStatus).await);
        let reopened = feeds(root.path(), http.clone(), seconds.clone());
        assert_eq!(reopened.snapshot(FeedName::CodexResetStatus).await, failed);
        seconds.store(600, Ordering::SeqCst);
        let (recovered, changed) = reopened.refresh(FeedName::CodexResetStatus, false).await;
        assert!(!changed && !recovered.stale && recovered.error.is_none());
        assert_eq!(recovered.body, first.body);
        assert_eq!(
            http.seen.lock().unwrap()[2].headers,
            vec![("If-None-Match".into(), "cached".into())]
        );
    }

    #[tokio::test]
    async fn failures_keep_the_last_good_body_and_back_off() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::EpochScores.url();
        let csv = "Model,Display name,eci,eci_ci_low\nGPT-6 Astra,GPT-6 Astra,166.6,163.0\n";
        let http = Script::new(vec![
            (url, ok(csv, None)),
            (url, ok("<html>maintenance</html>", None)),
            (url, status(503)),
        ]);
        let store = feeds(root.path(), http, seconds.clone());
        store.refresh(FeedName::EpochScores, false).await;
        seconds.store(12 * 3600, Ordering::SeqCst);
        let (shape, _) = store.refresh(FeedName::EpochScores, false).await;
        assert_eq!(shape.body.as_deref(), Some(csv));
        assert!(shape.stale && shape.error.is_some());
        seconds.store(12 * 3600 + 29 * 60, Ordering::SeqCst);
        assert!(!store.due(FeedName::EpochScores).await);
        seconds.store(12 * 3600 + 30 * 60, Ordering::SeqCst);
        assert!(store.due(FeedName::EpochScores).await);
        let (down, changed) = store.refresh(FeedName::EpochScores, false).await;
        assert!(!changed);
        assert_eq!(down.error.as_deref(), Some("HTTP 503"));
        assert_eq!(down.body.as_deref(), Some(csv));
    }

    #[tokio::test]
    async fn manual_refresh_is_throttled_to_once_a_minute() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::CodexResetStatus.url();
        let http = Script::new(vec![(url, ok(STATUS, None)), (url, ok(STATUS, None))]);
        let store = feeds(root.path(), http.clone(), seconds.clone());
        store.refresh(FeedName::CodexResetStatus, true).await;
        seconds.store(59, Ordering::SeqCst);
        store.refresh(FeedName::CodexResetStatus, true).await;
        assert_eq!(http.seen.lock().unwrap().len(), 1);
        seconds.store(60, Ordering::SeqCst);
        store.refresh(FeedName::CodexResetStatus, true).await;
        assert_eq!(http.seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn reset_history_follows_cursors_and_keeps_a_single_page_verbatim() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::CodexResets.url();
        let single = page(&["1"], None);
        let http = Script::new(vec![
            (
                url.to_owned(),
                ok(&page(&["3", "2"], Some("abc_DEF-9")), Some("\"p1\"")),
            ),
            (
                format!("{url}&cursor=abc_DEF-9"),
                ok(&page(&["1"], None), None),
            ),
            (url.to_owned(), ok(&single, Some("\"p2\""))),
            (url.to_owned(), ok(&page(&["9"], Some("../evil?x=1")), None)),
        ]);
        let store = feeds(root.path(), http.clone(), seconds.clone());
        let (merged, _) = store.refresh(FeedName::CodexResets, false).await;
        let body: Value = serde_json::from_str(merged.body.as_deref().unwrap()).unwrap();
        let ids: Vec<&str> = body["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["3", "2", "1"]);
        assert_eq!(body["pagination"]["has_more"], false);
        assert_eq!(body["meta"]["api_version"], "v1");

        seconds.store(1800, Ordering::SeqCst);
        let (one_page, _) = store.refresh(FeedName::CodexResets, false).await;
        assert_eq!(one_page.body.as_deref(), Some(single.as_str()));
        assert_eq!(
            http.seen.lock().unwrap()[2].headers,
            vec![("If-None-Match".to_owned(), "\"p1\"".to_owned())]
        );

        seconds.store(3600, Ordering::SeqCst);
        let (unsafe_cursor, _) = store.refresh(FeedName::CodexResets, false).await;
        let body: Value = serde_json::from_str(unsafe_cursor.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["data"].as_array().unwrap().len(), 1);
        assert_eq!(http.seen.lock().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn a_rate_limit_is_honoured_for_as_long_as_retry_after_says() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::CodexResetStatus.url();
        let http = Script::new(vec![
            (url, status_with(429, "retry-after", "3600")),
            (url, ok(STATUS, None)),
        ]);
        let store = feeds(root.path(), http, seconds.clone());
        let (limited, _) = store.refresh(FeedName::CodexResetStatus, false).await;
        assert_eq!(limited.error.as_deref(), Some("HTTP 429"));
        seconds.store(5 * 60, Ordering::SeqCst);
        assert!(!store.due(FeedName::CodexResetStatus).await);
        seconds.store(3599, Ordering::SeqCst);
        assert!(!store.due(FeedName::CodexResetStatus).await);
        seconds.store(3600, Ordering::SeqCst);
        assert!(store.due(FeedName::CodexResetStatus).await);
        let (recovered, changed) = store.refresh(FeedName::CodexResetStatus, false).await;
        assert!(changed && recovered.error.is_none());
        seconds.store(3600 + 5 * 60, Ordering::SeqCst);
        assert!(store.due(FeedName::CodexResetStatus).await);
    }

    #[tokio::test]
    async fn arena_reads_only_known_boards_from_a_dated_folder() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let board = r#"{"meta":{"leaderboard":"text","last_updated":"Sep 25, 2026"},"models":[{"rank":1,"model":"claude-opus-5.5-high","vendor":"Anthropic","score":1509,"ci":12,"votes":2307}]}"#;
        let mut responses = vec![(
            FeedName::Arena.url().to_owned(),
            ok(r#"{"date":"2026-09-26","path":"2026-09-26"}"#, None),
        )];
        for name in ARENA_BOARDS {
            let url = format!("{ARENA_ROOT}/2026-09-26/{name}.json");
            let response = if *name == "text" {
                ok(board, None)
            } else {
                status(404)
            };
            responses.push((url, response));
        }
        let http = Script::new(responses);
        let store = feeds(root.path(), http, seconds.clone());
        let (snapshot, _) = store.refresh(FeedName::Arena, false).await;
        let body: Value = serde_json::from_str(snapshot.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["date"], "2026-09-26");
        assert_eq!(body["boards"].as_object().unwrap().len(), 1);
        assert_eq!(body["boards"]["text"]["models"][0]["score"], 1509);

        let traversal = Script::new(vec![(
            FeedName::Arena.url(),
            ok(r#"{"path":"../../secrets"}"#, None),
        )]);
        let store = feeds(tempfile::tempdir().unwrap().path(), traversal, seconds);
        let (rejected, _) = store.refresh(FeedName::Arena, false).await;
        assert!(rejected.body.is_none() && rejected.error.is_some());
    }

    #[test]
    fn validates_each_feed_shape() {
        assert!(valid(FeedName::CodexResetStatus, STATUS));
        assert!(!valid(FeedName::CodexResetStatus, "{}"));
        assert!(valid(FeedName::CodexResets, r#"{"data":[]}"#));
        assert!(valid(
            FeedName::EpochBenchmarks,
            "model_id,benchmark_id,performance,benchmark,x\n"
        ));
        assert!(!valid(FeedName::EpochBenchmarks, "<!doctype html>"));
        assert!(valid(
            FeedName::Arena3d,
            r#"[{"name":"TRELLIS","rank":"1","score":1399,"votes":4833}]"#
        ));
        assert!(!valid(FeedName::Arena3d, r#"[{"error":"x"}]"#));
    }
}
