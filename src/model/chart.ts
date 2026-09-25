/**
 * Usage Trend helpers: bar heights for the inline sparkline and the hover chart (upstream
 * `UsageSparkline` / `UsageTrendDetail`), the per-day readout, and day labels in the display language
 * (the core sends English `Sep 26` labels).
 */
import { messagesFor, type Language } from "@/i18n";
import type { MetricChartPoint } from "@/lib/types";
import { formatNumber } from "./format";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAY_LABEL = /^([A-Z][a-z]{2}) (\d{1,2})$/;

/** A chart day label in `language`: `Sep 26` stays in English, becomes `26/9` in Vietnamese. */
export function chartDayLabel(label: string, language: Language): string {
  if (language === "en") return label;
  const match = DAY_LABEL.exec(label);
  if (!match) return label;
  const month = MONTHS.indexOf(match[1]!);
  return month < 0 ? label : `${Number(match[2])}/${month + 1}`;
}

/** The token readout for one day, e.g. `12.3K tokens` / `12,3 N token`. */
export function pointReadout(point: MetricChartPoint, language: Language): string {
  return messagesFor(language).dashboard.tokensReadout(formatNumber(point.value, "count", "row", language));
}

export function peakIndex(points: readonly MetricChartPoint[]): number | null {
  let best: number | null = null;
  points.forEach((point, index) => {
    if (best === null || point.value > points[best]!.value) best = index;
  });
  return best;
}

/** Inline sparkline bar height: zero days show a 2px stub, others at least 18% of the strip. */
export function sparklineBarHeight(value: number, max: number, height: number): number {
  if (!(value > 0)) return 2;
  return Math.max(height * 0.18, height * Math.min(1, value / Math.max(1, max)));
}

/** Hover chart bar height: zero days show a 2px stub, others at least 6% of the chart. */
export function detailBarHeight(value: number, max: number, height: number): number {
  if (!(value > 0)) return 2;
  return Math.max(height * 0.06, height * Math.min(1, value / Math.max(1, max)));
}

export function chartMax(points: readonly MetricChartPoint[]): number {
  return Math.max(1, ...points.map((point) => point.value));
}
