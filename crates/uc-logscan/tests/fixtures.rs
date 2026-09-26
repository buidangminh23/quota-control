use std::fs;

use chrono::{DateTime, FixedOffset, Utc};
use serde_json::{Value, json};
use uc_core::*;
use uc_logscan::*;

fn now() -> DateTime<Utc> {
    "2026-09-25T20:00:00Z".parse().unwrap()
}

fn write(dir: &std::path::Path, name: &str, rows: &[Value]) {
    let path = dir.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        rows.iter().map(|v| format!("{v}\n")).collect::<String>(),
    )
    .unwrap();
}

fn scanner(source: LogSource, dir: &std::path::Path) -> LogScanner {
    LogScanner::new(
        source,
        ScanOptions {
            roots: vec![dir.to_owned()],
            timezone: FixedOffset::east_opt(7 * 3600),
            ..ScanOptions::default()
        },
    )
}

fn claude(id: &str, timestamp: &str, output: i64) -> Value {
    json!({"type":"assistant","timestamp":timestamp,"message":{"id":id,"model":"claude-sonnet-4-20250514","usage":{"input_tokens":100,"output_tokens":output,"cache_read_input_tokens":200,"cache_creation_input_tokens":300}},"requestId":"r"})
}

fn codex(timestamp: &str, input: i64, output: i64, cached: i64) -> Value {
    json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"output_tokens":output,"cached_input_tokens":cached,"reasoning_output_tokens":output/2,"total_tokens":input+output}}}})
}

#[test]
fn claude_duplicate_updates_count_once_and_cache_buckets_are_disjoint() {
    let dir = tempfile::tempdir().unwrap();
    let a = claude("msg-1", "2026-09-25T18:00:00Z", 10);
    let b = claude("msg-1", "2026-09-25T18:00:00Z", 50);
    let mut sidechain = claude("msg-1", "2026-09-25T18:00:00Z", 500);
    sidechain["isSidechain"] = json!(true);
    sidechain["requestId"] = json!("replay");
    write(dir.path(), "main.jsonl", &[a, b]);
    write(dir.path(), "nested/subagents/replay.jsonl", &[sidechain]);
    let result = scanner(LogSource::Claude, dir.path()).scan(now());
    assert_eq!(result.records, 1);
    let day = &result.usage.series.daily[0];
    assert_eq!(day.date, "2026-09-26");
    assert_eq!(day.total_tokens, 650);
    assert_eq!(
        day.token_usage,
        Some(TokenUsage {
            input_tokens: 600,
            output_tokens: 50,
            cached_input_tokens: 200,
            cache_creation_input_tokens: 300
        })
    );
    assert!((day.cost_usd.unwrap() - 0.002235).abs() < 1e-10);
}

