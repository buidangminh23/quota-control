//! Model quality measured on the user's own projects, from the Claude Code and Codex transcripts
//! already on disk. Nothing is sent to a model: every number is counted from what happened in real
//! sessions (turns, interruptions, file edits, shell commands and the outcome of test, build, type
//! check and lint runs). Parsed files are cached by size and modification time, so a rescan only
//! rereads transcripts that changed.

pub(crate) mod checks;
mod transcript;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use chrono::{FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{LogScanner, LogSource, linked};
use transcript::{ClaudeTranscript, CodexTranscript, Tally};

const CACHE_VERSION: u32 = 3;
const PARSER_VERSION: u32 = 1;
const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
const MAX_FILES: usize = 50_000;
const MAX_DEPTH: usize = 32;
const MAX_WORKERS: usize = 4;

/// Everything counted for one model on one project over a period.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QualityCounts {
    /// Requests the model answered: a person's prompt, or an agent's task for a subagent.
    pub turns: u64,
    /// Turns a person started.
    pub human_turns: u64,
    /// Turns a person stopped before the model finished.
    pub interrupted_turns: u64,
    /// Turns that ran at least one check whose result could be read.
    pub verified_turns: u64,
    /// Verified turns whose last readable check passed.
    pub green_turns: u64,
    /// Check runs with a readable result.
    pub check_runs: u64,
    /// Check runs that failed.
    pub failed_check_runs: u64,
    /// Check runs whose result could not be read from the transcript.
    pub unknown_check_runs: u64,
    /// File edits the model attempted (edits a person declined are not counted).
    pub edits: u64,
    /// Edits that could not be applied.
    pub failed_edits: u64,
    /// Tool calls a person or a permission rule declined.
    pub denied_actions: u64,
    /// Shell commands the model ran.
    pub shell_commands: u64,
    /// Shell commands that exited with an error.
    pub failed_shell_commands: u64,
    pub output_tokens: u64,
    /// Turns with a known duration, and their total length.
    pub timed_turns: u64,
    pub turn_millis: u64,
}

