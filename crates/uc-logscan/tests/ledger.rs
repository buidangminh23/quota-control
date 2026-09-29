use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::UNIX_EPOCH;

use chrono::FixedOffset;
use serde_json::{Value, json};
use uc_logscan::ledger::{UsageGrouping, UsageLedger, UsageQuery};
use uc_logscan::{LogScanner, LogSource, ScanOptions};

fn scanner(source: LogSource, root: &Path) -> LogScanner {
    LogScanner::new(
        source,
        ScanOptions {
            roots: vec![root.to_path_buf()],
            timezone: Some(FixedOffset::east_opt(0).unwrap()),
            ..ScanOptions::default()
        },
    )
}

fn claude(id: &str, date: &str, input: i64, cost: Option<f64>) -> Value {
    let mut value = json!({
        "type":"assistant", "timestamp":format!("{date}T12:00:00Z"),
        "cwd":"Z:/ledger-fixture/project-one",
        "message":{"id":id,"model":"unknown-model","usage":{
            "input_tokens":input,"output_tokens":2,
            "cache_read_input_tokens":3,"cache_creation_input_tokens":4
        }}
    });
    if let Some(cost) = cost {
        value["costUSD"] = json!(cost);
    }
    value
}

fn append(path: &Path, values: &[Value]) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    for value in values {
        writeln!(file, "{value}").unwrap();
    }
}

fn query(group_by: UsageGrouping) -> UsageQuery {
    UsageQuery {
        from: None,
        to: None,
        group_by,
    }
}

fn total(ledger: &UsageLedger) -> i64 {
    ledger
        .summary(&query(UsageGrouping::Day))
        .unwrap()
        .iter()
        .map(|row| row.totals.total_tokens)
        .sum()
}

#[test]
fn parser_upgrade_reimports_unchanged_forks_without_losing_missing_log_history() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("codex");
    fs::create_dir(&logs).unwrap();
    let count = |at: &str, input: i64, output: i64| {
        json!({
            "type":"event_msg","timestamp":at,"payload":{"type":"token_count","info":{
                "total_token_usage":{"input_tokens":input,"output_tokens":output,"total_tokens":input+output}
            }}
        })
    };
    append(
        &logs.join("child.jsonl"),
        &[
            json!({"type":"session_meta","timestamp":"2026-09-25T18:00:00Z","payload":{"id":"child","forked_from_id":"parent"}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            count("2026-09-25T17:00:00Z", 100, 20),
            json!({"type":"event_msg","timestamp":"2026-09-25T18:00:01Z","payload":{"type":"task_started","turn_id":"own"}}),
            count("2026-09-25T18:00:02Z", 600, 120),
        ],
    );
    let claude_logs = dir.path().join("claude");
    fs::create_dir(&claude_logs).unwrap();
    let old_path = claude_logs.join("old.jsonl");
    append(&old_path, &[claude("historic", "2024-01-01", 1, None)]);
    let scans = [
        scanner(LogSource::Codex, &logs),
        scanner(LogSource::Claude, &claude_logs),
    ];
    let db = dir.path().join("ledger.sqlite3");
    let ledger = UsageLedger::open(&db).unwrap();
    ledger.import(&scans, |_| {}).unwrap();
    assert_eq!(total(&ledger), 610);
    drop(ledger);
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("DELETE FROM ledger_metadata WHERE key='codexParserVersion'; DELETE FROM usage_events WHERE source='codex';").unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM file_checkpoints WHERE source='codex'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(conn);
    fs::remove_file(old_path).unwrap();
    let upgraded = UsageLedger::open(&db).unwrap();
    let report = upgraded.import(&scans, |_| {}).unwrap();
    assert_eq!(report.files_read, 1);
    assert_eq!(report.events_written, 1);
    assert_eq!(total(&upgraded), 610);
    drop(upgraded);
    let reopened = UsageLedger::open(db).unwrap();
    assert_eq!(reopened.import(&scans, |_| {}).unwrap().files_read, 0);
    assert_eq!(total(&reopened), 610);
}

#[test]
fn legacy_migration_reimports_equal_events_and_unifies_repository_names() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bot tele");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(
        root.join(".git/config"),
        "[remote \"origin\"]\nurl=https://example.test/team/bot-tele.git\n",
    )
    .unwrap();
    let logs = dir.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let mut event = claude("priced", "2024-01-01", 1000, None);
    event["cwd"] = json!(root);
    event["message"]["model"] = json!("claude-sonnet-5");
    append(&logs.join("claude.jsonl"), &[event]);
    let db = dir.path().join("usage.sqlite3");
    let scan = scanner(LogSource::Claude, &logs);
    let ledger = UsageLedger::open(&db).unwrap();
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    let before = total(&ledger);
    drop(ledger);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "ALTER TABLE usage_events DROP COLUMN details;
         ALTER TABLE usage_events DROP COLUMN derivation_version;
         UPDATE usage_events SET cost=123,project='bot tele';
         PRAGMA user_version=1;",
        )
        .unwrap();
    drop(connection);
    let ledger = UsageLedger::open(&db).unwrap();
    assert_eq!(total(&ledger), before);
    let report = ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    assert_eq!(report.files_read, 1);
    assert_eq!(report.events_written, 1);
    assert_eq!(total(&ledger), before);
    let rows = ledger.summary(&query(UsageGrouping::Project)).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, "bot-tele");
    assert!((rows[0].totals.cost_usd.unwrap() - 0.0020306).abs() < 1e-10);
    assert_eq!(ledger.import(&[scan], |_| {}).unwrap().files_read, 0);
    drop(ledger);
    assert!(UsageLedger::open(db).is_ok());
}