#[test]
fn codex_cumulative_deltas_ignore_repeats_and_do_not_add_reasoning_again() {
    let dir = tempfile::tempdir().unwrap();
    let a = codex("2026-09-25T16:00:00Z", 1000, 200, 100);
    let mut repeated = a.clone();
    repeated["timestamp"] = json!("2026-09-25T16:01:00Z");
    let b = codex("2026-09-25T18:00:00Z", 1500, 300, 200);
    write(
        dir.path(),
        "nested/session.jsonl",
        &[
            json!({"type":"session_meta","payload":{"id":"session-1","forked_from_id":null}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            a,
            repeated,
            b,
        ],
    );
    let result = scanner(LogSource::Codex, dir.path()).scan(now());
    assert_eq!(result.records, 2);
    assert_eq!(result.usage.series.daily.len(), 2);
    assert_eq!(result.usage.series.daily[0].total_tokens, 1200);
    assert_eq!(result.usage.series.daily[1].total_tokens, 600);
    assert_eq!(
        result.usage.series.daily[1].token_usage,
        Some(TokenUsage {
            input_tokens: 500,
            output_tokens: 100,
            cached_input_tokens: 100,
            cache_creation_input_tokens: 0
        })
    );
}

#[test]
fn codex_child_replay_seeds_baseline_without_billing_parent() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "child.jsonl",
        &[
            json!({"type":"session_meta","timestamp":"2026-09-25T18:00:00Z","payload":{"id":"child","forked_from_id":"parent"}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            codex("2026-09-25T17:00:00Z", 1000, 200, 100),
            json!({"type":"event_msg","payload":{"type":"task_started","started_at":"2026-09-25T18:00:00Z".parse::<DateTime<Utc>>().unwrap().timestamp()}}),
            codex("2026-09-25T18:01:00Z", 1500, 300, 200),
        ],
    );
    let result = scanner(LogSource::Codex, dir.path()).scan(now());
    assert_eq!(result.records, 1);
    assert_eq!(result.usage.series.daily[0].total_tokens, 600);
}

#[test]
fn unknown_models_keep_tokens_without_fabricating_cost_or_clearing_api_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut event = claude("m1", "2026-09-25T18:00:00Z", 50);
    event["message"]["model"] = json!("future-model");
    write(dir.path(), "session.jsonl", &[event]);
    let result = scanner(LogSource::Claude, dir.path()).scan(now());
    assert_eq!(result.usage.series.daily[0].cost_usd, None);
    let mut snapshot = ProviderSnapshot::error_message(
        &Provider::new("claude", "Claude"),
        "Not logged in",
        Some(ErrorCategory::NotLoggedIn),
    );
    append_history(&mut snapshot, result, now());
    assert!(snapshot.is_error());
    assert_eq!(snapshot.error_category, Some(ErrorCategory::NotLoggedIn));
    let MetricLine::Values(today) = snapshot.line("Today").unwrap() else {
        panic!()
    };
    assert_eq!(today.values.len(), 1);
    assert_eq!(today.values[0].kind, MetricKind::Count);
    assert_eq!(today.unknown_models, ["future-model"]);
    assert_eq!(
        today
            .model_breakdown
            .as_ref()
            .unwrap()
            .token_usage
            .unwrap()
            .output_tokens,
        50
    );
    let MetricLine::Values(input) = snapshot.line("Input Tokens").unwrap() else {
        panic!()
    };
    assert_eq!(input.values[0].number, 600.0);
    assert!(
        !serde_json::to_string(&snapshot)
            .unwrap()
            .contains(dir.path().to_string_lossy().as_ref())
    );
}

#[test]
fn truncated_tail_and_oversized_lines_are_bounded_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let valid = claude("m1", "2026-09-25T18:00:00Z", 50);
    fs::write(
        dir.path().join("log.jsonl"),
        format!(
            "{}\n{valid}\n{{\"type\":\"assistant\",\"usage\":",
            "x".repeat(2000)
        ),
    )
    .unwrap();
    let mut scan = scanner(LogSource::Claude, dir.path());
    scan.options.max_line_bytes = 1000;
    let result = scan.scan(now());
    assert_eq!(result.records, 1);
    assert_eq!(result.skipped_lines, 2);
    assert!(!result.warnings.is_empty());
    scan.options.max_bytes = 10;
    let limited = LogScanner::new(scan.source, scan.options.clone()).scan(now());
    assert!(limited.incomplete);
    assert_eq!(limited.records, 0);
}

#[test]
fn empty_or_missing_logs_do_not_display_fake_zero_spend() {
    let dir = tempfile::tempdir().unwrap();
    let report = scanner(LogSource::Claude, &dir.path().join("missing")).scan(now());
    let provider = Provider::new("claude", "Claude");
    let mut snapshot = ProviderSnapshot::make(&provider, None, vec![], now());
    append_history(&mut snapshot, report, now());
    assert!(snapshot.line("Today").is_none());
    assert!(snapshot.usage_history.is_none());
    assert_eq!(history_descriptors(&provider).len(), 7);
}

#[test]
fn archived_copies_of_codex_sessions_do_not_count_twice() {
    let dir = tempfile::tempdir().unwrap();
    let rows = [
        json!({"type":"session_meta","payload":{"id":"shared"}}),
        codex("2026-09-25T18:00:00Z", 100, 50, 20),
    ];
    write(dir.path(), "sessions/a.jsonl", &rows);
    write(dir.path(), "archived_sessions/b.jsonl", &rows);
    let report = scanner(LogSource::Codex, dir.path()).scan(now());
    assert_eq!(report.records, 1);
    assert_eq!(report.usage.series.daily[0].total_tokens, 150);
}

