/**
 * Copy for the Token tab's views (usage history, charts, projects, context windows) and the Bảng giá
 * tab, kept apart from `messages.ts` so each catalog stays readable.
 */
import type { UsageSource } from "@/lib/types";
import type { PriceCurrency, PriceTier, TokenChart, TokenChartMetric, TokenRingBy, TokenView, TotalSpendPeriod } from "@/model/settings";

/** The parts of a context window bar, left to right. */
export type ContextSegmentKey = "base" | "conversation" | "lastTurn" | "free";

/** The scale a large đồng amount is shown in, e.g. `14,3 triệu đồng`. */
export type DongScale = "billion" | "million" | "thousand" | "one";

export interface UsageMessages {
  viewsLabel: string;
  view(key: TokenView): string;
  periodLabel: string;
  period(key: TotalSpendPeriod): string;
  loading: string;
  importing: string;
  failed: string;
  noData: string;
  source(key: UsageSource): string;
  other: string;
  unknownProject: string;
  tokens(count: string): string;
  ringByLabel: string;
  ringBy(key: TokenRingBy): string;
  ringAria(total: string, parts: number): string;
  yearsTitle: string;
  allTime: string;
  since(date: string): string;
  year(year: string, current: boolean): string;
  rateNote(rate: string, time: string, stale: boolean): string;
  dongUnit(scale: DongScale): string;
  monthTitle(year: number, month: number): string;
  previousMonth: string;
  nextMonth: string;
  monthTotal: string;
  dayTitle(date: Date): string;
  dayShort(date: Date): string;
  back(label: string): string;
  bySource: string;
  byModel: string;
  byProject: string;
  noUsage: string;
  chartLabel: string;
  chart(key: TokenChart): string;
  chartMetricLabel: string;
  chartMetric(key: TokenChartMetric): string;
  chartCaption(key: TokenChart): string;
  chartPeak(value: string): string;
  showAll(count: number): string;
  showFewer: string;
  contextTitle: string;
  contextHint: string;
  contextEmpty: string;
  contextUsage(used: string, window: string): string;
  contextUsed(used: string): string;
  contextSegment(key: ContextSegmentKey): string;
  contextApp(source: UsageSource): string;
  ago(duration: string | null): string;
}

export interface PriceMessages {
  providerLabel: string;
  tierLabel: string;
  tier(key: PriceTier): string;
  currencyLabel: string;
  currency(key: PriceCurrency): string;
  section(id: string): string;
  /** A column, group, heading or cell word from the price pages in this language, or the text itself. */
  term(text: string): string;
  /** The unit every bare price in the tables is in. */
  unitNote(currency: PriceCurrency): string;
  source(host: string, date: string): string;
  openSource(host: string): string;
}