#[test]
fn price_revision_recomputes_deleted_logs_from_exact_billing_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("display folder");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(
        root.join(".git/config"),
        "[remote \"origin\"]\nurl=https://example.test/team/repository.git\n",
    )
    .unwrap();
    let logs = dir.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let mut event = claude("hourly", "2024-01-01", 1000, None);
    event["cwd"] = json!(root);
    event["message"]["model"] = json!("claude-opus-5");
    event["message"]["usage"]["speed"] = json!("fast");
    event["message"]["usage"]["cache_creation"] =
        json!({"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":200});
    append(
        &logs.join("claude.jsonl"),
        &[event, claude("reported", "2024-01-01", 1, Some(0.75))],
    );
    let db = dir.path().join("usage.sqlite3");
    let scan = scanner(LogSource::Claude, &logs);
    let ledger = UsageLedger::open(&db).unwrap();
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    let before = ledger.summary(&query(UsageGrouping::Model)).unwrap();
    assert!((before[0].totals.cost_usd.unwrap() - 0.015353).abs() < 1e-10);
    let before_tokens = total(&ledger);
    drop(ledger);
    fs::remove_dir_all(logs).unwrap();
    fs::remove_dir_all(root).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("UPDATE usage_events SET cost=NULL,project='stale'; UPDATE ledger_metadata SET value='outdated' WHERE key='derivationVersion';").unwrap();
    drop(connection);
    let ledger = UsageLedger::open(&db).unwrap();
    assert_eq!(ledger.import(&[scan], |_| {}).unwrap().files_read, 0);
    assert_eq!(
        ledger.summary(&query(UsageGrouping::Model)).unwrap(),
        before
    );
    assert_eq!(total(&ledger), before_tokens);
    assert!(
        ledger
            .summary(&query(UsageGrouping::Project))
            .unwrap()
            .iter()
            .any(|row| row.key == "repository")
    );
}

#[test]
fn both_sources_share_one_repository_key_and_small_aggregate_codex_is_priced() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bot tele");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(
        root.join(".git/config"),
        "[remote \"origin\"]\nurl=https://example.test/team/bot-tele.git\n",
    )
    .unwrap();
    let claude_logs = dir.path().join("claude");
    let codex_logs = dir.path().join("codex");
    fs::create_dir(&claude_logs).unwrap();
    fs::create_dir(&codex_logs).unwrap();
    let mut event = claude("local", "2024-01-01", 1000, None);
    event["cwd"] = json!(root);
    append(&claude_logs.join("session.jsonl"), &[event]);
    append(
        &codex_logs.join("session.jsonl"),
        &[
            json!({"type":"session_meta","payload":{"id":"fixture","cwd":"Z:/missing/bot tele","git":{"repository_url":"https://example.test/team/bot-tele.git"}}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}),
            codex_count("2024-01-01T12:00:00Z", 1000, 10),
        ],
    );
    let ledger = UsageLedger::open(dir.path().join("usage.sqlite3")).unwrap();
    ledger
        .import(
            &[
                scanner(LogSource::Claude, &claude_logs),
                scanner(LogSource::Codex, &codex_logs),
            ],
            |_| {},
        )
        .unwrap();
    let rows = ledger.summary(&query(UsageGrouping::Project)).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.key == "bot-tele"));
    assert!(
        rows.iter()
            .find(|row| row.source == "codex")
            .unwrap()
            .totals
            .cost_usd
            .is_some()
    );
}

