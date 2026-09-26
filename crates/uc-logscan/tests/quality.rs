use std::path::Path;

use chrono::FixedOffset;
use serde_json::{Value, json};
use uc_logscan::quality::{QualityCounts, QualityQuery, QualityRow, QualityStore};
use uc_logscan::{LogScanner, LogSource, ScanOptions};

fn write_lines(path: &Path, lines: &[Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(path, text).unwrap();
}

fn scanner(source: LogSource, roots: Vec<std::path::PathBuf>) -> LogScanner {
    LogScanner::new(
        source,
        ScanOptions {
            roots,
            timezone: FixedOffset::east_opt(0),
            ..ScanOptions::default()
        },
    )
}

fn user(at: &str, content: Value, extra: Value) -> Value {
    let mut record = json!({"type":"user","timestamp":at,"cwd":"Z:\\missing\\demo-project","promptSource":"sdk","message":{"role":"user","content":content}});
    if let Value::Object(fields) = extra {
        for (key, value) in fields {
            record[key] = value;
        }
    }
    record
}

fn assistant(at: &str, model: &str, id: &str, output: u64, tools: Value) -> Value {
    json!({"type":"assistant","timestamp":at,"cwd":"Z:\\missing\\demo-project","message":{"id":id,"model":model,"role":"assistant","usage":{"input_tokens":10,"output_tokens":output},"content":tools}})
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"type":"tool_use","id":id,"name":name,"input":input})
}

fn tool_result(at: &str, id: &str, text: &str, is_error: bool, extra: Value) -> Value {
    user(
        at,
        json!([{"type":"tool_result","tool_use_id":id,"content":text,"is_error":is_error}]),
        extra,
    )
}

fn claude_session() -> Vec<Value> {
    vec![
        user(
            "2026-09-01T10:00:00Z",
            json!("fix the failing parser"),
            json!({}),
        ),
        assistant(
            "2026-09-01T10:00:05Z",
            "claude-opus-5-5",
            "m1",
            100,
            json!([tool_use("t1", "Edit", json!({"file_path":"a.rs"}))]),
        ),
        tool_result(
            "2026-09-01T10:00:06Z",
            "t1",
            "The file a.rs has been updated.",
            false,
            json!({}),
        ),
        assistant(
            "2026-09-01T10:00:07Z",
            "claude-opus-5-5",
            "m1",
            120,
            json!([tool_use(
                "t2",
                "Bash",
                json!({"command":"cargo test 2>&1 | tail -3"})
            )]),
        ),
        tool_result(
            "2026-09-01T10:00:20Z",
            "t2",
            "test result: FAILED. 1 passed; 1 failed",
            false,
            json!({}),
        ),
        assistant(
            "2026-09-01T10:00:30Z",
            "claude-opus-5-5",
            "m2",
            50,
            json!([tool_use("t3", "Bash", json!({"command":"cargo test"}))]),
        ),
        tool_result(
            "2026-09-01T10:01:00Z",
            "t3",
            "test result: ok. 2 passed; 0 failed",
            false,
            json!({}),
        ),
        user(
            "2026-09-02T09:00:00Z",
            json!("also rename the helper"),
            json!({}),
        ),
        assistant(
            "2026-09-02T09:00:04Z",
            "claude-opus-5-5",
            "m3",
            40,
            json!([tool_use("t4", "Edit", json!({"file_path":"b.rs"}))]),
        ),
        tool_result(
            "2026-09-02T09:00:05Z",
            "t4",
            "<tool_use_error>String to replace not found in file.</tool_use_error>",
            true,
            json!({}),
        ),
        user(
            "2026-09-02T09:00:06Z",
            json!([{"type":"text","text":"[Request interrupted by user for tool use]"}]),
            json!({}),
        ),
        user(
            "2026-09-02T09:00:07Z",
            json!("<task-notification>done</task-notification>"),
            json!({"promptSource":"system"}),
        ),
        user(
            "2026-09-02T09:00:08Z",
            json!("reminder"),
            json!({"isMeta":true}),
        ),
        user(
            "2026-09-02T09:01:00Z",
            json!("[From Codex · demo · review] Goal: lint"),
            json!({}),
        ),
        assistant(
            "2026-09-02T09:01:03Z",
            "claude-fable-5-1",
            "m4",
            30,
            json!([tool_use("t5", "Bash", json!({"command":"pnpm lint"}))]),
        ),
        tool_result(
            "2026-09-02T09:01:04Z",
            "t5",
            "The user doesn't want to proceed with this tool use.",
            true,
            json!({"toolDenialKind":"user-rejected"}),
        ),
        assistant(
            "2026-09-02T09:01:05Z",
            "claude-fable-5-1",
            "m5",
            10,
            json!([tool_use("t6", "Bash", json!({"command":"git status"}))]),
        ),
        tool_result(
            "2026-09-02T09:01:06Z",
            "t6",
            "Exit code 128\nfatal: not a git repository",
            true,
            json!({}),
        ),
    ]
}

