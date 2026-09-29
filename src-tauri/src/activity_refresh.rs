//! Reads Claude and Codex limits at the same moments on every computer, and sooner while they are
//! being used on this one. Claude Code and the Codex CLI append to their session logs as they
//! work; when a log grows, the brand's cards are read again soon, so the numbers go down while the
//! work goes on. Only file sizes and times are looked at; nothing in the logs is read.
//!
//! A brand's accounts are read when the clock passes a multiple of their pace ([`Brand::pace`]),
//! counted from 1970 and not from the reading before. Computers that watch one account therefore
//! ask for it at the same moments and show the same numbers. A reading the engine holds back
//! (the provider asked to be left alone, or the card is being read already) is asked for again at
//! every look until it is answered, so a short wait does not cost a whole span. A provider that
//! refuses a reading for asking too often is asked at [`QUIET_PACE`] for [`QUIET`], and for twice
//! as long each time it refuses again. A brand's history on this computer is read from its own
//! logs, so it keeps [`LOCAL_GAP`] whatever the provider says.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tauri::{AppHandle, Manager};
use uc_engine::{Engine, RefreshOutcome};

use crate::service::BackendService;

/// How often the session logs are looked at, on multiples of it on the clock.
const TICK: Duration = Duration::from_secs(5);
/// The most log entries one look visits.
const BUDGET: usize = 50_000;
/// The shortest time between two readings of a brand's history on this computer while its logs
/// grow.
const LOCAL_GAP: Duration = Duration::from_secs(30);
/// How often a brand's accounts are read after a provider refused a reading for asking too often:
/// as often as before the readings followed the use, which no provider refused.
const QUIET_PACE: Duration = Duration::from_secs(5 * 60);
/// How long a brand's accounts keep [`QUIET_PACE`] after a refusal.
const QUIET: Duration = Duration::from_secs(30 * 60);
/// The longest such time.
const QUIET_CAP: Duration = Duration::from_secs(4 * 3600);
/// A brand that was not refused for this long after its quiet time starts again from [`QUIET`].
const FORGIVEN_AFTER: Duration = Duration::from_secs(6 * 3600);

/// Seconds since 1970 on this computer's clock.
type Seconds = u64;

fn clock() -> Seconds {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Which span of `every` the moment `at` falls in. The spans are the same on every computer.
fn span(at: Seconds, every: Duration) -> u64 {
    at / every.as_secs().max(1)
}

/// How long until the clock reaches the next multiple of [`TICK`].
fn until_next_tick() -> Duration {
    let tick = TICK.as_millis();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    Duration::from_millis(u64::try_from(tick - now % tick).unwrap_or(0))
}

/// A brand whose use on this computer shows in its session logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Brand {
    Claude,
    Codex,
}

/// How often a brand's accounts are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pace {
    /// While the brand is used on this computer.
    in_use: Duration,
    /// While its logs stay still.
    steady: Duration,
}

impl Brand {
    fn family(self) -> &'static str {
        match self {
            Brand::Claude => "claude",
            Brand::Codex => "codex",
        }
    }

    /// Whether the card `id` is one of this brand's accounts, which its provider answers for.
    fn owns_account(self, id: &str) -> bool {
        id.strip_prefix(self.family())
            .is_some_and(|rest| rest.starts_with('@'))
    }

    /// Whether the card `id` is this brand's history on this computer.
    fn owns_history(self, id: &str) -> bool {
        id.strip_prefix(self.family()) == Some("-local")
    }

    /// Anthropic answers about 60 usage readings an hour for one account, counting every computer
    /// that watches it. Measured on 29/09/2026: two computers reading every 30 seconds were served
    /// 59 an hour and refused every six minutes, while 9 to 19 an hour had never been refused.
    /// Every computer now asks 15 times an hour, in use or not, so that they all show the same
    /// numbers. Three computers watching one account ask 45 times an hour, which leaves a quarter
    /// of what Anthropic answers for the readings after a limit reset and after a launch. The
    /// pace stays under the five minutes a reading is kept, so the engine's own batch finds the
    /// accounts read. OpenAI served 64 readings an hour for one account without refusing any.
    fn pace(self) -> Pace {
        match self {
            Brand::Claude => Pace {
                in_use: Duration::from_secs(4 * 60),
                steady: Duration::from_secs(4 * 60),
            },
            Brand::Codex => Pace {
                in_use: Duration::from_secs(30),
                steady: Duration::from_secs(2 * 60),
            },
        }
    }
}

/// When some cards were last read from here, and whether the logs changed since.
#[derive(Default)]
struct Reader {
    pending: bool,
    read_at: Option<Seconds>,
}