#[test]
fn thirty_day_window_excludes_older_and_future_entries() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "days.jsonl",
        &[
            claude("old", "2026-08-26T18:00:00Z", 50),
            claude("boundary", "2026-08-27T18:00:00Z", 50),
            claude("future", "2026-09-26T18:00:00Z", 50),
        ],
    );
    let result = scanner(LogSource::Claude, dir.path()).scan(now());
    assert_eq!(result.records, 1);
    assert_eq!(result.usage.series.daily[0].date, "2026-08-28");
}

#[test]
fn absent_input_output_breakdown_remains_absent() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "total.jsonl",
        &[
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            json!({"type":"event_msg","timestamp":"2026-09-25T18:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":123}}}}),
        ],
    );
    let report = scanner(LogSource::Codex, dir.path()).scan(now());
    let row = &report.usage.series.daily[0];
    assert_eq!(row.total_tokens, 123);
    assert_eq!(row.token_usage, None);
    assert_eq!(row.cost_usd, None);
    let serialized = serde_json::to_value(row).unwrap();
    assert!(serialized.get("tokenUsage").is_none());
    let mut snapshot =
        ProviderSnapshot::make(&Provider::new("codex", "Codex"), None, vec![], now());
    append_history(&mut snapshot, report, now());
    assert!(snapshot.line("Input Tokens").is_none());
}

#[tokio::test]
async fn machine_local_runtime_works_without_quota_credentials() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "session.jsonl",
        &[claude("m1", "2026-09-25T18:00:00Z", 50)],
    );
    let runtime = LocalHistoryRuntime::new(LogSource::Claude)
        .with_scanner(scanner(LogSource::Claude, dir.path()))
        .with_clock(fixed_clock(now()));
    assert_eq!(runtime.provider().id, "claude-local");
    assert_eq!(runtime.provider().icon, "claude");
    assert!(runtime.has_local_credentials().await);
    let snapshot = runtime.refresh(RefreshContext::manual()).await;
    assert!(!snapshot.is_error());
    assert!(snapshot.line("Output Tokens").is_some());
    assert!(
        runtime.widget_descriptors()[0]
            .history_resource
            .as_ref()
            .unwrap()
            .source_note
            .contains("not account-scoped")
    );
}

struct AccountRuntime(Provider);

#[async_trait::async_trait]
impl ProviderRuntime for AccountRuntime {
    fn provider(&self) -> &Provider {
        &self.0
    }
    fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
        vec![]
    }
    async fn refresh(&self, _: RefreshContext) -> ProviderSnapshot {
        ProviderSnapshot::make(&self.0, None, vec![], now())
    }
    async fn has_local_credentials(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn account_adapter_does_not_duplicate_machine_history() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "session.jsonl",
        &[claude("m1", "2026-09-25T18:00:00Z", 50)],
    );
    let account = std::sync::Arc::new(AccountRuntime(Provider::new("claude@account", "Claude")));
    let runtime = HistoryRuntime::new(account, scanner(LogSource::Claude, dir.path()))
        .with_clock(fixed_clock(now()));
    assert!(runtime.widget_descriptors().is_empty());
    assert!(
        runtime
            .refresh(RefreshContext::manual())
            .await
            .usage_history
            .is_none()
    );
}

#[test]
fn claude_cache_creation_subtypes_replace_aggregate_instead_of_doubling_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut event = claude("m1", "2026-09-25T18:00:00Z", 50);
    event["message"]["usage"]["cache_creation"] =
        json!({"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":200});
    write(dir.path(), "creation.jsonl", &[event]);
    let report = scanner(LogSource::Claude, dir.path()).scan(now());
    let day = &report.usage.series.daily[0];
    assert_eq!(day.total_tokens, 650);
    assert_eq!(day.token_usage.unwrap().cache_creation_input_tokens, 300);
    assert!((day.cost_usd.unwrap() - 0.002685).abs() < 1e-10);
}

#[test]
fn codex_counter_reset_keeps_new_request_without_negative_deltas() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "reset.jsonl",
        &[
            codex("2026-09-25T18:00:00Z", 1000, 200, 100),
            codex("2026-09-25T18:01:00Z", 200, 50, 20),
            codex("2026-09-25T18:02:00Z", 300, 75, 30),
        ],
    );
    let report = scanner(LogSource::Codex, dir.path()).scan(now());
    let day = &report.usage.series.daily[0];
    assert_eq!(day.total_tokens, 1575);
    assert_eq!(day.token_usage.unwrap().input_tokens, 1300);
    assert_eq!(day.token_usage.unwrap().output_tokens, 275);
}

