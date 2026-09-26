mod cache;
pub mod context;
mod context_models;
mod context_parser;
pub mod ledger;
mod parser;
mod presentation;
mod project;
pub mod quality;

use std::collections::BTreeMap;
use std::fs::{self, Metadata};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate, Utc};
use uc_core::*;

pub use presentation::{HistoryRuntime, LocalHistoryRuntime, append_history, history_descriptors};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogSource {
    Claude,
    Codex,
}

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub roots: Vec<PathBuf>,
    pub timezone: Option<FixedOffset>,
    pub max_files: usize,
    pub max_line_bytes: usize,
    pub max_bytes: u64,
    pub max_events: usize,
    pub max_scan_seconds: u64,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            timezone: None,
            max_files: 20_000,
            max_line_bytes: 4 * 1024 * 1024,
            max_bytes: 8 * 1024 * 1024 * 1024,
            max_events: 250_000,
            max_scan_seconds: 110,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScanReport {
    pub usage: LogUsageScan,
    pub warnings: Vec<String>,
    pub files_read: usize,
    pub bytes_read: u64,
    pub files_cached: usize,
    pub records: usize,
    pub skipped_lines: usize,
    pub incomplete: bool,
    pub today: Option<NaiveDate>,
}

#[derive(Clone, Debug)]
pub struct LogScanner {
    pub source: LogSource,
    pub options: ScanOptions,
    cache: Arc<Mutex<cache::ScanCache>>,
}

impl LogScanner {
    pub fn new(source: LogSource, options: ScanOptions) -> Self {
        Self {
            source,
            options,
            cache: Arc::default(),
        }
    }

    pub fn from_environment(source: LogSource) -> Self {
        let home = uc_core::paths::home_dir();
        let roots = match source {
            LogSource::Claude => vec![
                uc_core::paths::env_path("CLAUDE_CONFIG_DIR")
                    .unwrap_or_else(|| home.join(".claude"))
                    .join("projects"),
            ],
            LogSource::Codex => {
                let root =
                    uc_core::paths::env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
                vec![root.join("sessions"), root.join("archived_sessions")]
            }
        };
        Self::new(
            source,
            ScanOptions {
                roots,
                ..ScanOptions::default()
            },
        )
    }

