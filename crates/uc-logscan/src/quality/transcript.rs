//! Reads one Claude Code or Codex transcript into per-day quality counts for each model and
//! project. A turn is one request: a prompt and the work that answers it. File edits, shell
//! commands and check runs count against the model that issued them, on the day their turn began.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde_json::Value;

use super::QualityCounts;
use super::checks::{CheckOutcome, check_outcome};
use crate::day;

const MAX_TURN_MILLIS: u64 = 4 * 60 * 60 * 1000;
const EDIT_TOOLS: &[&str] = &["Edit", "MultiEdit", "Write", "NotebookEdit"];
const SHELL_TOOLS: &[&str] = &["Bash", "PowerShell"];
const CODEX_SHELL_CALLS: &[&str] = &[
    "shell_command",
    "shell",
    "exec_command",
    "local_shell",
    "container.exec",
];
const AGENT_PROMPT_PREFIXES: &[&str] = &[
    "[From ",
    "<codex_delegation>",
    "Another Claude session sent a message",
    "<cross-session-message",
];
const SYSTEM_PROMPT_PREFIXES: &[&str] = &[
    "<local-command-",
    "<task-notification>",
    "<system-reminder>",
    "Caveat: The messages below",
    "This session is being continued",
    "<environment_context>",
    "<user_instructions>",
    "# AGENTS.md",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DayKey {
    pub day: NaiveDate,
    pub model: String,
    pub project: String,
    /// The name came from a folder that is not a checkout on this machine, so it may be a local
    /// alias of a repository (the ledger's rule); see `QualityStore::scan`.
    pub unresolved: bool,
}

/// A turn's project name and whether it could be an alias to resolve later.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProjectName {
    pub name: String,
    pub unresolved: bool,
}

pub(crate) struct Tally {
    pub rows: BTreeMap<DayKey, QualityCounts>,
    /// Working directories that are checkouts on this machine, for the repository aliases.
    pub local_cwds: BTreeSet<String>,
    offset: Option<FixedOffset>,
}

impl Tally {
    pub fn new(offset: Option<FixedOffset>) -> Self {
        Self {
            rows: BTreeMap::new(),
            local_cwds: BTreeSet::new(),
            offset,
        }
    }

    fn counts(
        &mut self,
        at: DateTime<Utc>,
        model: &str,
        project: &ProjectName,
    ) -> &mut QualityCounts {
        self.rows
            .entry(DayKey {
                day: day(at, self.offset),
                model: model.to_owned(),
                project: project.name.clone(),
                unresolved: project.unresolved,
            })
            .or_default()
    }

    fn close(&mut self, turn: Turn) {
        let Some(model) = turn.model.clone() else {
            return;
        };
        let counts = self.counts(turn.started, &model, &turn.project);
        counts.turns += 1;
        if turn.human {
            counts.human_turns += 1;
            if turn.interrupted {
                counts.interrupted_turns += 1;
            }
        }
        if let Some(green) = turn.last_check {
            counts.verified_turns += 1;
            if green {
                counts.green_turns += 1;
            }
        }
        counts.output_tokens = counts.output_tokens.saturating_add(turn.output_tokens);
        let millis = turn.millis.unwrap_or_else(|| {
            u64::try_from(
                turn.last_at
                    .signed_duration_since(turn.started)
                    .num_milliseconds(),
            )
            .unwrap_or(0)
        });
        if millis > 0 && millis <= MAX_TURN_MILLIS {
            counts.timed_turns += 1;
            counts.turn_millis += millis;
        }
    }
}

fn record_check(counts: &mut QualityCounts, outcome: CheckOutcome) {
    match outcome {
        CheckOutcome::Pass => counts.check_runs += 1,
        CheckOutcome::Fail => {
            counts.check_runs += 1;
            counts.failed_check_runs += 1;
        }
        CheckOutcome::Unknown => counts.unknown_check_runs += 1,
    }
}

