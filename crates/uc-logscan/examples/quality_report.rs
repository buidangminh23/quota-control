//! Scan this machine's Claude Code and Codex transcripts and print the per-model quality counts.
//! `cargo run --release -p uc-logscan --example quality_report -- <cache.json> [from] [to]`

use std::time::Instant;

use uc_logscan::quality::{QualityQuery, QualityStore};
use uc_logscan::{LogScanner, LogSource};

fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let cache = arguments
        .next()
        .unwrap_or_else(|| "quality-report.json".into());
    let query = QualityQuery {
        from: arguments.next(),
        to: arguments.next(),
    };
    let store = QualityStore::open(cache);
    let scanners = [
        LogScanner::from_environment(LogSource::Claude),
        LogScanner::from_environment(LogSource::Codex),
    ];
    let started = Instant::now();
    let report = store.scan(&scanners)?;
    println!(
        "scan: {:.1}s, files {} seen / {} parsed / {} unreadable, {:.2} GB, {} skipped lines",
        started.elapsed().as_secs_f64(),
        report.files_seen,
        report.files_parsed,
        report.unreadable_files,
        report.bytes_parsed as f64 / 1e9,
        report.skipped_lines
    );
    let summary = store.summary(&query)?;
    println!("{}", serde_json::to_string(&summary.info)?);
    let mut models =
        std::collections::BTreeMap::<(String, String), uc_logscan::quality::QualityCounts>::new();
    for row in &summary.rows {
        let entry = models
            .entry((format!("{:?}", row.source), row.model.clone()))
            .or_default();
        let counts = &row.counts;
        entry.turns += counts.turns;
        entry.human_turns += counts.human_turns;
        entry.interrupted_turns += counts.interrupted_turns;
        entry.verified_turns += counts.verified_turns;
        entry.green_turns += counts.green_turns;
        entry.check_runs += counts.check_runs;
        entry.failed_check_runs += counts.failed_check_runs;
        entry.unknown_check_runs += counts.unknown_check_runs;
        entry.edits += counts.edits;
        entry.failed_edits += counts.failed_edits;
        entry.denied_actions += counts.denied_actions;
        entry.shell_commands += counts.shell_commands;
        entry.failed_shell_commands += counts.failed_shell_commands;
        entry.output_tokens += counts.output_tokens;
        entry.timed_turns += counts.timed_turns;
        entry.turn_millis += counts.turn_millis;
    }
    println!(
        "source model | turns human intr | verified green | checks fail unknown | edits fail | shell fail | denied | outTok/turn | sec/turn"
    );
    for ((source, model), c) in models {
        println!(
            "{source} {model} | {} {} {} | {} {} | {} {} {} | {} {} | {} {} | {} | {:.0} | {:.0}",
            c.turns,
            c.human_turns,
            c.interrupted_turns,
            c.verified_turns,
            c.green_turns,
            c.check_runs,
            c.failed_check_runs,
            c.unknown_check_runs,
            c.edits,
            c.failed_edits,
            c.shell_commands,
            c.failed_shell_commands,
            c.denied_actions,
            c.output_tokens as f64 / c.turns.max(1) as f64,
            c.turn_millis as f64 / 1000.0 / c.timed_turns.max(1) as f64
        );
    }
    Ok(())
}
