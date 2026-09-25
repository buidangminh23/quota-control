/**
 * Cross-provider Total Spend: sums each spend-capable provider's Today / Yesterday / Last 30 Days line
 * into ranked slices for the ring. Port of upstream `Support/TotalSpendAggregator.swift` and the ring
 * geometry + palette of `Views/TotalSpendCard.swift`.
 *
 * A provider contributes only when its snapshot carries the period's `values` line with dollars
 * and/or tokens; an idle period is excluded, never counted as zero.
 */
import type { Provider, ProviderSnapshot } from "@/lib/types";
import type { TotalSpendMetric } from "./format";
import type { TotalSpendPeriod } from "./settings";

/** The metric-line labels the spend tiles emit (English source text, stable on the wire). */
export const PERIOD_LINE_LABELS: Readonly<Record<TotalSpendPeriod, string>> = {
  today: "Today",
  yesterday: "Yesterday",
  last30: "Last 30 Days",
};

export const TOTAL_SPEND_PERIODS: readonly TotalSpendPeriod[] = ["today", "yesterday", "last30"];
export const TOTAL_SPEND_METRICS: readonly TotalSpendMetric[] = ["cost", "costPerMtok", "tokens"];

export interface TotalSpendSlice {
  provider: Provider;
  amountUSD: number;
  tokenCount: number;
  /** The dollars are a local estimate (log-scanned) rather than measured. */
  estimated: boolean;
}

export interface ProjectedSlice {
  provider: Provider;
  /** The amount that sizes the ring and ranks the legend under the chosen metric. */
  amount: number;
  estimated: boolean;
}

export interface TotalSpendProjection {
  metric: TotalSpendMetric;
  slices: ProjectedSlice[];
  center: number;
  estimated: boolean;
}

function costPerMtok(slice: TotalSpendSlice): number | null {
  return slice.amountUSD > 0 && slice.tokenCount > 0 ? (slice.amountUSD / slice.tokenCount) * 1_000_000 : null;
}

/** Every provider (in the given order) that had dollars or tokens in `period`. */
export function totalSpendSlices(
  period: TotalSpendPeriod,
  providers: readonly Provider[],
  snapshots: Readonly<Record<string, ProviderSnapshot | undefined>>,
): TotalSpendSlice[] {
  const label = PERIOD_LINE_LABELS[period];
  return providers.flatMap((provider) => {
    const line = snapshots[provider.id]?.lines.find((candidate) => candidate.label === label);
    if (!line || line.type !== "values") return [];
    const dollars = line.values.filter((value) => value.kind === "dollars");
    const amount = dollars.reduce((sum, value) => sum + value.number, 0);
    const tokens = line.values
      .filter((value) => value.kind === "count" && value.label === "tokens")
      .reduce((sum, value) => sum + value.number, 0);
    if (!(amount > 0) && !(tokens > 0)) return [];
    return [{ provider, amountUSD: Math.max(amount, 0), tokenCount: Math.max(tokens, 0), estimated: dollars.some((value) => value.estimated) }];
  });
}

/** Filter, rank and total the slices for one metric (upstream `TotalSpend.projection(for:)`). */
export function projectTotalSpend(slices: readonly TotalSpendSlice[], metric: TotalSpendMetric, compareNames: (a: string, b: string) => number): TotalSpendProjection {
  const included = slices.flatMap((slice) => {
    const amount = metric === "cost" ? slice.amountUSD : metric === "tokens" ? slice.tokenCount : costPerMtok(slice);
    return amount !== null && amount > 0 ? [{ slice, amount }] : [];
  });
  included.sort((a, b) => (a.amount !== b.amount ? b.amount - a.amount : compareNames(a.slice.provider.displayName, b.slice.provider.displayName)));
  const dollars = included.reduce((sum, item) => sum + item.slice.amountUSD, 0);
  const tokens = included.reduce((sum, item) => sum + item.slice.tokenCount, 0);
  const anyEstimated = included.some((item) => item.slice.estimated);
  const center = metric === "cost" ? dollars : metric === "tokens" ? tokens : tokens > 0 ? (dollars / tokens) * 1_000_000 : 0;
  return {
    metric,
    slices: included.map(({ slice, amount }) => ({ provider: slice.provider, amount, estimated: slice.estimated })),
    center,
    estimated: metric === "tokens" ? false : anyEstimated,
  };
}

export interface RingArc {
  providerId: string;
  /** Clockwise from 12 o'clock, 0...1 around the ring. */
  start: number;
  end: number;
}

/** Every slice gets at least this share so a tiny provider still shows a sliver (presentation only). */
export const MINIMUM_SLICE_SHARE = 0.025;

/** Ranked slices as cumulative ring fractions, floored and renormalized so the ring always closes. */
export function ringArcs(projection: TotalSpendProjection): RingArc[] {
  const total = projection.slices.reduce((sum, slice) => sum + slice.amount, 0);
  if (!(total > 0)) return [];
  const floored = projection.slices.map((slice) => Math.max(slice.amount / total, MINIMUM_SLICE_SHARE));
  const sum = floored.reduce((acc, share) => acc + share, 0);
  let cursor = 0;
  return projection.slices.map((slice, index) => {
    const width = floored[index]! / sum;
    const arc = { providerId: slice.provider.id, start: cursor, end: cursor + width };
    cursor += width;
    return arc;
  });
}

type Hex = `#${string}`;

/** Brand tints keyed by brand (never by rank), from upstream `TotalSpendPalette`; `[light, dark]`. */
const BRAND_COLORS: Readonly<Record<string, readonly [Hex, Hex]>> = {
  claude: ["#DE7356", "#DE7356"],
  codex: ["#10A37F", "#10A37F"],
  cursor: ["#13120A", "#F5F5F7"],
  grok: ["#8E8E93", "#98989D"],
  opencode: ["#6E6E73", "#AEAEB2"],
  openrouter: ["#6467F2", "#6467F2"],
  antigravity: ["#4285F4", "#4285F4"],
  copilot: ["#A855F7", "#A855F7"],
  amp: ["#F34E3F", "#F34E3F"],
  factory: ["#48484A", "#C7C7CC"],
  kimi: ["#0A66FF", "#0A66FF"],
  minimax: ["#F5433C", "#F5433C"],
  zai: ["#2D2D2D", "#D1D1D6"],
};

const FALLBACK_COLORS: readonly Hex[] = ["#34C759", "#5856D6", "#FF2D55", "#A2845E"];

/** The ring/legend color for a brand in the given appearance. */
export function brandColor(brand: string, dark: boolean): Hex {
  const known = BRAND_COLORS[brand];
  if (known) return dark ? known[1] : known[0];
  let hash = 0;
  for (const char of brand) hash = (hash * 31 + char.codePointAt(0)!) & 0xffff;
  return FALLBACK_COLORS[hash % FALLBACK_COLORS.length]!;
}