struct Turn {
    started: DateTime<Utc>,
    last_at: DateTime<Utc>,
    human: bool,
    interrupted: bool,
    model: Option<String>,
    project: ProjectName,
    last_check: Option<bool>,
    output_tokens: u64,
    millis: Option<u64>,
}

impl Turn {
    fn new(started: DateTime<Utc>, human: bool, project: &ProjectName) -> Self {
        Self {
            started,
            last_at: started,
            human,
            interrupted: false,
            model: None,
            project: project.clone(),
            last_check: None,
            output_tokens: 0,
            millis: None,
        }
    }

    fn observe(&mut self, outcome: CheckOutcome) {
        match outcome {
            CheckOutcome::Pass => self.last_check = Some(true),
            CheckOutcome::Fail => self.last_check = Some(false),
            CheckOutcome::Unknown => {}
        }
    }
}

#[derive(Default)]
struct Project {
    cwd: String,
    repository: Option<String>,
    name: ProjectName,
}

impl Project {
    fn update(&mut self, cwd: Option<&str>, repository: Option<&str>, tally: &mut Tally) {
        let mut changed = false;
        if let Some(cwd) =
            cwd.filter(|value| value.len() <= 32768 && !value.chars().any(char::is_control))
            && self.cwd != cwd
        {
            self.cwd = cwd.to_owned();
            changed = true;
        }
        if let Some(repository) = repository
            && self.repository.as_deref() != Some(repository)
        {
            self.repository = Some(repository.to_owned());
            changed = true;
        }
        if changed {
            let name = crate::project::resolve(&self.cwd, self.repository.as_deref());
            let local = crate::project::local_root(&self.cwd).is_some();
            if local && !tally.local_cwds.contains(&self.cwd) {
                tally.local_cwds.insert(self.cwd.clone());
            }
            let unresolved = !local && name == crate::project::resolve(&self.cwd, None);
            self.name = ProjectName { name, unresolved };
        }
    }
}

fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value["timestamp"].as_str()?)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value[key]
        .as_str()
        .filter(|s| s.len() <= 512 && !s.trim().is_empty())
}

fn model(value: &Value) -> Option<String> {
    text(value, "model")
        .or_else(|| text(value, "model_name"))
        .filter(|name| {
            *name != "<synthetic>"
                && name.len() <= 160
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.:/".contains(c))
        })
        .map(str::to_owned)
}

