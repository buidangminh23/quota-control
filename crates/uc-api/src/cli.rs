//! `usagectl`: print the `/v1/limits` JSON for every enabled provider, or for the providers one
//! token names, then exit (upstream `OpenUsageCLI` and `UsageReader`). It shares the app's providers,
//! documents and snapshot cache, reuses entries younger than five minutes and refreshes the rest;
//! `--force` refreshes regardless. The app does not need to be running.
//!
//! Exit codes: 0 success, 2 invalid arguments or an unknown provider, 4 a refresh or read failure
//! (the JSON is still printed, with the failure in its `errors`).

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use uc_accounts::AccountStore;
use uc_core::{Clock, ProviderRuntime, system_clock};
use uc_engine::{DocumentName, DocumentStore, EngineConfig, SnapshotCache};

use crate::state::matching_ids;
use crate::{ApiState, build_engine, provider_runtimes, respond};

pub const NAME: &str = "usagectl";

pub const HELP: &str = "Usage: usagectl [provider] [--force]

Read limits through Quota Control's shared five-minute cache and exit. Output is always JSON.

A provider is an exact card id (codex@1a2b...) or a family (claude, codex) naming all of its cards.

Options:
  --force      Refresh even when the shared cache is still fresh
  -v, --version
  -h, --help";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arguments {
    pub provider: Option<String>,
    pub force: bool,
    pub help: bool,
    pub version: bool,
}

pub fn parse<I: IntoIterator<Item = String>>(arguments: I) -> Result<Arguments, String> {
    let mut parsed = Arguments::default();
    for argument in arguments {
        match argument.as_str() {
            "--force" => parsed.force = true,
            "-h" | "--help" => parsed.help = true,
            "-v" | "--version" => parsed.version = true,
            option if option.starts_with('-') => return Err(format!("Unknown option: {option}")),
            provider => {
                if parsed.provider.is_some() {
                    return Err("Only one provider can be requested at a time.".into());
                }
                parsed.provider = Some(provider.to_lowercase());
            }
        }
    }
    Ok(parsed)
}

/// Where a one-shot read finds the app's providers and files.
pub struct Environment {
    pub runtimes: Vec<Arc<dyn ProviderRuntime>>,
    pub documents: DocumentStore,
    pub cache_path: PathBuf,
    pub clock: Clock,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub code: i32,
}

impl Output {
    fn fail(message: &str, code: i32) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: format!("{NAME}: {message}\n"),
            code,
        }
    }
}

/// Refresh what the request needs through the shared engine and cache, then render the limits.
pub async fn read(arguments: &Arguments, environment: Environment) -> Output {
    let config = EngineConfig::default();
    let cache = SnapshotCache::with_options(
        environment.cache_path,
        config.refresh_interval,
        true,
        environment.clock.clone(),
    );
    let engine = build_engine(
        environment.runtimes,
        cache,
        config,
        environment.clock.clone(),
    );
    let known = engine.provider_ids();
    let selected = match &arguments.provider {
        Some(token) => {
            let ids = matching_ids(&known, token);
            if ids.is_empty() {
                return Output::fail(&format!("Unknown provider: {token}"), 2);
            }
            ids
        }
        None => enabled_ids(&environment.documents, &known),
    };
    engine.set_enabled(&selected);
    let exporting: HashSet<String> = engine
        .catalog()
        .into_iter()
        .filter(|entry| {
            entry
                .descriptors
                .iter()
                .any(|descriptor| !descriptor.limit_resources.is_empty())
        })
        .map(|entry| entry.provider.id)
        .collect();
    let refreshed: Vec<&String> = selected
        .iter()
        .filter(|id| exporting.contains(*id))
        .collect();
    futures::future::join_all(
        refreshed
            .iter()
            .map(|id| engine.refresh(id, arguments.force)),
    )
    .await;

    let state = ApiState::capture(
        &engine,
        &saved_order(&environment.documents),
        (environment.clock)(),
    );
    let path = match &arguments.provider {
        Some(token) => format!("/v1/limits/{token}"),
        None => "/v1/limits".into(),
    };
    let Some(body) = respond("GET", &path, &state).body else {
        return Output::fail("The local read produced no data.", 4);
    };
    let warnings: Vec<String> = refreshed
        .iter()
        .filter_map(|id| {
            state
                .errors
                .get(*id)
                .map(|message| format!("{NAME}: warning: {id}: {message}\n"))
        })
        .collect();
    let mut stdout = ascii_json(&body);
    stdout.push(b'\n');
    Output {
        stdout,
        code: if warnings.is_empty() { 0 } else { 4 },
        stderr: warnings.concat(),
    }
}

