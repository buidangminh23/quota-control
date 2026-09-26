/**
 * Invented model-quality history for the browser mock: a few months of daily counts per model and
 * project, drawn from a fixed seed so the preview is stable. Not the user's data.
 */
import type { QualityCounts, QualityInfo, QualityQuery, QualityRow, QualitySummary } from "./insightsTypes";
import type { UsageSource } from "./types";

interface MockModel {
  source: UsageSource;
  model: string;
  /** Turns per active day. */
  daily: number;
  /** Days back the model first appears. */
  since: number;
  editFailure: number;
  green: number;
  interrupt: number;
  checks: boolean;
}

const MODELS: readonly MockModel[] = [
  { source: "claude", model: "claude-opus-5", daily: 12, since: 80, editFailure: 0.006, green: 0.78, interrupt: 0.008, checks: true },
  { source: "claude", model: "claude-opus-5-5", daily: 6, since: 20, editFailure: 0.003, green: 0.84, interrupt: 0.002, checks: true },
  { source: "claude", model: "claude-fable-5-1", daily: 3, since: 25, editFailure: 0.002, green: 0.85, interrupt: 0.012, checks: true },
  { source: "claude", model: "claude-sonnet-5", daily: 1, since: 40, editFailure: 0, green: 0, interrupt: 0, checks: false },
  { source: "codex", model: "gpt-5.6-sol", daily: 10, since: 75, editFailure: 0.01, green: 0.84, interrupt: 0.009, checks: true },
  { source: "codex", model: "gpt-6-astra", daily: 8, since: 20, editFailure: 0.04, green: 0.9, interrupt: 0.015, checks: true },
  { source: "codex", model: "gpt-5.6-luna", daily: 4, since: 60, editFailure: 0.08, green: 0.78, interrupt: 0.006, checks: true },
];

const PROJECTS: readonly [string, number][] = [
  ["demo-app", 0.5],
  ["api-server", 0.35],
  ["", 0.15],
];

function random(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = Math.imul(state ^ (state >>> 15), 1 | state);
    value = (value + Math.imul(value ^ (value >>> 7), 61 | value)) ^ value;
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

function binomial(next: () => number, trials: number, chance: number): number {
  let count = 0;
  for (let trial = 0; trial < trials; trial += 1) if (next() < chance) count += 1;
  return count;
}

function localDay(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

interface DayRow {
  day: string;
  row: QualityRow;
}

export function mockQualityHistory(now: Date): DayRow[] {
  const next = random(20260926);
  const rows: DayRow[] = [];
  for (let back = 80; back >= 0; back -= 1) {
    const date = new Date(now.getFullYear(), now.getMonth(), now.getDate() - back);
    for (const model of MODELS) {
      if (back > model.since || next() < 0.2) continue;
      for (const [project, share] of PROJECTS) {
        const turns = binomial(next, Math.round(model.daily * 2), share / 2 + 0.25);
        if (turns === 0) continue;
        const humanTurns = Math.round(turns * 0.6);
        const verifiedTurns = model.checks ? binomial(next, turns, 0.14) : 0;
        const edits = binomial(next, turns * 2, 0.6);
        const shellCommands = binomial(next, turns * 8, 0.7);
        const counts: QualityCounts = {
          turns,
          humanTurns,
          interruptedTurns: binomial(next, humanTurns, model.interrupt),
          verifiedTurns,
          greenTurns: binomial(next, verifiedTurns, model.green),
          checkRuns: verifiedTurns * 2,
          failedCheckRuns: binomial(next, verifiedTurns * 2, 0.3),
          unknownCheckRuns: binomial(next, verifiedTurns, 0.1),
          edits,
          failedEdits: binomial(next, edits, model.editFailure),
          deniedActions: binomial(next, turns, 0.02),
          shellCommands,
          failedShellCommands: binomial(next, shellCommands, 0.08),
          outputTokens: Math.round(turns * (8_000 + 30_000 * next())),
          timedTurns: turns,
          turnMillis: Math.round(turns * (120_000 + 600_000 * next())),
        };
        rows.push({ day: localDay(date), row: { source: model.source, model: model.model, project, counts } });
      }
    }
  }
  return rows;
}

export function mockQualitySummary(history: readonly DayRow[], query: QualityQuery, info: QualityInfo): QualitySummary {
  const grouped = new Map<string, QualityRow>();
  for (const { day, row } of history) {
    if ((query.from && day < query.from) || (query.to && day > query.to)) continue;
    const key = `${row.source}|${row.model}|${row.project}`;
    const existing = grouped.get(key);
    if (!existing) {
      grouped.set(key, { ...row, counts: { ...row.counts } });
      continue;
    }
    for (const field of Object.keys(row.counts) as (keyof QualityCounts)[]) existing.counts[field] += row.counts[field];
  }
  return { rows: [...grouped.values()], info };
}
