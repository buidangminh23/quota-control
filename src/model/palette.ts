/**
 * Chart colors. Claude and Codex keep their brand colors; a model takes a warm hue when it runs in
 * Claude and a cool one in Codex, and projects walk the full palette. Colors are handed out along the
 * all-time ranking (`assignColors`), so a model or project keeps one color in every view.
 */
import type { UsageSource } from "@/lib/types";

export const SOURCE_COLORS: Readonly<Record<UsageSource, string>> = { claude: "#DE7356", codex: "#10A37F" };

/** Distinct saturated hues that read on both the light and the dark card. */
export const CHART_PALETTE = [
  "#0A84FF",
  "#FF9F0A",
  "#30D158",
  "#BF5AF2",
  "#FF375F",
  "#FFC107",
  "#64D2FF",
  "#5E5CE6",
  "#FF6B6B",
  "#2EC4B6",
  "#E056FD",
  "#A2845E",
] as const;

const WARM = ["#FF7A45", "#FF375F", "#FF9F0A", "#E0457B", "#FFC107", "#C2410C"] as const;
const COOL = ["#10A37F", "#0A84FF", "#5E5CE6", "#64D2FF", "#30D158", "#2EC4B6"] as const;

/** The "other" slice and anything without a ranking. */
export const OTHER_COLOR = "#8E8E93";

/** Each dashboard tab's accent in the tab bar. */
export const TAB_COLORS = {
  quota: "#0A84FF",
  tokens: "#BF5AF2",
  prices: "#FF9F0A",
} as const;

/** The Token views' own accents, used for their picker and card icons. */
export const VIEW_COLORS = {
  overview: "#0A84FF",
  history: "#BF5AF2",
  charts: "#FF9F0A",
  projects: "#30D158",
  context: "#5E5CE6",
  years: "#FF375F",
  prices: "#FF9F0A",
} as const;

export interface RankedKey {
  key: string;
  /** Present for models: their source picks the warm or the cool family. */
  source?: UsageSource;
}

/** Colors for keys in ranked order; keys with a source draw from that source's family. */
export function assignColors(ranked: readonly RankedKey[]): Map<string, string> {
  const colors = new Map<string, string>();
  const nextInFamily: Partial<Record<UsageSource, number>> = {};
  let next = 0;
  for (const item of ranked) {
    if (colors.has(item.key)) continue;
    if (item.source) {
      const family = item.source === "claude" ? WARM : COOL;
      const index = nextInFamily[item.source] ?? 0;
      colors.set(item.key, family[index % family.length]!);
      nextInFamily[item.source] = index + 1;
    } else {
      colors.set(item.key, CHART_PALETTE[next % CHART_PALETTE.length]!);
      next += 1;
    }
  }
  return colors;
}

/** A key's color, or a stable hue from its name when the ranking has not seen it yet. */
export function colorOf(colors: ReadonlyMap<string, string>, key: string): string {
  const known = colors.get(key);
  if (known) return known;
  let hash = 0;
  for (const char of key) hash = (hash * 31 + char.codePointAt(0)!) & 0xffff;
  return CHART_PALETTE[hash % CHART_PALETTE.length]!;
}
