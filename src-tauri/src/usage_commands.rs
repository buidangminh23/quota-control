use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};
use uc_logscan::context::{ContextWindowSession, ContextWindows};
use uc_logscan::ledger::{UsageGroupRow, UsageLedger, UsageLedgerInfo, UsageQuery};
use uc_logscan::{LogScanner, LogSource};

use crate::exchange_rate::{ExchangeRate, ExchangeRateStore};

pub struct UsageService {
    ledger: Arc<UsageLedger>,
    rate: Arc<ExchangeRateStore>,
    contexts: Arc<Mutex<ContextWindows>>,
}

impl UsageService {
    pub fn new() -> anyhow::Result<Self> {
        let root = uc_core::paths::config_dir();
        Ok(Self {
            ledger: Arc::new(UsageLedger::open(root.join("usage-ledger.sqlite3"))?),
            rate: Arc::new(ExchangeRateStore::new(root)),
            contexts: Arc::new(Mutex::new(ContextWindows::from_environment())),
        })
    }

    pub fn start(&self, app: &AppHandle) {
        let ledger = self.ledger.clone();
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let ledger = ledger.clone();
                let handle = handle.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    let scanners = [
                        LogScanner::from_environment(LogSource::Claude),
                        LogScanner::from_environment(LogSource::Codex),
                    ];
                    let started = std::time::Instant::now();
                    match ledger.import(&scanners, |info| {
                        let _ = handle.emit_to("popup", "usage-ledger-changed", info);
                    }) {
                        Ok(report) => tracing::info!(
                            files = report.files_read,
                            bytes = report.bytes_read,
                            events = report.events_written,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "Usage ledger scan completed"
                        ),
                        Err(_) => tracing::warn!("Usage ledger scan could not complete"),
                    }
                    if let Ok(info) = ledger.info() {
                        let _ = handle.emit_to("popup", "usage-ledger-changed", info);
                    }
                })
                .await;
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
        let rate = self.rate.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                rate.get().await;
                tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
            }
        });
    }
}

#[tauri::command]
pub async fn usage_summary(
    service: State<'_, UsageService>,
    query: UsageQuery,
) -> Result<Vec<UsageGroupRow>, String> {
    let ledger = service.ledger.clone();
    tauri::async_runtime::spawn_blocking(move || ledger.summary(&query))
        .await
        .map_err(|_| "Usage history is unavailable".to_string())?
        .map_err(|_| "Could not query usage history. Check the date range.".to_string())
}

#[tauri::command]
pub async fn usage_ledger_info(
    service: State<'_, UsageService>,
) -> Result<UsageLedgerInfo, String> {
    let ledger = service.ledger.clone();
    tauri::async_runtime::spawn_blocking(move || ledger.info())
        .await
        .map_err(|_| "Usage history is unavailable".to_string())?
        .map_err(|_| "Could not read usage history status".to_string())
}

#[tauri::command]
pub async fn exchange_rate(
    service: State<'_, UsageService>,
) -> Result<Option<ExchangeRate>, String> {
    Ok(service.rate.get().await)
}

#[tauri::command]
pub async fn context_windows(
    service: State<'_, UsageService>,
) -> Result<Vec<ContextWindowSession>, String> {
    let contexts = service.contexts.clone();
    tauri::async_runtime::spawn_blocking(move || {
        contexts
            .lock()
            .map(|mut reader| reader.scan(chrono::Utc::now()))
            .map_err(|_| "Session context is unavailable".to_string())
    })
    .await
    .map_err(|_| "Session context is unavailable".to_string())?
}

pub fn start(app: &AppHandle) {
    app.state::<UsageService>().start(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    #[tokio::test]
    async fn context_command_returns_only_the_session_metadata_contract() {
        let directory = tempfile::tempdir().unwrap();
        let claude = directory.path().join("claude");
        let transcripts = claude.join("projects/fixture");
        std::fs::create_dir_all(&transcripts).unwrap();
        let timestamp = chrono::Utc::now().to_rfc3339();
        let record = json!({
            "type":"assistant", "sessionId":"session", "cwd":"Z:/fixtures/context-project", "timestamp":timestamp,
            "message":{"id":"request", "model":"claude-opus-5-5", "content":[{"type":"text","text":"not part of the command response"}],
                "usage":{"input_tokens":100,"cache_creation_input_tokens":20,"cache_read_input_tokens":30,"output_tokens":10}}
        });
        std::fs::write(transcripts.join("session.jsonl"), format!("{record}\n")).unwrap();
        let service = UsageService {
            ledger: Arc::new(UsageLedger::open(directory.path().join("ledger.sqlite3")).unwrap()),
            rate: Arc::new(ExchangeRateStore::new(directory.path().to_path_buf())),
            contexts: Arc::new(Mutex::new(ContextWindows::new(
                claude,
                directory.path().join("codex"),
            ))),
        };
        let app = mock_builder()
            .manage(service)
            .build(mock_context(noop_assets()))
            .unwrap();
        let sessions = context_windows(app.state()).await.unwrap();
        assert_eq!(
            serde_json::to_value(sessions).unwrap(),
            json!([{
                "source":"claude", "sessionId":"session", "project":"context-project", "model":"claude-opus-5-5",
                "usedTokens":160,"windowTokens":1000000,"baseTokens":150,"lastTurnTokens":0,"updatedAt":timestamp
            }])
        );
    }

    #[tokio::test]
    async fn ledger_commands_match_the_frontend_contract_without_starting_imports() {
        let directory = tempfile::tempdir().unwrap();
        let service = UsageService {
            ledger: Arc::new(UsageLedger::open(directory.path().join("ledger.sqlite3")).unwrap()),
            rate: Arc::new(ExchangeRateStore::new(directory.path().to_path_buf())),
            contexts: Arc::new(Mutex::new(ContextWindows::new(
                directory.path().join("claude"),
                directory.path().join("codex"),
            ))),
        };
        let app = mock_builder()
            .manage(service)
            .build(mock_context(noop_assets()))
            .unwrap();
        let query = serde_json::from_value(json!({"groupBy":"project"})).unwrap();
        let rows = usage_summary(app.state(), query).await.unwrap();
        assert!(rows.is_empty());
        assert!(context_windows(app.state()).await.unwrap().is_empty());
        let info = usage_ledger_info(app.state()).await.unwrap();
        assert_eq!(
            serde_json::to_value(info).unwrap(),
            json!({"firstDay":null,"updatedAt":null,"importing":false})
        );
        let invalid = serde_json::from_value(json!({"groupBy":"day","from":"invalid"})).unwrap();
        assert!(usage_summary(app.state(), invalid).await.is_err());
    }
}
