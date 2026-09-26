/**
 * A sample usage ledger for the mock backend and tests: about fourteen months of daily rows for Claude
 * and Codex across a few models and projects, from a seeded generator so every run gives the same
 * numbers relative to `today`. `summarizeUsage` answers a `UsageQuery` over such rows the way the
 * core's ledger does (inclusive local days, one row per key and source, empty rows left out).
 */
import { addDays } from "./days";
import type { UsageGroupRow, UsageQuery, UsageSource, UsageTotals } from "./types";

export interface LedgerRow {
  day: string;
  source: UsageSource;
  model: string;
  project: string;
  totals: UsageTotals;
}

const SAMPLE_DAYS = 430;

/** USD per million input, output, cache-read and cache-write tokens. */
type Prices = readonly [number, number, number, number];

const SAMPLE_STREAMS: ReadonlyArray<{ source: UsageSource; model: string; project: string; tokens: number; days: number; prices: Prices }> = [
  { source: "claude", model: "claude-opus-5-5", project: "quota-control", tokens: 180e6, days: 40, prices: [4, 20, 0.2, 5] },
  { source: "claude", model: "claude-opus-5", project: "PCC4SH", tokens: 240e6, days: 400, prices: [5, 25, 0.5, 6.25] },
  { source: "claude", model: "claude-sonnet-5", project: "bot-tele", tokens: 60e6, days: 300, prices: [2, 10, 0.2, 2.5] },
  { source: "claude", model: "claude-fable-5-1", project: "CongCuDoiTen", tokens: 30e6, days: 90, prices: [10, 50, 0.25, 12.5] },
  { source: "codex", model: "gpt-5.6-sol", project: "PCC4SH", tokens: 90e6, days: SAMPLE_DAYS, prices: [4, 20, 0.4, 5] },
  { source: "codex", model: "gpt-6-astra", project: "codex-mcp-bridge", tokens: 25e6, days: 200, prices: [10, 50, 1, 12.5] },
  { source: "codex", model: "gpt-5.6-luna", project: "", tokens: 15e6, days: SAMPLE_DAYS, prices: [0.2, 1.2, 0.02, 0.25] },
];

function seeded(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = Math.imul(state ^ (state >>> 15), 1 | state);
    value = (value + Math.imul(value ^ (value >>> 7), 61 | value)) ^ value;
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

function sampleTotals(tokens: number, prices: Prices): UsageTotals {
  const inputTokens = Math.round(tokens * 0.05);
  const outputTokens = Math.round(tokens * 0.02);
  const cacheCreationInputTokens = Math.round(tokens * 0.05);
  const cachedInputTokens = Math.round(tokens) - inputTokens - outputTokens - cacheCreationInputTokens;
  const costUSD = (inputTokens * prices[0] + outputTokens * prices[1] + cachedInputTokens * prices[2] + cacheCreationInputTokens * prices[3]) / 1e6;
  return { inputTokens, outputTokens, cachedInputTokens, cacheCreationInputTokens, totalTokens: Math.round(tokens), costUSD };
}

/** The sample rows for the `SAMPLE_DAYS` days ending on `today`, oldest first. */
export function sampleLedger(today: string): LedgerRow[] {
  const random = seeded(20260926);
  const rows: LedgerRow[] = [];
  for (let back = SAMPLE_DAYS - 1; back >= 0; back -= 1) {
    const day = addDays(today, -back);
    for (const stream of SAMPLE_STREAMS) {
      const draw = random();
      if (back >= stream.days || draw < 0.15) continue;
      rows.push({ day, source: stream.source, model: stream.model, project: stream.project, totals: sampleTotals(stream.tokens * (0.3 + draw * 1.4), stream.prices) });
    }
  }
  return rows;
}

function groupKey(row: LedgerRow, groupBy: UsageQuery["groupBy"]): string {
  switch (groupBy) {
    case "day":
      return row.day;
    case "month":
      return row.day.slice(0, 7);
    case "year":
      return row.day.slice(0, 4);
    case "model":
      return row.model;
    case "project":
      return row.project;
  }
}

function addInto(target: UsageTotals, extra: UsageTotals): void {
  target.inputTokens += extra.inputTokens;
  target.outputTokens += extra.outputTokens;
  target.cachedInputTokens += extra.cachedInputTokens;
  target.cacheCreationInputTokens += extra.cacheCreationInputTokens;
  target.totalTokens += extra.totalTokens;
  if (extra.costUSD !== undefined) target.costUSD = (target.costUSD ?? 0) + extra.costUSD;
}

/** Answer `query` over `rows` like the core's ledger. */
export function summarizeUsage(rows: readonly LedgerRow[], query: UsageQuery, today: string): UsageGroupRow[] {
  const from = query.from ?? "";
  const to = query.to ?? today;
  const groups = new Map<string, UsageGroupRow>();
  for (const row of rows) {
    if (row.day < from || row.day > to) continue;
    const key = groupKey(row, query.groupBy);
    const id = `${row.source}\u0000${key}`;
    const group = groups.get(id);
    if (group) addInto(group.totals, row.totals);
    else groups.set(id, { key, source: row.source, totals: { ...row.totals } });
  }
  return [...groups.values()].filter((group) => group.totals.totalTokens > 0);
}
