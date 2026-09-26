use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uc_pricing::Tokens;

use crate::LogSource;

#[derive(Clone, Debug)]
pub(crate) struct Event {
    pub key: String,
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub project: String,
    pub project_cwd: String,
    pub project_repository: String,
    pub tokens: Tokens,
    pub total: i64,
    pub token_usage: Option<uc_core::TokenUsage>,
    pub cost: Option<f64>,
    pub request_boundaries_known: bool,
    pub(crate) sidechain: bool,
}

impl Event {
    pub fn preferred_over(&self, other: &Self) -> bool {
        if self.sidechain != other.sidechain {
            return !self.sidechain;
        }
        self.total > other.total
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Parser {
    source: LogSource,
    session: String,
    model: String,
    previous: Option<RawTokens>,
    line: usize,
    saw_meta: bool,
    child_gate: Option<i64>,
    fast: bool,
    cwd: String,
    repository: Option<String>,
    project: String,
}

impl Parser {
    pub fn new(source: LogSource, path: &Path) -> Self {
        Self {
            source,
            session: path.to_string_lossy().into_owned(),
            model: "Unattributed".into(),
            previous: None,
            line: 0,
            saw_meta: false,
            child_gate: None,
            fast: false,
            cwd: String::new(),
            repository: None,
            project: String::new(),
        }
    }

    pub fn parse(&mut self, root: &Value) -> Option<Event> {
        self.line += 1;
        match self.source {
            LogSource::Claude => self.claude(root),
            LogSource::Codex => self.codex(root),
        }
    }

    fn update_project(&mut self, cwd: Option<&str>, repository: Option<&str>) {
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
            self.project = crate::project::resolve(&self.cwd, self.repository.as_deref());
        }
    }

    fn claude(&mut self, root: &Value) -> Option<Event> {
        self.update_project(root["cwd"].as_str(), None);
        if root["type"] != "assistant" {
            return None;
        }
        let timestamp = timestamp(root)?;
        let message = &root["message"];
        let usage = message.get("usage")?.as_object()?;
        let usage = Value::Object(usage.clone());
        let input = count(&usage, &["input_tokens"]);
        let output = count(&usage, &["output_tokens"]);
        let creation = &usage["cache_creation"];
        let (cache_write, cache_write_hour) = if creation.is_object() {
            (
                count(creation, &["ephemeral_5m_input_tokens"]),
                count(creation, &["ephemeral_1h_input_tokens"]),
            )
        } else {
            (count(&usage, &["cache_creation_input_tokens"]), 0)
        };
        let tokens = Tokens {
            input,
            output,
            cache_write,
            cache_write_hour,
            cache_read: count(&usage, &["cache_read_input_tokens"]),
            fast: usage["speed"] == "fast",
        };
        if tokens.total() == 0 {
            return None;
        }
        let key = text(message, "id")
            .or_else(|| text(root, "uuid"))
            .map(|id| format!("claude:{id}"))
            .unwrap_or_else(|| format!("{}:{}", self.session, self.line));
        let model = model(message).unwrap_or_else(|| "Unattributed".into());
        let cost = root["costUSD"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0);
        let token_usage = (usage["input_tokens"].as_i64().is_some()
            && usage["output_tokens"].as_i64().is_some())
        .then_some(uc_core::TokenUsage {
            input_tokens: tokens.prompt(),
            output_tokens: tokens.output,
            cached_input_tokens: tokens.cache_read,
            cache_creation_input_tokens: tokens.cache_write.saturating_add(tokens.cache_write_hour),
        });
        Some(Event {
            key,
            timestamp,
            model,
            project: self.project.clone(),
            project_cwd: self.cwd.clone(),
            project_repository: self.project.clone(),
            tokens,
            total: tokens.total(),
            token_usage,
            cost,
            request_boundaries_known: true,
            sidechain: root["isSidechain"].as_bool().unwrap_or(false),
        })
    }

    fn codex(&mut self, root: &Value) -> Option<Event> {
        let payload = &root["payload"];
        if root["type"] == "session_meta" {
            self.update_project(
                payload["cwd"].as_str(),
                text(&payload["git"], "repository_url"),
            );
        }
        if root["type"] == "session_meta" && !self.saw_meta {
            self.saw_meta = true;
            if let Some(id) = text(payload, "id") {
                self.session = id.to_owned();
            }
            let present =
                |v: &Value| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty());
            let child = present(&payload["forked_from_id"])
                || present(&payload["parent_thread_id"])
                || payload["thread_source"] == "subagent"
                || present(&payload["source"]["subagent"]);
            if child {
                self.child_gate = Some(timestamp(root).map_or(i64::MAX, |t| t.timestamp()));
            }
            return None;
        }
        if root["type"] == "turn_context" {
            self.update_project(payload["cwd"].as_str(), None);
            if let Some(model) = model(payload) {
                self.model = model;
            }
            return None;
        }
        if root["type"] != "event_msg" {
            return None;
        }
        if payload["type"] == "thread_settings_applied" {
            if let Some(tier) = text(&payload["thread_settings"], "service_tier")
                .or_else(|| text(payload, "service_tier"))
            {
                self.fast = tier == "fast" || tier == "priority";
            }
            return None;
        }
        if payload["type"] == "task_started" {
            if let (Some(gate), Some(started)) = (self.child_gate, payload["started_at"].as_f64()) {
                let threshold = if gate == i64::MAX {
                    timestamp(root)?.timestamp()
                } else {
                    gate
                };
                if started >= threshold as f64 {
                    self.child_gate = None;
                }
            }
            return None;
        }
        if payload["type"] != "token_count" {
            return None;
        }
        let time = timestamp(root)?;
        let info = &payload["info"];
        let totals = info
            .get("total_token_usage")
            .filter(|v| v.is_object())
            .map(RawTokens::read);
        if self.child_gate.is_some() {
            if totals.is_some() {
                self.previous = totals;
            }
            return None;
        }
        if totals.is_some() && totals == self.previous {
            return None;
        }
        let last = info
            .get("last_token_usage")
            .filter(|v| v.is_object())
            .map(RawTokens::read);
        let usage = match totals {
            Some(total) => match self.previous {
                Some(previous)
                    if total.total < previous.total
                        || total.input < previous.input
                        || total.output < previous.output =>
                {
                    last.unwrap_or(total)
                }
                Some(previous) => total.subtract(previous),
                None => last.unwrap_or(total),
            },
            None => last?,
        };
        if totals.is_some() {
            self.previous = totals;
        }
        if usage.total == 0 {
            return None;
        }
        if let Some(name) = model(payload).or_else(|| model(info)) {
            self.model = name;
        }
        let cached = usage.cached.min(usage.input);
        let tokens = Tokens {
            input: usage.input - cached,
            cache_read: cached,
            output: usage.output,
            fast: self.fast,
            ..Tokens::default()
        };
        let key = format!(
            "codex:{}:{}:{:?}",
            self.session,
            time.to_rfc3339(),
            totals.unwrap_or(usage)
        );
        let token_usage = usage.complete.then_some(uc_core::TokenUsage {
            input_tokens: usage.input,
            output_tokens: usage.output,
            cached_input_tokens: cached,
            cache_creation_input_tokens: 0,
        });
        Some(Event {
            key,
            timestamp: time,
            model: self.model.clone(),
            project: self.project.clone(),
            project_cwd: self.cwd.clone(),
            project_repository: self.project.clone(),
            tokens,
            total: usage.total,
            token_usage,
            cost: None,
            request_boundaries_known: last.is_some_and(|v| v == usage),
            sidechain: false,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct RawTokens {
    input: i64,
    cached: i64,
    output: i64,
    total: i64,
    complete: bool,
}

impl RawTokens {
    fn read(value: &Value) -> Self {
        let input = count(value, &["input_tokens", "prompt_tokens", "input"]);
        let output = count(value, &["output_tokens", "completion_tokens", "output"]);
        let cached = count(
            value,
            &[
                "cached_input_tokens",
                "cache_read_input_tokens",
                "cached_tokens",
            ],
        );
        let total = value["total_tokens"]
            .as_i64()
            .filter(|v| *v >= 0)
            .unwrap_or_else(|| input.saturating_add(output));
        let complete = ["input_tokens", "prompt_tokens", "input"]
            .iter()
            .any(|k| value[*k].as_i64().is_some())
            && ["output_tokens", "completion_tokens", "output"]
                .iter()
                .any(|k| value[*k].as_i64().is_some());
        Self {
            input,
            output,
            cached,
            total,
            complete,
        }
    }

    fn subtract(self, previous: Self) -> Self {
        Self {
            input: self.input.saturating_sub(previous.input).max(0),
            cached: self.cached.saturating_sub(previous.cached).max(0),
            output: self.output.saturating_sub(previous.output).max(0),
            total: self.total.saturating_sub(previous.total).max(0),
            complete: self.complete && previous.complete,
        }
    }
}

fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value["timestamp"].as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn count(value: &Value, names: &[&str]) -> i64 {
    names
        .iter()
        .find_map(|name| value[*name].as_i64())
        .unwrap_or(0)
        .max(0)
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value[key]
        .as_str()
        .filter(|s| s.len() <= 512 && !s.trim().is_empty())
}

fn model(value: &Value) -> Option<String> {
    text(value, "model")
        .or_else(|| text(value, "model_name"))
        .or_else(|| text(&value["metadata"], "model"))
        .filter(|s| {
            s.len() <= 160
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.:/".contains(c))
        })
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn codex_count(input: i64, output: i64) -> Value {
        json!({"type":"event_msg","timestamp":"2026-09-26T12:00:01Z","payload":{
            "type":"token_count","info":{"total_token_usage":{
                "input_tokens":input,"output_tokens":output,"cached_input_tokens":0,"total_tokens":input+output
            }}
        }})
    }

    fn claude_message(id: Option<&str>, cwd: Option<&str>, sidechain: bool) -> Value {
        json!({"type":"assistant","timestamp":"2026-09-26T12:00:01Z","cwd":cwd,"isSidechain":sidechain,
            "message":{"id":id,"model":"claude-sonnet-4","usage":{"input_tokens":10,"output_tokens":5}}})
    }

    fn same_event(actual: &Event, expected: &Event) {
        assert_eq!(actual.key, expected.key);
        assert_eq!(actual.timestamp, expected.timestamp);
        assert_eq!(actual.project, expected.project);
        assert_eq!(actual.model, expected.model);
        assert_eq!(actual.tokens, expected.tokens);
        assert_eq!(actual.total, expected.total);
        assert_eq!(actual.token_usage, expected.token_usage);
        assert_eq!(actual.cost, expected.cost);
        assert_eq!(
            actual.request_boundaries_known,
            expected.request_boundaries_known
        );
        assert_eq!(actual.sidechain, expected.sidechain);
    }

    #[test]
    fn codex_project_follows_session_metadata_and_changed_turn_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("second-project");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let mut parser = Parser::new(LogSource::Codex, Path::new("fixture.jsonl"));
        parser.parse(&json!({"type":"session_meta","payload":{"id":"session","cwd":"Z:\\missing\\checkout","git":{"repository_url":"git@github.com:team/first-project.git"}}}));
        assert_eq!(
            parser.parse(&codex_count(10, 5)).unwrap().project,
            "first-project"
        );
        parser.parse(&json!({"type":"turn_context","payload":{"cwd":root.join("src").to_string_lossy(),"model":"gpt-5"}}));
        let event = parser.parse(&codex_count(20, 10)).unwrap();
        assert_eq!(event.project, "second-project");
        assert_eq!(event.model, "gpt-5");
        parser.parse(&json!({"type":"session_meta","payload":{"id":"ignored-duplicate","cwd":"Z:\\missing\\third","git":{"repository_url":"https://github.com/team/third-project.git"}}}));
        let event = parser.parse(&codex_count(30, 15)).unwrap();
        assert_eq!(event.project, "third-project");
        assert!(event.key.starts_with("codex:session:"));
    }

    #[test]
    fn claude_project_uses_each_record_cwd_and_last_known_directory() {
        let mut parser = Parser::new(LogSource::Claude, Path::new("fixture.jsonl"));
        parser.parse(&json!({"type":"user","cwd":"Z:\\missing\\First Project"}));
        assert_eq!(
            parser
                .parse(&claude_message(Some("first"), None, false))
                .unwrap()
                .project,
            "First Project"
        );
        assert_eq!(
            parser
                .parse(&claude_message(
                    Some("second"),
                    Some("/missing/second-project"),
                    false
                ))
                .unwrap()
                .project,
            "second-project"
        );
        let mut unknown = Parser::new(LogSource::Claude, Path::new("fixture.jsonl"));
        assert_eq!(
            unknown
                .parse(&claude_message(None, None, false))
                .unwrap()
                .project,
            ""
        );
    }

    #[test]
    fn codex_checkpoint_resumes_child_gate_totals_model_and_project() {
        let mut original = Parser::new(LogSource::Codex, Path::new("fixture.jsonl"));
        original.parse(&json!({"type":"session_meta","timestamp":"2026-09-26T12:00:00Z","payload":{"id":"child","parent_thread_id":"parent","cwd":"/missing/sample-project"}}));
        original.parse(&json!({"type":"turn_context","payload":{"model":"gpt-5"}}));
        original.parse(&json!({"type":"event_msg","payload":{"type":"thread_settings_applied","service_tier":"priority"}}));
        assert!(original.parse(&codex_count(100, 20)).is_none());
        let checkpoint = serde_json::to_vec(&original).unwrap();
        let mut resumed: Parser = serde_json::from_slice(&checkpoint).unwrap();
        for parser in [&mut original, &mut resumed] {
            assert!(parser.parse(&codex_count(120, 25)).is_none());
            parser.parse(&json!({"type":"event_msg","timestamp":"2026-09-26T12:00:01Z","payload":{"type":"task_started","started_at":1790424001_i64}}));
        }
        let expected = original.parse(&codex_count(140, 30)).unwrap();
        let actual = resumed.parse(&codex_count(140, 30)).unwrap();
        same_event(&actual, &expected);
        assert_eq!(actual.total, 25);
        assert!(actual.tokens.fast);
        assert_eq!(actual.project, "sample-project");
        assert!(resumed.parse(&codex_count(140, 30)).is_none());
    }

    #[test]
    fn claude_checkpoint_keeps_fallback_key_and_sidechain_preference() {
        let mut original = Parser::new(LogSource::Claude, Path::new("fixture.jsonl"));
        original.parse(&claude_message(
            None,
            Some("/missing/sample-project"),
            false,
        ));
        let mut resumed: Parser =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        let expected = original.parse(&claude_message(None, None, true)).unwrap();
        let actual = resumed.parse(&claude_message(None, None, true)).unwrap();
        same_event(&actual, &expected);
        assert_eq!(actual.key, "fixture.jsonl:2");
        assert_eq!(actual.project, "sample-project");
        let primary = original.parse(&claude_message(None, None, false)).unwrap();
        assert!(primary.preferred_over(&actual));
        assert!(!actual.preferred_over(&primary));
    }
}
