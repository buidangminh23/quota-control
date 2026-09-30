//! Public data shown beside the user's own numbers: Codex reset announcements from
//! codex-resets.com (which follows @thsottiaux on X), Claude reset announcements from
//! claude-resets.com (which follows @ClaudeDevs on X), model benchmark scores from Epoch AI and
//! human-preference leaderboards from Arena and 3D Arena. Only these fixed addresses are fetched:
//! the popup names a feed, never a URL. Each body is validated, cached on disk with its ETag and
//! refreshed on its own schedule, so the tabs work offline and one slow source never blocks another.

use std::collections::{HashMap, HashSet};
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
/// The published Claude catalog, read when the live one (`FeedName::ClaudeResets`) cannot be.
const CLAUDE_RESETS_DATASET: &str = "https://claude-resets.com/data/resets.json";
/// The published catalog only gains an announcement once the site has reviewed it, about a day
/// after the live one shows it. A live body read within this time is therefore never older than
/// the published one, and is kept when the live catalog cannot be read.
const CLAUDE_DATASET_LAG_HOURS: i64 = 24;
/// Of more announcements than this in one answer, the newest are kept.
const MAX_CLAUDE_EVENTS: usize = 500;
/// What the popup reads of a Claude announcement; any other key is dropped.
const CLAUDE_EVENT_KEYS: &[&str] = &[
    "id",
    "date",
    "kind",
    "resetType",
    "usableUntil",
    "account",
    "scope",
    "note",
    "url",
    "verification",
];
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
    ClaudeResets,
    EpochScores,
    EpochBenchmarks,
    Arena,
    Arena3d,
}

impl FeedName {
    pub const ALL: [FeedName; 7] = [
        FeedName::CodexResetStatus,
        FeedName::CodexResets,
        FeedName::ClaudeResets,
        FeedName::EpochScores,
        FeedName::EpochBenchmarks,
        FeedName::Arena,
        FeedName::Arena3d,
    ];

