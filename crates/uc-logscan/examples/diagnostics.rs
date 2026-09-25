use std::time::Instant;

use chrono::Utc;
use serde_json::json;
use uc_core::TokenUsage;
use uc_logscan::{LogScanner, LogSource};

fn main() {
    for (name, source) in [("claude", LogSource::Claude), ("codex", LogSource::Codex)] {
        let scanner = LogScanner::from_environment(source);
        let now = Utc::now();
        for pass in ["full", "cached"] {
            let started = Instant::now();
            let report = scanner.scan(now);
            let totals = report
                .usage
                .series
                .daily
                .iter()
                .fold(0_i64, |sum, day| sum.saturating_add(day.total_tokens));
            let tokens = report
                .usage
                .series
                .daily
                .iter()
                .try_fold(TokenUsage::default(), |sum, day| {
                    day.token_usage.map(|usage| sum.saturating_add(usage))
                });
            println!(
                "{}",
                json!({
                    "source": name,
                    "pass": pass,
                "filesRead": report.files_read,
                "filesCached": report.files_cached,
                "bytesRead": report.bytes_read,
                    "records": report.records,
                    "days": report.usage.series.daily.len(),
                    "totalTokens": totals,
                    "tokenUsage": tokens,
                    "skippedLines": report.skipped_lines,
                    "incomplete": report.incomplete,
                    "warnings": report.warnings,
                    "elapsedMs": started.elapsed().as_millis(),
                })
            );
        }
    }
}