#[test]
fn historical_folder_aliases_follow_one_verified_repository_but_not_ambiguous_names() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("current/bot tele");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(
        root.join(".git/config"),
        "[remote \"origin\"]\nurl=https://example.test/team/bot-tele.git\n",
    )
    .unwrap();
    let logs = dir.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let mut known = claude("known", "2024-01-01", 1, None);
    known["cwd"] = json!(root);
    let mut historical = claude("historical", "2024-01-01", 2, None);
    historical["cwd"] = json!(dir.path().join("removed/bot tele"));
    append(&logs.join("first.jsonl"), &[known, historical.clone()]);
    let db = dir.path().join("usage.sqlite3");
    let scan = scanner(LogSource::Claude, &logs);
    let ledger = UsageLedger::open(&db).unwrap();
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    let rows = ledger.summary(&query(UsageGrouping::Project)).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, "bot-tele");
    historical["message"]["id"] = json!("appended");
    append(&logs.join("first.jsonl"), &[historical]);
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    assert_eq!(
        ledger
            .summary(&query(UsageGrouping::Project))
            .unwrap()
            .len(),
        1
    );
    let other_root = dir.path().join("other/bot-tele");
    fs::create_dir_all(other_root.join(".git")).unwrap();
    fs::write(
        other_root.join(".git/config"),
        "[remote \"origin\"]\nurl=https://example.test/team/other-repository.git\n",
    )
    .unwrap();
    let mut other = claude("alias-target", "2024-01-01", 1, None);
    other["cwd"] = json!(other_root);
    append(&logs.join("other.jsonl"), &[other]);
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    let rows = ledger.summary(&query(UsageGrouping::Project)).unwrap();
    assert_eq!(
        rows.iter()
            .find(|row| row.key == "bot-tele")
            .unwrap()
            .totals
            .total_tokens,
        32
    );
    assert!(rows.iter().any(|row| row.key == "other-repository"));
    for (parent, remote) in [("one", "repository-one"), ("two", "repository-two")] {
        let cwd = dir.path().join(parent).join("ambiguous");
        fs::create_dir_all(cwd.join(".git")).unwrap();
        fs::write(
            cwd.join(".git/config"),
            format!("[remote \"origin\"]\nurl=https://example.test/team/{remote}.git\n"),
        )
        .unwrap();
        let mut event = claude(parent, "2024-01-01", 1, None);
        event["cwd"] = json!(cwd);
        append(&logs.join("ambiguity.jsonl"), &[event]);
    }
    let mut missing = claude("missing", "2024-01-01", 1, None);
    missing["cwd"] = json!(dir.path().join("removed/ambiguous"));
    append(&logs.join("ambiguity.jsonl"), &[missing]);
    ledger.import(&[scan], |_| {}).unwrap();
    let rows = ledger.summary(&query(UsageGrouping::Project)).unwrap();
    assert!(rows.iter().any(|row| row.key == "ambiguous"));
    assert!(rows.iter().any(|row| row.key == "repository-one"));
    assert!(rows.iter().any(|row| row.key == "repository-two"));
}

