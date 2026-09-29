/**
 * The Benchmark and Reset tabs' contract with the Rust core (`src-tauri/src/insights_commands.rs`,
 * `crates/uc-logscan/src/quality`, `src-tauri/src/public_feeds.rs`).
 */
import type { UsageSource } from "./types";

/** Everything counted for one model on one project over a period, from the local transcripts. */
export interface QualityCounts {
  /** Requests the model answered: a person's prompt, or an agent's task for a subagent. */
  turns: number;
  /** Turns a person started. */
  humanTurns: number;
  /** Turns a person stopped before the model finished. */
  interruptedTurns: number;
  /** Turns that ran at least one check (test, build, type check, lint) whose result could be read. */
  verifiedTurns: number;
  /** Verified turns whose last readable check passed. */
  greenTurns: number;
  /** Check runs with a readable result. */
  checkRuns: number;
  failedCheckRuns: number;
  /** Check runs whose result could not be read from the transcript. */
  unknownCheckRuns: number;
  /** File edits the model attempted (edits a person declined are not counted). */
  edits: number;
  /** Edits that could not be applied. */
  failedEdits: number;
  /** Tool calls a person or a permission rule declined. */
  deniedActions: number;
  shellCommands: number;
  /** Shell commands that exited with an error. */
  failedShellCommands: number;
  outputTokens: number;
  /** Turns with a known duration, and their total length. */
  timedTurns: number;
  turnMillis: number;
}

export interface QualityRow {
  source: UsageSource;
  model: string;
  /** Repository name under the ledger's project rule; empty when unknown. */
  project: string;
  counts: QualityCounts;
}

/** Inclusive `YYYY-MM-DD` bounds in the local calendar; either may be left open. */
export interface QualityQuery {
  from?: string | null;
  to?: string | null;
}

export interface QualityInfo {
  /** When the transcripts were last scanned (ISO), `null` before the first scan. */
  scannedAt: string | null;
  /** A scan is running now. */
  scanning: boolean;
  /** Transcript files in the scan. */
  files: number;
  /** First and last local day with a counted turn. */
  firstDay: string | null;
  lastDay: string | null;
}

export interface QualitySummary {
  rows: QualityRow[];
  info: QualityInfo;
}

/** The public feeds the core fetches from fixed addresses. */
export type PublicFeedName = "codexResetStatus" | "codexResets" | "claudeResets" | "epochScores" | "epochBenchmarks" | "arena" | "arena3d";

export interface PublicFeedSnapshot {
  name: PublicFeedName;
  /** The last body that passed validation (JSON or CSV text), or `null` before the first success. */
  body: string | null;
  /** When `body` was downloaded (ISO). */
  fetchedAt: string | null;
  /** When the core last asked the source, successful or not (ISO). */
  checkedAt: string | null;
  /** Why the latest attempt failed, while the cached body is still shown. */
  error: string | null;
  /** The body is older than its refresh interval or the last attempt failed. */
  stale: boolean;
}
