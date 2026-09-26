use std::collections::BTreeSet;
use std::time::Instant;

use quota_control_lib::exchange_rate::ExchangeRateStore;
use serde_json::json;
use uc_logscan::ledger::{UsageLedger, UsageQuery};
use uc_logscan::{LogScanner, LogSource};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root = uc_core::paths::config_dir();
    if std::env::args().any(|arg| arg == "--rate-only") {
        println!(
            "{}",
            json!({"rate":ExchangeRateStore::new(root).get().await})
        );
        return Ok(());
    }
    let path = root.join("usage-ledger.sqlite3");
    let ledger = UsageLedger::open(&path)?;
    if std::env::args().any(|arg| arg == "--import") {
        let started = Instant::now();
        let mut last_report = Instant::now();
        let report = ledger.import(
            &[
                LogScanner::from_environment(LogSource::Claude),
                LogScanner::from_environment(LogSource::Codex),
            ],
            |_| {
                if last_report.elapsed().as_secs() >= 30 {
                    println!(
                        "{}",
                        json!({"importing":true,"seconds":started.elapsed().as_secs()})
                    );
                    last_report = Instant::now();
                }
            },
        )?;
        println!(
            "{}",
            json!({
                "seconds":started.elapsed().as_secs_f64(),
                "files":report.files_read,
                "bytes":report.bytes_read,
                "events":report.events_written,
                "skippedFiles":report.skipped_files,
                "skippedLines":report.skipped_lines
            })
        );
    }
    let query: UsageQuery = serde_json::from_value(json!({"groupBy":"project"}))?;
    let rows = ledger.summary(&query)?;
    let projects: BTreeSet<_> = rows
        .iter()
        .filter(|row| !row.key.is_empty() && row.totals.total_tokens > 0)
        .map(|row| row.key.as_str())
        .collect();
    let rate = ExchangeRateStore::new(root).get().await;
    let info = ledger.info()?;
    drop(ledger);
    println!(
        "{}",
        json!({
            "ledgerBytes":std::fs::metadata(path)?.len(),
            "projects":projects.len(),
            "hasQuotaControl":projects.contains("quota-control"),
            "hasPcc4sh":projects.iter().any(|name| name.eq_ignore_ascii_case("PCC4SH")),
            "info":info,
            "rate":rate
        })
    );
    Ok(())
}
