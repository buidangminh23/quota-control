/**
 * Biểu đồ: token use or cost compared over the last 30 days, the last 12 months or every year (columns
 * stacked Claude over Codex), or ranked by model or project for a period (colored bars).
 */
import { useMemo } from "react";
import { messagesFor, type Language } from "@/i18n";
import { addDays, dayDate, daysBetween, monthRange } from "@/lib/days";
import type { ExchangeRate, UsageGrouping, UsageQuery, UsageTotals } from "@/lib/types";
import { formatMoney } from "@/model/currency";
import { colorOf, OTHER_COLOR, SOURCE_COLORS, VIEW_COLORS } from "@/model/palette";
import { TOKEN_CHART_METRICS, TOKEN_CHARTS, TOTAL_SPEND_PERIODS, type TokenChart, type TokenChartMetric } from "@/model/settings";
import { lastMonths, metricAmount, modelLabel, OTHER_KEY, periodQuery, rankedSlices, series, totalsByKey, USAGE_SOURCES, yearSpan, type SeriesPoint } from "@/model/usage";
import { useSettings } from "@/state/hooks";
import { updateSettings, useApp } from "@/state/store";
import { useToday, useUsageColors, useUsageRows } from "@/state/usage";
import { BarChartIcon } from "../ui/icons";
import { Capsules, ChipHeader, RankedBars, SourceLegend, StackedColumns, tokenCount, UsageNote, type RankedBar, type StackColumn } from "./parts";

const DAYS = 30;
const MONTHS = 12;
const RANKED_BARS = 10;

function amountOf(totals: UsageTotals, metric: TokenChartMetric): number {
  return metricAmount(totals, metric) ?? 0;
}

function amountText(value: number, metric: TokenChartMetric, language: Language, rate: ExchangeRate | null): string {
  return metric === "tokens" ? tokenCount(value, language) : formatMoney(value, language, rate);
}

function timeQuery(chart: TokenChart, today: string): UsageQuery {
  if (chart === "day") return { from: addDays(today, -(DAYS - 1)), to: today, groupBy: "day" };
  if (chart === "month") return { from: monthRange(lastMonths(today, MONTHS)[0]!).from, to: today, groupBy: "month" };
  return { to: today, groupBy: "year" };
}

function timeKeys(chart: TokenChart, today: string, years: string[]): string[] {
  if (chart === "day") return daysBetween(addDays(today, -(DAYS - 1)), today);
  if (chart === "month") return lastMonths(today, MONTHS);
  return years;
}

function pointLabel(chart: TokenChart, key: string, language: Language): string {
  const messages = messagesFor(language).usage;
  if (chart === "day") return messages.dayShort(dayDate(key));
  if (chart === "month") {
    const [year, month] = key.split("-").map(Number);
    return messages.monthTitle(year!, month!);
  }
  return key;
}

function axisLabel(chart: TokenChart, key: string, language: Language): string {
  if (chart === "day") return messagesFor(language).format.monthDay(dayDate(key));
  if (chart === "month") {
    const [year, month] = key.split("-");
    return language === "vi" ? `${Number(month)}/${year!.slice(2)}` : `${month}/${year!.slice(2)}`;
  }
  return key;
}

function TimeChart({ chart, metric, language, rate }: { chart: TokenChart; metric: TokenChartMetric; language: Language; rate: ExchangeRate | null }) {
  const messages = messagesFor(language).usage;
  const today = useToday();
  const { rows, failed } = useUsageRows(timeQuery(chart, today));
  const points: SeriesPoint[] = useMemo(() => (rows ? series(rows, timeKeys(chart, today, yearSpan(rows, today))) : []), [rows, chart, today]);
  if (!rows) return <UsageNote text={failed ? messages.failed : messages.loading} />;
  if (points.every((point) => point.total.totalTokens === 0)) return <UsageNote text={messages.noData} />;

  const columns: StackColumn[] = points.map((point) => {
    const lines = [
      pointLabel(chart, point.key, language),
      ...USAGE_SOURCES.map((source) => `${messages.source(source)}: ${amountText(amountOf(point.bySource[source], metric), metric, language, rate)}`),
    ];
    return {
      key: point.key,
      segments: USAGE_SOURCES.map((source) => ({ color: SOURCE_COLORS[source], value: amountOf(point.bySource[source], metric) })),
      tooltip: lines.join("\n"),
      current: point.key === points[points.length - 1]!.key,
    };
  });
  const peak = Math.max(...points.map((point) => amountOf(point.total, metric)));
  const middle = points[Math.floor((points.length - 1) / 2)]!;
  const axis = points.length > 2 ? [points[0]!, middle, points[points.length - 1]!] : points;
  const sums = Object.fromEntries(
    USAGE_SOURCES.map((source) => [source, amountText(points.reduce((sum, point) => sum + amountOf(point.bySource[source], metric), 0), metric, language, rate)]),
  );
  return (
    <>
      <StackedColumns columns={columns} peak={messages.chartPeak(amountText(peak, metric, language, rate))} axis={axis.map((point) => axisLabel(chart, point.key, language))} />
      <SourceLegend language={language} values={sums} />
    </>
  );
}

