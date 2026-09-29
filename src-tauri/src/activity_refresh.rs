//! Reads Claude and Codex limits again while they are being used on this computer. Claude Code and
//! the Codex CLI append to their session logs as they work; when a log grows, every card of that
//! brand is read again soon, so the numbers go down while the work goes on instead of at the next
//! scheduled refresh. Only file sizes and times are looked at; nothing in the logs is read.
//!
//! How soon depends on what the provider allows ([`Brand::pace`]). A provider that refuses a
//! reading for asking too often is left to the engine's interval for [`QUIET`], and for twice as
//! long each time it refuses again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tauri::{AppHandle, Manager};
use uc_engine::Engine;

use crate::service::BackendService;

/// How often the session logs are looked at.
const TICK: Duration = Duration::from_secs(5);
/// The most log entries one look visits.
const BUDGET: usize = 50_000;
/// How long a brand keeps to the engine's interval after a provider refused a reading for asking
/// too often.
const QUIET: Duration = Duration::from_secs(30 * 60);
/// The longest such time.
const QUIET_CAP: Duration = Duration::from_secs(4 * 3600);
/// A brand that was not refused for this long after its quiet time starts again from [`QUIET`].
const FORGIVEN_AFTER: Duration = Duration::from_secs(6 * 3600);

/// A brand whose use on this computer shows in its session logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Brand {
    Claude,
    Codex,
}

/// How soon a brand's cards are read again outside the engine's interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pace {
    /// The shortest time between two readings the brand's use asks for.
    in_use: Duration,
    /// How often the cards are read while the logs stay still; `None` leaves that to the engine's
    /// interval.
    steady: Option<Duration>,
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

    /// Anthropic answers about 60 usage readings an hour for one account, counting every computer
    /// that watches it. Measured on 29/09/2026: two computers reading every 30 seconds were served
    /// 59 an hour and refused every six minutes, while 9 to 19 an hour had never been refused. One
    /// reading every three minutes here leaves room for two more computers and for Claude's own
    /// apps. OpenAI served 64 readings an hour for one account without refusing any.
    fn pace(self) -> Pace {
        match self {
            Brand::Claude => Pace {
                in_use: Duration::from_secs(3 * 60),
                steady: None,
            },
            Brand::Codex => Pace {
                in_use: Duration::from_secs(30),
                steady: Some(Duration::from_secs(2 * 60)),
            },
        }
    }
}

/// When a brand was last seen working, last read because of it, and last refused.
struct Watch {
    brand: Brand,
    roots: Vec<PathBuf>,
    depth: usize,
    newest: Option<SystemTime>,
    pending: bool,
    read_at: Option<Instant>,
    refusals: u32,
    quiet_until: Option<Instant>,
}

impl Watch {
    fn new(brand: Brand, roots: Vec<PathBuf>, depth: usize, read_at: Option<Instant>) -> Self {
        Self {
            brand,
            roots,
            depth,
            newest: None,
            pending: false,
            read_at,
            refusals: 0,
            quiet_until: None,
        }
    }

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

    /// Whether the brand keeps to the engine's interval because a provider refused a reading.
    fn quiet(&self, now: Instant) -> bool {
        self.quiet_until.is_some_and(|until| now < until)
    }

    /// A provider refused one of the brand's cards for asking too often: how long the brand now
    /// keeps to the engine's interval, or `None` while it already does.
    fn refused(&mut self, now: Instant) -> Option<Duration> {
        if self.quiet(now) {
            return None;
        }
        let again = self
            .quiet_until
            .is_some_and(|until| now.saturating_duration_since(until) < FORGIVEN_AFTER);
        self.refusals = if again {
            self.refusals.saturating_add(1)
        } else {
            1
        };
        let doubled = 1_u32 << (self.refusals - 1).min(8);
        let quiet = QUIET.saturating_mul(doubled).min(QUIET_CAP);
        self.quiet_until = Some(now + quiet);
        Some(quiet)
    }

    /// Whether a reading is due: the logs changed and the last reading is one in-use gap old, or
    /// the last reading is one steady gap old. Never while the brand is quiet.
    fn due(&self, now: Instant) -> bool {
        if self.quiet(now) {
            return false;
        }
        let since = self
            .read_at
            .map_or(Duration::MAX, |at| now.saturating_duration_since(at));
        let pace = self.brand.pace();
        (self.pending && since >= pace.in_use) || pace.steady.is_some_and(|gap| since >= gap)
    }

    /// A reading starts now; true when the brand's use asked for it.
    fn read(&mut self, now: Instant) -> bool {
        let in_use = self.pending;
        self.pending = false;
        self.read_at = Some(now);
        in_use
    }
}