/// The providers the dashboard refreshes; every known one when the settings leave it at the
/// default or cannot be read, as the app does.
fn enabled_ids(documents: &DocumentStore, known: &[String]) -> Vec<String> {
    let stored = documents
        .load(DocumentName::Settings)
        .ok()
        .flatten()
        .and_then(|settings| settings.get("enabledProviders").cloned())
        .and_then(|ids| serde_json::from_value::<Vec<String>>(ids).ok());
    match stored {
        Some(ids) => known
            .iter()
            .filter(|id| ids.contains(id))
            .cloned()
            .collect(),
        None => known.to_vec(),
    }
}

fn saved_order(documents: &DocumentStore) -> Vec<String> {
    documents
        .load(DocumentName::Layout)
        .ok()
        .flatten()
        .and_then(|layout| layout.get("providerOrder").cloned())
        .and_then(|order| serde_json::from_value(order).ok())
        .unwrap_or_default()
}

/// JSON with every non-ASCII character escaped: identical once parsed, and it survives consoles
/// and shells that do not decode UTF-8 (Windows PowerShell reads native output in the OEM page).
fn ascii_json(body: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(body);
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii() {
            out.push(ch);
        } else {
            let mut units = [0_u16; 2];
            for unit in ch.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out.into_bytes()
}

/// The whole command: parse, read, print. Returns the process exit code.
pub fn main<I: IntoIterator<Item = String>>(arguments: I) -> i32 {
    let arguments = match parse(arguments) {
        Ok(arguments) => arguments,
        Err(message) => {
            eprintln!("{NAME}: {message}\nRun '{NAME} --help' for usage.");
            return 2;
        }
    };
    if arguments.help {
        println!("{HELP}");
        return 0;
    }
    if arguments.version {
        println!("{NAME} {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    let runtimes = match provider_runtimes(Arc::new(AccountStore::default_store())) {
        Ok(runtimes) => runtimes,
        Err(error) => {
            eprintln!("{NAME}: {error}");
            return 4;
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{NAME}: {error}");
            return 4;
        }
    };
    let environment = Environment {
        runtimes,
        documents: DocumentStore::default_store(),
        cache_path: SnapshotCache::default_path(),
        clock: system_clock(),
    };
    let output = runtime.block_on(read(&arguments, environment));
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(&output.stdout)
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return 4;
    }
    eprint!("{}", output.stderr);
    output.code
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use async_trait::async_trait;
    use chrono::{DateTime, Duration, Utc};
    use serde_json::Value;
    use uc_core::{
        ErrorCategory, MetricLine, ProgressFormat, Provider, ProviderSnapshot, RefreshContext,
        WidgetDescriptor, fixed_clock,
    };

    use super::*;

    struct Fake {
        provider: Provider,
        exports: bool,
        calls: AtomicUsize,
        fail: AtomicBool,
        now: DateTime<Utc>,
    }

    impl Fake {
        fn new(id: &str, name: &str, exports: bool, now: DateTime<Utc>) -> Arc<Self> {
            Arc::new(Self {
                provider: Provider::new(id, name),
                exports,
                calls: AtomicUsize::new(0),
                fail: AtomicBool::new(false),
                now,
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ProviderRuntime for Fake {
        fn provider(&self) -> &Provider {
            &self.provider
        }

        fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
            let session = WidgetDescriptor::percent(
                format!("{}.session", self.provider.id),
                &self.provider,
                "Session",
                None,
                None,
            );
            vec![if self.exports {
                session.exporting_progress("session", "percent")
            } else {
                session
            }]
        }

        async fn refresh(&self, _: RefreshContext) -> ProviderSnapshot {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return ProviderSnapshot::error_message(
                    &self.provider,
                    "Session expired.",
                    Some(ErrorCategory::AuthExpired),
                );
            }
            let line = MetricLine::progress("Session", 25.0, 100.0, ProgressFormat::Percent);
            ProviderSnapshot::make(&self.provider, None, vec![line.into()], self.now)
        }

        async fn has_local_credentials(&self) -> bool {
            true
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        now: DateTime<Utc>,
        work: Arc<Fake>,
        home: Arc<Fake>,
        local: Arc<Fake>,
    }

    impl Fixture {
        fn new() -> Self {
            let now = DateTime::parse_from_rfc3339("2026-09-26T08:00:00Z")
                .unwrap()
                .with_timezone(&Utc);
            Self {
                dir: tempfile::tempdir().unwrap(),
                now,
                work: Fake::new("codex@aa", "Codex · Công việc", true, now),
                home: Fake::new("claude@bb", "Claude · Home", true, now),
                local: Fake::new("claude-local", "Claude Local Usage", false, now),
            }
        }

        fn documents(&self) -> DocumentStore {
            DocumentStore::new(self.dir.path().join("config"))
        }

        fn environment(&self, at: DateTime<Utc>) -> Environment {
            Environment {
                runtimes: vec![self.work.clone(), self.home.clone(), self.local.clone()],
                documents: self.documents(),
                cache_path: self.dir.path().join("cache.json"),
                clock: fixed_clock(at),
            }
        }

        async fn run(&self, arguments: &[&str], at: DateTime<Utc>) -> (Value, Output) {
            let arguments = parse(arguments.iter().map(|argument| argument.to_string())).unwrap();
            let output = read(&arguments, self.environment(at)).await;
            let json = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
            (json, output)
        }
    }

    #[test]
    fn arguments_match_upstream() {
        let parsed = parse(["CODEX".to_string(), "--force".to_string()]).unwrap();
        assert_eq!(parsed.provider.as_deref(), Some("codex"));
        assert!(parsed.force);
        assert!(parse(["-h".to_string()]).unwrap().help);
        assert!(parse(["--version".to_string()]).unwrap().version);
        assert_eq!(
            parse(["--json".to_string()]).unwrap_err(),
            "Unknown option: --json"
        );
        assert!(parse(["claude".to_string(), "codex".to_string()]).is_err());
    }

    #[tokio::test]
    async fn enabled_exporting_providers_refresh_once_and_then_read_the_cache() {
        let fixture = Fixture::new();
        fixture
            .documents()
            .save(
                DocumentName::Settings,
                &serde_json::json!({"enabledProviders": ["codex@aa", "claude-local"]}),
            )
            .unwrap();
        let (json, output) = fixture.run(&[], fixture.now).await;
        assert_eq!(output.code, 0, "{}", output.stderr);
        assert_eq!(json["schema"], crate::SCHEMA);
        assert!(json["errors"].as_array().unwrap().is_empty());
        assert_eq!(json["providers"].as_object().unwrap().len(), 1);
        assert_eq!(
            json["providers"]["codex@aa"]["resources"]["session"]["used"],
            25
        );
        assert_eq!(
            (
                fixture.work.calls(),
                fixture.home.calls(),
                fixture.local.calls()
            ),
            (1, 0, 0)
        );

        let later = fixture.now + Duration::minutes(4);
        let (_, output) = fixture.run(&[], later).await;
        assert_eq!(output.code, 0);
        assert_eq!(fixture.work.calls(), 1);
        fixture.run(&["--force"], later).await;
        assert_eq!(fixture.work.calls(), 2);
        fixture.run(&[], later + Duration::minutes(6)).await;
        assert_eq!(fixture.work.calls(), 3);
    }

    #[tokio::test]
    async fn a_named_family_is_read_even_when_disabled() {
        let fixture = Fixture::new();
        fixture
            .documents()
            .save(
                DocumentName::Settings,
                &serde_json::json!({"enabledProviders": []}),
            )
            .unwrap();
        let (json, output) = fixture.run(&["claude"], fixture.now).await;
        assert_eq!(output.code, 0);
        let providers = json["providers"].as_object().unwrap();
        assert_eq!(providers.keys().collect::<Vec<_>>(), ["claude@bb"]);
        assert_eq!((fixture.home.calls(), fixture.local.calls()), (1, 0));
        let (json, output) = fixture.run(&["claude-local"], fixture.now).await;
        assert_eq!(
            (output.code, json["providers"].as_object().unwrap().len()),
            (0, 0)
        );
    }

    #[tokio::test]
    async fn an_unknown_provider_is_a_usage_error() {
        let fixture = Fixture::new();
        let (_, output) = fixture.run(&["cursor"], fixture.now).await;
        assert_eq!(output.code, 2);
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, "usagectl: Unknown provider: cursor\n");
    }

    #[tokio::test]
    async fn a_failed_refresh_still_prints_and_exits_four() {
        let fixture = Fixture::new();
        fixture.run(&["codex"], fixture.now).await;
        fixture.work.fail.store(true, Ordering::SeqCst);
        let (json, output) = fixture.run(&["codex", "--force"], fixture.now).await;
        assert_eq!(output.code, 4);
        assert_eq!(
            output.stderr,
            "usagectl: warning: codex@aa: Session expired.\n"
        );
        assert_eq!(json["errors"][0]["providerId"], "codex@aa");
        assert_eq!(
            json["providers"]["codex@aa"]["resources"]["session"]["used"],
            25
        );
    }

    #[tokio::test]
    async fn output_is_ascii_json_that_parses_back_to_the_same_text() {
        let fixture = Fixture::new();
        let (json, output) = fixture.run(&["codex"], fixture.now).await;
        assert!(output.stdout.is_ascii());
        assert!(output.stdout.ends_with(b"\n"));
        assert_eq!(
            json["providers"]["codex@aa"]["displayName"],
            "Codex · Công việc"
        );
    }
}