#[test]
fn unchanged_file_cache_is_shared_across_scanner_clones() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "session.jsonl",
        &[claude("m1", "2026-09-25T18:00:00Z", 50)],
    );
    let scanner = scanner(LogSource::Claude, dir.path());
    let first = scanner.scan(now());
    let second = scanner.clone().scan(now());
    assert_eq!(first.files_read, 1);
    assert_eq!(first.files_cached, 0);
    assert_eq!(second.files_read, 0);
    assert_eq!(second.files_cached, 1);
    assert_eq!(second.bytes_read, 0);
    assert_eq!(first.usage, second.usage);
    assert!(!second.incomplete);
}

#[test]
fn appended_codex_file_reparses_cumulative_baseline_without_double_counting() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "session.jsonl",
        &[codex("2026-09-25T18:00:00Z", 1000, 200, 100)],
    );
    let scanner = scanner(LogSource::Codex, dir.path());
    assert_eq!(scanner.scan(now()).usage.series.daily[0].total_tokens, 1200);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(dir.path().join("session.jsonl"))
        .unwrap();
    writeln!(file, "{}", codex("2026-09-25T18:01:00Z", 1500, 300, 200)).unwrap();
    let second = scanner.scan(now());
    assert_eq!(second.files_read, 1);
    assert_eq!(second.files_cached, 0);
    assert_eq!(second.usage.series.daily[0].total_tokens, 1800);
    assert_eq!(scanner.scan(now()).files_cached, 1);
}

#[test]
fn same_length_modification_invalidates_cache_and_deleted_files_are_evicted() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "session.jsonl",
        &[claude("m1", "2026-09-25T18:00:00Z", 50)],
    );
    let scanner = scanner(LogSource::Claude, dir.path());
    let original = scanner.scan(now());
    let path = dir.path().join("session.jsonl");
    let old_modified = fs::metadata(&path).unwrap().modified().unwrap();
    write(
        dir.path(),
        "session.jsonl",
        &[claude("m1", "2026-09-25T18:00:00Z", 70)],
    );
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            fs::FileTimes::new().set_modified(old_modified + std::time::Duration::from_secs(2)),
        )
        .unwrap();
    let second = scanner.scan(now());
    assert_eq!(second.files_read, 1);
    assert_eq!(second.files_cached, 0);
    assert_eq!(
        second.usage.series.daily[0].total_tokens,
        original.usage.series.daily[0].total_tokens + 20
    );
    fs::remove_file(&path).unwrap();
    let removed = scanner.scan(now());
    assert_eq!(removed.records, 0);
    assert_eq!(removed.files_cached, 0);
}

#[test]
fn cached_and_reparsed_duplicate_files_still_deduplicate_globally() {
    let dir = tempfile::tempdir().unwrap();
    let event = claude("m1", "2026-09-25T18:00:00Z", 50);
    write(dir.path(), "a.jsonl", std::slice::from_ref(&event));
    write(dir.path(), "b.jsonl", &[event]);
    let scanner = scanner(LogSource::Claude, dir.path());
    assert_eq!(scanner.scan(now()).records, 1);
    write(
        dir.path(),
        "b.jsonl",
        &[claude("m1", "2026-09-25T18:00:00Z", 150)],
    );
    let second = scanner.scan(now());
    assert_eq!(second.files_cached, 1);
    assert_eq!(second.files_read, 1);
    assert_eq!(second.records, 1);
    assert_eq!(second.usage.series.daily[0].total_tokens, 750);
}

#[test]
fn candidate_age_uses_modification_time_and_cache_reaggregates_calendar_days() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "old-name-2000.jsonl",
        &[claude("m1", "2026-09-25T18:00:00Z", 50)],
    );
    write(
        dir.path(),
        "old.jsonl",
        &[claude("old", "2026-08-01T00:00:00Z", 50)],
    );
    fs::File::options()
        .write(true)
        .open(dir.path().join("old.jsonl"))
        .unwrap()
        .set_times(
            fs::FileTimes::new().set_modified(std::time::SystemTime::from(
                now() - chrono::Duration::days(40),
            )),
        )
        .unwrap();
    let scanner = scanner(LogSource::Claude, dir.path());
    let first = scanner.scan(now());
    assert_eq!(first.files_read, 1);
    let later = scanner.scan(now() + chrono::Duration::days(30));
    assert_eq!(later.records, 0);
}