pub async fn run(app: AppHandle) {
    let home = uc_core::paths::home_dir();
    let claude =
        uc_core::paths::env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"));
    let codex = uc_core::paths::env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
    let started = Some(Instant::now());
    let mut watches = vec![
        Watch::new(Brand::Claude, vec![claude.join("projects")], 4, started),
        Watch::new(Brand::Codex, vec![codex.join("sessions")], 5, started),
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
        let engine = app.state::<BackendService>().engine();
        for watch in &mut watches {
            let cards = enabled_cards(&engine, watch.brand);
            if cards.iter().any(|id| engine.rate_limited(id))
                && let Some(quiet) = watch.refused(now)
            {
                tracing::warn!(
                    target: "refresh",
                    "{:?} refused a reading for asking too often: its cards keep the regular interval for {} minutes",
                    watch.brand,
                    quiet.as_secs() / 60
                );
            }
            if cards.is_empty() || !watch.due(now) {
                continue;
            }
            let in_use = watch.read(now);
            read_cards(&engine, watch.brand, cards, in_use).await;
        }
    }
}

/// The enabled cards of `brand`.
fn enabled_cards(engine: &Engine, brand: Brand) -> Vec<String> {
    engine
        .provider_ids()
        .into_iter()
        .filter(|id| brand.owns(id) && engine.is_enabled(id))
        .collect()
}

/// Read `cards` again, past their cached readings.
async fn read_cards(engine: &Arc<Engine>, brand: Brand, cards: Vec<String>, in_use: bool) {
    let why = if in_use { "in use here" } else { "steady read" };
    tracing::info!(target: "refresh", "{brand:?} {why}: reading {} cards again", cards.len());
    let mut reads = tokio::task::JoinSet::new();
    for id in cards {
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

    const HOUR: Duration = Duration::from_secs(3600);

    fn watch(brand: Brand, read_at: Option<Instant>) -> Watch {
        Watch::new(brand, Vec::new(), 0, read_at)
    }

    /// How many readings `watch` asks for from `start` for `span` while its logs grow at every
    /// look.
    fn readings_while_working(watch: &mut Watch, start: Instant, span: Duration) -> usize {
        let looks = span.as_secs() / TICK.as_secs();
        (1..=looks)
            .filter(|look| {
                let now = start + TICK * u32::try_from(*look).unwrap();
                watch.pending = true;
                let due = watch.due(now);
                if due {
                    watch.read(now);
                }
                due
            })
            .count()
    }

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
        let mut watch = Watch::new(Brand::Codex, vec![dir.path().to_path_buf()], 4, None);
        let pace = Brand::Codex.pace();
        let steady = pace.steady.unwrap();
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
        assert!(watch.read(now));
        assert!(!watch.due(now + Duration::from_secs(5)));
        watch.pending = true;
        assert!(!watch.due(now + Duration::from_secs(5)));
        assert!(watch.due(now + pace.in_use));
        watch.pending = false;
        assert!(!watch.due(now + pace.in_use));
        assert!(watch.due(now + steady));
    }

    #[test]
    fn a_busy_hour_asks_claude_for_a_third_of_what_anthropic_allows() {
        let start = Instant::now();
        let mut claude = watch(Brand::Claude, Some(start));
        assert_eq!(readings_while_working(&mut claude, start, HOUR), 20);
        let mut codex = watch(Brand::Codex, Some(start));
        assert_eq!(readings_while_working(&mut codex, start, HOUR), 120);
    }

    #[test]
    fn idle_claude_cards_are_left_to_the_interval() {
        let start = Instant::now();
        let claude = watch(Brand::Claude, Some(start));
        assert!(!claude.due(start + 24 * HOUR));
        let codex = watch(Brand::Codex, Some(start));
        assert!(codex.due(start + Brand::Codex.pace().steady.unwrap()));
    }

    #[test]
    fn a_refusal_leaves_the_brand_to_the_interval_for_a_while() {
        let start = Instant::now();
        let mut claude = watch(Brand::Claude, Some(start));
        let refused = start + Duration::from_secs(17 * 60);
        assert_eq!(claude.refused(refused), Some(QUIET));
        assert_eq!(
            claude.refused(refused + TICK),
            None,
            "one refusal is counted once"
        );
        assert_eq!(
            readings_while_working(&mut claude, refused, QUIET - TICK),
            0
        );
        claude.pending = true;
        assert!(claude.due(refused + QUIET));
    }

    #[test]
    fn refusals_in_a_row_double_the_quiet_time_up_to_the_cap() {
        let mut claude = watch(Brand::Claude, None);
        let mut now = Instant::now();
        let mut quiets = Vec::new();
        for _ in 0..6 {
            let quiet = claude.refused(now).unwrap();
            quiets.push(quiet.as_secs() / 60);
            now += quiet + Duration::from_secs(10 * 60);
        }
        assert_eq!(quiets, [30, 60, 120, 240, 240, 240]);
    }

    #[test]
    fn a_long_time_without_a_refusal_starts_again_from_the_shortest_quiet_time() {
        let mut claude = watch(Brand::Claude, None);
        let start = Instant::now();
        assert_eq!(claude.refused(start), Some(QUIET));
        let soon = start + QUIET + HOUR;
        assert_eq!(claude.refused(soon), Some(QUIET * 2));
        let much_later = soon + QUIET * 2 + FORGIVEN_AFTER;
        assert_eq!(claude.refused(much_later), Some(QUIET));
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