#[test]
fn all_groupings_inclusive_dates_and_cost_wire_shape() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let mut second = claude("b", "2024-02-01", 20, Some(0.2));
    second["cwd"] = json!("Z:/ledger-fixture/project-two");
    second["message"]["model"] = json!("other-unknown");
    append(
        &logs.join("all.jsonl"),
        &[
            claude("a", "2023-12-31", 10, Some(0.1)),
            claude("c", "2024-01-31", 30, None),
            second,
        ],
    );
    let ledger = UsageLedger::open(dir.path().join("usage.sqlite3")).unwrap();
    let report = ledger
        .import(&[scanner(LogSource::Claude, &logs)], |_| {})
        .unwrap();
    assert_eq!(report.events_written, 3);
    let day = ledger.summary(&query(UsageGrouping::Day)).unwrap();
    assert_eq!(
        day.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(),
        ["2023-12-31", "2024-01-31", "2024-02-01"]
    );
    assert_eq!(day[0].totals.input_tokens, 17);
    assert_eq!(day[0].totals.output_tokens, 2);
    assert_eq!(day[0].totals.cached_input_tokens, 3);
    assert_eq!(day[0].totals.cache_creation_input_tokens, 4);
    assert_eq!(day[0].totals.total_tokens, 19);
    assert_eq!(day[0].totals.cost_usd, Some(0.1));
    assert_eq!(day[1].totals.cost_usd, None);
    let wire = serde_json::to_value(&day).unwrap();
    assert_eq!(wire[0]["totals"]["costUSD"], json!(0.1));
    assert!(wire[1]["totals"].get("costUSD").is_none());
    let month = ledger.summary(&query(UsageGrouping::Month)).unwrap();
    assert_eq!(
        month.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(),
        ["2023-12", "2024-01", "2024-02"]
    );
    let year = ledger.summary(&query(UsageGrouping::Year)).unwrap();
    assert_eq!(year.len(), 2);
    assert_eq!(year[1].totals.total_tokens, 68);
    assert_eq!(year[1].totals.cost_usd, Some(0.2));
    let model = ledger.summary(&query(UsageGrouping::Model)).unwrap();
    assert_eq!(model.len(), 2);
    assert_eq!(model[1].key, "unknown-model");
    assert_eq!(model[1].totals.total_tokens, 58);
    let project = ledger.summary(&query(UsageGrouping::Project)).unwrap();
    assert_eq!(
        project
            .iter()
            .map(|row| row.key.as_str())
            .collect::<Vec<_>>(),
        ["project-one", "project-two"]
    );
    let range = UsageQuery {
        from: Some("2024-01-31".into()),
        to: Some("2024-02-01".into()),
        group_by: UsageGrouping::Day,
    };
    assert_eq!(ledger.summary(&range).unwrap().len(), 2);
    let empty = UsageQuery {
        from: Some("2020-01-01".into()),
        to: Some("2020-12-31".into()),
        group_by: UsageGrouping::Day,
    };
    assert!(ledger.summary(&empty).unwrap().is_empty());
    assert_eq!(
        ledger.info().unwrap().first_day.as_deref(),
        Some("2023-12-31")
    );
}

#[test]
fn preferred_duplicates_replace_totals_and_deleted_files_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let db = dir.path().join("usage.sqlite3");
    let path = logs.join("first.jsonl");
    let mut sidechain = claude("same", "2024-01-01", 100, Some(3.0));
    sidechain["isSidechain"] = json!(true);
    append(&path, &[sidechain]);
    let scan = scanner(LogSource::Claude, &logs);
    let ledger = UsageLedger::open(&db).unwrap();
    assert_eq!(
        ledger
            .import(std::slice::from_ref(&scan), |_| {})
            .unwrap()
            .events_written,
        1
    );
    assert_eq!(total(&ledger), 109);
    let main = claude("same", "2024-01-01", 5, Some(0.5));
    append(&logs.join("main.jsonl"), &[main.clone(), main]);
    assert_eq!(
        ledger
            .import(std::slice::from_ref(&scan), |_| {})
            .unwrap()
            .events_written,
        1
    );
    assert_eq!(total(&ledger), 14);
    append(
        &logs.join("larger.jsonl"),
        &[claude("same", "2024-01-01", 15, Some(0.75))],
    );
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    assert_eq!(total(&ledger), 24);
    assert_eq!(
        ledger.summary(&query(UsageGrouping::Day)).unwrap()[0]
            .totals
            .cost_usd,
        Some(0.75)
    );
    fs::remove_dir_all(&logs).unwrap();
    drop(ledger);
    let reopened = UsageLedger::open(&db).unwrap();
    assert!(!reopened.info().unwrap().importing);
    assert_eq!(reopened.import(&[scan], |_| {}).unwrap().files_read, 0);
    assert_eq!(total(&reopened), 24);
}

#[test]
fn unchanged_append_new_rewrite_truncate_and_partial_tail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let scan = scanner(LogSource::Claude, dir.path());
    let ledger = UsageLedger::open(dir.path().join("usage.sqlite3")).unwrap();
    append(&path, &[claude("a", "2024-01-01", 1, None)]);
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    assert_eq!(
        ledger
            .import(std::slice::from_ref(&scan), |_| {})
            .unwrap()
            .bytes_read,
        0
    );
    let added = claude("b", "2024-01-01", 2, None);
    let bytes = format!("{added}\n").into_bytes();
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    assert_eq!(
        ledger
            .import(std::slice::from_ref(&scan), |_| {})
            .unwrap()
            .bytes_read,
        bytes.len() as u64
    );
    assert_eq!(total(&ledger), 21);
    append(
        &dir.path().join("new.jsonl"),
        &[claude("c", "2024-01-01", 3, None)],
    );
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    assert_eq!(total(&ledger), 33);
    let rewrite = format!("{}\n", claude("d", "2024-01-01", 4, None));
    fs::write(&path, &rewrite).unwrap();
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    assert_eq!(total(&ledger), 46);
    fs::write(&path, "").unwrap();
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    assert_eq!(total(&ledger), 46);
    let partial = format!("{}\n", claude("e", "2024-01-01", 5, None));
    let cut = partial.len() / 2;
    fs::write(&path, &partial[..cut]).unwrap();
    assert_eq!(
        ledger
            .import(std::slice::from_ref(&scan), |_| {})
            .unwrap()
            .events_written,
        0
    );
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&partial.as_bytes()[cut..])
        .unwrap();
    assert_eq!(ledger.import(&[scan], |_| {}).unwrap().events_written, 1);
    assert_eq!(total(&ledger), 60);
}

