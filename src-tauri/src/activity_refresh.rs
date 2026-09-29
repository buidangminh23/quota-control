//! Reads Claude and Codex limits again while they are being used on this computer. Claude Code and
//! the Codex CLI append to their session logs as they work; when a log grows, every Claude (or
//! Codex) card is read again soon, so the numbers go down while the work goes on instead of at the
//! next scheduled refresh. Only file sizes and times are looked at; nothing in the logs is read.
//! Claude and Codex cards are also read every [`STEADY_GAP`] when idle, so two computers that
//! watch the same account stay within that of each other while the work happens on the other one.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use tauri::{AppHandle, Manager};

use crate::service::BackendService;

/// How often the session logs are looked at.
const TICK: Duration = Duration::from_secs(5);
/// The shortest time between two readings a brand's use asks for.
const IN_USE_GAP: Duration = Duration::from_secs(30);
/// How often a brand's cards are read when its logs stay still.
const STEADY_GAP: Duration = Duration::from_secs(2 * 60);
/// The most log entries one look visits.
const BUDGET: usize = 50_000;

/// A brand whose use on this computer shows in its session logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Brand {
    Claude,
    Codex,
}

impl Brand {
    /// Whether the card `id` belongs to this brand: its accounts and its local history.
    fn owns(self, id: &str) -> bool {
        let family = match self {
            Brand::Claude => "claude",
            Brand::Codex => "codex",
        };
        id.strip_prefix(family)
            .is_some_and(|rest| rest.starts_with('@') || rest == "-local")
    }
}

/// When a brand was last seen working and last read because of it.
struct Watch {
    brand: Brand,
    roots: Vec<PathBuf>,
    depth: usize,
    newest: Option<SystemTime>,
    pending: bool,
    read_at: Option<Instant>,
}

impl Watch {
    /// Look at the logs; true when they changed since the last look.
    fn look(&mut self) -> bool {
        let newest = self
            .roots
            .iter()
            .filter_map(|root| newest_log(root, self.depth))
            .max();
        let changed = newest.is_some() && self.newest.is_some() && newest > self.newest;
        if newest > self.newest {
            self.newest = newest;
        }
        changed
    }

    /// Whether a reading is due: the logs changed and the last one is [`IN_USE_GAP`] old, or the
    /// last one is [`STEADY_GAP`] old.
    fn due(&self, now: Instant) -> bool {
        let since = self
            .read_at
            .map_or(Duration::MAX, |at| now.saturating_duration_since(at));
        (self.pending && since >= IN_USE_GAP) || since >= STEADY_GAP
    }
}

pub async fn run(app: AppHandle) {
    let home = uc_core::paths::home_dir();
    let claude =
        uc_core::paths::env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"));
    let codex = uc_core::paths::env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
    let started = Some(Instant::now());
    let mut watches = vec![
        Watch {
            brand: Brand::Claude,
            roots: vec![claude.join("projects")],
            depth: 4,
            newest: None,
            pending: false,
            read_at: started,
        },
        Watch {
            brand: Brand::Codex,
            roots: vec![codex.join("sessions")],
            depth: 5,
            newest: None,
            pending: false,
            read_at: started,
        },
    ];
    let mut ticks = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        let looked = tauri::async_runtime::spawn_blocking(move || {
            for watch in &mut watches {
                if watch.look() {
                    watch.pending = true;
                }
            }
            watches
        })
        .await;
        let Ok(looked) = looked else {
            return;
        };
        watches = looked;
        let now = Instant::now();
        for watch in watches.iter_mut().filter(|watch| watch.due(now)) {
            let in_use = watch.pending;
            watch.pending = false;
            watch.read_at = Some(now);
            read_brand(&app, watch.brand, in_use).await;
        }
    }
}

/// Read every enabled card of `brand` again, past its cached reading.
async fn read_brand(app: &AppHandle, brand: Brand, in_use: bool) {
    let engine = app.state::<BackendService>().engine();
    let ids: Vec<String> = engine
        .provider_ids()
        .into_iter()
        .filter(|id| brand.owns(id) && engine.is_enabled(id))
        .collect();
    if ids.is_empty() {
        return;
    }
    let why = if in_use { "in use here" } else { "steady read" };
    tracing::info!(target: "refresh", "{brand:?} {why}: reading {} cards again", ids.len());
    let mut reads = tokio::task::JoinSet::new();
    for id in ids {
        let engine = engine.clone();
        reads.spawn(async move {
            engine.refresh_in_use(&id).await;
        });
    }
    while reads.join_next().await.is_some() {}
}

/// The newest modification time of a `.jsonl` log under `root`, down to `depth` folders.
fn newest_log(root: &Path, depth: usize) -> Option<SystemTime> {
    let mut budget = BUDGET;
    let mut newest = None;
    visit(root, depth, &mut budget, &mut newest);
    newest
}

fn visit(dir: &Path, depth: usize, budget: &mut usize, newest: &mut Option<SystemTime>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            if depth > 0 {
                visit(&path, depth - 1, budget, newest);
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
            && let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified())
            && newest.is_none_or(|known| modified > known)
        {
            *newest = Some(modified);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_brand_owns_its_accounts_and_its_local_history_only() {
        assert!(Brand::Claude.owns("claude@abc"));
        assert!(Brand::Claude.owns("claude-local"));
        assert!(!Brand::Claude.owns("claude-code@abc"));
        assert!(!Brand::Claude.owns("codex@abc"));
        assert!(Brand::Codex.owns("codex-local"));
        assert!(!Brand::Codex.owns("antigravity@abc"));
    }

    #[test]
    fn a_growing_log_asks_for_a_reading_at_most_once_per_gap() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let log = project.join("session.jsonl");
        std::fs::write(&log, "{}\n").unwrap();
        let mut watch = Watch {
            brand: Brand::Claude,
            roots: vec![dir.path().to_path_buf()],
            depth: 4,
            newest: None,
            pending: false,
            read_at: None,
        };
        assert!(
            !watch.look(),
            "the first look only learns where the logs stand"
        );
        assert!(!watch.look());
        let later = SystemTime::now() + Duration::from_secs(60);
        std::fs::File::options()
            .append(true)
            .open(&log)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(watch.look());
        watch.pending = true;
        let now = Instant::now();
        assert!(watch.due(now));
        watch.read_at = Some(now);
        assert!(!watch.due(now + Duration::from_secs(5)));
        assert!(watch.due(now + IN_USE_GAP));
        watch.pending = false;
        assert!(!watch.due(now + IN_USE_GAP));
        assert!(watch.due(now + STEADY_GAP));
    }

    #[test]
    fn logs_deeper_than_the_depth_and_other_files_are_not_counted() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("x.jsonl"), "").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "").unwrap();
        assert!(newest_log(dir.path(), 1).is_none());
        assert!(newest_log(dir.path(), 2).is_some());
    }
}
