//! The Benchmark and Reset tabs' data: quality counted from this machine's Claude Code and Codex
//! transcripts, and the public feeds (Codex resets, Epoch AI, Arena, 3D Arena). Both refresh in
//! the background and announce changes to the popup, which asks for the data it shows.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};
use uc_logscan::quality::{QualityInfo, QualityQuery, QualityStore, QualitySummary};
use uc_logscan::{LogScanner, LogSource};

use crate::public_feeds::{FeedName, FeedSnapshot, PublicFeeds};

const FIRST_SCAN_DELAY: Duration = Duration::from_secs(20);
const SCAN_INTERVAL: Duration = Duration::from_secs(10 * 60);
const FEED_TICK: Duration = Duration::from_secs(60);

pub struct InsightsService {
    quality: Arc<QualityStore>,
    feeds: Arc<PublicFeeds>,
    scan_now: Arc<tokio::sync::Notify>,
}

impl InsightsService {
    pub fn new() -> Self {
        Self {
            quality: Arc::new(QualityStore::open(
                uc_core::paths::cache_dir().join("model-quality.json"),
            )),
            feeds: Arc::new(PublicFeeds::new(uc_core::paths::config_dir().join("feeds"))),
            scan_now: Arc::new(tokio::sync::Notify::new()),
        }
    }

    fn start(&self, app: &AppHandle) {
        let quality = self.quality.clone();
        let scan_now = self.scan_now.clone();
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_SCAN_DELAY).await;
            loop {
                let store = quality.clone();
                let emitter = handle.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    let _ = emitter.emit_to(
                        "popup",
                        "model-quality-changed",
                        QualityInfo {
                            scanning: true,
                            ..store.info()
                        },
                    );
                    let scanners = [
                        LogScanner::from_environment(LogSource::Claude),
                        LogScanner::from_environment(LogSource::Codex),
                    ];
                    let started = std::time::Instant::now();
                    match store.scan(&scanners) {
                        Ok(report) => tracing::info!(
                            files = report.files_seen,
                            parsed = report.files_parsed,
                            bytes = report.bytes_parsed,
                            unreadable = report.unreadable_files,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "Model quality scan completed"
                        ),
                        Err(error) => {
                            tracing::warn!("Model quality scan could not complete: {error:#}")
                        }
                    }
                    let _ = emitter.emit_to("popup", "model-quality-changed", store.info());
                })
                .await;
                tokio::select! {
                    _ = tokio::time::sleep(SCAN_INTERVAL) => {}
                    _ = scan_now.notified() => {}
                }
            }
        });
        let feeds = self.feeds.clone();
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                for name in FeedName::ALL {
                    if !feeds.due(name).await {
                        continue;
                    }
                    let (_, changed) = feeds.refresh(name, false).await;
                    if changed {
                        let _ = handle.emit_to("popup", "public-feed-changed", name);
                    }
                }
                tokio::time::sleep(FEED_TICK).await;
            }
        });
    }
}

#[tauri::command]
pub async fn model_quality(
    service: State<'_, InsightsService>,
    query: QualityQuery,
) -> Result<QualitySummary, String> {
    let store = service.quality.clone();
    tauri::async_runtime::spawn_blocking(move || store.summary(&query))
        .await
        .map_err(|_| "Model quality is unavailable".to_string())?
        .map_err(|_| "Could not read model quality. Check the date range.".to_string())
}

#[tauri::command]
pub fn rescan_model_quality(service: State<'_, InsightsService>) {
    service.scan_now.notify_one();
}

#[tauri::command]
pub async fn public_feed(
    service: State<'_, InsightsService>,
    name: FeedName,
) -> Result<FeedSnapshot, String> {
    Ok(service.feeds.snapshot(name).await)
}

#[tauri::command]
pub async fn refresh_public_feed(
    app: AppHandle,
    service: State<'_, InsightsService>,
    name: FeedName,
) -> Result<FeedSnapshot, String> {
    let (snapshot, changed) = service.feeds.refresh(name, true).await;
    if changed {
        let _ = app.emit_to("popup", "public-feed-changed", name);
    }
    Ok(snapshot)
}

pub fn start(app: &AppHandle) {
    app.state::<InsightsService>().start(app);
}
