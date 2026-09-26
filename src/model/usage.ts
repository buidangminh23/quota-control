/**
 * Pure helpers over the usage ledger's rows (`UsageGroupRow`, one per key and source): the date range
 * of each Token period, totals per key or source, ranked slices with an "other" tail, gap-filled time
 * series and readable model and project names.
 */
import { addDays, addMonths, monthOf, monthRange } from "@/lib/days";
import type { UsageGroupRow, UsageGrouping, UsageQuery, UsageSource, UsageTotals } from "@/lib/types";
import type { TotalSpendMetric } from "./format";
import type { TotalSpendPeriod } from "./settings";

export const USAGE_SOURCES: readonly UsageSource[] = ["claude", "codex"];

/** The key of the folded "other" slice; no ledger key starts with a NUL. */
export const OTHER_KEY = "\u0000other";

/** The ledger query for `period` ending on `today`, grouped by `groupBy`. */
export function periodQuery(period: TotalSpendPeriod, today: string, groupBy: UsageGrouping): UsageQuery {
  switch (period) {
    case "today":
      return { from: today, to: today, groupBy };
    case "last30":
      return { from: addDays(today, -29), to: today, groupBy };
    case "last365":
      return { from: addDays(today, -364), to: today, groupBy };
    case "all":
      return { to: today, groupBy };
  }
}

/** The days of `month` up to `today`, or `null` when the month has not started yet. */
export function monthQuery(month: string, today: string, groupBy: UsageGrouping): UsageQuery | null {
  const { from, to } = monthRange(month);
  if (from > today) return null;
  return { from, to: to < today ? to : today, groupBy };
}

export function emptyTotals(): UsageTotals {
  return { inputTokens: 0, outputTokens: 0, cachedInputTokens: 0, cacheCreationInputTokens: 0, totalTokens: 0 };
}

/** The sum of two totals; the cost stays absent only when neither side has one. */
export function addTotals(a: UsageTotals, b: UsageTotals): UsageTotals {
  const sum: UsageTotals = {
    inputTokens: a.inputTokens + b.inputTokens,
    outputTokens: a.outputTokens + b.outputTokens,
    cachedInputTokens: a.cachedInputTokens + b.cachedInputTokens,
    cacheCreationInputTokens: a.cacheCreationInputTokens + b.cacheCreationInputTokens,
    totalTokens: a.totalTokens + b.totalTokens,
  };
  if (a.costUSD !== undefined || b.costUSD !== undefined) sum.costUSD = (a.costUSD ?? 0) + (b.costUSD ?? 0);
  return sum;
}

export interface KeyTotals {
  key: string;
  total: UsageTotals;
  bySource: Partial<Record<UsageSource, UsageTotals>>;
  /** The source that logged the most tokens under this key (models belong to one). */
  source: UsageSource;
}

/** Rows merged per key, both sources together, in first-seen order. */
export function totalsByKey(rows: readonly UsageGroupRow[]): KeyTotals[] {
  const byKey = new Map<string, KeyTotals>();
  for (const row of rows) {
    const existing = byKey.get(row.key);
    if (!existing) {
      byKey.set(row.key, { key: row.key, total: row.totals, bySource: { [row.source]: row.totals }, source: row.source });
      continue;
    }
    existing.total = addTotals(existing.total, row.totals);
    const own = existing.bySource[row.source];
    existing.bySource[row.source] = own ? addTotals(own, row.totals) : row.totals;
    if ((existing.bySource[row.source]?.totalTokens ?? 0) > (existing.bySource[existing.source]?.totalTokens ?? 0)) existing.source = row.source;
  }
  return [...byKey.values()];
}

/** Each source's totals over `rows`, zero where a source logged nothing. */
export function totalsBySource(rows: readonly UsageGroupRow[]): Record<UsageSource, UsageTotals> {
  const result: Record<UsageSource, UsageTotals> = { claude: emptyTotals(), codex: emptyTotals() };
  for (const row of rows) result[row.source] = addTotals(result[row.source], row.totals);
  return result;
}

export function grandTotal(rows: readonly UsageGroupRow[]): UsageTotals {
  return rows.reduce((sum, row) => addTotals(sum, row.totals), emptyTotals());
}