    fn file(self) -> &'static str {
        match self {
            FeedName::CodexResetStatus => "codex-reset-status.json",
            FeedName::CodexResets => "codex-resets.json",
            FeedName::ClaudeResets => "claude-resets.json",
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
            FeedName::ClaudeResets => "https://claude-resets.com/api/resets",
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
            FeedName::CodexResetStatus | FeedName::ClaudeResets => chrono::Duration::minutes(5),
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
            FeedName::CodexResetStatus | FeedName::ClaudeResets => chrono::Duration::minutes(5),
            _ => chrono::Duration::minutes(30),
        }
    }

    fn max_bytes(self) -> usize {
        match self {
            FeedName::CodexResetStatus | FeedName::Arena3d => 256 * 1024,
            FeedName::ClaudeResets => 512 * 1024,
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
    /// When the source last answered with a usable body: a new one, or word that the cached one is
    /// current. Caches saved before this was kept fall back to `fetched_at`.
    #[serde(default)]
    verified_at: Option<DateTime<Utc>>,
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
    /// When the source last confirmed `body`, by sending it or by answering that it had not changed.
    pub verified_at: Option<String>,
    /// Why the latest attempt failed, while the cached body is still shown.
    pub error: Option<String>,
    /// The source has failed for three refresh intervals since it last confirmed `body`, which may
    /// therefore be out of date. A single failed attempt is not stale.
    pub stale: bool,
}

pub struct PublicFeeds {
    root: PathBuf,
    http: SharedHttpClient,
    clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    feeds: Mutex<HashMap<FeedName, StoredFeed>>,
    /// The feeds this run has asked for. A failure saved by an earlier run does not hold back the
    /// first attempt of this one: the network it failed on may be back.
    tried: Mutex<HashSet<FeedName>>,
    /// Whether each feed was stale when the popup was last told about it.
    announced: Mutex<HashMap<FeedName, bool>>,
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
                    .map(confirmed_by_last_check)
                    .unwrap_or_default();
                (*name, stored)
            })
            .collect();
        Self {
            root,
            http: ReqwestHttpClient::shared(),
            clock: Arc::new(Utc::now),
            feeds: Mutex::new(feeds),
            tried: Mutex::new(HashSet::new()),
            announced: Mutex::new(HashMap::new()),
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

    /// Whether the feed is due: never checked, past its interval, past its retry wait, or failed
    /// before this run asked for it, and past any `Retry-After` the source asked for.
    pub async fn due(&self, name: FeedName) -> bool {
        let stored = {
            let feeds = self.feeds.lock().await;
            feeds.get(&name).cloned().unwrap_or_default()
        };
        let tried = self.tried.lock().await.contains(&name);
        let now = (self.clock)();
        if stored.retry_after.is_some_and(|until| now < until) {
            return false;
        }
        if stored.error.is_some() && !tried {
            return true;
        }
        let wait = if stored.error.is_some() {
            name.retry()
        } else {
            name.interval()
        };
        stored
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
        self.tried.lock().await.insert(name);
        let mut next = previous.clone();
        next.checked_at = Some(now);
        next.retry_after = None;
        match self.fetch(name, &previous, now).await {
            Ok(Fetched::Unchanged) => {
                next.verified_at = Some((self.clock)());
                next.error = None;
            }
            Ok(Fetched::Body { body, etag }) => {
                next.body = Some(body);
                next.etag = etag;
                next.fetched_at = Some((self.clock)());
                next.verified_at = next.fetched_at;
                next.error = None;
            }
            Err(failure) => {
                next.retry_after = failure
                    .held_until(now)
                    .or_else(|| failure.asked_to_wait().then(|| now + name.retry()));
                next.error = Some(failure.message);
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

    /// One pass of the background loop: check each wanted feed that is due, and name the feeds
    /// the popup should read again. That is every feed just checked, a failure and the recovery
    /// from it included, and every feed whose staleness changed since the popup was last told,
    /// which a hold of up to hours (`Retry-After`) would otherwise keep from it.
    pub async fn tick(&self, wanted: impl Fn(FeedName) -> bool) -> Vec<FeedName> {
        let mut announce = Vec::new();
        for name in FeedName::ALL {
            if !wanted(name) {
                continue;
            }
            let stale = if self.due(name).await {
                announce.push(name);
                self.refresh(name, false).await.0.stale
            } else {
                let stale = self.snapshot(name).await.stale;
                let told = self.announced.lock().await.get(&name).copied();
                if told.is_some_and(|told| told != stale) {
                    announce.push(name);
                }
                stale
            };
            self.announced.lock().await.insert(name, stale);
        }
        announce
    }

    async fn fetch(
        &self,
        name: FeedName,
        previous: &StoredFeed,
        now: DateTime<Utc>,
    ) -> Result<Fetched, FetchFailure> {
        let etag = previous.etag.as_deref();
        if name == FeedName::Arena {
            return self
                .fetch_arena()
                .await
                .map(|body| Fetched::Body { body, etag: None })
                .map_err(FetchFailure::from);
        }
        if name == FeedName::ClaudeResets {
            return self
                .fetch_claude_resets(previous, now)
                .await
                .map(|body| Fetched::Body { body, etag: None });
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

    /// The live catalog carries what the site detected in the last day, before its review. When
    /// it cannot be read, the published dataset stands in, with two exceptions: the site asked to
    /// be left alone (a rate limit or `Retry-After`), or a live body read within the last day is
    /// cached, which the published one could only be older than. A failure of both is reported as
    /// the live one's.
    async fn fetch_claude_resets(
        &self,
        previous: &StoredFeed,
        now: DateTime<Utc>,
    ) -> Result<String, FetchFailure> {
        let live = self
            .claude_catalog(FeedName::ClaudeResets.url(), true)
            .await;
        let failure = match live {
            Ok(body) => return Ok(body),
            Err(failure) => failure,
        };
        if failure.asked_to_wait() || cached_live_catalog_is_current(previous, now) {
            return Err(failure);
        }
        match self.claude_catalog(CLAUDE_RESETS_DATASET, false).await {
            Ok(body) => Ok(body),
            Err(_) => Err(failure),
        }
    }

    async fn claude_catalog(&self, url: &str, live: bool) -> Result<String, FetchFailure> {
        let limit = FeedName::ClaudeResets.max_bytes();
        let response = self.get(HttpRequest::get(url), limit).await?;
        let text = successful_text(&response, limit)?;
        claude_catalog(&text, live)
            .ok_or_else(|| "The source answered with data in an unexpected shape".into())
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
    /// The HTTP status, when the source answered at all.
    status: Option<u16>,
    retry_after: Option<RetryAfter>,
}

/// `Retry-After` in either of its forms.
#[derive(Clone, Copy)]
enum RetryAfter {
    Wait(chrono::Duration),
    Until(DateTime<Utc>),
}

impl FetchFailure {
    /// The source is rate limiting or named a time to come back: nothing else of it is asked.
    fn asked_to_wait(&self) -> bool {
        self.status == Some(429) || self.retry_after.is_some()
    }

    /// When the source may be called again, trusted no further than the cap.
    fn held_until(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let soonest = now + chrono::Duration::seconds(1);
        let latest = now + chrono::Duration::seconds(RETRY_AFTER_CAP_SECONDS);
        let until = match self.retry_after? {
            RetryAfter::Wait(wait) => now + wait,
            RetryAfter::Until(until) => until,
        };
        Some(until.clamp(soonest, latest))
    }
}

impl From<String> for FetchFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            status: None,
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

/// `Retry-After` as seconds or as an HTTP date; anything else is not a wait the app can keep.
fn retry_after(response: &uc_core::HttpResponse) -> Option<RetryAfter> {
    let value = response.header("retry-after")?.trim();
    if let Ok(seconds) = value.parse::<i64>() {
        let seconds = seconds.clamp(1, RETRY_AFTER_CAP_SECONDS);
        return Some(RetryAfter::Wait(chrono::Duration::seconds(seconds)));
    }
    DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|until| RetryAfter::Until(until.with_timezone(&Utc)))
}

/// Whether the cached body is the live catalog, read recently enough that the published one
/// cannot hold anything it lacks.
fn cached_live_catalog_is_current(cached: &StoredFeed, now: DateTime<Utc>) -> bool {
    let live = cached
        .body
        .as_deref()
        .and_then(|body| serde_json::from_str::<Value>(body).ok())
        .is_some_and(|body| body["live"] == true);
    live && cached.fetched_at.is_some_and(|fetched| {
        now.signed_duration_since(fetched) < chrono::Duration::hours(CLAUDE_DATASET_LAG_HOURS)
    })
}

fn successful_text(
    response: &uc_core::HttpResponse,
    max_bytes: usize,
) -> Result<String, FetchFailure> {
    if !response.is_success() {
        return Err(FetchFailure {
            message: format!("HTTP {}", response.status),
            status: Some(response.status),
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

/// The Claude side of claude-resets.com's catalog, reduced to what the popup reads: the
/// announcements, who posts them, whether the site's detector is up to date and which entries
/// still await its review. The Codex side (read from codex-resets.com itself) and every time that
/// moves with each poll are left out, so the stored body only changes when the catalog does.
fn claude_catalog(body: &str, live: bool) -> Option<String> {
    let root: Value = serde_json::from_str(body).ok()?;
    let provider = root["providers"]["claude"].as_object()?;
    let listed: Vec<&Value> = provider
        .get("events")?
        .as_array()?
        .iter()
        .filter(|event| {
            ["id", "date", "kind"]
                .iter()
                .all(|key| event[*key].as_str().is_some_and(|text| !text.is_empty()))
        })
        .collect();
    let events: Vec<Value> = newest(listed, MAX_CLAUDE_EVENTS)
        .into_iter()
        .map(|event| {
            let kept = CLAUDE_EVENT_KEYS
                .iter()
                .filter(|key| event[**key].is_string())
                .map(|key| ((*key).to_owned(), event[*key].clone()));
            Value::Object(kept.collect())
        })
        .collect();
    if events.is_empty() {
        return None;
    }
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let ids = |value: &Value| -> Vec<String> {
        let listed = value.as_array().map(Vec::as_slice).unwrap_or_default();
        listed.iter().filter_map(text).collect()
    };
    let named = |key: &str| provider.get(key).and_then(text);
    let meta = &root["meta"];
    Some(
        serde_json::json!({
            "account": named("account"),
            "product": named("product"),
            "events": events,
            "live": live,
            "detector": text(&meta["detector"]["status"]),
            "provisionalEventIds": ids(&meta["provisionalEventIds"]),
            "provisionalPolicyIds": ids(&meta["provisionalPolicyIds"]),
        })
        .to_string(),
    )
}

/// The `limit` newest announcements by their date, in the order the site listed them. The dates
/// are ISO 8601 in UTC, which sort as text.
fn newest(mut events: Vec<&Value>, limit: usize) -> Vec<&Value> {
    if events.len() <= limit {
        return events;
    }
    let date = |event: &Value| event["date"].as_str().unwrap_or_default().to_owned();
    let mut dates: Vec<String> = events.iter().map(|event| date(event)).collect();
    dates.sort_unstable();
    let oldest_kept = dates[dates.len() - limit].clone();
    events.retain(|event| date(event) >= oldest_kept);
    let surplus = events.len().saturating_sub(limit);
    events.drain(..surplus);
    events
}

fn valid(name: FeedName, body: &str) -> bool {
    match name {
        FeedName::CodexResetStatus => serde_json::from_str::<Value>(body)
            .is_ok_and(|value| value["data"]["stats"].is_object()),
        FeedName::CodexResets => {
            serde_json::from_str::<Value>(body).is_ok_and(|value| value["data"].is_array())
        }
        FeedName::ClaudeResets => serde_json::from_str::<Value>(body).is_ok_and(|value| {
            value["live"].is_boolean()
                && value["events"]
                    .as_array()
                    .is_some_and(|events| !events.is_empty())
        }),
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

/// A cache saved before `verified_at` was kept: its last check confirmed the body unless it
/// failed, since a check that got `304` moved only `checked_at`.
fn confirmed_by_last_check(mut stored: StoredFeed) -> StoredFeed {
    if stored.verified_at.is_none() {
        stored.verified_at = if stored.error.is_none() {
            stored.checked_at.or(stored.fetched_at)
        } else {
            stored.fetched_at
        };
    }
    stored
}

fn snapshot(name: FeedName, stored: StoredFeed, now: DateTime<Utc>) -> FeedSnapshot {
    let verified_at = stored.verified_at.or(stored.fetched_at);
    let stale = stored.error.is_some()
        && stored.body.is_some()
        && verified_at
            .is_none_or(|verified| now.signed_duration_since(verified) >= name.interval() * 3);
    FeedSnapshot {
        name,
        body: stored.body,
        fetched_at: stored.fetched_at.map(|at| at.to_rfc3339()),
        checked_at: stored.checked_at.map(|at| at.to_rfc3339()),
        verified_at: verified_at.map(|at| at.to_rfc3339()),
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
            } else if url == CLAUDE_RESETS_DATASET {
                FeedName::ClaudeResets.max_bytes()
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
            .filter(|name| *name != FeedName::Arena && *name != FeedName::ClaudeResets)
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
        assert!(!changed && !failed.stale && failed.error.is_some());
        assert_eq!(failed.body, first.body);
        assert_eq!(failed.fetched_at, first.fetched_at);
        assert!(!store.due(FeedName::CodexResetStatus).await);
        let reopened = feeds(root.path(), http.clone(), seconds.clone());
        assert_eq!(reopened.snapshot(FeedName::CodexResetStatus).await, failed);
        assert!(reopened.due(FeedName::CodexResetStatus).await);
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
        assert!(!shape.stale && shape.error.is_some());
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
    async fn a_body_turns_stale_only_after_three_intervals_without_confirmation() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::CodexResetStatus.url();
        let http = Script::new(vec![
            (url, ok(STATUS, Some("\"v1\""))),
            (url, status(503)),
            (url, Err(HttpError::Transport("offline".into()))),
            (url, status(503)),
            (url, status(304)),
        ]);
        let store = feeds(root.path(), http, seconds.clone());
        let (first, _) = store.refresh(FeedName::CodexResetStatus, false).await;
        assert!(!first.stale);
        assert_eq!(first.verified_at, first.fetched_at);

        seconds.store(300, Ordering::SeqCst);
        let (once, _) = store.refresh(FeedName::CodexResetStatus, false).await;
        assert!(once.error.is_some() && !once.stale);
        assert_eq!(once.verified_at, first.verified_at);

        seconds.store(600, Ordering::SeqCst);
        let (twice, _) = store.refresh(FeedName::CodexResetStatus, false).await;
        assert!(twice.error.is_some() && !twice.stale);

        seconds.store(900, Ordering::SeqCst);
        let (thrice, _) = store.refresh(FeedName::CodexResetStatus, false).await;
        assert!(thrice.error.is_some() && thrice.stale);
        assert_eq!(thrice.body, first.body);

        seconds.store(1200, Ordering::SeqCst);
        let (confirmed, changed) = store.refresh(FeedName::CodexResetStatus, false).await;
        assert!(!changed && confirmed.error.is_none() && !confirmed.stale);
        assert_eq!(confirmed.fetched_at, first.fetched_at);
        assert_eq!(
            confirmed.verified_at,
            Some(
                DateTime::from_timestamp(1_790_400_000 + 1200, 0)
                    .unwrap()
                    .to_rfc3339()
            )
        );
    }

    #[tokio::test]
    async fn a_failure_saved_by_an_earlier_run_is_retried_when_the_app_starts() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::CodexResets.url();
        let http = Script::new(vec![
            (url, ok(&page(&["1"], None), Some("\"p1\""))),
            (
                url,
                Err(HttpError::Transport("error sending request".into())),
            ),
            (url, status(304)),
        ]);
        let before = feeds(root.path(), http.clone(), seconds.clone());
        before.refresh(FeedName::CodexResets, false).await;
        seconds.store(1800, Ordering::SeqCst);
        let (failed, _) = before.refresh(FeedName::CodexResets, false).await;
        assert!(failed.error.is_some());
        seconds.store(1860, Ordering::SeqCst);
        assert!(!before.due(FeedName::CodexResets).await);

        let after = feeds(root.path(), http.clone(), seconds.clone());
        assert!(after.due(FeedName::CodexResets).await);
        let (recovered, _) = after.refresh(FeedName::CodexResets, false).await;
        assert!(recovered.error.is_none() && !recovered.stale);
        assert_eq!(recovered.body, failed.body);
        seconds.store(1920, Ordering::SeqCst);
        assert!(!after.due(FeedName::CodexResets).await);
        assert_eq!(http.seen.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_new_run_still_waits_out_retry_after() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::CodexResetStatus.url();
        let http = Script::new(vec![(url, status_with(429, "retry-after", "3600"))]);
        let before = feeds(root.path(), http.clone(), seconds.clone());
        before.refresh(FeedName::CodexResetStatus, false).await;
        seconds.store(60, Ordering::SeqCst);
        let after = feeds(root.path(), http.clone(), seconds.clone());
        assert!(!after.due(FeedName::CodexResetStatus).await);
        seconds.store(3600, Ordering::SeqCst);
        assert!(after.due(FeedName::CodexResetStatus).await);
    }

    #[tokio::test]
    async fn a_tick_announces_every_check_and_a_feed_that_turns_stale_while_held() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::CodexResetStatus.url();
        let http = Script::new(vec![
            (url, ok(STATUS, Some("\"v1\""))),
            (url, status(503)),
            (url, status(304)),
            (url, status_with(429, "retry-after", "3600")),
        ]);
        let store = feeds(root.path(), http, seconds.clone());
        let status_only = |name: FeedName| name == FeedName::CodexResetStatus;
        assert_eq!(store.tick(status_only).await, vec![FeedName::CodexResetStatus]);
        seconds.store(60, Ordering::SeqCst);
        assert!(store.tick(status_only).await.is_empty());
        seconds.store(300, Ordering::SeqCst);
        assert_eq!(store.tick(status_only).await, vec![FeedName::CodexResetStatus]);
        assert!(store.snapshot(FeedName::CodexResetStatus).await.error.is_some());
        seconds.store(600, Ordering::SeqCst);
        assert_eq!(store.tick(status_only).await, vec![FeedName::CodexResetStatus]);
        assert!(store.snapshot(FeedName::CodexResetStatus).await.error.is_none());

        seconds.store(900, Ordering::SeqCst);
        assert_eq!(store.tick(status_only).await, vec![FeedName::CodexResetStatus]);
        let held = store.snapshot(FeedName::CodexResetStatus).await;
        assert!(held.error.is_some() && !held.stale);
        seconds.store(1400, Ordering::SeqCst);
        assert!(store.tick(status_only).await.is_empty());
        seconds.store(1500, Ordering::SeqCst);
        assert_eq!(store.tick(status_only).await, vec![FeedName::CodexResetStatus]);
        assert!(store.snapshot(FeedName::CodexResetStatus).await.stale);
        seconds.store(1800, Ordering::SeqCst);
        assert!(store.tick(status_only).await.is_empty());
        assert!(store.tick(|_| false).await.is_empty());
    }

    #[tokio::test]
    async fn a_rate_limit_without_retry_after_still_holds_the_next_run() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::CodexResets.url();
        let http = Script::new(vec![(url, status(429))]);
        let before = feeds(root.path(), http.clone(), seconds.clone());
        before.refresh(FeedName::CodexResets, false).await;
        seconds.store(60, Ordering::SeqCst);
        let after = feeds(root.path(), http.clone(), seconds.clone());
        assert!(!after.due(FeedName::CodexResets).await);
        seconds.store(30 * 60, Ordering::SeqCst);
        assert!(after.due(FeedName::CodexResets).await);
    }

    #[tokio::test]
    async fn a_cache_from_before_verified_at_counts_its_last_good_check() {
        let root = tempfile::tempdir().unwrap();
        let at = |offset: i64| {
            DateTime::from_timestamp(1_790_400_000 + offset, 0)
                .unwrap()
                .to_rfc3339()
        };
        let old = serde_json::json!({
            "etag": "\"p1\"",
            "body": page(&["1"], None),
            "fetchedAt": at(-3 * 86_400),
            "checkedAt": at(-600),
            "error": null,
            "retryAfter": null,
        });
        std::fs::write(
            root.path().join(FeedName::CodexResets.file()),
            serde_json::to_vec(&old).unwrap(),
        )
        .unwrap();
        let url = FeedName::CodexResets.url();
        let http = Script::new(vec![(url, Err(HttpError::Transport("offline".into())))]);
        let store = feeds(root.path(), http, Arc::new(AtomicI64::new(0)));
        assert_eq!(
            store.snapshot(FeedName::CodexResets).await.verified_at,
            Some(at(-600))
        );
        let (failed, _) = store.refresh(FeedName::CodexResets, false).await;
        assert!(failed.error.is_some() && !failed.stale);

        let failing = serde_json::json!({
            "body": page(&["1"], None),
            "fetchedAt": at(-3 * 86_400),
            "checkedAt": at(-600),
            "error": "HTTP 503",
        });
        std::fs::write(
            root.path().join(FeedName::CodexResets.file()),
            serde_json::to_vec(&failing).unwrap(),
        )
        .unwrap();
        let reopened = feeds(
            root.path(),
            Script::new(Vec::<(String, _)>::new()),
            Arc::new(AtomicI64::new(0)),
        );
        let snapshot = reopened.snapshot(FeedName::CodexResets).await;
        assert_eq!(snapshot.verified_at, Some(at(-3 * 86_400)));
        assert!(snapshot.stale);
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

    fn claude_answer(as_of: &str, notes: &[(&str, &str)], provisional: &[&str]) -> String {
        let events = notes
            .iter()
            .map(|(id, note)| {
                let verification = if provisional.contains(id) {
                    "provisional"
                } else {
                    "curated"
                };
                format!(
                    r#"{{"id":"{id}","date":"2026-09-22T16:44:06Z","kind":"reset","scope":"all","note":"{note}","url":"https://x.com/ClaudeDevs/status/{id}","verification":"{verification}","internal":{{"score":1}}}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let ids = provisional
            .iter()
            .map(|id| format!(r#""{id}""#))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"providers":{{"claude":{{"name":"Claude","product":"Claude Code","account":"ClaudeDevs","accountUrl":"https://x.com/ClaudeDevs","events":[{events}]}},"codex":{{"name":"Codex","events":[{{"id":"1","date":"2026-09-26T18:17:54Z","kind":"reset"}}]}}}},"meta":{{"asOf":"{as_of}","detector":{{"status":"fresh","lastCheckedAt":"{as_of}"}},"provisionalEventIds":[{ids}],"provisionalPolicyIds":[]}}}}"#
        )
    }

    #[tokio::test]
    async fn claude_catalog_keeps_its_own_side_and_only_changes_with_the_data() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::ClaudeResets.url();
        let first = claude_answer("2026-09-29T12:56:49Z", &[("7", "Reset for all.")], &[]);
        let polled = claude_answer("2026-09-29T13:01:49Z", &[("7", "Reset for all.")], &[]);
        let detected = claude_answer(
            "2026-09-29T13:06:49Z",
            &[("7", "Reset for all."), ("8", "Limits reset.")],
            &["8"],
        );
        let http = Script::new(vec![
            (url, ok(&first, None)),
            (url, ok(&polled, None)),
            (url, ok(&detected, None)),
        ]);
        let store = feeds(root.path(), http.clone(), seconds.clone());
        let (snapshot, changed) = store.refresh(FeedName::ClaudeResets, false).await;
        assert!(changed && snapshot.error.is_none());
        let body: Value = serde_json::from_str(snapshot.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["live"], true);
        assert_eq!(body["account"], "ClaudeDevs");
        assert_eq!(body["detector"], "fresh");
        assert_eq!(body["events"].as_array().unwrap().len(), 1);
        assert_eq!(body["events"][0]["note"], "Reset for all.");
        assert!(body["events"][0].get("internal").is_none());
        assert!(body.get("providers").is_none() && body.get("meta").is_none());
        assert!(http.seen.lock().unwrap()[0].headers.is_empty());

        seconds.store(299, Ordering::SeqCst);
        assert!(!store.due(FeedName::ClaudeResets).await);
        seconds.store(300, Ordering::SeqCst);
        assert!(store.due(FeedName::ClaudeResets).await);
        let (same, changed) = store.refresh(FeedName::ClaudeResets, false).await;
        assert!(!changed);
        assert_eq!(same.body, snapshot.body);

        seconds.store(600, Ordering::SeqCst);
        let (next, changed) = store.refresh(FeedName::ClaudeResets, false).await;
        assert!(changed);
        let body: Value = serde_json::from_str(next.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["events"][1]["verification"], "provisional");
        assert_eq!(body["provisionalEventIds"], serde_json::json!(["8"]));
    }

    #[tokio::test]
    async fn claude_catalog_falls_back_to_the_published_dataset() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::ClaudeResets.url();
        let dataset = r#"{"providers":{"claude":{"account":"ClaudeDevs","events":[{"id":"7","date":"2026-09-22T16:44:06Z","kind":"reset","url":"https://x.com/ClaudeDevs/status/7"},{"id":"","date":"x","kind":"reset"},{"date":"2026-09-22T16:44:06Z","kind":"reset"}]}}}"#;
        let http = Script::new(vec![
            (url, status(503)),
            (CLAUDE_RESETS_DATASET, ok(dataset, Some("d1"))),
            (url, ok("<html>maintenance</html>", None)),
            (CLAUDE_RESETS_DATASET, status(500)),
            (url, status_with(429, "retry-after", "900")),
        ]);
        let store = feeds(root.path(), http.clone(), seconds.clone());
        let (snapshot, changed) = store.refresh(FeedName::ClaudeResets, false).await;
        assert!(changed && snapshot.error.is_none());
        let body: Value = serde_json::from_str(snapshot.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["live"], false);
        assert_eq!(body["detector"], Value::Null);
        assert_eq!(body["events"].as_array().unwrap().len(), 1);

        seconds.store(300, Ordering::SeqCst);
        let (down, changed) = store.refresh(FeedName::ClaudeResets, false).await;
        assert!(!changed && !down.stale);
        assert_eq!(
            down.error.as_deref(),
            Some("The source answered with data in an unexpected shape")
        );
        assert_eq!(down.body, snapshot.body);

        seconds.store(600, Ordering::SeqCst);
        let (limited, _) = store.refresh(FeedName::ClaudeResets, false).await;
        assert_eq!(limited.error.as_deref(), Some("HTTP 429"));
        assert_eq!(http.seen.lock().unwrap().len(), 5);
        seconds.store(600 + 899, Ordering::SeqCst);
        assert!(!store.due(FeedName::ClaudeResets).await);
        seconds.store(600 + 900, Ordering::SeqCst);
        assert!(store.due(FeedName::ClaudeResets).await);
    }

    #[tokio::test]
    async fn a_failed_live_read_keeps_the_live_body_it_has() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let url = FeedName::ClaudeResets.url();
        let live = claude_answer(
            "2026-09-29T13:06:49Z",
            &[("7", "Reset for all."), ("8", "Limits reset.")],
            &["8"],
        );
        let published = claude_answer("2026-09-29T00:00:00Z", &[("7", "Reset for all.")], &[]);
        let http = Script::new(vec![
            (url, ok(&live, None)),
            (url, status(503)),
            (url, status(503)),
            (CLAUDE_RESETS_DATASET, ok(&published, None)),
        ]);
        let store = feeds(root.path(), http.clone(), seconds.clone());
        let (first, _) = store.refresh(FeedName::ClaudeResets, false).await;

        seconds.store(300, Ordering::SeqCst);
        let (down, changed) = store.refresh(FeedName::ClaudeResets, false).await;
        assert!(!changed && !down.stale);
        assert_eq!(down.error.as_deref(), Some("HTTP 503"));
        assert_eq!(down.body, first.body);
        assert_eq!(http.seen.lock().unwrap().len(), 2);

        seconds.store(24 * 3600, Ordering::SeqCst);
        let (older, changed) = store.refresh(FeedName::ClaudeResets, false).await;
        assert!(changed && older.error.is_none());
        let body: Value = serde_json::from_str(older.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["live"], false);
        assert_eq!(body["events"].as_array().unwrap().len(), 1);
        assert_eq!(http.seen.lock().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn a_rate_limit_is_never_answered_with_a_second_request() {
        let day = "Tue, 29 Sep 2026 13:00:00 GMT";
        for (answer, held_for) in [
            (status(429), None),
            (status_with(429, "retry-after", day), Some(3600)),
            (status_with(503, "retry-after", day), Some(3600)),
            (
                status_with(503, "retry-after", "Thu, 01 Jan 2026 00:00:00 GMT"),
                Some(1),
            ),
            (
                status_with(503, "retry-after", "Fri, 01 Jan 2027 00:00:00 GMT"),
                Some(RETRY_AFTER_CAP_SECONDS),
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let seconds = Arc::new(AtomicI64::new(0));
            let http = Script::new(vec![(FeedName::ClaudeResets.url(), answer)]);
            let store = PublicFeeds::new(root.path().to_path_buf())
                .with_http(http.clone())
                .with_clock(Arc::new({
                    let seconds = seconds.clone();
                    move || {
                        DateTime::parse_from_rfc3339("2026-09-29T12:00:00Z")
                            .unwrap()
                            .with_timezone(&Utc)
                            + chrono::Duration::seconds(seconds.load(Ordering::SeqCst))
                    }
                }));
            let (limited, changed) = store.refresh(FeedName::ClaudeResets, false).await;
            assert!(!changed && limited.body.is_none() && limited.error.is_some());
            assert_eq!(http.seen.lock().unwrap().len(), 1);
            let held_for = held_for.unwrap_or(5 * 60).max(5 * 60);
            seconds.store(held_for - 1, Ordering::SeqCst);
            assert!(!store.due(FeedName::ClaudeResets).await);
            seconds.store(held_for, Ordering::SeqCst);
            assert!(store.due(FeedName::ClaudeResets).await);
        }
    }

    #[test]
    fn a_long_catalog_keeps_its_newest_announcements() {
        let event = |index: usize| {
            format!(
                r#"{{"id":"{index}","date":"2026-01-01T00:{:02}:{:02}Z","kind":"reset"}}"#,
                index / 60,
                index % 60
            )
        };
        let catalog = |events: Vec<String>| {
            format!(
                r#"{{"providers":{{"claude":{{"events":[{}]}}}}}}"#,
                events.join(",")
            )
        };
        let ids = |body: String| -> Vec<String> {
            let body: Value = serde_json::from_str(&body).unwrap();
            let events = body["events"].as_array().unwrap();
            events
                .iter()
                .map(|event| event["id"].as_str().unwrap().to_owned())
                .collect()
        };
        let count = MAX_CLAUDE_EVENTS + 3;
        let oldest_first =
            ids(claude_catalog(&catalog((0..count).map(event).collect()), true).unwrap());
        assert_eq!(oldest_first.len(), MAX_CLAUDE_EVENTS);
        assert_eq!(oldest_first.first().map(String::as_str), Some("3"));
        assert_eq!(oldest_first.last().map(String::as_str), Some("502"));
        let newest_first =
            ids(claude_catalog(&catalog((0..count).rev().map(event).collect()), true).unwrap());
        assert_eq!(newest_first.len(), MAX_CLAUDE_EVENTS);
        assert_eq!(newest_first.first().map(String::as_str), Some("502"));
        assert_eq!(newest_first.last().map(String::as_str), Some("3"));
        let few = ids(claude_catalog(&catalog((0..3).map(event).collect()), true).unwrap());
        assert_eq!(few, ["0", "1", "2"]);
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
            FeedName::ClaudeResets,
            r#"{"live":true,"events":[{"id":"7","date":"2026-09-22T16:44:06Z","kind":"reset"}]}"#
        ));
        assert!(!valid(
            FeedName::ClaudeResets,
            r#"{"live":true,"events":[]}"#
        ));
        assert!(!valid(FeedName::ClaudeResets, r#"{"providers":{}}"#));
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
