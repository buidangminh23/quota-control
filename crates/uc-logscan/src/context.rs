use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, Metadata};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::LogSource;
use crate::context_parser::ContextParser;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextWindowSession {
    pub source: LogSource,
    pub session_id: String,
    pub project: String,
    pub model: String,
    pub used_tokens: i64,
    pub window_tokens: Option<i64>,
    pub base_tokens: i64,
    pub last_turn_tokens: i64,
    pub updated_at: String,
}

const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
const FINGERPRINT_BYTES: u64 = 4096;

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextScanStats {
    pub files_read: usize,
    pub bytes_read: u64,
    pub files_cached: usize,
}

pub struct ContextWindows {
    claude_root: PathBuf,
    codex_root: PathBuf,
    files: BTreeMap<PathBuf, CachedContext>,
    stats: ContextScanStats,
}

struct CachedContext {
    parser: ContextParser,
    length: u64,
    modified: SystemTime,
    fingerprint: [u8; 32],
    offset: u64,
    oversized: bool,
}

impl ContextWindows {
    pub fn new(claude_root: PathBuf, codex_root: PathBuf) -> Self {
        Self {
            claude_root,
            codex_root,
            files: BTreeMap::new(),
            stats: ContextScanStats::default(),
        }
    }

    pub fn from_environment() -> Self {
        let home = uc_core::paths::home_dir();
        Self::new(
            uc_core::paths::env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            uc_core::paths::env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
        )
    }

    pub fn stats(&self) -> ContextScanStats {
        self.stats
    }

    pub fn scan(&mut self, now: DateTime<Utc>) -> Vec<ContextWindowSession> {
        self.stats = ContextScanStats::default();
        let since = now - Duration::hours(24);
        let mut candidates = BTreeMap::new();
        let mut budget = 200_000;
        for (source, root) in [
            (LogSource::Claude, self.claude_root.join("projects")),
            (LogSource::Codex, self.codex_root.join("sessions")),
            (LogSource::Codex, self.codex_root.join("archived_sessions")),
        ] {
            if safe_path(&root) {
                discover(&root, source, 0, since, &mut candidates, &mut budget);
            }
        }
        self.files.retain(|path, _| candidates.contains_key(path));
        for (path, (source, metadata)) in candidates {
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if self.files.get(&path).is_some_and(|cached| {
                cached.length == metadata.len() && cached.modified == modified
            }) {
                self.stats.files_cached += 1;
                continue;
            }
            if !safe_path(&path) {
                self.files.remove(&path);
                continue;
            }
            let old = self.files.remove(&path);
            if let Ok(cached) = read_file(&path, source, &metadata, old, &mut self.stats) {
                self.files.insert(path, cached);
            }
        }
        let configured_model = configured_model(&self.claude_root.join("settings.json"));
        let mut aliases: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for cached in self.files.values() {
            if let Some((alias, canonical)) = cached.parser.repository_alias() {
                aliases.entry(alias).or_default().insert(canonical);
            }
        }
        let mut sessions: BTreeMap<(u8, String), (DateTime<Utc>, ContextWindowSession)> =
            BTreeMap::new();
        for cached in self.files.values() {
            let Some(mut snapshot) = cached.parser.snapshot(configured_model.as_deref()) else {
                continue;
            };
            let Ok(updated_at) = DateTime::parse_from_rfc3339(&snapshot.updated_at) else {
                continue;
            };
            let updated_at = updated_at.with_timezone(&Utc);
            if updated_at < since || updated_at > now {
                continue;
            }
            if !cached.parser.project_is_resolved()
                && let Some(names) = aliases.get(&snapshot.project)
                && names.len() == 1
                && let Some(name) = names.first()
            {
                snapshot.project.clone_from(name);
            }
            let key = (
                match snapshot.source {
                    LogSource::Claude => 0,
                    LogSource::Codex => 1,
                },
                snapshot.session_id.clone(),
            );
            if sessions.get(&key).is_none_or(|(old, _)| updated_at > *old) {
                sessions.insert(key, (updated_at, snapshot));
            }
        }
        let mut sessions: Vec<_> = sessions.into_values().collect();
        sessions.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.session_id.cmp(&b.1.session_id)));
        sessions.truncate(12);
        sessions.into_iter().map(|(_, session)| session).collect()
    }
}

fn safe_path(path: &Path) -> bool {
    !crate::linked_ancestry(path)
}

