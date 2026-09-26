use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use serde::{Deserialize, de::IgnoredAny};

use crate::{LogSource, context::ContextWindowSession};

pub(crate) struct ContextParser {
    source: LogSource,
    session: String,
    cwd: String,
    repository: Option<String>,
    project: String,
    model: String,
    evidence: i64,
    window: Option<i64>,
    excluded: bool,
    latest: Option<Request>,
    previous_used: Option<i64>,
    base: Option<i64>,
    identities: VecDeque<String>,
    cumulative: Option<Usage>,
}

struct Request {
    identity: String,
    prompt: i64,
    used: i64,
    time: DateTime<Utc>,
    model: String,
    project: String,
    cwd: String,
    repository: Option<String>,
    window: Option<i64>,
    evidence: i64,
}

impl ContextParser {
    pub(crate) fn new(source: LogSource, fallback_session: String) -> Self {
        Self {
            source,
            session: fallback_session,
            cwd: String::new(),
            repository: None,
            project: String::new(),
            model: String::new(),
            evidence: 0,
            window: None,
            excluded: false,
            latest: None,
            previous_used: None,
            base: None,
            identities: VecDeque::new(),
            cumulative: None,
        }
    }

    pub(crate) fn parse(&mut self, line: &[u8]) {
        match self.source {
            LogSource::Claude => {
                if let Ok(record) = serde_json::from_slice::<ClaudeRecord>(line) {
                    self.claude(record);
                }
            }
            LogSource::Codex => {
                if let Ok(record) = serde_json::from_slice::<CodexRecord>(line) {
                    self.codex(record);
                }
            }
        }
    }

    pub(crate) fn repository_alias(&self) -> Option<(String, String)> {
        crate::project::repository_alias(
            self.latest.as_ref().map_or(&self.cwd, |latest| &latest.cwd),
        )
    }

    pub(crate) fn project_is_resolved(&self) -> bool {
        let (cwd, repository) = self
            .latest
            .as_ref()
            .map_or((&self.cwd, &self.repository), |latest| {
                (&latest.cwd, &latest.repository)
            });
        repository.is_some() || crate::project::local_root(cwd).is_some()
    }

    pub(crate) fn snapshot(&self, configured_model: Option<&str>) -> Option<ContextWindowSession> {
        if self.excluded {
            return None;
        }
        let latest = self.latest.as_ref()?;
        Some(ContextWindowSession {
            source: self.source,
            session_id: self.session.clone(),
            project: latest.project.clone(),
            model: latest.model.clone(),
            used_tokens: latest.used,
            window_tokens: match self.source {
                LogSource::Claude => crate::context_models::claude_window(
                    &latest.model,
                    latest.evidence,
                    configured_model,
                ),
                LogSource::Codex => latest.window,
            },
            base_tokens: self.base.unwrap_or(latest.prompt),
            last_turn_tokens: self
                .previous_used
                .map_or(0, |previous| latest.used.saturating_sub(previous).max(0)),
            updated_at: latest.time.to_rfc3339(),
        })
    }

    fn metadata(&mut self, cwd: Option<String>, repository: Option<String>) {
        let mut changed = false;
        if let Some(cwd) = cwd.filter(|value| safe(value, 32768))
            && self.cwd != cwd
        {
            self.cwd = cwd;
            changed = true;
        }
        if let Some(repository) = repository.filter(|value| safe(value, 4096))
            && self.repository.as_ref() != Some(&repository)
        {
            self.repository = Some(repository);
            changed = true;
        }
        if changed {
            self.project = crate::project::resolve(&self.cwd, self.repository.as_deref());
        }
    }

    fn set_model(&mut self, model: Option<String>) {
        if let Some(model) = model.filter(|value| safe(value, 256))
            && model != self.model
        {
            if !self.model.is_empty() {
                self.evidence = 0;
            }
            self.model = model;
            self.window = None;
        }
    }

    fn compact(&mut self) {
        self.base = None;
        self.previous_used = None;
        self.latest = None;
    }

