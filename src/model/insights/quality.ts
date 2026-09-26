/**
 * Quality of each model on the user's own projects, from counts the core read in the local Claude
 * Code and Codex transcripts. Nothing here is estimated or sent anywhere: a score exists only when
 * every part of it has enough real samples, and each part carries its 95% interval.
 *
 * The score is the plain mean of three rates, each a share of something the user can check:
 * - edits: file edits the model attempted that applied (the rest failed to match or to write);
 * - green: turns that ran a test, build, type check or lint whose last readable result passed;
 * - steady: turns a person started that the person did not have to stop halfway.
 * Its interval uses each part's interval at 1 − 0.05/3, so all three hold together at 95% and the
 * mean of the lower (upper) ends bounds the score from below (above).
 */
import type { QualityCounts, QualityRow } from "@/lib/insightsTypes";
import type { UsageSource } from "@/lib/types";
import { displayModel } from "./modelNames";
import { wilson, Z95, Z95_OF_THREE, type Interval } from "./stats";

export type QualityPartKey = "edits" | "green" | "steady";
export const QUALITY_PARTS: readonly QualityPartKey[] = ["edits", "green", "steady"];

/** Samples each part needs before it counts toward a score. */
export const MINIMUM_SAMPLES: Record<QualityPartKey, number> = { edits: 20, green: 10, steady: 10 };

export interface QualityPart {
  key: QualityPartKey;
  successes: number;
  samples: number;
  minimum: number;
  /** 95% interval, `null` without samples. */
  interval: Interval | null;
  enough: boolean;
}

export interface QualityScore {
  /** 0..100. */
  value: number;
  low: number;
  high: number;
}

export interface ModelQuality {
  key: string;
  source: UsageSource;
  model: string;
  name: string;
  counts: QualityCounts;
  parts: Record<QualityPartKey, QualityPart>;
  score: QualityScore | null;
}

export const EMPTY_COUNTS: QualityCounts = {
  turns: 0,
  humanTurns: 0,
  interruptedTurns: 0,
  verifiedTurns: 0,
  greenTurns: 0,
  checkRuns: 0,
  failedCheckRuns: 0,
  unknownCheckRuns: 0,
  edits: 0,
  failedEdits: 0,
  deniedActions: 0,
  shellCommands: 0,
  failedShellCommands: 0,
  outputTokens: 0,
  timedTurns: 0,
  turnMillis: 0,
};

export function addCounts(total: QualityCounts, more: QualityCounts): QualityCounts {
  const sum = { ...total };
  for (const key of Object.keys(EMPTY_COUNTS) as (keyof QualityCounts)[]) sum[key] = total[key] + (more[key] ?? 0);
  return sum;
}

function partSamples(key: QualityPartKey, counts: QualityCounts): { successes: number; samples: number } {
  switch (key) {
    case "edits":
      return { successes: counts.edits - counts.failedEdits, samples: counts.edits };
    case "green":
      return { successes: counts.greenTurns, samples: counts.verifiedTurns };
    case "steady":
      return { successes: counts.humanTurns - counts.interruptedTurns, samples: counts.humanTurns };
  }
}

function part(key: QualityPartKey, counts: QualityCounts): QualityPart {
  const { successes, samples } = partSamples(key, counts);
  const minimum = MINIMUM_SAMPLES[key];
  return { key, successes, samples, minimum, interval: wilson(successes, samples, Z95), enough: samples >= minimum };
}

export function qualityScore(counts: QualityCounts): QualityScore | null {
  const joint = QUALITY_PARTS.map((key) => {
    const { successes, samples } = partSamples(key, counts);
    return samples >= MINIMUM_SAMPLES[key] ? wilson(successes, samples, Z95_OF_THREE) : null;
  });
  if (joint.some((interval) => interval === null)) return null;
  const intervals = joint as Interval[];
  const mean = (pick: (interval: Interval) => number) => (100 * intervals.reduce((sum, interval) => sum + pick(interval), 0)) / intervals.length;
  return { value: mean((interval) => interval.rate), low: mean((interval) => interval.low), high: mean((interval) => interval.high) };
}

export function modelQuality(source: UsageSource, model: string, counts: QualityCounts): ModelQuality {
  const parts = Object.fromEntries(QUALITY_PARTS.map((key) => [key, part(key, counts)])) as Record<QualityPartKey, QualityPart>;
  return { key: `${source}:${model}`, source, model, name: displayModel(model), counts, parts, score: qualityScore(counts) };
}

/** `null` means every project. The empty string is a project the transcripts did not name. */
export type ProjectFilter = string | null;

/** One entry per model, scored ones first (best first), then the rest by how much they were used. */
export function summarizeQuality(rows: readonly QualityRow[], project: ProjectFilter): ModelQuality[] {
  const byModel = new Map<string, { source: UsageSource; model: string; counts: QualityCounts }>();
  for (const row of rows) {
    if (project !== null && row.project !== project) continue;
    const key = `${row.source}:${row.model}`;
    const entry = byModel.get(key);
    if (entry) entry.counts = addCounts(entry.counts, row.counts);
    else byModel.set(key, { source: row.source, model: row.model, counts: addCounts(EMPTY_COUNTS, row.counts) });
  }
  return [...byModel.values()]
    .map(({ source, model, counts }) => modelQuality(source, model, counts))
    .sort((a, b) => {
      if (a.score && b.score) return b.score.value - a.score.value || b.counts.turns - a.counts.turns;
      if (a.score || b.score) return a.score ? -1 : 1;
      return b.counts.turns - a.counts.turns || a.name.localeCompare(b.name);
    });
}

export interface ProjectUsage {
  project: string;
  turns: number;
}

/** Projects that have turns in the rows, most used first; the unnamed project goes last. */
export function qualityProjects(rows: readonly QualityRow[]): ProjectUsage[] {
  const turns = new Map<string, number>();
  for (const row of rows) turns.set(row.project, (turns.get(row.project) ?? 0) + row.counts.turns);
  return [...turns.entries()]
    .filter(([, count]) => count > 0)
    .map(([project, count]) => ({ project, turns: count }))
    .sort((a, b) => {
      if ((a.project === "") !== (b.project === "")) return a.project === "" ? 1 : -1;
      return b.turns - a.turns || a.project.localeCompare(b.project);
    });
}

export type QualityRange = "7" | "30" | "90" | "all";
export const QUALITY_RANGES: readonly QualityRange[] = ["7", "30", "90", "all"];

function localDay(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

/** The inclusive local-calendar days a range covers, ending today. */
export function rangeQuery(range: QualityRange, now: Date): { from: string | null; to: string | null } {
  if (range === "all") return { from: null, to: null };
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (Number(range) - 1));
  return { from: localDay(start), to: localDay(now) };
}

/** Average output tokens and seconds per turn, when there are turns to average. */
export function perTurn(counts: QualityCounts): { tokens: number | null; seconds: number | null } {
  return {
    tokens: counts.turns > 0 ? counts.outputTokens / counts.turns : null,
    seconds: counts.timedTurns > 0 ? counts.turnMillis / 1000 / counts.timedTurns : null,
  };
}

/** Share of check runs that passed and of shell commands that succeeded, when any ran. */
export function successRates(counts: QualityCounts): { checks: number | null; shell: number | null } {
  return {
    checks: counts.checkRuns > 0 ? 1 - counts.failedCheckRuns / counts.checkRuns : null,
    shell: counts.shellCommands > 0 ? 1 - counts.failedShellCommands / counts.shellCommands : null,
  };
}