fn discover(
    path: &Path,
    source: LogSource,
    depth: usize,
    since: DateTime<Utc>,
    files: &mut BTreeMap<PathBuf, (LogSource, Metadata)>,
    budget: &mut usize,
) {
    if *budget == 0 || depth > 32 || (source == LogSource::Claude && depth > 2) {
        return;
    }
    *budget -= 1;
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if crate::linked(&metadata) {
        return;
    }
    if metadata.is_file() {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
            && !(source == LogSource::Claude && name.starts_with("agent-"))
            && metadata
                .modified()
                .is_ok_and(|modified| DateTime::<Utc>::from(modified) >= since)
        {
            files.insert(path.to_owned(), (source, metadata));
        }
    } else if metadata.is_dir()
        && !(source == LogSource::Claude && depth == 2)
        && let Ok(entries) = fs::read_dir(path)
    {
        for entry in entries.flatten() {
            discover(&entry.path(), source, depth + 1, since, files, budget);
            if *budget == 0 {
                break;
            }
        }
    }
}

fn fingerprint(file: &mut File, length: u64) -> std::io::Result<[u8; 32]> {
    let mut hash = Sha256::new();
    for offset in [0, length.saturating_sub(FINGERPRINT_BYTES)] {
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; length.saturating_sub(offset).min(FINGERPRINT_BYTES) as usize];
        file.read_exact(&mut bytes)?;
        hash.update(&bytes);
    }
    Ok(hash.finalize().into())
}

fn read_file(
    path: &Path,
    source: LogSource,
    metadata: &Metadata,
    old: Option<CachedContext>,
    stats: &mut ContextScanStats,
) -> std::io::Result<CachedContext> {
    let mut file = File::open(path)?;
    let old = old.filter(|old| {
        metadata.len() > old.length
            && fingerprint(&mut file, old.length).is_ok_and(|value| value == old.fingerprint)
    });
    let mut cached = old.unwrap_or_else(|| CachedContext {
        parser: ContextParser::new(
            source,
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        ),
        length: 0,
        modified: SystemTime::UNIX_EPOCH,
        fingerprint: [0; 32],
        offset: 0,
        oversized: false,
    });
    let remaining = metadata.len().saturating_sub(cached.offset);
    file.seek(SeekFrom::Start(cached.offset))?;
    let mut reader = BufReader::new((&mut file).take(remaining));
    let mut tail = Vec::new();
    let mut position = cached.offset;
    stats.files_read += 1;
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            break;
        }
        let newline = bytes.iter().position(|value| *value == b'\n');
        let count = newline.map_or(bytes.len(), |index| index + 1);
        if !cached.oversized {
            if tail.len().saturating_add(count) <= MAX_LINE_BYTES {
                tail.extend_from_slice(&bytes[..count]);
            } else {
                tail.clear();
                cached.oversized = true;
            }
        }
        if newline.is_some() {
            if !cached.oversized {
                cached.parser.parse(&tail);
            }
            tail.clear();
            cached.oversized = false;
            cached.offset = position + count as u64;
        }
        reader.consume(count);
        position += count as u64;
        if cached.oversized {
            cached.offset = position;
        }
        stats.bytes_read += count as u64;
    }
    cached.length = position;
    cached.fingerprint = fingerprint(&mut file, cached.length)?;
    cached.modified = metadata.modified()?;
    Ok(cached)
}