    fn accept(&mut self, identity: String, prompt: i64, used: i64, time: DateTime<Utc>) {
        if let Some(latest) = &mut self.latest {
            if latest.identity == identity {
                if time >= latest.time {
                    latest.prompt = prompt;
                    latest.used = used;
                    latest.time = time;
                    latest.model = self.model.clone();
                    latest.project = self.project.clone();
                    latest.cwd = self.cwd.clone();
                    latest.repository = self.repository.clone();
                    latest.window = self.window;
                    latest.evidence = self.evidence;
                    if self.previous_used.is_none() {
                        self.base = Some(prompt);
                    }
                }
                return;
            }
            if time < latest.time {
                return;
            }
        }
        if self.identities.contains(&identity) {
            return;
        }
        self.previous_used = self.latest.as_ref().map(|request| request.used);
        self.base.get_or_insert(prompt);
        self.identities.push_back(identity.clone());
        while self.identities.len() > 256 {
            self.identities.pop_front();
        }
        self.latest = Some(Request {
            identity,
            prompt,
            used,
            time,
            model: self.model.clone(),
            project: self.project.clone(),
            cwd: self.cwd.clone(),
            repository: self.repository.clone(),
            window: self.window,
            evidence: self.evidence,
        });
    }

    fn claude(&mut self, record: ClaudeRecord) {
        if record.is_sidechain || record.agent_id.is_some() {
            return;
        }
        if let Some(session) = record.session_id.filter(|value| safe(value, 256)) {
            self.session = session;
        }
        self.metadata(record.cwd, None);
        if record.kind == "system" && record.subtype.as_deref() == Some("compact_boundary") {
            if let Some(metadata) = record.compact_metadata {
                self.evidence = self
                    .evidence
                    .max(metadata.pre_tokens.unwrap_or(0))
                    .max(metadata.post_tokens.unwrap_or(0));
            }
            self.compact();
            return;
        }
        if record.kind != "assistant" {
            return;
        }
        let Some(message) = record.message else {
            return;
        };
        let Some(usage) = message.usage else {
            return;
        };
        let Some(time) = timestamp(record.timestamp.as_deref()) else {
            return;
        };
        let Some(identity) = message
            .id
            .or(record.request_id)
            .filter(|value| safe(value, 512))
        else {
            return;
        };
        if self
            .latest
            .as_ref()
            .is_some_and(|latest| time < latest.time)
            || (self.identities.contains(&identity)
                && self
                    .latest
                    .as_ref()
                    .is_none_or(|latest| latest.identity != identity))
        {
            return;
        }
        let Some(input) = usage.input_tokens.filter(|value| *value >= 0) else {
            return;
        };
        let written = usage
            .cache_creation
            .and_then(|split| {
                (split.ephemeral_5m_input_tokens.is_some()
                    || split.ephemeral_1h_input_tokens.is_some())
                .then(|| {
                    nonnegative(split.ephemeral_5m_input_tokens)
                        .saturating_add(nonnegative(split.ephemeral_1h_input_tokens))
                })
            })
            .unwrap_or_else(|| nonnegative(usage.cache_creation_input_tokens));
        let prompt = input
            .saturating_add(nonnegative(usage.cache_read_input_tokens))
            .saturating_add(written);
        let used = prompt.saturating_add(nonnegative(usage.output_tokens));
        if used == 0 {
            return;
        }
        self.set_model(message.model);
        self.evidence = self.evidence.max(prompt);
        self.accept(identity, prompt, used, time);
    }

