import type { UsageGroupRow, UsageTotals } from "@/lib/types";
import { assignColors, colorOf, SOURCE_COLORS } from "./palette";
import {
  addTotals,
  grandTotal,
  lastMonths,
  metricAmount,
  modelLabel,
  monthQuery,
  OTHER_KEY,
  periodQuery,
  rankedSlices,
  ringFractions,
  series,
  shortModelLabel,
  totalsByKey,
  totalsBySource,
  yearSpan,
} from "./usage";

function totals(totalTokens: number, costUSD?: number): UsageTotals {
  return { inputTokens: 0, outputTokens: 0, cachedInputTokens: 0, cacheCreationInputTokens: 0, totalTokens, ...(costUSD === undefined ? {} : { costUSD }) };
}

const row = (key: string, source: "claude" | "codex", tokens: number, cost?: number): UsageGroupRow => ({ key, source, totals: totals(tokens, cost) });

describe("usage periods", () => {
  it("turns each period into an inclusive ledger query ending today", () => {
    expect(periodQuery("today", "2026-09-26", "model")).toEqual({ from: "2026-09-26", to: "2026-09-26", groupBy: "model" });
    expect(periodQuery("last30", "2026-09-26", "day")).toEqual({ from: "2026-08-28", to: "2026-09-26", groupBy: "day" });
    expect(periodQuery("last365", "2026-09-26", "year")).toEqual({ from: "2025-09-27", to: "2026-09-26", groupBy: "year" });
    expect(periodQuery("all", "2026-09-26", "project")).toEqual({ to: "2026-09-26", groupBy: "project" });
  });

  it("stops the current month at today and skips months that have not started", () => {
    expect(monthQuery("2026-09", "2026-09-26", "day")).toEqual({ from: "2026-09-01", to: "2026-09-26", groupBy: "day" });
    expect(monthQuery("2026-02", "2026-09-26", "day")).toEqual({ from: "2026-02-01", to: "2026-02-28", groupBy: "day" });
    expect(monthQuery("2026-10", "2026-09-26", "day")).toBeNull();
    expect(lastMonths("2026-02-10", 3)).toEqual(["2025-12", "2026-01", "2026-02"]);
  });
});

describe("usage totals", () => {
  it("keeps a cost only where some row had one", () => {
    expect(addTotals(totals(1), totals(2)).costUSD).toBeUndefined();
    expect(addTotals(totals(1, 0.5), totals(2)).costUSD).toBe(0.5);
    expect(metricAmount(totals(2_000_000, 3), "costPerMtok")).toBeCloseTo(1.5, 10);
    expect(metricAmount(totals(10), "cost")).toBeNull();
  });

  it("merges a key's sources and remembers the larger one", () => {
    const merged = totalsByKey([row("PCC4SH", "claude", 5), row("PCC4SH", "codex", 9), row("bot-tele", "claude", 1)]);
    expect(merged.map((item) => [item.key, item.total.totalTokens, item.source])).toEqual([
      ["PCC4SH", 14, "codex"],
      ["bot-tele", 1, "claude"],
    ]);
    const bySource = totalsBySource([row("a", "claude", 5), row("b", "claude", 2)]);
    expect(bySource.claude.totalTokens).toBe(7);
    expect(bySource.codex.totalTokens).toBe(0);
    expect(grandTotal([row("a", "claude", 5), row("b", "codex", 2)]).totalTokens).toBe(7);
  });

  it("ranks the largest keys and folds the rest into one other slice", () => {
    const items = totalsByKey(["a", "b", "c", "d", "e", "f", "g"].map((key, index) => row(key, "claude", 70 - index * 10, 7 - index)));
    const slices = rankedSlices(items, "tokens", 5);
    expect(slices.map((slice) => slice.key)).toEqual(["a", "b", "c", "d", "e", OTHER_KEY]);
    expect(slices[5]!.amount).toBe(20 + 10);
    expect(slices[5]!.totals.costUSD).toBe(2 + 1);
    expect(rankedSlices(items.slice(0, 6), "tokens", 5)).toHaveLength(6);
    expect(rankedSlices(totalsByKey([row("free", "codex", 10)]), "cost", 5)).toEqual([]);
  });

  it("closes the ring and still shows a sliver for a tiny slice", () => {
    const arcs = ringFractions([1000, 1]);
    expect(arcs.at(-1)!.end).toBeCloseTo(1, 10);
    expect(arcs[1]!.end - arcs[1]!.start).toBeGreaterThan(0.02);
    expect(ringFractions([0, 0])).toEqual([]);
  });

  it("fills a series with zero where nothing was logged", () => {
    const points = series([row("2026-09-02", "codex", 4), row("2026-09-02", "claude", 1)], ["2026-09-01", "2026-09-02"]);
    expect(points.map((point) => point.total.totalTokens)).toEqual([0, 5]);
    expect(points[1]!.bySource.codex.totalTokens).toBe(4);
    expect(yearSpan([row("2024", "claude", 1)], "2026-01-01")).toEqual(["2024", "2025", "2026"]);
    expect(yearSpan([], "2026-01-01")).toEqual(["2026"]);
  });
});

describe("model names and colors", () => {
  it("writes Claude ids like the price page and leaves others as logged", () => {
    expect(modelLabel("claude-opus-5-5")).toBe("Claude Opus 5.5");
    expect(modelLabel("claude-opus-5")).toBe("Claude Opus 5");
    expect(modelLabel("claude-sonnet-4-5-20250929")).toBe("Claude Sonnet 4.5");
    expect(shortModelLabel("claude-fable-5-1")).toBe("Fable 5.1");
    expect(modelLabel("gpt-5.6-sol")).toBe("gpt-5.6-sol");
    expect(modelLabel("codex-auto-review")).toBe("codex-auto-review");
  });

  it("gives Claude models warm hues and Codex models cool ones, stable by ranking", () => {
    const colors = assignColors([
      { key: "claude-opus-5", source: "claude" },
      { key: "gpt-5.6-sol", source: "codex" },
      { key: "claude-opus-5-5", source: "claude" },
    ]);
    expect(colors.get("claude-opus-5")).toBe("#FF7A45");
    expect(colors.get("gpt-5.6-sol")).toBe("#10A37F");
    expect(colors.get("claude-opus-5-5")).toBe("#FF375F");
    expect(colorOf(colors, "unranked")).toBe(colorOf(colors, "unranked"));
    expect(SOURCE_COLORS.claude).toBe("#DE7356");
  });
});
