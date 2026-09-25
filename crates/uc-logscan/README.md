# Local usage history

Register one `LocalHistoryRuntime::new(LogSource::Claude)` and one
`LocalHistoryRuntime::new(LogSource::Codex)` alongside the quota/account runtimes.
Their IDs are `claude-local` and `codex-local`; their icons remain the provider
family icons. These cards include all sessions recorded on this machine. They
are not account-scoped and do not require quota credentials. Never register one
local-history runtime per connected account.

`LogScanner::from_environment` resolves `CLAUDE_CONFIG_DIR/projects` and
`CODEX_HOME/{sessions,archived_sessions}`, falling back to the usual home
directories. `LogScanner::new` accepts explicit `ScanOptions` for fixture tests
and portable deployments. `scan(now)` is blocking; both provided runtimes run
it on Tokio's blocking pool.

`ScanReport` contains normalized daily history, generic warnings, and counters.
It never contains prompts, raw events, credentials, or filesystem paths.
`cargo run -p uc-logscan --example diagnostics` scans the configured local roots
twice using the same scanner and prints only aggregate counts, byte counts, cache
hits, generic warnings, and elapsed time.
`append_history` keeps existing quota metrics and error badges/categories.
`HistoryRuntime` is an optional adapter for a single legacy quota runtime; it
refuses to augment account-scoped IDs containing `@`. The dedicated local cards
are the recommended integration.

## Accounting

- `totalTokens` is retained from provider usage records.
- `tokenUsage.inputTokens` counts all prompt tokens, including cached reads and
  cache creation. `cachedInputTokens` and `cacheCreationInputTokens` are subsets.
- `outputTokens` counts generated output. Codex reasoning is already included
  in its output counter and is never added again.
- An unavailable input/output breakdown is omitted, not invented as zero.
- Spend rows carry the period breakdown under `modelBreakdown.tokenUsage`.
  Dedicated Input Tokens, Output Tokens, and Cached Input Tokens rows show Today.
- Codex cumulative counters are differenced; repeated counters are ignored.
  Counter resets use the last request when supplied, otherwise the new baseline.
  Child-session replay seeds the baseline without charging parent history again.
- Claude assistant updates deduplicate by message ID (or UUID), preferring the
  parent copy and then the largest observed usage. Sidechain replays are removed.

Dates use the local timezone at each event, including historical DST. Tests may
provide a fixed UTC offset. History covers today and the previous 29 calendar
days; future events are excluded. No matching records produce a no-data status,
not fabricated zero spending. Missing model prices retain token counts and omit
the affected cost totals.

## Limits

Discovery skips symlinks and Windows reparse points, including linked roots.
Depth, directory entries, files, line sizes, total bytes, and retained usage
events are bounded. Limits and unreadable files produce incomplete-history
warnings. Overlong lines are discarded while streaming; a malformed tail does
not discard earlier complete events. No raw transcript content is cached.

Scanner clones share an in-memory per-file cache keyed by path, byte length,
modification time, source, retained time window, and parsing options. Only
normalized usage events are retained, never transcript text or JSON. Unchanged
files reuse parsed events; changed files are reparsed completely so cumulative
Codex baselines and Claude message updates remain correct. Cross-file
deduplication runs over both cached and new events on every scan. Removed files
are evicted. Cache retention is bounded by the same file/event limits.

Discovery uses modification time with a 32-day lookback margin before the exact
30-calendar-day event filter. Old creation dates and old filenames do not exclude
recently active sessions. Claude project `memory/` directories contain memory
documents rather than transcripts and are excluded before inspecting links;
linked memory stores therefore do not produce false incomplete-history warnings.
Each scan may read at most 8 GiB and runs with a
110-second time budget; bounds still produce truthful incomplete-history notices.
Pricing and dates are reaggregated from cached records every refresh.

The scanner does not persist cache to disk, parse only appended tails, determine
log-account ownership, sync remote sessions, or reconstruct historical pricing
revisions. Exact bundled prices are API-equivalent estimates, never subscription
bills. See `uc-pricing/README.md` for provenance.