/** The figure a metric reads from totals; `null` when the totals carry no price. */
export function metricAmount(totals: UsageTotals, metric: TotalSpendMetric): number | null {
  switch (metric) {
    case "tokens":
      return totals.totalTokens;
    case "cost":
      return totals.costUSD ?? null;
    case "costPerMtok":
      return totals.costUSD !== undefined && totals.totalTokens > 0 ? (totals.costUSD / totals.totalTokens) * 1_000_000 : null;
  }
}

export interface Slice {
  key: string;
  amount: number;
  totals: UsageTotals;
  source: UsageSource | null;
}

/**
 * The `limit` largest keys by the metric, largest first, then one "other" slice folding the rest. Keys
 * the metric cannot measure (no price) are left out; ties keep the name order.
 */
export function rankedSlices(items: readonly KeyTotals[], metric: TotalSpendMetric, limit: number): Slice[] {
  const measured = items.flatMap((item) => {
    const amount = metricAmount(item.total, metric);
    return amount !== null && amount > 0 ? [{ key: item.key, amount, totals: item.total, source: item.source }] : [];
  });
  measured.sort((a, b) => (a.amount !== b.amount ? b.amount - a.amount : a.key.localeCompare(b.key)));
  if (measured.length <= limit + 1) return measured;
  const rest = measured.slice(limit).reduce((sum, slice) => addTotals(sum, slice.totals), emptyTotals());
  const otherAmount = metricAmount(rest, metric) ?? 0;
  return [...measured.slice(0, limit), { key: OTHER_KEY, amount: otherAmount, totals: rest, source: null }];
}

/** Every slice gets at least this share of the ring so a small one still shows (presentation only). */
export const MINIMUM_RING_SHARE = 0.025;

/** Cumulative ring fractions (0...1 clockwise from 12 o'clock) for positive amounts. */
export function ringFractions(amounts: readonly number[]): Array<{ start: number; end: number }> {
  const total = amounts.reduce((sum, amount) => sum + amount, 0);
  if (!(total > 0)) return [];
  const floored = amounts.map((amount) => Math.max(amount / total, MINIMUM_RING_SHARE));
  const sum = floored.reduce((acc, share) => acc + share, 0);
  let cursor = 0;
  return floored.map((share) => {
    const start = cursor;
    cursor += share / sum;
    return { start, end: cursor };
  });
}

export interface SeriesPoint {
  key: string;
  bySource: Record<UsageSource, UsageTotals>;
  total: UsageTotals;
}

/** One point per key in `keys` (days, months or years, in order), zero where nothing was logged. */
export function series(rows: readonly UsageGroupRow[], keys: readonly string[]): SeriesPoint[] {
  const points = new Map(keys.map((key) => [key, { key, bySource: { claude: emptyTotals(), codex: emptyTotals() }, total: emptyTotals() } as SeriesPoint]));
  for (const row of rows) {
    const point = points.get(row.key);
    if (!point) continue;
    point.bySource[row.source] = addTotals(point.bySource[row.source], row.totals);
    point.total = addTotals(point.total, row.totals);
  }
  return keys.map((key) => points.get(key)!);
}

/** The `count` months ending with `today`'s month, oldest first (`YYYY-MM`). */
export function lastMonths(today: string, count: number): string[] {
  const current = monthOf(today);
  return Array.from({ length: count }, (_, index) => addMonths(current, index - count + 1));
}

/** The years from the first one in `rows` (or this year) through this year, oldest first. */
export function yearSpan(rows: readonly UsageGroupRow[], today: string): string[] {
  const last = Number(today.slice(0, 4));
  const first = rows.reduce((min, row) => Math.min(min, Number(row.key) || last), last);
  return Array.from({ length: last - first + 1 }, (_, index) => String(first + index));
}

const CLAUDE_MODEL = /^claude-([a-z]+)-(\d+)(?:-(\d{1,2}))?(?:-\d{8})?$/i;

/** A readable model name: Claude ids read like the price page (`Claude Opus 5.5`); others stay as logged. */
export function modelLabel(model: string): string {
  const short = shortModelLabel(model);
  return short === model ? model : `Claude ${short}`;
}

/** The model name without the brand where space is tight: `Opus 5.5`; other ids stay as logged. */
export function shortModelLabel(model: string): string {
  const match = CLAUDE_MODEL.exec(model);
  if (!match) return model;
  const family = match[1]!;
  return `${family.charAt(0).toUpperCase()}${family.slice(1).toLowerCase()} ${match[2]}${match[3] ? `.${match[3]}` : ""}`;
}