fn codex_count(timestamp: &str, input: i64, output: i64) -> Value {
    json!({"type":"event_msg","timestamp":timestamp,"payload":{"type":"token_count","info":{
        "total_token_usage":{"input_tokens":input,"output_tokens":output,"cached_input_tokens":2,"total_tokens":input+output}
    }}})
}

#[test]
fn codex_checkpoint_restores_cumulative_counters_and_sources_stay_separate() {
    let dir = tempfile::tempdir().unwrap();
    let codex = dir.path().join("codex");
    fs::create_dir(&codex).unwrap();
    let claude_root = dir.path().join("claude");
    fs::create_dir(&claude_root).unwrap();
    let path = codex.join("session.jsonl");
    append(
        &path,
        &[
            json!({"type":"session_meta","timestamp":"2024-01-01T10:00:00Z","payload":{"id":"fixture-session","cwd":"Z:/ledger-fixture/project-one"}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
            codex_count("2024-01-01T12:00:00Z", 10, 5),
        ],
    );
    append(
        &claude_root.join("session.jsonl"),
        &[claude("other", "2024-01-01", 1, None)],
    );
    let db = dir.path().join("usage.sqlite3");
    let scans = [
        scanner(LogSource::Codex, &codex),
        scanner(LogSource::Claude, &claude_root),
    ];
    let ledger = UsageLedger::open(&db).unwrap();
    ledger.import(&scans, |_| {}).unwrap();
    drop(ledger);
    append(&path, &[codex_count("2024-01-01T13:00:00Z", 30, 9)]);
    let reopened = UsageLedger::open(&db).unwrap();
    let report = reopened.import(&scans, |_| {}).unwrap();
    assert_eq!(report.events_written, 1);
    let rows = reopened.summary(&query(UsageGrouping::Day)).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].source, "claude");
    assert_eq!(rows[0].totals.total_tokens, 10);
    assert_eq!(rows[1].source, "codex");
    assert_eq!(rows[1].totals.total_tokens, 39);
    assert!(rows[1].totals.cost_usd.is_some());
}

#[test]
fn incomplete_checkpoint_with_unchanged_metadata_resumes_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let db = dir.path().join("usage.sqlite3");
    let scan = scanner(LogSource::Claude, dir.path());
    append(&path, &[claude("a", "2024-01-01", 1, None)]);
    let ledger = UsageLedger::open(&db).unwrap();
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    drop(ledger);
    append(&path, &[claude("b", "2024-01-01", 2, None)]);
    let metadata = fs::metadata(&path).unwrap();
    let modified = metadata
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute(
            "UPDATE file_checkpoints SET length=?1,modified=?2,complete=0",
            rusqlite::params![metadata.len() as i64, modified],
        )
        .unwrap();
    drop(connection);
    let ledger = UsageLedger::open(&db).unwrap();
    let report = ledger.import(&[scan], |_| {}).unwrap();
    assert_eq!(report.events_written, 1);
    assert!(report.bytes_read < metadata.len());
    assert_eq!(total(&ledger), 21);
}

#[test]
fn oversized_malformed_and_future_records_are_safe() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut scan = scanner(LogSource::Claude, dir.path());
    scan.options.max_line_bytes = 1024;
    fs::write(&path, format!("{}\nnot-json\n", "x".repeat(10_000))).unwrap();
    append(
        &path,
        &[
            claude("a", "2024-01-01", 1, None),
            claude("future", "2999-01-01", 100, Some(1.0)),
        ],
    );
    let ledger = UsageLedger::open(dir.path().join("usage.sqlite3")).unwrap();
    let mut states = Vec::new();
    let report = ledger.import(&[scan], |info| states.push(info)).unwrap();
    assert_eq!(report.skipped_lines, 2);
    assert_eq!(total(&ledger), 10);
    assert!(states.first().unwrap().importing);
    assert!(!states.last().unwrap().importing);
    assert!(states.last().unwrap().updated_at.is_some());
    let rows = ledger
        .summary(&UsageQuery {
            from: None,
            to: Some("2999-12-31".into()),
            group_by: UsageGrouping::Year,
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, "2024");
}

