use std::fs::{self, File, Metadata};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::parser::{Event, Parser};
use crate::{LogScanner, LogSource, day, linked};

const BATCH_LINES: usize = 512;
const BATCH_BYTES: u64 = 4 * 1024 * 1024;
const PROBE_BYTES: u64 = 4096;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UsageGrouping {
    Day,
    Month,
    Year,
    Model,
    Project,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub group_by: UsageGrouping,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub total_tokens: i64,
    #[serde(rename = "costUSD", skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroupRow {
    pub key: String,
    pub source: String,
    pub totals: UsageTotals,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageLedgerInfo {
    pub first_day: Option<String>,
    pub updated_at: Option<String>,
    pub importing: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    pub files_read: usize,
    pub bytes_read: u64,
    pub events_written: usize,
    pub skipped_files: usize,
    pub skipped_lines: usize,
}

pub struct UsageLedger {
    connection: Mutex<Connection>,
    import_lock: Mutex<()>,
    importing: AtomicBool,
}

struct Checkpoint {
    offset: u64,
    length: u64,
    modified: String,
    prefix: Vec<u8>,
    boundary: Vec<u8>,
    parser: String,
    line_limit: usize,
    complete: bool,
}

struct ImportPass<'a, F> {
    report: ImportReport,
    callback: &'a mut F,
    last_notification: Instant,
}