fn claude_subagent() -> Vec<Value> {
    vec![
        user("2026-09-01T11:00:00Z", json!("Check the types"), json!({})),
        assistant(
            "2026-09-01T11:00:02Z",
            "claude-sonnet-5",
            "s1",
            5,
            json!([tool_use(
                "u1",
                "PowerShell",
                json!({"command":"cd web && npx tsc --noEmit"})
            )]),
        ),
        tool_result("2026-09-01T11:00:30Z", "u1", "", false, json!({})),
    ]
}

fn codex_legacy() -> Vec<Value> {
    let event =
        |at: &str, payload: Value| json!({"timestamp":at,"type":"event_msg","payload":payload});
    let item =
        |at: &str, payload: Value| json!({"timestamp":at,"type":"response_item","payload":payload});
    vec![
        json!({"timestamp":"2026-07-01T08:00:00Z","type":"session_meta","payload":{"id":"s","cwd":"Z:\\missing\\codex-project","thread_source":"user","source":"vscode"}}),
        event(
            "2026-07-01T08:00:01Z",
            json!({"type":"task_started","turn_id":"a"}),
        ),
        json!({"timestamp":"2026-07-01T08:00:01Z","type":"turn_context","payload":{"turn_id":"a","cwd":"Z:\\missing\\codex-project","model":"gpt-5.6-sol"}}),
        event(
            "2026-07-01T08:00:02Z",
            json!({"type":"user_message","message":"please fix the tests"}),
        ),
        item(
            "2026-07-01T08:00:03Z",
            json!({"type":"function_call","name":"shell_command","call_id":"c1","arguments":"{\"command\":\"pnpm test\"}"}),
        ),
        item(
            "2026-07-01T08:00:09Z",
            json!({"type":"function_call_output","call_id":"c1","output":"Exit code: 1\nWall time: 5 seconds\nOutput:\n Tests  1 failed | 3 passed (4)"}),
        ),
        item(
            "2026-07-01T08:00:10Z",
            json!({"type":"custom_tool_call","name":"apply_patch","call_id":"c2","input":"*** Begin Patch"}),
        ),
        item(
            "2026-07-01T08:00:10Z",
            json!({"type":"custom_tool_call_output","call_id":"c2","output":"apply_patch verification failed: Failed to find expected lines"}),
        ),
        event(
            "2026-07-01T08:00:11Z",
            json!({"type":"patch_apply_end","call_id":"c3","success":true}),
        ),
        item(
            "2026-07-01T08:00:12Z",
            json!({"type":"function_call","name":"shell_command","call_id":"c4","arguments":"{\"command\":\"pnpm test\"}"}),
        ),
        item(
            "2026-07-01T08:00:18Z",
            json!({"type":"function_call_output","call_id":"c4","output":"Exit code: 0\nWall time: 5 seconds\nOutput:\n Tests  4 passed (4)"}),
        ),
        event(
            "2026-07-01T08:00:19Z",
            json!({"type":"token_count","info":{"last_token_usage":{"output_tokens":30}}}),
        ),
        event(
            "2026-07-01T08:00:20Z",
            json!({"type":"task_complete","turn_id":"a","duration_ms":19000}),
        ),
    ]
}