#[test]
fn strict_dates_local_day_and_empty_database() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = UsageLedger::open(dir.path().join("usage.sqlite3")).unwrap();
    assert!(ledger.info().unwrap().first_day.is_none());
    assert!(
        ledger
            .summary(&query(UsageGrouping::Day))
            .unwrap()
            .is_empty()
    );
    for invalid in ["2024-1-1", "bad", "2024-02-30"] {
        assert!(
            ledger
                .summary(&UsageQuery {
                    from: Some(invalid.into()),
                    ..query(UsageGrouping::Day)
                })
                .is_err()
        );
    }
    assert!(
        ledger
            .summary(&UsageQuery {
                from: Some("2024-02-01".into()),
                to: Some("2024-01-01".into()),
                group_by: UsageGrouping::Day
            })
            .is_err()
    );
    let mut event = claude("local", "2024-01-01", 1, None);
    event["timestamp"] = json!("2024-01-01T23:00:00Z");
    append(&dir.path().join("session.jsonl"), &[event]);
    let mut scan = scanner(LogSource::Claude, dir.path());
    scan.options.timezone = FixedOffset::east_opt(7 * 3600);
    ledger.import(&[scan], |_| {}).unwrap();
    assert_eq!(
        ledger.summary(&query(UsageGrouping::Day)).unwrap()[0].key,
        "2024-01-02"
    );
}

#[test]
fn failed_middle_batch_rolls_back_events_and_checkpoint_then_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let db = dir.path().join("usage.sqlite3");
    let mut file = fs::File::create(&path).unwrap();
    for index in 0..1200 {
        writeln!(
            file,
            "{}",
            claude(&format!("entry-{index}"), "2024-01-01", 1, None)
        )
        .unwrap();
    }
    drop(file);
    let scan = scanner(LogSource::Claude, dir.path());
    let ledger = UsageLedger::open(&db).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_batch BEFORE INSERT ON usage_events WHEN NEW.event_key='claude:entry-700' BEGIN SELECT RAISE(ABORT,'fixture interruption'); END;").unwrap();
    assert!(ledger.import(std::slice::from_ref(&scan), |_| {}).is_err());
    assert!(!ledger.info().unwrap().importing);
    assert_eq!(total(&ledger), 5120);
    assert_eq!(
        connection
            .query_row("SELECT complete FROM file_checkpoints", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    connection.execute_batch("DROP TRIGGER fail_batch").unwrap();
    drop(connection);
    drop(ledger);
    let reopened = UsageLedger::open(&db).unwrap();
    let report = reopened
        .import(&[scan], |info| {
            assert_eq!(reopened.info().unwrap().importing, info.importing);
        })
        .unwrap();
    assert_eq!(report.events_written, 688);
    assert!(report.bytes_read < fs::metadata(&path).unwrap().len());
    assert_eq!(total(&reopened), 12000);
}

#[test]
fn same_size_and_growing_rewrites_reparse_instead_of_reusing_old_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let scan = scanner(LogSource::Claude, dir.path());
    let ledger = UsageLedger::open(dir.path().join("usage.sqlite3")).unwrap();
    append(&path, &[claude("a", "2024-01-01", 1, None)]);
    ledger.import(std::slice::from_ref(&scan), |_| {}).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, format!("{}\n", claude("b", "2024-01-01", 2, None))).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified + std::time::Duration::from_secs(1)))
        .unwrap();
    assert_eq!(
        ledger
            .import(std::slice::from_ref(&scan), |_| {})
            .unwrap()
            .events_written,
        1
    );
    assert_eq!(total(&ledger), 21);
    fs::write(
        &path,
        format!(
            "{}\n{}\n",
            claude("c", "2024-01-01", 3, None),
            claude("d", "2024-01-01", 4, None)
        ),
    )
    .unwrap();
    assert_eq!(ledger.import(&[scan], |_| {}).unwrap().events_written, 2);
    assert_eq!(total(&ledger), 46);
}