impl UsageLedger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
            fs::create_dir_all(parent).context("Cannot create usage ledger directory")?;
        }
        let connection = Connection::open(path).context("Cannot open usage ledger")?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA cache_size=-8192;
             PRAGMA temp_store=FILE;",
        )?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != 0 && version != 1 {
            bail!("The usage ledger version is unsupported");
        }
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS usage_events (
                source TEXT NOT NULL, event_key TEXT NOT NULL,
                timestamp INTEGER NOT NULL, day TEXT NOT NULL,
                model TEXT NOT NULL, project TEXT NOT NULL,
                input INTEGER NOT NULL, output INTEGER NOT NULL,
                cached INTEGER NOT NULL, creation INTEGER NOT NULL,
                total INTEGER NOT NULL, cost REAL, sidechain INTEGER NOT NULL,
                PRIMARY KEY(source,event_key)
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS usage_events_day ON usage_events(day,source);
             CREATE TABLE IF NOT EXISTS file_checkpoints (
                source TEXT NOT NULL, path TEXT NOT NULL,
                offset INTEGER NOT NULL, length INTEGER NOT NULL, modified TEXT NOT NULL,
                prefix BLOB NOT NULL, boundary BLOB NOT NULL,
                parser TEXT NOT NULL, line_limit INTEGER NOT NULL, complete INTEGER NOT NULL,
                PRIMARY KEY(source,path)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS ledger_metadata (
                key TEXT PRIMARY KEY, value TEXT NOT NULL
             ) WITHOUT ROWID;
             PRAGMA user_version=1;
             COMMIT;",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
            import_lock: Mutex::new(()),
            importing: AtomicBool::new(false),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("The usage ledger lock failed"))
    }

    pub fn info(&self) -> Result<UsageLedgerInfo> {
        let connection = self.connection()?;
        let first_day = connection.query_row(
            "SELECT MIN(day) FROM usage_events WHERE timestamp<=?1",
            [Utc::now().timestamp_millis()],
            |row| row.get(0),
        )?;
        let updated_at = connection
            .query_row(
                "SELECT value FROM ledger_metadata WHERE key='updatedAt'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(UsageLedgerInfo {
            first_day,
            updated_at,
            importing: self.importing.load(Ordering::Acquire),
        })
    }

    pub fn summary(&self, query: &UsageQuery) -> Result<Vec<UsageGroupRow>> {
        let from = query.from.as_deref().map(parse_date).transpose()?;
        let to = query
            .to
            .as_deref()
            .map(parse_date)
            .transpose()?
            .unwrap_or_else(|| Local::now().date_naive());
        if from.is_some_and(|from| from > to) {
            bail!("Usage date range is reversed");
        }
        let key = match query.group_by {
            UsageGrouping::Day => "day",
            UsageGrouping::Month => "substr(day,1,7)",
            UsageGrouping::Year => "substr(day,1,4)",
            UsageGrouping::Model => "model",
            UsageGrouping::Project => "project",
        };
        let sql = format!(
            "SELECT {key} AS grouping,source,input,output,cached,creation,total,cost
             FROM usage_events WHERE (?1 IS NULL OR day>=?1) AND day<=?2 AND timestamp<=?3
             ORDER BY grouping,source"
        );
        let connection = self.connection()?;
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query(params![
            from.map(|date| date.to_string()),
            to.to_string(),
            Utc::now().timestamp_millis()
        ])?;
        let mut result: Vec<UsageGroupRow> = Vec::new();
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let source: String = row.get(1)?;
            if result
                .last()
                .is_none_or(|last| last.key != key || last.source != source)
            {
                result.push(UsageGroupRow {
                    key,
                    source,
                    totals: UsageTotals::default(),
                });
            }
            let totals = &mut result
                .last_mut()
                .expect("A usage group was inserted")
                .totals;
            totals.input_tokens = totals.input_tokens.saturating_add(row.get(2)?);
            totals.output_tokens = totals.output_tokens.saturating_add(row.get(3)?);
            totals.cached_input_tokens = totals.cached_input_tokens.saturating_add(row.get(4)?);
            totals.cache_creation_input_tokens = totals
                .cache_creation_input_tokens
                .saturating_add(row.get(5)?);
            totals.total_tokens = totals.total_tokens.saturating_add(row.get(6)?);
            if let Some(cost) = row.get::<_, Option<f64>>(7)? {
                totals.cost_usd = Some((totals.cost_usd.unwrap_or(0.0) + cost).min(f64::MAX));
            }
        }
        Ok(result)
    }

    pub fn import(
        &self,
        scanners: &[LogScanner],
        mut callback: impl FnMut(UsageLedgerInfo),
    ) -> Result<ImportReport> {
        let _lock = self
            .import_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("The usage import lock failed"))?;
        self.importing.store(true, Ordering::Release);
        let outcome = (|| {
            callback(self.info()?);
            let mut pass = ImportPass {
                report: ImportReport::default(),
                callback: &mut callback,
                last_notification: Instant::now(),
            };
            for scanner in scanners {
                for root in &scanner.options.roots {
                    if unsafe_path(root) {
                        pass.report.skipped_files += 1;
                        continue;
                    }
                    self.walk(root, 0, scanner, &mut pass)?;
                }
            }
            self.connection()?.execute(
                "INSERT INTO ledger_metadata(key,value) VALUES('updatedAt',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [Utc::now().to_rfc3339()],
            )?;
            Ok(pass.report)
        })();
        self.importing.store(false, Ordering::Release);
        if let Ok(info) = self.info() {
            callback(info);
        }
        outcome
    }

    fn walk<F: FnMut(UsageLedgerInfo)>(
        &self,
        path: &Path,
        depth: usize,
        scanner: &LogScanner,
        pass: &mut ImportPass<'_, F>,
    ) -> Result<()> {
        if depth > 32 {
            pass.report.skipped_files += 1;
            return Ok(());
        }
        if scanner.source == LogSource::Claude
            && depth == 2
            && path.file_name().is_some_and(|name| name == "memory")
        {
            return Ok(());
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => {
                pass.report.skipped_files += 1;
                return Ok(());
            }
        };
        if linked(&metadata) {
            pass.report.skipped_files += 1;
        } else if metadata.is_dir() {
            match fs::read_dir(path) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(entry) => self.walk(&entry.path(), depth + 1, scanner, pass)?,
                            Err(_) => pass.report.skipped_files += 1,
                        }
                    }
                }
                Err(_) => pass.report.skipped_files += 1,
            }
        } else if metadata.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            && let Err(error) = self.import_file(path, scanner, pass)
        {
            if error.downcast_ref::<std::io::Error>().is_some() {
                pass.report.skipped_files += 1;
            } else {
                return Err(error);
            }
        }
        Ok(())
    }

    fn import_file<F: FnMut(UsageLedgerInfo)>(
        &self,
        path: &Path,
        scanner: &LogScanner,
        pass: &mut ImportPass<'_, F>,
    ) -> Result<()> {
        if unsafe_path(path) {
            pass.report.skipped_files += 1;
            return Ok(());
        }
        let source = source_name(scanner.source);
        let path_key = path.to_string_lossy().into_owned();
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        let length = metadata.len();
        if length > i64::MAX as u64 {
            bail!("The local usage log is too large");
        }
        let modified = modified(&metadata);
        let line_limit = scanner.options.max_line_bytes.clamp(1, 4 * 1024 * 1024);
        let saved = self.connection()?.query_row(
            "SELECT offset,length,modified,prefix,boundary,parser,line_limit,complete FROM file_checkpoints WHERE source=?1 AND path=?2",
            params![source,path_key],
            |row| Ok(Checkpoint { offset:row.get::<_,i64>(0)? as u64, length:row.get::<_,i64>(1)? as u64, modified:row.get(2)?, prefix:row.get(3)?, boundary:row.get(4)?, parser:row.get(5)?, line_limit:row.get::<_,i64>(6)? as usize, complete:row.get(7)? }),
        ).optional()?;
        if saved.as_ref().is_some_and(|saved| {
            saved.complete
                && saved.length == length
                && !modified.is_empty()
                && saved.modified == modified
                && saved.line_limit == line_limit
        }) {
            return Ok(());
        }
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let mut parser = Parser::new(scanner.source, path);
        let mut offset = 0;
        if let Some(saved) = saved
            && saved.offset <= length
            && saved.line_limit == line_limit
        {
            let probes = fingerprint(&mut reader, saved.offset)?;
            if probes.0 == saved.prefix
                && probes.1 == saved.boundary
                && (length > saved.length || saved.offset < saved.length || !saved.complete)
                && let Ok(restored) = serde_json::from_str(&saved.parser)
            {
                parser = restored;
                offset = saved.offset;
            }
        }
        reader.seek(SeekFrom::Start(offset))?;
        pass.report.files_read += 1;
        let mut events = Vec::with_capacity(BATCH_LINES);
        let mut batch_lines = 0;
        let mut batch_bytes = 0;
        loop {
            let (line, consumed, complete, oversized) =
                bounded_line(&mut reader, length.saturating_sub(offset), line_limit)?;
            pass.report.bytes_read = pass.report.bytes_read.saturating_add(consumed);
            if !complete {
                break;
            }
            offset += consumed;
            batch_lines += 1;
            batch_bytes += consumed;
            if oversized {
                pass.report.skipped_lines += 1;
            } else if let Ok(value) = serde_json::from_slice(&line) {
                if let Some(event) = parser.parse(&value) {
                    events.push(event);
                }
            } else if line.iter().any(|byte| !byte.is_ascii_whitespace()) {
                pass.report.skipped_lines += 1;
            }
            if batch_lines >= BATCH_LINES || batch_bytes >= BATCH_BYTES {
                self.persist_batch(
                    &mut reader,
                    scanner,
                    &path_key,
                    &metadata,
                    offset,
                    &parser,
                    &mut events,
                    false,
                    pass,
                )?;
                batch_lines = 0;
                batch_bytes = 0;
            }
        }
        self.persist_batch(
            &mut reader,
            scanner,
            &path_key,
            &metadata,
            offset,
            &parser,
            &mut events,
            true,
            pass,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_batch<F: FnMut(UsageLedgerInfo)>(
        &self,
        reader: &mut BufReader<File>,
        scanner: &LogScanner,
        path: &str,
        metadata: &Metadata,
        offset: u64,
        parser: &Parser,
        events: &mut Vec<Event>,
        complete: bool,
        pass: &mut ImportPass<'_, F>,
    ) -> Result<()> {
        let (prefix, boundary) = fingerprint(reader, offset)?;
        let checkpoint = serde_json::to_string(parser)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut written = 0;
        {
            let mut insert = transaction.prepare_cached(
                "INSERT INTO usage_events(source,event_key,timestamp,day,model,project,input,output,cached,creation,total,cost,sidechain)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
                 ON CONFLICT(source,event_key) DO UPDATE SET
                    timestamp=excluded.timestamp,day=excluded.day,model=excluded.model,project=excluded.project,
                    input=excluded.input,output=excluded.output,cached=excluded.cached,creation=excluded.creation,
                    total=excluded.total,cost=excluded.cost,sidechain=excluded.sidechain
                 WHERE excluded.sidechain<usage_events.sidechain
                    OR (excluded.sidechain=usage_events.sidechain AND excluded.total>usage_events.total)"
            )?;
            for event in events.iter() {
                let cost = event.cost.or_else(|| {
                    event.token_usage.and_then(|_| {
                        uc_pricing::estimate(
                            &event.model,
                            event.tokens,
                            event.request_boundaries_known,
                        )
                    })
                });
                written += insert.execute(params![
                    source_name(scanner.source),
                    event.key,
                    event.timestamp.timestamp_millis(),
                    day(event.timestamp, scanner.options.timezone).to_string(),
                    event.model,
                    event.project,
                    event.tokens.prompt(),
                    event.tokens.output,
                    event.tokens.cache_read,
                    event
                        .tokens
                        .cache_write
                        .saturating_add(event.tokens.cache_write_hour),
                    event.total,
                    cost,
                    event.sidechain
                ])?;
            }
        }
        transaction.execute(
            "INSERT INTO file_checkpoints(source,path,offset,length,modified,prefix,boundary,parser,line_limit,complete)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(source,path) DO UPDATE SET offset=excluded.offset,length=excluded.length,
             modified=excluded.modified,prefix=excluded.prefix,boundary=excluded.boundary,parser=excluded.parser,line_limit=excluded.line_limit,complete=excluded.complete",
            params![source_name(scanner.source),path,offset as i64,metadata.len() as i64,modified(metadata),prefix,boundary,checkpoint,scanner.options.max_line_bytes.clamp(1,4 * 1024 * 1024) as i64,complete],
        )?;
        transaction.execute(
            "INSERT INTO ledger_metadata(key,value) VALUES('updatedAt',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
        drop(connection);
        events.clear();
        pass.report.events_written += written;
        if pass.last_notification.elapsed() >= Duration::from_millis(750) {
            (pass.callback)(self.info()?);
            pass.last_notification = Instant::now();
        }
        Ok(())
    }
}

fn parse_date(value: &str) -> Result<NaiveDate> {
    let date =
        NaiveDate::parse_from_str(value, "%Y-%m-%d").context("Usage dates must use YYYY-MM-DD")?;
    if date.to_string() != value {
        bail!("Usage dates must use YYYY-MM-DD");
    }
    Ok(date)
}

fn source_name(source: LogSource) -> &'static str {
    match source {
        LogSource::Claude => "claude",
        LogSource::Codex => "codex",
    }
}

fn modified(metadata: &Metadata) -> String {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_default()
}

fn unsafe_path(path: &Path) -> bool {
    path.ancestors()
        .any(|ancestor| fs::symlink_metadata(ancestor).is_ok_and(|metadata| linked(&metadata)))
}

fn fingerprint(
    reader: &mut (impl Read + Seek),
    offset: u64,
) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let position = reader.stream_position()?;
    let size = offset.min(PROBE_BYTES) as usize;
    let mut buffer = vec![0; size];
    reader.seek(SeekFrom::Start(0))?;
    reader.read_exact(&mut buffer)?;
    let prefix = Sha256::digest(&buffer).to_vec();
    reader.seek(SeekFrom::Start(offset.saturating_sub(PROBE_BYTES)))?;
    reader.read_exact(&mut buffer)?;
    let boundary = Sha256::digest(&buffer).to_vec();
    reader.seek(SeekFrom::Start(position))?;
    Ok((prefix, boundary))
}

fn bounded_line(
    reader: &mut impl BufRead,
    available: u64,
    maximum: usize,
) -> std::io::Result<(Vec<u8>, u64, bool, bool)> {
    let mut line = Vec::new();
    let mut consumed = 0;
    let mut oversized = false;
    while consumed < available {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            break;
        }
        let limit = buffer
            .len()
            .min((available - consumed).min(usize::MAX as u64) as usize);
        let buffer = &buffer[..limit];
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let length = newline.map_or(buffer.len(), |position| position + 1);
        if !oversized && line.len().saturating_add(length) <= maximum {
            line.extend_from_slice(&buffer[..length]);
        } else {
            oversized = true;
            line.clear();
        }
        reader.consume(length);
        consumed += length as u64;
        if newline.is_some() {
            return Ok((line, consumed, true, oversized));
        }
    }
    Ok((line, consumed, false, oversized))
}