#[test]
fn claude_project_memory_directory_is_not_a_transcript_source() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "project/session.jsonl",
        &[claude("m1", "2026-09-25T18:00:00Z", 50)],
    );
    write(
        dir.path(),
        "project/memory/not-a-session.jsonl",
        &[claude("memory", "2026-09-25T18:00:00Z", 50)],
    );
    let report = scanner(LogSource::Claude, dir.path()).scan(now());
    assert_eq!(report.files_read, 1);
    assert_eq!(report.records, 1);
    assert!(!report.incomplete);
}

#[test]
fn budget_interrupted_file_is_not_cached_as_complete() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "session.jsonl",
        &[
            codex("2026-09-25T18:00:00Z", 1000, 200, 100),
            codex("2026-09-25T18:01:00Z", 1500, 300, 200),
        ],
    );
    let mut scanner = scanner(LogSource::Codex, dir.path());
    scanner.options.max_bytes = 10;
    assert!(scanner.scan(now()).incomplete);
    scanner.options.max_bytes = 100_000;
    let full = scanner.scan(now());
    assert_eq!(full.files_read, 1);
    assert_eq!(full.files_cached, 0);
    assert!(!full.incomplete);
    assert_eq!(full.usage.series.daily[0].total_tokens, 1800);
    assert_eq!(scanner.clone().scan(now()).files_cached, 1);
}

#[test]
fn oversized_codex_compaction_does_not_claim_lost_usage() {
    let dir = tempfile::tempdir().unwrap();
    let compacted = format!(
        "{{\"timestamp\":\"2026-09-25T18:00:00Z\",\"type\":\"compacted\",\"payload\":{{\"message\":\"{}\"}}}}\n",
        "x".repeat(20_000)
    );
    let token = codex("2026-09-25T18:00:00Z", 1000, 200, 100);
    fs::write(
        dir.path().join("session.jsonl"),
        format!("{compacted}{token}\n"),
    )
    .unwrap();
    let mut scanner = scanner(LogSource::Codex, dir.path());
    scanner.options.max_line_bytes = 1000;
    let report = scanner.scan(now());
    assert_eq!(report.records, 1);
    assert_eq!(report.skipped_lines, 0);
    assert!(!report.incomplete);
    assert!(report.warnings.is_empty());
}

#[test]
fn oversized_token_records_still_warn_even_with_nested_compaction_marker() {
    let dir = tempfile::tempdir().unwrap();
    let content = format!(
        "{{\"timestamp\":\"2026-09-25T18:00:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"irrelevant\":{{\"type\":\"compacted\"}},\"text\":\"{}\"}}}}\n",
        "x".repeat(20_000)
    );
    fs::write(dir.path().join("session.jsonl"), content).unwrap();
    let mut scanner = scanner(LogSource::Codex, dir.path());
    scanner.options.max_line_bytes = 1000;
    assert_eq!(scanner.scan(now()).skipped_lines, 1);
}

#[cfg(unix)]
#[test]
fn directory_symlinks_are_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(
        outside.path(),
        "secret.jsonl",
        &[claude("m1", "2026-09-25T18:00:00Z", 50)],
    );
    std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
    assert_eq!(
        scanner(LogSource::Claude, dir.path()).scan(now()).records,
        0
    );
}

#[cfg(unix)]
#[test]
fn roots_behind_user_symlinks_are_refused_while_system_links_are_not() {
    let real = tempfile::tempdir().unwrap();
    write(
        real.path(),
        "session.jsonl",
        &[claude("m1", "2026-09-25T18:00:00Z", 50)],
    );
    assert_eq!(
        scanner(LogSource::Claude, real.path()).scan(now()).records,
        1
    );

    let links = tempfile::tempdir().unwrap();
    let link = links.path().join("projects");
    std::os::unix::fs::symlink(real.path(), &link).unwrap();
    let through_link = scanner(LogSource::Claude, &link).scan(now());
    assert_eq!(through_link.records, 0);
    assert!(through_link.incomplete);
}