    fn codex(&mut self, record: CodexRecord) {
        if record.kind == "compacted" {
            self.compact();
            return;
        }
        let Some(payload) = record.payload else {
            return;
        };
        if record.kind == "session_meta" {
            if let Some(session) = payload.id.filter(|value| safe(value, 256)) {
                self.session = session;
            }
            self.excluded |= payload.source.is_some_and(|source| match source {
                SessionSource::Name(name) => name == "subagent",
                SessionSource::Metadata(metadata) => metadata.subagent.is_some(),
            });
            self.metadata(payload.cwd, payload.git.and_then(|git| git.repository_url));
            return;
        }
        if self.excluded {
            return;
        }
        if record.kind == "turn_context" {
            self.metadata(payload.cwd, None);
            self.set_model(payload.model);
            return;
        }
        if record.kind == "event_msg" && payload.kind.as_deref() == Some("context_compacted") {
            self.compact();
            return;
        }
        if record.kind != "event_msg" || payload.kind.as_deref() != Some("token_count") {
            return;
        }
        let Some(info) = payload.info else {
            return;
        };
        let Some(usage) = info.last_token_usage else {
            return;
        };
        let Some(time) = timestamp(record.timestamp.as_deref()) else {
            return;
        };
        if self
            .latest
            .as_ref()
            .is_some_and(|latest| time < latest.time)
        {
            return;
        }
        let Some(prompt) = usage.input_tokens.filter(|value| *value >= 0) else {
            return;
        };
        let Some(output) = usage.output_tokens.filter(|value| *value >= 0) else {
            return;
        };
        if info.total_token_usage.is_some() && self.cumulative == info.total_token_usage {
            return;
        }
        let identity = match &info.total_token_usage {
            Some(total) => format!("{:?}", total),
            None => format!("{}:{usage:?}", time.to_rfc3339()),
        };
        if self.identities.contains(&identity) {
            return;
        }
        self.set_model(payload.model.or(info.model));
        if let Some(window) = info.model_context_window.filter(|value| *value > 0) {
            self.window = Some(window);
        }
        self.cumulative = info.total_token_usage;
        self.accept(identity, prompt, prompt.saturating_add(output), time);
    }
}