impl Reader {
    fn since(&self, now: Seconds) -> Duration {
        self.read_at.map_or(Duration::MAX, |at| {
            Duration::from_secs(now.saturating_sub(at))
        })
    }

    /// Whether the clock passed a multiple of `every` since the cards were read.
    fn passed(&self, now: Seconds, every: Duration) -> bool {
        self.read_at
            .is_none_or(|at| span(at, every) != span(now, every))
    }

    fn read(&mut self, now: Seconds) {
        self.pending = false;
        self.read_at = Some(now);
    }
}

/// Why a brand's accounts are read now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    InUse,
    Steady,
    Refused,
}

/// A reading of a brand's accounts: no card read already in the clock's span of `every` is asked.
/// It is asked for at every look until [`Watch::answered`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reading {
    every: Duration,
    why: Why,
}

/// What one look at a brand asks for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Plan {
    accounts: Option<Reading>,
    history: bool,
    /// The brand's accounts keep [`QUIET_PACE`] for this long from now on.
    quiet: Option<Duration>,
}

/// When a brand was last seen working, last read, and last refused.
struct Watch {
    brand: Brand,
    roots: Vec<PathBuf>,
    depth: usize,
    newest: Option<SystemTime>,
    accounts: Reader,
    history: Reader,
    /// A reading of the accounts was held back at the last look.
    waiting: bool,
    refusals: u32,
    /// When the quiet time ends, and how long it was set for.
    quiet_until: Option<(Seconds, Duration)>,
}

