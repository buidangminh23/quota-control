use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use chrono::{DateTime, Utc};

use crate::{LogSource, ScanOptions, parser, read_line};

#[derive(Debug, Default)]
pub(crate) struct ScanCache {
    pub files: BTreeMap<PathBuf, ParsedFile>,
    pub retained_events: usize,
}

impl ScanCache {
    pub fn remove(&mut self, path: &Path) {
        if let Some(entry) = self.files.remove(path) {
            self.retained_events = self.retained_events.saturating_sub(entry.events.len());
        }
    }
}

#[derive(Debug)]
pub(crate) struct ParsedFile {
    pub length: u64,
    pub modified: Option<SystemTime>,
    pub source: LogSource,
    pub since: DateTime<Utc>,
    pub max_line_bytes: usize,
    pub events: Vec<parser::Event>,
    pub skipped_lines: usize,
    pub complete: bool,
    pub stable: bool,
}

impl ParsedFile {
    pub fn matches(
        &self,
        length: u64,
        modified: Option<SystemTime>,
        source: LogSource,
        since: DateTime<Utc>,
        max_line_bytes: usize,
    ) -> bool {
        self.complete
            && self.length == length
            && self.modified == modified
            && self.source == source
            && self.since <= since
            && self.max_line_bytes == max_line_bytes
    }
}

pub(crate) fn parse_file(
    path: &Path,
    source: LogSource,
    options: &ScanOptions,
    since: DateTime<Utc>,
    remaining: &mut u64,
    deadline: Instant,
) -> std::io::Result<ParsedFile> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    let length = metadata.len();
    let modified = metadata.modified().ok();
    let mut result = ParsedFile {
        length,
        modified,
        source,
        since,
        max_line_bytes: options.max_line_bytes,
        events: Vec::new(),
        skipped_lines: 0,
        complete: false,
        stable: true,
    };
    let mut reader = BufReader::new(file);
    let mut parser = parser::Parser::new(source, path);
    loop {
        if *remaining == 0
            || Instant::now() >= deadline
            || result.events.len() >= options.max_events
        {
            break;
        }
        match read_line(&mut reader, options.max_line_bytes, remaining)? {
            Some((prefix, true)) => {
                if source != LogSource::Codex || !is_compaction_header(&prefix) {
                    result.skipped_lines += 1;
                }
            }
            Some((line, false)) => {
                let relevant = std::str::from_utf8(&line).is_ok_and(|s| match source {
                    LogSource::Claude => s.contains("\"assistant\"") && s.contains("\"usage\""),
                    LogSource::Codex => [
                        "token_count",
                        "turn_context",
                        "session_meta",
                        "thread_settings_applied",
                        "task_started",
                    ]
                    .iter()
                    .any(|marker| s.contains(marker)),
                });
                if !relevant {
                    continue;
                }
                match serde_json::from_slice(&line) {
                    Ok(value) => {
                        if let Some(event) = parser.parse(&value)
                            && event.timestamp >= since
                        {
                            result.events.push(event);
                        }
                    }
                    Err(_) => result.skipped_lines += 1,
                }
            }
            None => {
                result.complete = true;
                break;
            }
        }
    }
    if let Ok(after) = reader.get_ref().metadata() {
        if after.len() != length || after.modified().ok() != modified {
            result.stable = false;
        }
    } else {
        result.stable = false;
    }
    Ok(result)
}

fn is_compaction_header(prefix: &[u8]) -> bool {
    let Some(payload) = prefix
        .windows(b"\"payload\"".len())
        .position(|window| window == b"\"payload\"")
    else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&prefix[..payload]) else {
        return false;
    };
    let header = format!("{}}}", text.trim_end().trim_end_matches(',').trim_end());
    serde_json::from_str::<serde_json::Value>(&header)
        .is_ok_and(|value| value["type"] == "compacted")
}