fn safe(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn nonnegative(value: Option<i64>) -> i64 {
    value.unwrap_or(0).max(0)
}

fn timestamp(value: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

#[derive(Deserialize)]
struct ClaudeRecord {
    #[serde(rename = "type")]
    kind: String,
    subtype: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    #[serde(default, rename = "isSidechain")]
    is_sidechain: bool,
    #[serde(rename = "agentId")]
    agent_id: Option<IgnoredAny>,
    message: Option<ClaudeMessage>,
    #[serde(rename = "compactMetadata")]
    compact_metadata: Option<CompactMetadata>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
}

#[derive(Deserialize)]
struct ClaudeUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    cache_creation: Option<CacheCreation>,
}

#[derive(Deserialize)]
struct CacheCreation {
    ephemeral_5m_input_tokens: Option<i64>,
    ephemeral_1h_input_tokens: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompactMetadata {
    pre_tokens: Option<i64>,
    post_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct CodexRecord {
    #[serde(rename = "type")]
    kind: String,
    timestamp: Option<String>,
    payload: Option<CodexPayload>,
}

#[derive(Deserialize)]
struct CodexPayload {
    #[serde(rename = "type")]
    kind: Option<String>,
    id: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    source: Option<SessionSource>,
    git: Option<GitMetadata>,
    info: Option<CodexInfo>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SessionSource {
    Name(String),
    Metadata(SourceMetadata),
}

#[derive(Deserialize)]
struct SourceMetadata {
    subagent: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct GitMetadata {
    repository_url: Option<String>,
}

#[derive(Deserialize)]
struct CodexInfo {
    last_token_usage: Option<Usage>,
    total_token_usage: Option<Usage>,
    model_context_window: Option<i64>,
    model: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
struct Usage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cached_input_tokens: Option<i64>,
    total_tokens: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude(parser: &mut ContextParser, id: &str, second: usize, input: i64, output: i64) {
        parser.parse(format!(r#"{{"type":"assistant","timestamp":"2026-09-26T01:00:{second:02}Z","sessionId":"session","cwd":"/missing/repo","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_read_input_tokens":100,"cache_creation_input_tokens":20}}}}}}"#).as_bytes());
    }

    fn codex(parser: &mut ContextParser, second: usize, input: i64, total: i64) {
        parser.parse(format!(r#"{{"type":"event_msg","timestamp":"2026-09-26T01:00:{second:02}Z","payload":{{"type":"token_count","info":{{"model_context_window":272000,"last_token_usage":{{"input_tokens":{input},"cached_input_tokens":40,"output_tokens":10}},"total_token_usage":{{"total_tokens":{total}}}}}}}}}"#).as_bytes());
    }

    #[test]
    fn claude_deduplicates_response_blocks_and_retains_the_previous_distinct_request() {
        let mut parser = ContextParser::new(LogSource::Claude, "fallback".into());
        claude(&mut parser, "first", 1, 10, 5);
        claude(&mut parser, "first", 2, 10, 15);
        claude(&mut parser, "second", 3, 100, 20);
        claude(&mut parser, "second", 4, 100, 25);
        claude(&mut parser, "first", 5, 10, 15);
        let result = parser.snapshot(None).unwrap();
        assert_eq!(result.used_tokens, 245);
        assert_eq!(result.base_tokens, 130);
        assert_eq!(result.last_turn_tokens, 100);
        assert_eq!(result.project, "repo");
        assert_eq!(result.session_id, "session");
    }

    #[test]
    fn claude_compaction_resets_base_and_sidechains_do_not_change_the_session() {
        let mut parser = ContextParser::new(LogSource::Claude, "fallback".into());
        claude(&mut parser, "first", 1, 500000, 5);
        parser.parse(br#"{"type":"system","subtype":"compact_boundary","compactMetadata":{"preTokens":967123,"postTokens":79241}}"#);
        claude(&mut parser, "second", 2, 100, 5);
        parser.parse(br#"{"type":"assistant","isSidechain":true,"sessionId":"child","timestamp":"2026-09-26T02:00:00Z","message":{"id":"child","model":"other","usage":{"input_tokens":123}}}"#);
        let result = parser.snapshot(None).unwrap();
        assert_eq!(result.base_tokens, 220);
        assert_eq!(result.last_turn_tokens, 0);
        assert_eq!(result.window_tokens, Some(1000000));
        assert_eq!(result.session_id, "session");
    }

    #[test]
    fn split_cache_writes_replace_aggregate_and_invalid_timestamps_are_rejected() {
        let mut parser = ContextParser::new(LogSource::Claude, "fallback".into());
        parser.parse(br#"{"type":"assistant","timestamp":"2026-09-26T02:00:00Z","requestId":"request","message":{"model":"unknown-model","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":20,"cache_creation_input_tokens":100,"cache_creation":{"ephemeral_5m_input_tokens":7,"ephemeral_1h_input_tokens":3}}}}"#);
        let result = parser.snapshot(None).unwrap();
        assert_eq!(result.used_tokens, 45);
        assert_eq!(result.base_tokens, 40);
        assert_eq!(result.window_tokens, None);
        parser.parse(br#"{"type":"assistant","timestamp":"invalid","message":{"id":"bad","model":"other","usage":{"input_tokens":123}}}"#);
        assert_eq!(parser.snapshot(None).unwrap().used_tokens, 45);
    }

    #[test]
    fn initial_compaction_evidence_survives_first_model_but_not_a_model_switch() {
        let mut parser = ContextParser::new(LogSource::Claude, "fallback".into());
        parser.parse(br#"{"type":"system","subtype":"compact_boundary","compactMetadata":{"preTokens":967123,"postTokens":79241}}"#);
        parser.parse(br#"{"type":"assistant","timestamp":"2026-09-26T02:00:00Z","message":{"id":"first","model":"claude-sonnet-4-5","usage":{"input_tokens":1000,"output_tokens":5}}}"#);
        assert_eq!(parser.snapshot(None).unwrap().window_tokens, Some(1000000));
        parser.parse(br#"{"type":"assistant","timestamp":"2026-09-26T02:00:01Z","message":{"id":"second","model":"claude-haiku-4-5","usage":{"input_tokens":1100,"output_tokens":5}}}"#);
        assert_eq!(parser.snapshot(None).unwrap().window_tokens, Some(200000));
    }

    #[test]
    fn codex_uses_last_request_not_cumulative_and_does_not_add_cached_input_twice() {
        let mut parser = ContextParser::new(LogSource::Codex, "fallback".into());
        parser.parse(br#"{"type":"session_meta","payload":{"id":"thread","cwd":"/missing/checkout","git":{"repository_url":"https://github.com/team/repo.git"},"source":"cli"}}"#);
        parser.parse(br#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#);
        codex(&mut parser, 1, 100, 10000);
        codex(&mut parser, 2, 200, 20000);
        codex(&mut parser, 3, 200, 20000);
        let result = parser.snapshot(None).unwrap();
        assert_eq!(result.used_tokens, 210);
        assert_eq!(result.base_tokens, 100);
        assert_eq!(result.last_turn_tokens, 100);
        assert_eq!(result.window_tokens, Some(272000));
        assert_eq!(result.session_id, "thread");
        assert_eq!(result.project, "repo");
        assert_eq!(result.updated_at, "2026-09-26T01:00:02+00:00");
        parser.parse(br#"{"type":"compacted"}"#);
        codex(&mut parser, 4, 50, 20100);
        let result = parser.snapshot(None).unwrap();
        assert_eq!(result.base_tokens, 50);
        assert_eq!(result.last_turn_tokens, 0);
        codex(&mut parser, 5, 20, 20200);
        assert_eq!(parser.snapshot(None).unwrap().last_turn_tokens, 0);
    }

    #[test]
    fn codex_subagent_sessions_are_excluded_and_missing_window_stays_unknown() {
        let mut parser = ContextParser::new(LogSource::Codex, "fallback".into());
        parser.parse(br#"{"type":"session_meta","payload":{"source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}}}"#);
        codex(&mut parser, 1, 100, 1000);
        assert!(parser.snapshot(None).is_none());
        let mut parser = ContextParser::new(LogSource::Codex, "fallback".into());
        parser.parse(br#"{"type":"event_msg","timestamp":"2026-09-26T01:00:01Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":50,"output_tokens":10}}}}"#);
        assert_eq!(parser.snapshot(None).unwrap().window_tokens, None);
    }

    #[test]
    fn pending_codex_model_and_project_do_not_relabel_the_latest_request() {
        let mut parser = ContextParser::new(LogSource::Codex, "fallback".into());
        parser.parse(
            br#"{"type":"turn_context","payload":{"model":"model-a","cwd":"/missing/project-a"}}"#,
        );
        codex(&mut parser, 1, 100, 1000);
        parser.parse(
            br#"{"type":"turn_context","payload":{"model":"model-b","cwd":"/missing/project-b"}}"#,
        );
        let result = parser.snapshot(None).unwrap();
        assert_eq!(result.model, "model-a");
        assert_eq!(result.project, "project-a");
        assert_eq!(result.window_tokens, Some(272000));
        assert_eq!(result.used_tokens, 110);
        parser.parse(br#"{"type":"event_msg","timestamp":"2026-09-26T01:00:02Z","payload":{"type":"token_count","model":"duplicate-model","info":{"model_context_window":999,"last_token_usage":{"input_tokens":100,"output_tokens":10},"total_token_usage":{"total_tokens":1000}}}}"#);
        parser.parse(br#"{"type":"event_msg","timestamp":"2026-09-26T01:00:03Z","payload":{"type":"token_count","model":"invalid-model","info":{"model_context_window":888,"last_token_usage":{"input_tokens":200}}}}"#);
        let result = parser.snapshot(None).unwrap();
        assert_eq!(result.model, "model-a");
        assert_eq!(result.window_tokens, Some(272000));
        assert_eq!(result.used_tokens, 110);
        parser.parse(br#"{"type":"event_msg","timestamp":"2026-09-26T01:00:04Z","payload":{"type":"token_count","info":{"model_context_window":1000000,"last_token_usage":{"input_tokens":200,"output_tokens":20},"total_token_usage":{"total_tokens":2000}}}}"#);
        let result = parser.snapshot(None).unwrap();
        assert_eq!(result.model, "model-b");
        assert_eq!(result.project, "project-b");
        assert_eq!(result.window_tokens, Some(1000000));
        assert_eq!(result.used_tokens, 220);
        assert_eq!(result.last_turn_tokens, 110);
    }

    #[test]
    fn synthetic_zero_usage_does_not_replace_the_latest_claude_request() {
        let mut parser = ContextParser::new(LogSource::Claude, "fallback".into());
        claude(&mut parser, "first", 1, 100, 10);
        let before = parser.snapshot(None).unwrap();
        parser.parse(br#"{"type":"assistant","timestamp":"2026-09-26T02:00:00Z","message":{"id":"synthetic","model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#);
        let after = parser.snapshot(None).unwrap();
        assert_eq!(after.used_tokens, before.used_tokens);
        assert_eq!(after.base_tokens, before.base_tokens);
        assert_eq!(after.last_turn_tokens, before.last_turn_tokens);
        assert_eq!(after.model, before.model);
        assert_eq!(after.window_tokens, before.window_tokens);
        assert_eq!(after.updated_at, before.updated_at);
    }
}