function RankingChart({ chart, metric, language, rate }: { chart: "model" | "project"; metric: TokenChartMetric; language: Language; rate: ExchangeRate | null }) {
  const settings = useSettings();
  const messages = messagesFor(language).usage;
  const today = useToday();
  const colors = useUsageColors();
  const groupBy: UsageGrouping = chart;
  const { rows, failed } = useUsageRows(periodQuery(settings.tokenChartPeriod, today, groupBy));
  const bars: RankedBar[] = useMemo(() => {
    if (!rows) return [];
    return rankedSlices(totalsByKey(rows), metric, RANKED_BARS).map((slice) => {
      const other = slice.key === OTHER_KEY;
      const label = other ? messages.other : chart === "model" ? modelLabel(slice.key) : slice.key || messages.unknownProject;
      const secondary = metric === "tokens" ? slice.totals.costUSD : undefined;
      return {
        key: slice.key,
        label,
        color: other ? OTHER_COLOR : colorOf(chart === "model" ? colors.models : colors.projects, slice.key),
        value: slice.amount,
        valueText: amountText(slice.amount, metric, language, rate),
        detail: secondary !== undefined ? formatMoney(secondary, language, rate) : metric === "cost" ? messages.tokens(tokenCount(slice.totals.totalTokens, language)) : undefined,
        tooltip: label,
      };
    });
  }, [rows, metric, chart, colors, language, rate, messages]);
  return (
    <>
      <Capsules
        label={messages.periodLabel}
        options={TOTAL_SPEND_PERIODS.map((value) => ({ value, label: messages.period(value) }))}
        value={settings.tokenChartPeriod}
        onChange={(value) => updateSettings({ tokenChartPeriod: value })}
      />
      {!rows ? <UsageNote text={failed ? messages.failed : messages.loading} /> : bars.length === 0 ? <UsageNote text={messages.noData} /> : <RankedBars bars={bars} />}
    </>
  );
}

export function ChartsView() {
  const settings = useSettings();
  const language = settings.language;
  const messages = messagesFor(language).usage;
  const rate = useApp((state) => state.exchangeRate);
  const chart = settings.tokenChart;
  const metric = settings.tokenChartMetric;
  return (
    <section className="uc-section">
      <ChipHeader
        icon={<BarChartIcon size={11} />}
        color={VIEW_COLORS.charts}
        title={messages.chartCaption(chart)}
        trailing={
          <Capsules
            small
            label={messages.chartMetricLabel}
            options={TOKEN_CHART_METRICS.map((value) => ({ value, label: messages.chartMetric(value) }))}
            value={metric}
            onChange={(value) => updateSettings({ tokenChartMetric: value })}
          />
        }
      />
      <div className="uc-card uc-chart-card">
        <Capsules
          label={messages.chartLabel}
          options={TOKEN_CHARTS.map((value) => ({ value, label: messages.chart(value), color: VIEW_COLORS.charts }))}
          value={chart}
          onChange={(value) => updateSettings({ tokenChart: value })}
        />
        {chart === "model" || chart === "project" ? (
          <RankingChart chart={chart} metric={metric} language={language} rate={rate} />
        ) : (
          <TimeChart chart={chart} metric={metric} language={language} rate={rate} />
        )}
      </div>
    </section>
  );
}