impl QualityCounts {
    fn add(&mut self, other: &Self) {
        self.turns += other.turns;
        self.human_turns += other.human_turns;
        self.interrupted_turns += other.interrupted_turns;
        self.verified_turns += other.verified_turns;
        self.green_turns += other.green_turns;
        self.check_runs += other.check_runs;
        self.failed_check_runs += other.failed_check_runs;
        self.unknown_check_runs += other.unknown_check_runs;
        self.edits += other.edits;
        self.failed_edits += other.failed_edits;
        self.denied_actions += other.denied_actions;
        self.shell_commands += other.shell_commands;
        self.failed_shell_commands += other.failed_shell_commands;
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.timed_turns += other.timed_turns;
        self.turn_millis = self.turn_millis.saturating_add(other.turn_millis);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QualityRow {
    pub source: LogSource,
    pub model: String,
    pub project: String,
    pub counts: QualityCounts,
}

/// Inclusive `YYYY-MM-DD` bounds in the local calendar; either may be left open.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QualityQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QualityInfo {
    pub scanned_at: Option<String>,
    pub scanning: bool,
    pub files: usize,
    pub first_day: Option<String>,
    pub last_day: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QualitySummary {
    pub rows: Vec<QualityRow>,
    pub info: QualityInfo,
}

#[derive(Clone, Debug, Default)]
pub struct QualityScanReport {
    pub files_seen: usize,
    pub files_parsed: usize,
    pub bytes_parsed: u64,
    pub skipped_lines: usize,
    pub unreadable_files: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct CachedRow {
    day: NaiveDate,
    model: String,
    project: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    unresolved: bool,
    counts: QualityCounts,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct CachedFile {
    source: LogSource,
    #[serde(default)]
    parser_version: u32,
    length: u64,
    modified: i64,
    rows: Vec<CachedRow>,
    /// Working directories in the file that are checkouts on this machine.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    local_cwds: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct CacheDocument {
    version: u32,
    scanned_at: Option<String>,
    files: BTreeMap<String, CachedFile>,
    /// A checkout's folder name mapped to its repository's name, where the two differ and the
    /// folder name points at a single repository: the ledger's rule for projects whose folder is
    /// not a checkout on this machine.
    #[serde(default)]
    aliases: BTreeMap<String, String>,
}

struct Candidate {
    source: LogSource,
    path: PathBuf,
    length: u64,
    modified: i64,
    offset: Option<FixedOffset>,
}

pub struct QualityStore {
    path: PathBuf,
    cache: Mutex<CacheDocument>,
    scan_lock: Mutex<()>,
    scanning: AtomicBool,
}

impl QualityStore {
    /// Open the cache at `path`; a missing, unreadable or older-format cache starts empty.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let cache = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CacheDocument>(&bytes).ok())
            .filter(|document| (2..=CACHE_VERSION).contains(&document.version))
            .unwrap_or_default();
        Self {
            path,
            cache: Mutex::new(cache),
            scan_lock: Mutex::new(()),
            scanning: AtomicBool::new(false),
        }
    }

    fn cache(&self) -> MutexGuard<'_, CacheDocument> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn info(&self) -> QualityInfo {
        let cache = self.cache();
        let days = || {
            cache
                .files
                .values()
                .flat_map(|file| file.rows.iter().map(|row| row.day))
        };
        QualityInfo {
            scanned_at: cache.scanned_at.clone(),
            scanning: self.scanning.load(Ordering::Acquire),
            files: cache.files.len(),
            first_day: days().min().map(|day| day.to_string()),
            last_day: days().max().map(|day| day.to_string()),
        }
    }

    pub fn summary(&self, query: &QualityQuery) -> Result<QualitySummary> {
        let from = query.from.as_deref().map(parse_day).transpose()?;
        let to = query.to.as_deref().map(parse_day).transpose()?;
        let mut grouped: BTreeMap<(u8, String, String), (LogSource, QualityCounts)> =
            BTreeMap::new();
        {
            let cache = self.cache();
            for file in cache.files.values() {
                for row in &file.rows {
                    if from.is_some_and(|from| row.day < from) || to.is_some_and(|to| row.day > to)
                    {
                        continue;
                    }
                    let project = row
                        .unresolved
                        .then(|| cache.aliases.get(&row.project))
                        .flatten()
                        .unwrap_or(&row.project);
                    grouped
                        .entry((rank(file.source), row.model.clone(), project.clone()))
                        .or_insert_with(|| (file.source, QualityCounts::default()))
                        .1
                        .add(&row.counts);
                }
            }
        }
        let rows = grouped
            .into_iter()
            .map(|((_, model, project), (source, counts))| QualityRow {
                source,
                model,
                project,
                counts,
            })
            .collect();
        Ok(QualitySummary {
            rows,
            info: self.info(),
        })
    }

    /// Reread every transcript that changed since the last scan and forget the ones that are gone.
    pub fn scan(&self, scanners: &[LogScanner]) -> Result<QualityScanReport> {
        let _guard = self
            .scan_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.scanning.store(true, Ordering::Release);
        let outcome = self.scan_locked(scanners);
        self.scanning.store(false, Ordering::Release);
        outcome
    }

    fn scan_locked(&self, scanners: &[LogScanner]) -> Result<QualityScanReport> {
        let mut report = QualityScanReport::default();
        let mut candidates = Vec::new();
        for scanner in scanners {
            for root in &scanner.options.roots {
                if crate::linked_ancestry(root) {
                    continue;
                }
                walk(root, 0, scanner, &mut candidates);
            }
        }
        candidates.truncate(MAX_FILES);
        report.files_seen = candidates.len();
        let previous = self.cache().files.clone();
        let mut files = BTreeMap::new();
        let mut stale = Vec::new();
        for candidate in candidates {
            let key = candidate.path.to_string_lossy().into_owned();
            match previous.get(&key) {
                Some(cached)
                    if cached.parser_version == PARSER_VERSION
                        && cached.length == candidate.length
                        && cached.modified == candidate.modified
                        && cached.source == candidate.source =>
                {
                    files.insert(key, cached.clone());
                }
                _ => stale.push((key, candidate)),
            }
        }
        let parsed = parse_all(&stale);
        for ((key, candidate), result) in stale.into_iter().zip(parsed) {
            match result {
                Ok(parsed) => {
                    report.files_parsed += 1;
                    report.bytes_parsed += candidate.length;
                    report.skipped_lines += parsed.skipped_lines;
                    files.insert(
                        key,
                        CachedFile {
                            source: candidate.source,
                            parser_version: PARSER_VERSION,
                            length: candidate.length,
                            modified: candidate.modified,
                            rows: parsed.rows,
                            local_cwds: parsed.local_cwds,
                        },
                    );
                }
                Err(_) => {
                    report.unreadable_files += 1;
                    if let Some(cached) = previous.get(&key) {
                        files.insert(key, cached.clone());
                    }
                }
            }
        }
        let document = CacheDocument {
            version: CACHE_VERSION,
            scanned_at: Some(Utc::now().to_rfc3339()),
            aliases: repository_aliases(&files),
            files,
        };
        let bytes =
            serde_json::to_vec(&document).context("Serializing the quality cache failed")?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).ok();
        }
        uc_core::paths::write_atomic(&self.path, &bytes)
            .context("Saving the quality cache failed")?;
        *self.cache() = document;
        Ok(report)
    }
}

fn rank(source: LogSource) -> u8 {
    match source {
        LogSource::Claude => 0,
        LogSource::Codex => 1,
    }
}

fn parse_day(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").context("Quality dates must use YYYY-MM-DD")
}