fn configured_model(path: &Path) -> Option<String> {
    #[derive(Deserialize)]
    struct Settings {
        model: Option<String>,
    }
    if !safe_path(path) {
        return None;
    }
    let file = File::open(path).ok()?;
    if file.metadata().ok()?.len() > 1024 * 1024 {
        return None;
    }
    serde_json::from_reader::<_, Settings>(file.take(1024 * 1024))
        .ok()?
        .model
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn roots() -> (tempfile::TempDir, ContextWindows) {
        let temp = tempfile::tempdir().unwrap();
        let reader = ContextWindows::new(temp.path().join("claude"), temp.path().join("codex"));
        (temp, reader)
    }

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn request(session: &str, id: &str, timestamp: DateTime<Utc>, input: i64) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "type":"assistant", "sessionId":session, "timestamp":timestamp.to_rfc3339(),
                "message":{"id":id,"model":"claude-sonnet-4-5","usage":{
                    "input_tokens":input,"cache_creation_input_tokens":10,
                    "cache_read_input_tokens":20,"output_tokens":5
                }}
            })
        )
    }

    #[test]
    fn caches_unchanged_files_and_reads_only_appended_complete_records() {
        let (_temp, mut scanner) = roots();
        let now = Utc::now();
        let path = scanner.claude_root.join("projects/project/session.jsonl");
        let first = request("session", "first", now - Duration::minutes(2), 100);
        write(&path, &first);
        assert_eq!(scanner.scan(now)[0].used_tokens, 135);
        assert_eq!(scanner.stats().bytes_read, first.len() as u64);
        scanner.scan(now);
        assert_eq!(scanner.stats().files_cached, 1);
        assert_eq!(scanner.stats().bytes_read, 0);
        let second = request("session", "second", now - Duration::minutes(1), 200);
        let split = second.len() / 2;
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&second.as_bytes()[..split]).unwrap();
        assert_eq!(scanner.scan(now)[0].used_tokens, 135);
        assert_eq!(scanner.stats().bytes_read, split as u64);
        file.write_all(&second.as_bytes()[split..]).unwrap();
        let sessions = scanner.scan(now);
        assert_eq!(sessions[0].used_tokens, 235);
        assert_eq!(sessions[0].base_tokens, 130);
        assert_eq!(sessions[0].last_turn_tokens, 100);
        assert_eq!(scanner.stats().bytes_read, second.len() as u64);
    }

    #[test]
    fn truncation_and_growing_rewrites_reset_cached_parser_state() {
        let (_temp, mut scanner) = roots();
        let now = Utc::now();
        let path = scanner.claude_root.join("projects/project/session.jsonl");
        write(
            &path,
            &(request("old", "one", now, 12345) + &request("old", "two", now, 22345)),
        );
        assert_eq!(scanner.scan(now)[0].session_id, "old");
        write(&path, &request("new", "one", now, 100));
        assert_eq!(scanner.scan(now)[0].used_tokens, 135);
        let rewritten =
            request("replacement", "one", now, 300) + &request("replacement", "two", now, 400);
        write(&path, &rewritten);
        let sessions = scanner.scan(now);
        assert_eq!(sessions[0].session_id, "replacement");
        assert_eq!(sessions[0].base_tokens, 330);
        assert_eq!(scanner.stats().bytes_read, rewritten.len() as u64);
    }

    #[test]
    fn requires_recent_file_and_request_and_skips_subagent_paths() {
        let (_temp, mut scanner) = roots();
        let now = Utc::now();
        let project = scanner.claude_root.join("projects/project");
        write(
            &project.join("old-request.jsonl"),
            &request("old", "one", now - Duration::hours(25), 100),
        );
        let old_file = project.join("old-file.jsonl");
        write(&old_file, &request("old-file", "one", now, 100));
        File::options()
            .write(true)
            .open(&old_file)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(SystemTime::from(now - Duration::hours(25))),
            )
            .unwrap();
        write(
            &project.join("session/subagents/agent-hidden.jsonl"),
            &request("hidden", "one", now, 100),
        );
        write(
            &project.join("agent-hidden.jsonl"),
            &request("hidden", "one", now, 100),
        );
        write(
            &project.join("memory/hidden.jsonl"),
            &request("hidden", "one", now, 100),
        );
        write(
            &project.join("live.jsonl"),
            &request("live", "one", now, 100),
        );
        assert_eq!(
            scanner
                .scan(now)
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["live"]
        );
        assert_eq!(scanner.stats().files_read, 2);
    }

    #[test]
    fn deduplicates_copies_and_limits_newest_sessions() {
        let (_temp, mut scanner) = roots();
        let now = Utc::now();
        let project = scanner.claude_root.join("projects/project");
        for index in 0..15 {
            write(
                &project.join(format!("{index}.jsonl")),
                &request(
                    &format!("session-{index}"),
                    "one",
                    now - Duration::minutes(index),
                    100,
                ),
            );
        }
        write(
            &project.join("duplicate.jsonl"),
            &request("session-0", "copy", now - Duration::seconds(1), 999),
        );
        let sessions = scanner.scan(now);
        assert_eq!(sessions.len(), 12);
        assert_eq!(sessions[0].session_id, "session-0");
        assert_eq!(sessions[0].used_tokens, 135);
        assert_eq!(sessions[11].session_id, "session-11");
    }

    #[test]
    fn codex_copies_deduplicate_but_equal_ids_from_other_sources_do_not() {
        let (_temp, mut scanner) = roots();
        let now = Utc::now();
        let codex = format!(
            "{}\n{}\n{}\n",
            serde_json::json!({"type":"session_meta","timestamp":now.to_rfc3339(),"payload":{"id":"same-id","cwd":"/missing/example"}}),
            serde_json::json!({"type":"turn_context","timestamp":now.to_rfc3339(),"payload":{"model":"gpt-5.4"}}),
            serde_json::json!({"type":"event_msg","timestamp":now.to_rfc3339(),"payload":{"type":"token_count","info":{"model_context_window":272000,"last_token_usage":{"input_tokens":1000,"cached_input_tokens":900,"output_tokens":100,"total_tokens":1100}}}})
        );
        write(
            &scanner
                .codex_root
                .join("sessions/2026/09/26/rollout-live.jsonl"),
            &codex,
        );
        write(
            &scanner.codex_root.join("archived_sessions/copy.jsonl"),
            &codex,
        );
        write(
            &scanner.claude_root.join("projects/project/live.jsonl"),
            &request("same-id", "one", now, 100),
        );
        let sessions = scanner.scan(now);
        assert_eq!(sessions.len(), 2);
        assert_eq!(
            sessions
                .iter()
                .filter(|s| s.source == LogSource::Codex)
                .count(),
            1
        );
    }

    #[test]
    fn oversized_partial_lines_are_bounded_and_recover_after_newline() {
        let (_temp, mut scanner) = roots();
        let now = Utc::now();
        let path = scanner.claude_root.join("projects/project/live.jsonl");
        write(&path, &"x".repeat(MAX_LINE_BYTES + 100));
        assert!(scanner.scan(now).is_empty());
        assert!(scanner.files[&path].oversized);
        let tail = format!("\n{}", request("live", "one", now, 100));
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(tail.as_bytes())
            .unwrap();
        assert_eq!(scanner.scan(now)[0].used_tokens, 135);
        assert_eq!(scanner.stats().bytes_read, tail.len() as u64);
    }

    #[test]
    fn typed_settings_model_is_applied_without_reparsing_logs() {
        let (_temp, mut scanner) = roots();
        let now = Utc::now();
        write(
            &scanner.claude_root.join("projects/project/live.jsonl"),
            &request("live", "one", now, 100),
        );
        scanner.scan(now);
        write(
            &scanner.claude_root.join("settings.json"),
            r#"{"model":"sonnet[1m]","unrelated":{"ignored":[1,2,3]}}"#,
        );
        assert_eq!(scanner.scan(now)[0].window_tokens, Some(1_000_000));
        assert_eq!(scanner.stats().files_cached, 1);
    }

    #[test]
    fn verified_aliases_normalize_missing_checkouts_but_preserve_known_repositories() {
        let (temp, mut scanner) = roots();
        let now = Utc::now();
        let local = temp.path().join("bot tele");
        write(
            &local.join(".git/config"),
            "[remote \"origin\"]\nurl = https://github.com/team/bot-tele.git\n",
        );
        for (session, cwd) in [
            ("current", local),
            ("missing", temp.path().join("missing/bot tele")),
        ] {
            let mut record: serde_json::Value =
                serde_json::from_str(&request(session, "one", now, 100)).unwrap();
            record["cwd"] = serde_json::json!(cwd);
            write(
                &scanner
                    .claude_root
                    .join(format!("projects/project/{session}.jsonl")),
                &format!("{record}\n"),
            );
        }
        let codex = format!(
            "{}\n{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"known","cwd":"/missing/bot tele","git":{"repository_url":"https://github.com/other/bot tele.git"}}}),
            serde_json::json!({"type":"event_msg","timestamp":now.to_rfc3339(),"payload":{"type":"token_count","info":{"model_context_window":272000,"last_token_usage":{"input_tokens":1000,"output_tokens":100,"total_tokens":1100}}}})
        );
        write(&scanner.codex_root.join("sessions/known.jsonl"), &codex);
        let sessions = scanner.scan(now);
        assert_eq!(sessions.len(), 3);
        for session in sessions {
            assert_eq!(
                session.project,
                if session.session_id == "known" {
                    "bot tele"
                } else {
                    "bot-tele"
                }
            );
        }
    }
}