    pub fn scan(&self, now: DateTime<Utc>) -> ScanReport {
        let started = Instant::now();
        let deadline = started + std::time::Duration::from_secs(self.options.max_scan_seconds);
        let today = day(now, self.options.timezone);
        let since = today - Duration::days(29);
        let cache_since = now - Duration::days(32);
        let mut report = ScanReport {
            today: Some(today),
            ..ScanReport::default()
        };
        let mut files = Vec::new();
        let mut entries_budget = self.options.max_files.saturating_mul(20).max(1);
        for root in &self.options.roots {
            if root
                .ancestors()
                .any(|p| fs::symlink_metadata(p).is_ok_and(|m| linked(&m)))
            {
                report.incomplete = true;
                continue;
            }
            discover(
                root,
                0,
                self.source,
                &mut files,
                &mut entries_budget,
                &mut report,
            );
        }
        files.retain(|(modified, _)| {
            *modified == SystemTime::UNIX_EPOCH || DateTime::<Utc>::from(*modified) >= cache_since
        });
        files.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        files.dedup_by(|a, b| a.1 == b.1);
        if files.len() > self.options.max_files {
            files.truncate(self.options.max_files);
            report.incomplete = true;
        }
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let candidates: std::collections::BTreeSet<_> =
            files.iter().map(|(_, path)| path.clone()).collect();
        cache.files.retain(|path, _| candidates.contains(path));
        cache.retained_events = cache.files.values().map(|file| file.events.len()).sum();
        if cache.retained_events > self.options.max_events {
            cache.files.clear();
            cache.retained_events = 0;
        }
        let mut events: BTreeMap<String, parser::Event> = BTreeMap::new();
        let mut remaining = self.options.max_bytes;
        for (_, path) in files {
            if Instant::now() >= deadline || events.len() >= self.options.max_events {
                report.incomplete = true;
                break;
            }
            if path
                .ancestors()
                .any(|p| fs::symlink_metadata(p).is_ok_and(|m| linked(&m)))
            {
                cache.remove(&path);
                report.incomplete = true;
                continue;
            }
            let Ok(metadata) = fs::metadata(&path) else {
                cache.remove(&path);
                report.incomplete = true;
                continue;
            };
            if let Some(entry) = cache.files.get(&path)
                && entry.matches(
                    metadata.len(),
                    metadata.modified().ok(),
                    self.source,
                    cache_since,
                    self.options.max_line_bytes,
                )
            {
                report.files_cached += 1;
                report.skipped_lines += entry.skipped_lines;
                merge_events(
                    &mut events,
                    &entry.events,
                    since,
                    now,
                    &self.options,
                    &mut report,
                );
                continue;
            }
            cache.remove(&path);
            if remaining == 0 {
                report.incomplete = true;
                continue;
            }
            match cache::parse_file(
                &path,
                self.source,
                &self.options,
                cache_since,
                &mut remaining,
                deadline,
            ) {
                Ok(parsed) => {
                    report.files_read += 1;
                    report.skipped_lines += parsed.skipped_lines;
                    report.incomplete |= !parsed.complete;
                    merge_events(
                        &mut events,
                        &parsed.events,
                        since,
                        now,
                        &self.options,
                        &mut report,
                    );
                    if parsed.complete
                        && parsed.stable
                        && cache.retained_events.saturating_add(parsed.events.len())
                            <= self.options.max_events
                    {
                        cache.retained_events += parsed.events.len();
                        cache.files.insert(path, parsed);
                    }
                }
                Err(_) => report.incomplete = true,
            }
        }
        report.bytes_read = self.options.max_bytes.saturating_sub(remaining);
        report.records = events.len();
        report.usage = aggregate(events.into_values(), self.options.timezone);
        if report.incomplete {
            report.warnings.push("Local usage history is incomplete: some files could not be read or scan limits were reached.".into());
        }
        if report.skipped_lines > 0 {
            report.warnings.push(
                "Some malformed, truncated or oversized local log records were skipped.".into(),
            );
        }
        report
    }
}

fn merge_events(
    target: &mut BTreeMap<String, parser::Event>,
    source: &[parser::Event],
    since: NaiveDate,
    now: DateTime<Utc>,
    options: &ScanOptions,
    report: &mut ScanReport,
) {
    for event in source {
        if day(event.timestamp, options.timezone) < since || event.timestamp > now {
            continue;
        }
        if target.len() >= options.max_events && !target.contains_key(&event.key) {
            report.incomplete = true;
            break;
        }
        if target
            .get(&event.key)
            .is_none_or(|old| event.preferred_over(old))
        {
            target.insert(event.key.clone(), event.clone());
        }
    }
}
fn day(timestamp: DateTime<Utc>, offset: Option<FixedOffset>) -> NaiveDate {
    match offset {
        Some(offset) => timestamp.with_timezone(&offset).date_naive(),
        None => timestamp.with_timezone(&Local).date_naive(),
    }
}

fn linked(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn discover(
    path: &Path,
    depth: usize,
    source: LogSource,
    files: &mut Vec<(std::time::SystemTime, PathBuf)>,
    budget: &mut usize,
    report: &mut ScanReport,
) {
    if source == LogSource::Claude
        && depth == 2
        && path.file_name().is_some_and(|name| name == "memory")
    {
        return;
    }
    if *budget == 0 || depth > 32 {
        report.incomplete = true;
        return;
    }
    *budget -= 1;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            report.incomplete = true;
            return;
        }
    };
    if linked(&metadata) {
        report.incomplete = true;
        return;
    }
    if metadata.is_file() {
        if path.extension().is_some_and(|e| e == "jsonl") {
            files.push((
                metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                path.to_owned(),
            ));
        }
        return;
    }
    if metadata.is_dir() {
        match fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    if *budget == 0 {
                        report.incomplete = true;
                        break;
                    }
                    match entry {
                        Ok(entry) => {
                            discover(&entry.path(), depth + 1, source, files, budget, report)
                        }
                        Err(_) => report.incomplete = true,
                    }
                }
            }
            Err(_) => report.incomplete = true,
        }
    }
}

