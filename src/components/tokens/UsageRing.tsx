/**
 * The Token ring over the usage ledger: a metric menu (tokens, cost, cost per million tokens), what the
 * ring is split by (source, model, project) and the period (today, 30 days, a year, everything). The
 * five largest parts are drawn, the rest fold into "Khác"; costs show in đồng on the Vietnamese UI.
 */
import { useMemo } from "react";
import { messagesFor, type Language } from "@/i18n";
import type { ExchangeRate, UsageGroupRow } from "@/lib/types";
import { moneyRingCenter, formatMoney } from "@/model/currency";
import { totalSpendRingCenter, type TotalSpendMetric } from "@/model/format";
import { colorOf, OTHER_COLOR, SOURCE_COLORS } from "@/model/palette";
import { ringSectorPath } from "@/model/ringPath";
import { TOKEN_RING_BYS, TOTAL_SPEND_PERIODS, type TokenRingBy } from "@/model/settings";
import { TOTAL_SPEND_METRICS } from "@/model/totalSpend";
import { grandTotal, metricAmount, modelLabel, OTHER_KEY, shortModelLabel, periodQuery, rankedSlices, ringFractions, totalsByKey, totalsBySource, USAGE_SOURCES, type KeyTotals, type Slice } from "@/model/usage";
import { useSettings } from "@/state/hooks";
import { updateSettings, useApp } from "@/state/store";
import { useToday, useUsageColors, useUsageRows, type UsageColors } from "@/state/usage";
import { ChevronDown } from "../ui/icons";
import { openMenuAt } from "../ui/menu";
import { tooltipProps } from "../ui/tooltip";
import { Capsules, tokenCount, UsageNote } from "./parts";

const RING_SIZE = 92;
const RING_PARTS = 5;

function ringItems(rows: readonly UsageGroupRow[], by: TokenRingBy): KeyTotals[] {
  if (by !== "source") return totalsByKey(rows);
  const bySource = totalsBySource(rows);
  return USAGE_SOURCES.map((source) => ({ key: source, total: bySource[source], bySource: { [source]: bySource[source] }, source }));
}

export function sliceLabel(slice: Pick<Slice, "key">, by: TokenRingBy, language: Language, short = false): string {
  const messages = messagesFor(language).usage;
  if (slice.key === OTHER_KEY) return messages.other;
  if (by === "source") return messages.source(slice.key as "claude" | "codex");
  if (by === "model") return short ? shortModelLabel(slice.key) : modelLabel(slice.key);
  return slice.key || messages.unknownProject;
}

export function sliceColor(slice: Pick<Slice, "key">, by: TokenRingBy, colors: UsageColors): string {
  if (slice.key === OTHER_KEY) return OTHER_COLOR;
  if (by === "source") return SOURCE_COLORS[slice.key as "claude" | "codex"] ?? OTHER_COLOR;
  return colorOf(by === "model" ? colors.models : colors.projects, slice.key);
}

function amountText(value: number, metric: TotalSpendMetric, language: Language, rate: ExchangeRate | null): string {
  switch (metric) {
    case "tokens":
      return tokenCount(value, language);
    case "cost":
      return formatMoney(value, language, rate);
    case "costPerMtok":
      return messagesFor(language).totalSpend.costPerMtok(formatMoney(value, language, rate));
  }
}

