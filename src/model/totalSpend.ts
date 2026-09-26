/**
 * What the Token ring can measure and each brand's own color. The ring itself reads the usage ledger
 * (`usage.ts`); the palette is upstream `TotalSpendPalette`, which also tints each brand's mark.
 */
import type { TotalSpendMetric } from "./format";

export const TOTAL_SPEND_METRICS: readonly TotalSpendMetric[] = ["cost", "costPerMtok", "tokens"];

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

/** A brand's own color in the given appearance (it also tints the brand's mark), or `null` when the palette has none. */
export function knownBrandColor(brand: string, dark: boolean): Hex | null {
  const known = BRAND_COLORS[brand];
  if (!known) return null;
  return dark ? known[1] : known[0];
}