fn joined_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| {
                block["text"]
                    .as_str()
                    .or_else(|| block["input_text"].as_str())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn agent_written(text: &str) -> bool {
    let text = text.trim_start();
    AGENT_PROMPT_PREFIXES
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

fn system_written(text: &str) -> bool {
    let text = text.trim_start();
    SYSTEM_PROMPT_PREFIXES
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

enum ClaudeTool {
    Edit,
    Shell(String),
}

struct PendingTool {
    tool: ClaudeTool,
    model: String,
}

/// A Claude Code session or subagent transcript.
pub(crate) struct ClaudeTranscript {
    subagent: bool,
    project: Project,
    turn: Option<Turn>,
    tools: HashMap<String, PendingTool>,
    message_tokens: HashMap<String, u64>,
}

impl ClaudeTranscript {
    pub fn new(path: &Path) -> Self {
        Self {
            subagent: path
                .components()
                .any(|part| part.as_os_str() == "subagents"),
            project: Project::default(),
            turn: None,
            tools: HashMap::new(),
            message_tokens: HashMap::new(),
        }
    }

    pub fn line(&mut self, root: &Value, tally: &mut Tally) {
        self.project.update(root["cwd"].as_str(), None, tally);
        let Some(at) = timestamp(root) else {
            return;
        };
        match root["type"].as_str() {
            Some("assistant") => self.assistant(root, at),
            Some("user") => self.user(root, at, tally),
            _ => {}
        }
    }

    pub fn finish(mut self, tally: &mut Tally) {
        self.close(tally);
    }

    fn close(&mut self, tally: &mut Tally) {
        if let Some(mut turn) = self.turn.take() {
            turn.output_tokens = self.message_tokens.drain().map(|(_, tokens)| tokens).sum();
            tally.close(turn);
        }
        self.message_tokens.clear();
    }

    fn assistant(&mut self, root: &Value, at: DateTime<Utc>) {
        let message = &root["message"];
        let Some(model) = model(message) else {
            return;
        };
        let turn = self
            .turn
            .get_or_insert_with(|| Turn::new(at, false, &self.project.name));
        turn.model = Some(model.clone());
        turn.last_at = turn.last_at.max(at);
        if let (Some(id), Some(tokens)) = (
            message["id"].as_str(),
            message["usage"]["output_tokens"].as_u64(),
        ) {
            let entry = self.message_tokens.entry(id.to_owned()).or_default();
            *entry = (*entry).max(tokens);
        }
        let Some(blocks) = message["content"].as_array() else {
            return;
        };
        for block in blocks {
            if block["type"] != "tool_use" {
                continue;
            }
            let (Some(id), Some(name)) = (block["id"].as_str(), block["name"].as_str()) else {
                continue;
            };
            let tool = if EDIT_TOOLS.contains(&name) {
                ClaudeTool::Edit
            } else if SHELL_TOOLS.contains(&name) {
                ClaudeTool::Shell(
                    block["input"]["command"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                )
            } else {
                continue;
            };
            self.tools.insert(
                id.to_owned(),
                PendingTool {
                    tool,
                    model: model.clone(),
                },
            );
        }
    }

    fn user(&mut self, root: &Value, at: DateTime<Utc>, tally: &mut Tally) {
        let content = &root["message"]["content"];
        if let Some(blocks) = content.as_array()
            && blocks.iter().any(|block| block["type"] == "tool_result")
        {
            for block in blocks.iter().filter(|block| block["type"] == "tool_result") {
                self.tool_result(root, block, at, tally);
            }
            if let Some(turn) = &mut self.turn {
                turn.last_at = turn.last_at.max(at);
            }
            return;
        }
        if root["isMeta"] == true
            || root["isCompactSummary"] == true
            || root["isVisibleInTranscriptOnly"] == true
        {
            return;
        }
        let prompt = joined_text(content);
        if prompt
            .trim_start()
            .starts_with("[Request interrupted by user")
        {
            if let Some(turn) = &mut self.turn
                && turn.human
            {
                turn.interrupted = true;
            }
            return;
        }
        let attachment = content.as_array().is_some_and(|blocks| {
            blocks
                .iter()
                .any(|block| block["type"] == "image" || block["type"] == "document")
        });
        if (prompt.trim().is_empty() && !attachment)
            || system_written(&prompt)
            || root["promptSource"] == "system"
        {
            return;
        }
        let origin = root["turnOrigin"]
            .as_str()
            .or_else(|| root["origin"]["kind"].as_str());
        let human =
            !self.subagent && !agent_written(&prompt) && origin.is_none_or(|kind| kind == "human");
        self.close(tally);
        self.turn = Some(Turn::new(at, human, &self.project.name));
    }

    fn tool_result(&mut self, root: &Value, block: &Value, at: DateTime<Utc>, tally: &mut Tally) {
        let Some(pending) = block["tool_use_id"]
            .as_str()
            .and_then(|id| self.tools.remove(id))
        else {
            return;
        };
        let output = joined_text(&block["content"]);
        let is_error = block["is_error"].as_bool().unwrap_or(false);
        let started = self.turn.as_ref().map_or(at, |turn| turn.started);
        let project = self
            .turn
            .as_ref()
            .map_or(self.project.name.clone(), |turn| turn.project.clone());
        let counts = tally.counts(started, &pending.model, &project);
        let denied = root["toolDenialKind"].is_string()
            || output.starts_with("The user doesn't want to proceed with this tool use")
            || (is_error
                && output.starts_with("Permission to use")
                && output.contains("has been denied"));
        if denied {
            counts.denied_actions += 1;
            return;
        }
        match pending.tool {
            ClaudeTool::Edit => {
                counts.edits += 1;
                if is_error {
                    counts.failed_edits += 1;
                }
            }
            ClaudeTool::Shell(command) => {
                if output.trim_start().starts_with("<tool_use_error>") {
                    return;
                }
                counts.shell_commands += 1;
                if is_error {
                    counts.failed_shell_commands += 1;
                }
                if let Some(outcome) = check_outcome(&command, &output, Some(!is_error)) {
                    record_check(counts, outcome);
                    if let Some(turn) = &mut self.turn {
                        turn.observe(outcome);
                    }
                }
            }
        }
    }
}

struct CommandResult {
    failed: bool,
    check: Option<CheckOutcome>,
}

enum CodexCall {
    Shell(String),
    Patch,
    Script,
}

/// A Codex rollout. Commands and edits appear as `item_completed` events in newer rollouts and
/// as `function_call_output` / `patch_apply_end` in older ones; a turn uses whichever it has, so
/// the two never count the same command twice. A patch that fails verification inside a code-mode
/// script reaches neither: it only shows in the script's error text, and is counted from there.
pub(crate) struct CodexTranscript {
    saw_meta: bool,
    child_gate: crate::parser::CodexChildGate,
    human_thread: bool,
    project: Project,
    model: Option<String>,
    turn: Option<Turn>,
    prompt_seen: bool,
    calls: HashMap<String, CodexCall>,
    item_commands: Vec<CommandResult>,
    legacy_commands: Vec<CommandResult>,
    item_edits: Vec<bool>,
    legacy_edits: Vec<bool>,
    script_edit_failures: u64,
}

impl CodexTranscript {
    pub fn new() -> Self {
        Self {
            saw_meta: false,
            child_gate: crate::parser::CodexChildGate::default(),
            human_thread: false,
            project: Project::default(),
            model: None,
            turn: None,
            prompt_seen: false,
            calls: HashMap::new(),
            item_commands: Vec::new(),
            legacy_commands: Vec::new(),
            item_edits: Vec::new(),
            legacy_edits: Vec::new(),
            script_edit_failures: 0,
        }
    }

    pub fn line(&mut self, root: &Value, tally: &mut Tally) {
        let payload = &root["payload"];
        let at = timestamp(root);
        if root["type"] == "session_meta" && !self.saw_meta {
            self.child_gate = crate::parser::CodexChildGate::from_meta(root);
        }
        self.child_gate.observe(root);
        if self.child_gate.waiting
            && root["type"] != "session_meta"
            && root["type"] != "turn_context"
        {
            return;
        }
        match root["type"].as_str() {
            Some("session_meta") => self.meta(payload, tally),
            Some("turn_context") => {
                self.project.update(payload["cwd"].as_str(), None, tally);
                if let Some(model) = model(payload) {
                    if let Some(turn) = &mut self.turn {
                        turn.model = Some(model.clone());
                        turn.project = self.project.name.clone();
                    }
                    self.model = Some(model);
                }
            }
            Some("event_msg") => self.event(payload, at, tally),
            Some("response_item") => self.response(payload),
            _ => {}
        }
    }

    pub fn finish(mut self, tally: &mut Tally) {
        self.close(tally);
    }

    fn meta(&mut self, payload: &Value, tally: &mut Tally) {
        if self.saw_meta {
            return;
        }
        self.saw_meta = true;
        self.project.update(
            payload["cwd"].as_str(),
            text(&payload["git"], "repository_url"),
            tally,
        );
        let present =
            |value: &Value| !value.is_null() && value.as_str().is_none_or(|s| !s.trim().is_empty());
        let child = present(&payload["forked_from_id"])
            || present(&payload["parent_thread_id"])
            || payload["thread_source"] == "subagent"
            || present(&payload["source"]["subagent"]);
        self.human_thread =
            payload["thread_source"] == "user" && payload["source"] != "exec" && !child;
    }

    fn event(&mut self, payload: &Value, at: Option<DateTime<Utc>>, tally: &mut Tally) {
        match payload["type"].as_str() {
            Some("task_started") => {
                self.close(tally);
                let Some(started) = at else {
                    return;
                };
                let mut turn = Turn::new(started, self.human_thread, &self.project.name);
                turn.model = self.model.clone();
                self.turn = Some(turn);
                self.prompt_seen = false;
            }
            Some("task_complete") | Some("turn_aborted") => {
                if let Some(turn) = &mut self.turn {
                    if let Some(at) = at {
                        turn.last_at = turn.last_at.max(at);
                    }
                    turn.millis = payload["duration_ms"].as_u64();
                    if payload["type"] == "turn_aborted"
                        && payload["reason"] == "interrupted"
                        && turn.human
                    {
                        turn.interrupted = true;
                    }
                }
                self.close(tally);
            }
            Some("user_message") => self.prompt(payload["message"].as_str().unwrap_or_default()),
            Some("patch_apply_end") => self.legacy_edits.push(payload["success"] != false),
            Some("token_count") => {
                if let (Some(turn), Some(tokens)) = (
                    &mut self.turn,
                    payload["info"]["last_token_usage"]["output_tokens"].as_u64(),
                ) {
                    turn.output_tokens = turn.output_tokens.saturating_add(tokens);
                }
            }
            Some("item_completed") => self.item(&payload["item"]),
            _ => {}
        }
    }

    fn item(&mut self, item: &Value) {
        match item["type"].as_str() {
            Some("CommandExecution") => {
                let command = item_command(&item["command"]);
                let exit_ok = item["exit_code"]
                    .as_i64()
                    .map(|code| code == 0)
                    .or_else(|| item["status"].as_str().map(|status| status == "completed"));
                let output = item["aggregated_output"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        format!(
                            "{}\n{}",
                            item["stdout"].as_str().unwrap_or_default(),
                            item["stderr"].as_str().unwrap_or_default()
                        )
                    });
                self.item_commands.push(CommandResult {
                    failed: exit_ok == Some(false),
                    check: check_outcome(&command, &output, exit_ok),
                });
            }
            Some("FileChange") => self.item_edits.push(item["status"] == "completed"),
            Some("UserMessage") => self.prompt(&joined_text(&item["content"])),
            _ => {}
        }
    }

    fn response(&mut self, payload: &Value) {
        match payload["type"].as_str() {
            Some("function_call") => {
                let (Some(name), Some(id)) =
                    (payload["name"].as_str(), payload["call_id"].as_str())
                else {
                    return;
                };
                if !CODEX_SHELL_CALLS.contains(&name) {
                    return;
                }
                let arguments: Value = payload["arguments"]
                    .as_str()
                    .and_then(|raw| serde_json::from_str(raw).ok())
                    .unwrap_or(Value::Null);
                let command = match &arguments["command"] {
                    Value::Null => item_command(&arguments["cmd"]),
                    other => item_command(other),
                };
                self.calls.insert(id.to_owned(), CodexCall::Shell(command));
            }
            Some("function_call_output") => {
                let Some(CodexCall::Shell(command)) = payload["call_id"]
                    .as_str()
                    .and_then(|id| self.calls.remove(id))
                else {
                    return;
                };
                let (exit_ok, output) = legacy_output(&payload["output"]);
                self.legacy_commands.push(CommandResult {
                    failed: exit_ok == Some(false),
                    check: check_outcome(&command, &output, exit_ok),
                });
            }
            Some("custom_tool_call") => {
                let call = match payload["name"].as_str() {
                    Some("apply_patch") => CodexCall::Patch,
                    Some("exec") => CodexCall::Script,
                    _ => return,
                };
                if let Some(id) = payload["call_id"].as_str() {
                    self.calls.insert(id.to_owned(), call);
                }
            }
            Some("custom_tool_call_output") => {
                let output = joined_text(&payload["output"]);
                match payload["call_id"]
                    .as_str()
                    .and_then(|id| self.calls.remove(id))
                {
                    Some(CodexCall::Patch)
                        if output.starts_with("apply_patch verification failed") =>
                    {
                        self.legacy_edits.push(false);
                    }
                    Some(CodexCall::Script)
                        if output.contains("apply_patch verification failed") =>
                    {
                        self.script_edit_failures += 1;
                    }
                    _ => {}
                }
            }
            Some("message") if payload["role"] == "user" => {
                self.prompt(&joined_text(&payload["content"]))
            }
            _ => {}
        }
    }

    fn prompt(&mut self, text: &str) {
        if self.prompt_seen || text.trim().is_empty() || system_written(text) {
            return;
        }
        self.prompt_seen = true;
        if agent_written(text)
            && let Some(turn) = &mut self.turn
        {
            turn.human = false;
        }
    }

    fn close(&mut self, tally: &mut Tally) {
        let commands = if self.item_commands.is_empty() {
            std::mem::take(&mut self.legacy_commands)
        } else {
            std::mem::take(&mut self.item_commands)
        };
        let edits = if self.item_edits.is_empty() {
            std::mem::take(&mut self.legacy_edits)
        } else {
            std::mem::take(&mut self.item_edits)
        };
        self.item_commands.clear();
        self.legacy_commands.clear();
        self.item_edits.clear();
        self.legacy_edits.clear();
        self.calls.clear();
        let script_edit_failures = std::mem::take(&mut self.script_edit_failures);
        let Some(mut turn) = self.turn.take() else {
            return;
        };
        if turn.model.is_none() {
            turn.model = self.model.clone();
        }
        if let Some(model) = turn.model.clone() {
            let counts = tally.counts(turn.started, &model, &turn.project);
            for applied in edits {
                counts.edits += 1;
                if !applied {
                    counts.failed_edits += 1;
                }
            }
            counts.edits += script_edit_failures;
            counts.failed_edits += script_edit_failures;
            for command in commands {
                counts.shell_commands += 1;
                if command.failed {
                    counts.failed_shell_commands += 1;
                }
                if let Some(outcome) = command.check {
                    record_check(counts, outcome);
                    turn.observe(outcome);
                }
            }
        }
        tally.close(turn);
    }
}

fn item_command(value: &Value) -> String {
    match value {
        Value::String(command) => command.clone(),
        Value::Array(parts) => {
            let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            let script = parts
                .iter()
                .position(|part| {
                    matches!(
                        part.to_ascii_lowercase().as_str(),
                        "-command" | "-c" | "-lc" | "/c"
                    )
                })
                .and_then(|index| parts.get(index + 1));
            script.map_or_else(|| parts.join(" "), |script| (*script).to_owned())
        }
        _ => String::new(),
    }
}

fn legacy_output(output: &Value) -> (Option<bool>, String) {
    let Some(raw) = output.as_str() else {
        return (None, String::new());
    };
    if let Some(rest) = raw.strip_prefix("Exit code: ") {
        let code: String = rest.chars().take_while(char::is_ascii_digit).collect();
        return (
            code.parse::<i64>().ok().map(|code| code == 0),
            raw.to_owned(),
        );
    }
    if let Ok(parsed) = serde_json::from_str::<Value>(raw)
        && parsed.is_object()
    {
        let exit_ok = parsed["metadata"]["exit_code"]
            .as_i64()
            .map(|code| code == 0);
        return (
            exit_ok,
            parsed["output"].as_str().unwrap_or_default().to_owned(),
        );
    }
    (None, raw.to_owned())
}