impl Watch {
    fn new(brand: Brand, roots: Vec<PathBuf>, depth: usize, read_at: Option<Seconds>) -> Self {
        Self {
            brand,
            roots,
            depth,
            newest: None,
            accounts: Reader {
                pending: false,
                read_at,
            },
            history: Reader {
                pending: false,
                read_at,
            },
            waiting: false,
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

    /// The brand was used on this computer since the last look.
    fn used(&mut self) {
        self.accounts.pending = true;
        self.history.pending = true;
    }

    /// Whether the brand's accounts keep [`QUIET_PACE`] because a provider refused a reading. A
    /// quiet time with more left than it was set for was set before the clock was set back, and
    /// no longer counts.
    fn quiet(&self, now: Seconds) -> bool {
        self.quiet_until
            .is_some_and(|(until, length)| now < until && until - now <= length.as_secs())
    }

    /// A provider refused one of the brand's accounts for asking too often: how long the accounts
    /// now keep [`QUIET_PACE`], or `None` while they already do.
    fn refused(&mut self, now: Seconds) -> Option<Duration> {
        if self.quiet(now) {
            return None;
        }
        let again = self
            .quiet_until
            .is_some_and(|(until, _)| until <= now && now - until < FORGIVEN_AFTER.as_secs());
        self.refusals = if again {
            self.refusals.saturating_add(1)
        } else {
            1
        };
        let doubled = 1_u32 << (self.refusals - 1).min(8);
        let quiet = QUIET.saturating_mul(doubled).min(QUIET_CAP);
        self.quiet_until = Some((now + quiet.as_secs(), quiet));
        Some(quiet)
    }

    /// The reading of the accounts that [`Watch::plan`] asked for at `now` was answered, by the
    /// provider or from a reading taken in the same span.
    fn answered(&mut self, now: Seconds) {
        self.accounts.read(now);
        self.waiting = false;
    }

    /// What is due now. `refused` tells that a provider's last answer to one of the brand's
    /// accounts refused the reading for asking too often. A reading of the accounts stays due
    /// until it is [`Watch::answered`].
    fn plan(&mut self, now: Seconds, refused: bool) -> Plan {
        let quiet = if refused { self.refused(now) } else { None };
        let pace = self.brand.pace();
        let (every, why) = if self.quiet(now) {
            (QUIET_PACE, Why::Refused)
        } else if self.accounts.pending {
            (pace.in_use, Why::InUse)
        } else {
            (pace.steady, Why::Steady)
        };
        let accounts = self
            .accounts
            .passed(now, every)
            .then_some(Reading { every, why });
        let history = self.history.pending && self.history.since(now) >= LOCAL_GAP;
        if history {
            self.history.read(now);
        }
        Plan {
            accounts,
            history,
            quiet,
        }
    }
}

pub async fn run(app: AppHandle) {
    let home = uc_core::paths::home_dir();
    let claude =
        uc_core::paths::env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"));
    let codex = uc_core::paths::env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
    let started = Some(clock());
    let mut watches = vec![
        Watch::new(Brand::Claude, vec![claude.join("projects")], 4, started),
        Watch::new(Brand::Codex, vec![codex.join("sessions")], 5, started),
    ];
    loop {
        tokio::time::sleep(until_next_tick()).await;
        let looked = tauri::async_runtime::spawn_blocking(move || {
            for watch in &mut watches {
                if watch.look() {
                    watch.used();
                }
            }
            watches
        })
        .await;
        let Ok(looked) = looked else {
            return;
        };
        watches = looked;
        let now = clock();
        let engine = app.state::<BackendService>().engine();
        for watch in &mut watches {
            let brand = watch.brand;
            let accounts = enabled_cards(&engine, |id| brand.owns_account(id));
            let refused = accounts.iter().any(|id| engine.rate_limited(id));
            let plan = watch.plan(now, refused);
            if let Some(quiet) = plan.quiet {
                tracing::warn!(
                    target: "refresh",
                    "{brand:?} refused a reading for asking too often: its accounts are read every {} minutes for {} minutes",
                    QUIET_PACE.as_secs() / 60,
                    quiet.as_secs() / 60
                );
            }
            if let Some(reading) = plan.accounts {
                if !watch.waiting && !accounts.is_empty() {
                    let why = match reading.why {
                        Why::InUse => "in use here",
                        Why::Steady => "on the clock",
                        Why::Refused => "after a refusal",
                    };
                    tracing::info!(target: "refresh", "{brand:?} {why}: reading {} cards again", accounts.len());
                }
                let outcomes = read_cards(&engine, accounts, reading.every).await;
                if held_back(&outcomes) {
                    watch.waiting = true;
                } else {
                    watch.answered(now);
                }
            }
            if plan.history {
                let history = enabled_cards(&engine, |id| brand.owns_history(id));
                read_cards(&engine, history, LOCAL_GAP).await;
            }
        }
    }
}

/// The enabled cards that `owned` picks.
fn enabled_cards(engine: &Engine, owned: impl Fn(&str) -> bool) -> Vec<String> {
    engine
        .provider_ids()
        .into_iter()
        .filter(|id| owned(id) && engine.is_enabled(id))
        .collect()
}

/// Whether the engine held one of the readings back: the provider asked to be left alone for a
/// while, or the card was being read already. Such a reading is asked for again at the next look.
fn held_back(outcomes: &[RefreshOutcome]) -> bool {
    outcomes
        .iter()
        .any(|outcome| matches!(outcome, RefreshOutcome::BackedOff | RefreshOutcome::Skipped))
}

/// Read `cards` again, past their cached readings, except those read already in the clock's
/// current span of `every`.
async fn read_cards(
    engine: &Arc<Engine>,
    cards: Vec<String>,
    every: Duration,
) -> Vec<RefreshOutcome> {
    let mut reads = tokio::task::JoinSet::new();
    for id in cards {
        let engine = engine.clone();
        reads.spawn(async move { engine.refresh_on_the_clock(&id, every).await });
    }
    let mut outcomes = Vec::new();
    while let Some(read) = reads.join_next().await {
        if let Ok(outcome) = read {
            outcomes.push(outcome);
        }
    }
    outcomes
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
    /// A full hour on the clock.
    const NOON: Seconds = 497_000 * 3600;

    fn watch(brand: Brand, read_at: Option<Seconds>) -> Watch {
        Watch::new(brand, Vec::new(), 0, read_at)
    }

    /// The plans of every look from `start` for `span`, with the moment of each. `working` tells
    /// whether the logs grow at every look. Every reading of the accounts is answered.
    fn plans(
        watch: &mut Watch,
        start: Seconds,
        span: Duration,
        working: bool,
    ) -> Vec<(Seconds, Plan)> {
        let tick = TICK.as_secs();
        (1..=span.as_secs() / tick)
            .map(|look| {
                if working {
                    watch.used();
                }
                let now = start + tick * look;
                let plan = watch.plan(now, false);
                if plan.accounts.is_some() {
                    watch.answered(now);
                }
                (now, plan)
            })
            .collect()
    }

    fn account_readings(plans: &[(Seconds, Plan)]) -> Vec<Seconds> {
        plans
            .iter()
            .filter(|(_, plan)| plan.accounts.is_some())
            .map(|(at, _)| *at)
            .collect()
    }

    fn history_readings(plans: &[(Seconds, Plan)]) -> usize {
        plans.iter().filter(|(_, plan)| plan.history).count()
    }

    #[test]
    fn a_brand_owns_its_accounts_and_its_history_only() {
        assert!(Brand::Claude.owns_account("claude@abc"));
        assert!(!Brand::Claude.owns_account("claude-local"));
        assert!(Brand::Claude.owns_history("claude-local"));
        assert!(!Brand::Claude.owns_history("claude@abc"));
        assert!(!Brand::Claude.owns_account("claude-code@abc"));
        assert!(!Brand::Claude.owns_account("codex@abc"));
        assert!(Brand::Codex.owns_history("codex-local"));
        assert!(!Brand::Codex.owns_account("antigravity@abc"));
        assert!(!Brand::Codex.owns_history("codex-locals"));
    }

    #[test]
    fn a_growing_log_asks_for_a_reading_once_in_each_span() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let log = project.join("session.jsonl");
        std::fs::write(&log, "{}\n").unwrap();
        let mut watch = Watch::new(Brand::Codex, vec![dir.path().to_path_buf()], 4, Some(NOON));
        let pace = Brand::Codex.pace();
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
        watch.used();
        assert_eq!(
            watch.plan(NOON + 25, false),
            Plan::default(),
            "the accounts were read in this half minute"
        );
        let in_use = Some(Reading {
            every: pace.in_use,
            why: Why::InUse,
        });
        assert_eq!(
            watch.plan(NOON + 30, false),
            Plan {
                accounts: in_use,
                history: true,
                quiet: None
            }
        );
        watch.answered(NOON + 30);
        watch.used();
        assert_eq!(watch.plan(NOON + 55, false), Plan::default());
        assert_eq!(
            watch.plan(NOON + 60, false),
            Plan {
                accounts: in_use,
                history: true,
                quiet: None
            }
        );
        watch.answered(NOON + 60);
        assert_eq!(watch.plan(NOON + 115, false), Plan::default());
        assert_eq!(
            watch.plan(NOON + 120, false).accounts,
            Some(Reading {
                every: pace.steady,
                why: Why::Steady
            })
        );
    }

    #[test]
    fn computers_started_at_different_moments_read_claude_at_the_same_ones() {
        let every = Brand::Claude.pace().steady;
        let desk = NOON + 17;
        let closet = NOON + 101;
        let worked = account_readings(&plans(
            &mut watch(Brand::Claude, Some(desk)),
            desk,
            HOUR,
            true,
        ));
        let idle = account_readings(&plans(
            &mut watch(Brand::Claude, Some(closet)),
            closet,
            HOUR,
            false,
        ));
        assert_eq!(worked.len(), 15);
        assert_eq!(idle.len(), 15);
        for (worked, idle) in worked.iter().zip(&idle) {
            assert_eq!(span(*worked, every), span(*idle, every));
            assert!(
                worked.abs_diff(*idle) < TICK.as_secs(),
                "{worked} and {idle} are less than one look apart"
            );
        }
    }

    #[test]
    fn an_hour_asks_claude_for_a_quarter_of_what_anthropic_answers() {
        let claude = plans(&mut watch(Brand::Claude, Some(NOON)), NOON, HOUR, true);
        assert_eq!(account_readings(&claude).len(), 15);
        assert_eq!(history_readings(&claude), 120);
        let idle = plans(&mut watch(Brand::Claude, Some(NOON)), NOON, HOUR, false);
        assert_eq!(account_readings(&idle).len(), 15);
        assert_eq!(history_readings(&idle), 0);
    }

    #[test]
    fn the_accounts_are_read_more_often_than_a_reading_is_kept() {
        let kept = uc_engine::EngineConfig::default().refresh_interval;
        for brand in [Brand::Claude, Brand::Codex] {
            let pace = brand.pace();
            assert!(pace.in_use < kept && pace.steady < kept);
            assert_eq!(
                pace.steady.as_secs() % pace.in_use.as_secs(),
                0,
                "a computer in use reads at the moments an idle one does"
            );
        }
    }

    #[test]
    fn a_reading_that_was_held_back_is_asked_for_again_at_the_next_look() {
        let mut claude = watch(Brand::Claude, Some(NOON + 10));
        let every = Brand::Claude.pace().steady;
        let mark = NOON + every.as_secs();
        let due = Some(Reading {
            every,
            why: Why::Steady,
        });
        assert_eq!(claude.plan(mark - 5, false).accounts, None);
        assert_eq!(claude.plan(mark, false).accounts, due);
        assert_eq!(
            claude.plan(mark + 5, false).accounts,
            due,
            "nobody answered the reading at the mark"
        );
        claude.answered(mark + 50);
        assert_eq!(claude.plan(mark + 55, false).accounts, None);
        assert_eq!(
            claude.plan(mark + every.as_secs(), false).accounts,
            due,
            "the late answer does not move the next mark"
        );
    }

    #[test]
    fn a_held_back_reading_is_one_the_engine_did_not_answer() {
        use RefreshOutcome::{BackedOff, CacheHit, Failed, Refreshed, Skipped};
        assert!(!held_back(&[]));
        assert!(!held_back(&[Refreshed, CacheHit, Failed]));
        assert!(held_back(&[Refreshed, BackedOff]));
        assert!(held_back(&[Skipped]));
    }

    #[test]
    fn a_quiet_time_set_before_the_clock_was_set_back_no_longer_counts() {
        let mut claude = watch(Brand::Claude, None);
        assert_eq!(claude.refused(NOON), Some(QUIET));
        assert!(claude.quiet(NOON + QUIET.as_secs() - 1));
        assert!(!claude.quiet(NOON + QUIET.as_secs()));
        assert!(
            !claude.quiet(NOON - 2 * 3600),
            "two and a half hours of quiet were never set"
        );
    }

    #[test]
    fn codex_is_read_every_half_minute_in_use_and_every_two_minutes_otherwise() {
        let codex = plans(&mut watch(Brand::Codex, Some(NOON)), NOON, HOUR, true);
        assert_eq!(account_readings(&codex).len(), 120);
        assert_eq!(history_readings(&codex), 120);
        let idle = plans(&mut watch(Brand::Codex, Some(NOON)), NOON, HOUR, false);
        let readings = account_readings(&idle);
        assert_eq!(readings.len(), 30);
        assert!(readings.iter().all(|at| at % 120 == 0));
    }

    #[test]
    fn a_refusal_slows_the_accounts_down_and_leaves_the_history_alone() {
        let mut claude = watch(Brand::Claude, Some(NOON));
        let refused = NOON + 18 * 60;
        claude.used();
        assert_eq!(
            claude.plan(refused, true),
            Plan {
                accounts: Some(Reading {
                    every: QUIET_PACE,
                    why: Why::Refused
                }),
                history: true,
                quiet: Some(QUIET)
            },
            "the reading is asked for; the engine holds it back while the provider must be left alone"
        );
        claude.answered(refused);
        assert_eq!(
            claude.plan(refused + TICK.as_secs(), true).quiet,
            None,
            "one refusal is counted once"
        );
        let quiet = plans(&mut claude, refused, QUIET - TICK, true);
        let readings = account_readings(&quiet);
        assert_eq!(readings.len(), 6);
        assert!(readings.iter().all(|at| at % QUIET_PACE.as_secs() == 0));
        assert_eq!(history_readings(&quiet), 59);
        claude.used();
        assert_eq!(
            claude.plan(refused + QUIET.as_secs(), false).accounts,
            Some(Reading {
                every: Brand::Claude.pace().in_use,
                why: Why::InUse
            })
        );
    }

    #[test]
    fn codex_is_slowed_down_when_it_is_refused_too() {
        let mut codex = watch(Brand::Codex, Some(NOON));
        assert_eq!(codex.plan(NOON + 3600, true).quiet, Some(QUIET));
        codex.answered(NOON + 3600);
        let quiet = plans(&mut codex, NOON + 3600, QUIET - TICK, true);
        assert_eq!(account_readings(&quiet).len(), 5);
    }

    #[test]
    fn refusals_in_a_row_double_the_quiet_time_up_to_the_cap() {
        let mut claude = watch(Brand::Claude, None);
        let mut now = NOON;
        let mut quiets = Vec::new();
        for _ in 0..6 {
            let quiet = claude.refused(now).unwrap();
            quiets.push(quiet.as_secs() / 60);
            now += quiet.as_secs() + 10 * 60;
        }
        assert_eq!(quiets, [30, 60, 120, 240, 240, 240]);
    }

    #[test]
    fn a_long_time_without_a_refusal_starts_again_from_the_shortest_quiet_time() {
        let mut claude = watch(Brand::Claude, None);
        assert_eq!(claude.refused(NOON), Some(QUIET));
        let soon = NOON + QUIET.as_secs() + 3600;
        assert_eq!(claude.refused(soon), Some(QUIET * 2));
        let much_later = soon + 2 * QUIET.as_secs() + FORGIVEN_AFTER.as_secs();
        assert_eq!(claude.refused(much_later), Some(QUIET));
    }

    #[test]
    fn the_looks_fall_on_the_clock() {
        let wait = until_next_tick();
        assert!(wait > Duration::ZERO && wait <= TICK);
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