fn codex_items() -> Vec<Value> {
    let event =
        |at: &str, payload: Value| json!({"timestamp":at,"type":"event_msg","payload":payload});
    vec![
        json!({"timestamp":"2026-09-01T08:00:00Z","type":"session_meta","payload":{"id":"t","cwd":"Z:\\missing\\codex-project","thread_source":"user","source":"vscode"}}),
        event(
            "2026-09-01T08:00:01Z",
            json!({"type":"task_started","turn_id":"b"}),
        ),
        json!({"timestamp":"2026-09-01T08:00:01Z","type":"turn_context","payload":{"turn_id":"b","cwd":"Z:\\missing\\codex-project","model":"gpt-6-astra"}}),
        event(
            "2026-09-01T08:00:02Z",
            json!({"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"[From Claude Code · demo · handoff] Goal: fix"}]}}),
        ),
        event(
            "2026-09-01T08:00:05Z",
            json!({"type":"item_completed","item":{"type":"CommandExecution","command":["C:\\pwsh.exe","-Command","cargo test"],"exit_code":101,"status":"failed","aggregated_output":"test result: FAILED. 0 passed; 1 failed"}}),
        ),
        json!({"timestamp":"2026-09-01T08:00:06Z","type":"response_item","payload":{"type":"function_call","name":"shell_command","call_id":"d1","arguments":"{\"command\":\"cargo build\"}"}}),
        json!({"timestamp":"2026-09-01T08:00:07Z","type":"response_item","payload":{"type":"function_call_output","call_id":"d1","output":"Exit code: 0\nOutput:\nFinished `dev` profile"}}),
        event(
            "2026-09-01T08:00:08Z",
            json!({"type":"item_completed","item":{"type":"FileChange","status":"completed","changes":{}}}),
        ),
        json!({"timestamp":"2026-09-01T08:00:08Z","type":"response_item","payload":{"type":"custom_tool_call","name":"exec","call_id":"e1","input":"await tools.apply_patch(patch)"}}),
        json!({"timestamp":"2026-09-01T08:00:08Z","type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"e1","output":[{"type":"input_text","text":"Script failed
Wall time 0.2 seconds
Output:
"},{"type":"input_text","text":"Script error:
apply_patch verification failed: Failed to find expected lines in a.rs"}]}}),
        event(
            "2026-09-01T08:00:09Z",
            json!({"type":"turn_aborted","turn_id":"b","reason":"interrupted","duration_ms":8000}),
        ),
        event(
            "2026-09-01T09:00:00Z",
            json!({"type":"task_started","turn_id":"c"}),
        ),
        event(
            "2026-09-01T09:00:01Z",
            json!({"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"stop that and explain"}]}}),
        ),
        event(
            "2026-09-01T09:00:05Z",
            json!({"type":"turn_aborted","turn_id":"c","reason":"interrupted","duration_ms":4000}),
        ),
    ]
}

fn row<'a>(rows: &'a [QualityRow], source: LogSource, model: &str) -> &'a QualityCounts {
    &rows
        .iter()
        .find(|row| row.source == source && row.model == model)
        .unwrap_or_else(|| panic!("missing row for {model}"))
        .counts
}

fn fixture(root: &Path) -> (QualityStore, Vec<LogScanner>) {
    let claude = root.join("claude");
    let codex = root.join("codex");
    write_lines(
        &claude.join("Z--demo").join("session.jsonl"),
        &claude_session(),
    );
    write_lines(
        &claude
            .join("Z--demo")
            .join("session")
            .join("subagents")
            .join("agent-a.jsonl"),
        &claude_subagent(),
    );
    write_lines(
        &claude.join("Z--demo").join("memory").join("ignored.jsonl"),
        &claude_session(),
    );
    write_lines(
        &codex
            .join("2026")
            .join("07")
            .join("01")
            .join("rollout-a.jsonl"),
        &codex_legacy(),
    );
    write_lines(
        &codex
            .join("2026")
            .join("09")
            .join("01")
            .join("rollout-b.jsonl"),
        &codex_items(),
    );
    (
        QualityStore::open(root.join("cache").join("quality.json")),
        vec![
            scanner(LogSource::Claude, vec![claude]),
            scanner(LogSource::Codex, vec![codex]),
        ],
    )
}

#[test]
fn counts_turns_edits_commands_and_checks_per_model() {
    let temp = tempfile::tempdir().unwrap();
    let (store, scanners) = fixture(temp.path());
    let report = store.scan(&scanners).unwrap();
    assert_eq!(report.files_seen, 4);
    let summary = store.summary(&QualityQuery::default()).unwrap();
    assert!(
        summary
            .rows
            .iter()
            .all(|row| row.project == "demo-project" || row.project == "codex-project")
    );

    let opus = row(&summary.rows, LogSource::Claude, "claude-opus-5-5");
    assert_eq!(
        opus,
        &QualityCounts {
            turns: 2,
            human_turns: 2,
            interrupted_turns: 1,
            verified_turns: 1,
            green_turns: 1,
            check_runs: 2,
            failed_check_runs: 1,
            unknown_check_runs: 0,
            edits: 2,
            failed_edits: 1,
            denied_actions: 0,
            shell_commands: 2,
            failed_shell_commands: 0,
            output_tokens: 210,
            timed_turns: 2,
            turn_millis: 60_000 + 5_000,
        }
    );

    let fable = row(&summary.rows, LogSource::Claude, "claude-fable-5-1");
    assert_eq!(
        (fable.turns, fable.human_turns, fable.denied_actions),
        (1, 0, 1)
    );
    assert_eq!(
        (
            fable.shell_commands,
            fable.failed_shell_commands,
            fable.check_runs
        ),
        (1, 1, 0)
    );

    let sonnet = row(&summary.rows, LogSource::Claude, "claude-sonnet-5");
    assert_eq!(
        (
            sonnet.turns,
            sonnet.human_turns,
            sonnet.verified_turns,
            sonnet.green_turns
        ),
        (1, 0, 1, 1)
    );

    let sol = row(&summary.rows, LogSource::Codex, "gpt-5.6-sol");
    assert_eq!(
        (sol.turns, sol.human_turns, sol.interrupted_turns),
        (1, 1, 0)
    );
    assert_eq!((sol.edits, sol.failed_edits), (2, 1));
    assert_eq!((sol.shell_commands, sol.failed_shell_commands), (2, 1));
    assert_eq!(
        (
            sol.check_runs,
            sol.failed_check_runs,
            sol.verified_turns,
            sol.green_turns
        ),
        (2, 1, 1, 1)
    );
    assert_eq!(
        (sol.output_tokens, sol.timed_turns, sol.turn_millis),
        (30, 1, 19_000)
    );

    let astra = row(&summary.rows, LogSource::Codex, "gpt-6-astra");
    assert_eq!(
        (astra.turns, astra.human_turns, astra.interrupted_turns),
        (2, 1, 1)
    );
    assert_eq!(
        (
            astra.shell_commands,
            astra.failed_shell_commands,
            astra.check_runs
        ),
        (1, 1, 1)
    );
    assert_eq!(
        (
            astra.verified_turns,
            astra.green_turns,
            astra.edits,
            astra.failed_edits
        ),
        (1, 0, 2, 1)
    );

    assert_eq!(summary.info.files, 4);
    assert_eq!(summary.info.first_day.as_deref(), Some("2026-07-01"));
    assert_eq!(summary.info.last_day.as_deref(), Some("2026-09-02"));
}

#[test]
fn date_bounds_filter_by_the_day_a_turn_began() {
    let temp = tempfile::tempdir().unwrap();
    let (store, scanners) = fixture(temp.path());
    store.scan(&scanners).unwrap();
    let day = store
        .summary(&QualityQuery {
            from: Some("2026-09-02".into()),
            to: Some("2026-09-02".into()),
        })
        .unwrap();
    let opus = row(&day.rows, LogSource::Claude, "claude-opus-5-5");
    assert_eq!(
        (opus.turns, opus.edits, opus.failed_edits, opus.check_runs),
        (1, 1, 1, 0)
    );
    assert!(day.rows.iter().all(|row| row.source == LogSource::Claude));
    assert!(
        store
            .summary(&QualityQuery {
                from: Some("26/09/2026".into()),
                to: None
            })
            .is_err()
    );
}

#[test]
fn rescans_only_changed_transcripts_and_survives_reopening() {
    let temp = tempfile::tempdir().unwrap();
    let (store, scanners) = fixture(temp.path());
    assert_eq!(store.scan(&scanners).unwrap().files_parsed, 4);
    assert_eq!(store.scan(&scanners).unwrap().files_parsed, 0);
    let reopened = QualityStore::open(temp.path().join("cache").join("quality.json"));
    assert_eq!(reopened.scan(&scanners).unwrap().files_parsed, 0);
    let before = row(
        &reopened.summary(&QualityQuery::default()).unwrap().rows,
        LogSource::Claude,
        "claude-sonnet-5",
    )
    .clone();

    let path = temp
        .path()
        .join("claude")
        .join("Z--demo")
        .join("session")
        .join("subagents")
        .join("agent-a.jsonl");
    let mut lines = claude_subagent();
    lines.push(user(
        "2026-09-01T12:00:00Z",
        json!("One more check"),
        json!({}),
    ));
    lines.push(assistant(
        "2026-09-01T12:00:02Z",
        "claude-sonnet-5",
        "s2",
        5,
        json!([tool_use("u2", "Bash", json!({"command":"pnpm test"}))]),
    ));
    lines.push(tool_result(
        "2026-09-01T12:00:09Z",
        "u2",
        "Exit code 1\n Tests  2 failed | 1 passed (3)",
        true,
        json!({}),
    ));
    write_lines(&path, &lines);
    assert_eq!(reopened.scan(&scanners).unwrap().files_parsed, 1);
    let after = row(
        &reopened.summary(&QualityQuery::default()).unwrap().rows,
        LogSource::Claude,
        "claude-sonnet-5",
    )
    .clone();
    assert_eq!(after.turns, before.turns + 1);
    assert_eq!(
        (after.verified_turns, after.green_turns),
        (before.verified_turns + 1, before.green_turns)
    );

    std::fs::remove_file(&path).unwrap();
    reopened.scan(&scanners).unwrap();
    let rows = reopened.summary(&QualityQuery::default()).unwrap().rows;
    assert!(rows.iter().all(|row| row.model != "claude-sonnet-5"));
}