export function UsageRingCard() {
  const settings = useSettings();
  const language = settings.language;
  const messages = messagesFor(language);
  const today = useToday();
  const rate = useApp((state) => state.exchangeRate);
  const colors = useUsageColors();
  const metric = settings.totalSpendMetric;
  const by = settings.tokenRingBy;
  const period = settings.totalSpendPeriod;
  const { rows, failed } = useUsageRows(periodQuery(period, today, by === "source" ? "year" : by));

  const slices = useMemo(() => (rows ? rankedSlices(ringItems(rows, by), metric, RING_PARTS) : null), [rows, by, metric]);
  const total = rows ? metricAmount(grandTotal(rows), metric) : null;

  const menu = (element: HTMLElement) =>
    openMenuAt(
      element,
      [
        ...TOTAL_SPEND_METRICS.map((option) => ({
          kind: "item" as const,
          label: messages.totalSpend.metric(option),
          checked: option === metric,
          onSelect: () => updateSettings({ totalSpendMetric: option }),
        })),
        { kind: "separator" as const },
        ...TOKEN_RING_BYS.map((option) => ({
          kind: "item" as const,
          label: `${messages.usage.ringByLabel}: ${messages.usage.ringBy(option)}`,
          checked: option === by,
          onSelect: () => updateSettings({ tokenRingBy: option }),
        })),
      ],
      { checkable: true },
    );

  return (
    <section className="uc-section">
      <div className="uc-section-header is-spend">
        <button
          type="button"
          className="uc-spend-metric"
          aria-haspopup="menu"
          aria-label={`${messages.totalSpend.metricMenuLabel}: ${messages.totalSpend.metric(metric)}`}
          onClick={(event) => menu(event.currentTarget)}
        >
          <span>{messages.totalSpend.metric(metric)}</span>
          <span className="uc-spend-by">· {messages.usage.ringBy(by)}</span>
          <ChevronDown size={9} />
        </button>
      </div>
      <div className="uc-card uc-spend-card">
        <Capsules
          label={messages.usage.periodLabel}
          options={TOTAL_SPEND_PERIODS.map((value) => ({ value, label: messages.usage.period(value) }))}
          value={period}
          onChange={(value) => updateSettings({ totalSpendPeriod: value })}
        />
        {slices === null ? (
          <UsageNote text={failed ? messages.usage.failed : messages.usage.loading} />
        ) : slices.length === 0 || total === null ? (
          <p className="uc-spend-empty">{messages.totalSpend.empty(metric)}</p>
        ) : (
          <Ring slices={slices} total={total} by={by} metric={metric} colors={colors} language={language} rate={rate} />
        )}
      </div>
    </section>
  );
}

function Ring({
  slices,
  total,
  by,
  metric,
  colors,
  language,
  rate,
}: {
  slices: Slice[];
  total: number;
  by: TokenRingBy;
  metric: TotalSpendMetric;
  colors: UsageColors;
  language: Language;
  rate: ExchangeRate | null;
}) {
  const messages = messagesFor(language);
  const arcs = ringFractions(slices.map((slice) => slice.amount));
  const center = metric === "tokens" ? totalSpendRingCenter(total, "tokens", language) : moneyRingCenter(total, metric, language, rate);
  const exact = metric === "tokens" ? messages.usage.tokens(total.toLocaleString(language === "vi" ? "vi-VN" : "en-US")) : amountText(total, metric, language, rate);
  return (
    <div className="uc-ring-content is-usage">
      <div className="uc-ring" role="img" aria-label={messages.usage.ringAria(exact, slices.length)}>
        <svg width={RING_SIZE} height={RING_SIZE} viewBox={`0 0 ${RING_SIZE} ${RING_SIZE}`} aria-hidden="true">
          {arcs.map((arc, index) => (
            <path key={slices[index]!.key} d={ringSectorPath(arc.start, arc.end, { size: RING_SIZE })} fill={sliceColor(slices[index]!, by, colors)} />
          ))}
        </svg>
        <div className="uc-ring-center" {...tooltipProps(exact)}>
          <span className="uc-ring-primary uc-num">{center.primary}</span>
          <span className="uc-ring-unit">{center.unit}</span>
        </div>
      </div>
      <ul className="uc-legend">
        {slices.map((slice) => (
          <li key={slice.key} className="uc-legend-row">
            <span className="uc-legend-dot" style={{ background: sliceColor(slice, by, colors) }} />
            <span className="uc-legend-name uc-truncate" {...tooltipProps(sliceLabel(slice, by, language))}>
              {sliceLabel(slice, by, language, true)}
            </span>
            <span className="uc-legend-value uc-num">{amountText(slice.amount, metric, language, rate)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