fn walk(path: &Path, depth: usize, scanner: &LogScanner, out: &mut Vec<Candidate>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    if scanner.source == LogSource::Claude
        && depth == 2
        && path.file_name().is_some_and(|name| name == "memory")
    {
        return;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if linked(&metadata) {
        return;
    }
    if metadata.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        children.sort();
        for child in children {
            walk(&child, depth + 1, scanner, out);
        }
    } else if metadata.is_file()
        && path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
    {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| {
                i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
            });
        out.push(Candidate {
            source: scanner.source,
            path: path.to_path_buf(),
            length: metadata.len(),
            modified,
            offset: scanner.options.timezone,
        });
    }
}

struct ParsedFile {
    rows: Vec<CachedRow>,
    local_cwds: Vec<String>,
    skipped_lines: usize,
}

/// The ledger's alias rule: a checkout's folder name stands for its repository's name when every
/// checkout with that folder name belongs to the same repository.
fn repository_aliases(files: &BTreeMap<String, CachedFile>) -> BTreeMap<String, String> {
    let cwds: BTreeSet<&str> = files
        .values()
        .flat_map(|file| file.local_cwds.iter().map(String::as_str))
        .collect();
    let mut names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for cwd in cwds {
        if let Some((alias, canonical)) = crate::project::repository_alias(cwd) {
            names.entry(alias).or_default().insert(canonical);
        }
    }
    names
        .into_iter()
        .filter_map(|(alias, canonical)| {
            let mut canonical = canonical.into_iter();
            let only = canonical.next()?;
            (canonical.next().is_none() && only != alias).then_some((alias, only))
        })
        .collect()
}

fn parse_all(stale: &[(String, Candidate)]) -> Vec<Result<ParsedFile>> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .clamp(1, MAX_WORKERS);
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Result<ParsedFile>>>> =
        Mutex::new((0..stale.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..workers.min(stale.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((_, candidate)) = stale.get(index) else {
                        break;
                    };
                    let result = parse_file(candidate);
                    results
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)[index] = Some(result);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .into_iter()
        .map(|result| {
            result.unwrap_or_else(|| Err(anyhow::anyhow!("The transcript was not parsed")))
        })
        .collect()
}

enum Reader {
    Claude(ClaudeTranscript),
    Codex(CodexTranscript),
}

fn parse_file(candidate: &Candidate) -> Result<ParsedFile> {
    let file = File::open(&candidate.path)?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut tally = Tally::new(candidate.offset);
    let mut transcript = match candidate.source {
        LogSource::Claude => Reader::Claude(ClaudeTranscript::new(&candidate.path)),
        LogSource::Codex => Reader::Codex(CodexTranscript::new()),
    };
    let mut line = Vec::with_capacity(64 * 1024);
    let mut skipped_lines = 0;
    loop {
        line.clear();
        let read = read_capped_line(&mut reader, &mut line)?;
        if read == 0 {
            break;
        }
        if line.len() > MAX_LINE_BYTES {
            skipped_lines += 1;
            continue;
        }
        if !relevant(candidate.source, &line) {
            continue;
        }
        let Ok(root) = serde_json::from_slice::<Value>(&line) else {
            skipped_lines += 1;
            continue;
        };
        match &mut transcript {
            Reader::Claude(reader) => reader.line(&root, &mut tally),
            Reader::Codex(reader) => reader.line(&root, &mut tally),
        }
    }
    match transcript {
        Reader::Claude(reader) => reader.finish(&mut tally),
        Reader::Codex(reader) => reader.finish(&mut tally),
    }
    let rows = tally
        .rows
        .into_iter()
        .map(|(key, counts)| CachedRow {
            day: key.day,
            model: key.model,
            project: key.project,
            unresolved: key.unresolved,
            counts,
        })
        .collect();
    Ok(ParsedFile {
        rows,
        local_cwds: tally.local_cwds.into_iter().collect(),
        skipped_lines,
    })
}

/// Read one line; a line longer than the cap is consumed but only its first bytes are kept, so an
/// oversized record (a pasted image, a huge tool output) is skipped without being buffered.
fn read_capped_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> std::io::Result<usize> {
    let mut total = 0;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(total);
        }
        let (chunk, done) = match memchr::memchr(b'\n', available) {
            Some(position) => (&available[..=position], true),
            None => (available, false),
        };
        let length = chunk.len();
        if line.len() <= MAX_LINE_BYTES {
            line.extend_from_slice(chunk);
        }
        reader.consume(length);
        total += length;
        if done {
            return Ok(total);
        }
    }
}

fn relevant(source: LogSource, line: &[u8]) -> bool {
    match source {
        LogSource::Claude => {
            memchr::memmem::find(line, b"\"type\":\"user\"").is_some()
                || memchr::memmem::find(line, b"\"type\":\"assistant\"").is_some()
        }
        LogSource::Codex => {
            memchr::memmem::find(line, b"\"payload\":{\"type\":\"reasoning\"").is_none()
        }
    }
}