fn read_line(
    reader: &mut impl BufRead,
    max: usize,
    remaining: &mut u64,
) -> std::io::Result<Option<(Vec<u8>, bool)>> {
    let mut result = Vec::new();
    let mut oversized = false;
    let mut read_any = false;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(read_any.then_some((result, oversized)));
        }
        if *remaining == 0 {
            return Ok(Some((Vec::new(), true)));
        }
        let newline = buf.iter().position(|b| *b == b'\n');
        let len = newline
            .map_or(buf.len(), |i| i + 1)
            .min(*remaining as usize);
        read_any = true;
        if !oversized && result.len().saturating_add(len) <= max {
            result.extend_from_slice(&buf[..len]);
        } else {
            oversized = true;
            if result.is_empty() {
                result.extend_from_slice(&buf[..len.min(4096)]);
            } else {
                result.truncate(4096);
            }
        }
        let complete = newline.is_some_and(|i| i < len);
        reader.consume(len);
        *remaining -= len as u64;
        if complete {
            return Ok(Some((result, oversized)));
        }
        if *remaining == 0 {
            return Ok(Some((Vec::new(), true)));
        }
    }
}

fn aggregate(
    events: impl Iterator<Item = parser::Event>,
    timezone: Option<FixedOffset>,
) -> LogUsageScan {
    type ModelTotals = BTreeMap<String, (i64, Option<f64>, Option<TokenUsage>)>;
    let mut by_day: BTreeMap<String, ModelTotals> = BTreeMap::new();
    let mut unknown = ModelsByDay::new();
    for event in events {
        let date = day(event.timestamp, timezone).to_string();
        let cost = event.cost.or_else(|| {
            event.token_usage.and_then(|_| {
                uc_pricing::estimate(&event.model, event.tokens, event.request_boundaries_known)
            })
        });
        if cost.is_none() {
            unknown
                .entry(date.clone())
                .or_default()
                .insert(event.model.clone());
        }
        let value = by_day
            .entry(date)
            .or_default()
            .entry(event.model)
            .or_insert((0, Some(0.0), Some(TokenUsage::default())));
        value.0 = value.0.saturating_add(event.total);
        value.1 = value.1.zip(cost).map(|(a, b)| a + b);
        value.2 = value
            .2
            .zip(event.token_usage)
            .map(|(a, b)| a.saturating_add(b));
    }
    let mut series = DailyUsageSeries::default();
    let mut models = ModelUsageSeries::default();
    for (date, entries) in by_day {
        let rows: Vec<_> = entries
            .into_iter()
            .map(|(model, (tokens, cost, token_usage))| {
                let mut row = ModelUsageEntry::new(model, tokens, cost);
                row.token_usage = token_usage;
                row
            })
            .collect();
        let total_tokens = rows
            .iter()
            .fold(0_i64, |a, r| a.saturating_add(r.total_tokens));
        let cost_usd = rows.iter().try_fold(0.0, |a, r| r.cost_usd.map(|c| a + c));
        let token_usage = rows.iter().try_fold(TokenUsage::default(), |a, r| {
            r.token_usage.map(|b| a.saturating_add(b))
        });
        series.daily.push(DailyUsageEntry {
            date: date.clone(),
            total_tokens,
            cost_usd,
            token_usage,
        });
        models
            .daily
            .push(DailyModelUsageEntry { date, models: rows });
    }
    LogUsageScan {
        series,
        model_usage: Some(models),
        unknown_models_by_day: unknown,
        fallback_pricing_models_by_day: None,
    }
}
