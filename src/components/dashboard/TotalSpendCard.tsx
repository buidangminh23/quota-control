/**
 * Total Spend across providers (upstream `TotalSpendCard`): a metric menu header, a capsule period
 * picker and the ring with its ranked legend. Only enabled providers that ship spend tiles count.
 */
import { useMemo } from "react";
import { messagesFor, type Language } from "@/i18n";
import type { Provider } from "@/lib/types";
import { formatCostPerMtok, formatNumber, totalSpendRingCenter, type TotalSpendMetric } from "@/model/format";
import { spendCapableProviders } from "@/model/layout";
import { providerBrand, spendLegendName } from "@/model/providerText";
import { ringSectorPath } from "@/model/ringPath";
import type { TotalSpendPeriod } from "@/model/settings";
import { brandColor, projectTotalSpend, ringArcs, TOTAL_SPEND_METRICS, TOTAL_SPEND_PERIODS, totalSpendSlices, type TotalSpendProjection } from "@/model/totalSpend";
import { useIsDark, useIsEnabled, useSettings } from "@/state/hooks";
import { updateSettings, useApp } from "@/state/store";
import { ChevronDown, InfoCircle } from "../ui/icons";
import { openMenuAt } from "../ui/menu";
import { tooltipProps } from "../ui/tooltip";

const RING_SIZE = 104;

function formatAmount(value: number, metric: TotalSpendMetric, style: "row" | "full", language: Language): string {
  switch (metric) {
    case "cost":
      return formatNumber(value, "dollars", style, language);
    case "tokens":
      return formatNumber(value, "count", style, language);
    case "costPerMtok":
      return formatCostPerMtok(value, style, language);
  }
}

export function TotalSpendCard() {
  const settings = useSettings();
  const language = settings.language;
  const messages = messagesFor(language);
  const catalog = useApp((state) => state.catalog);
  const layout = useApp((state) => state.layout);
  const engine = useApp((state) => state.engine);
  const isEnabled = useIsEnabled();
  const providers = useMemo(() => spendCapableProviders(layout, catalog, isEnabled), [layout, catalog, isEnabled]);
  const metric = settings.totalSpendMetric;
  const period = settings.totalSpendPeriod;
  const projection = useMemo(() => {
    const snapshots = Object.fromEntries(Object.entries(engine?.providers ?? {}).map(([id, runtime]) => [id, runtime.snapshot]));
    const collator = new Intl.Collator(language === "vi" ? "vi-VN" : "en-US");
    return projectTotalSpend(totalSpendSlices(period, providers, snapshots), metric, collator.compare);
  }, [engine, providers, period, metric, language]);

  const names = messages.format.list(providers.map((provider) => spendLegendName(provider, language)));
  const metricMenu = (element: HTMLElement) =>
    openMenuAt(
      element,
      TOTAL_SPEND_METRICS.map((option) => ({
        kind: "item" as const,
        label: messages.totalSpend.metric(option),
        checked: option === metric,
        onSelect: () => updateSettings({ totalSpendMetric: option }),
      })),
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
          onClick={(event) => metricMenu(event.currentTarget)}
        >
          <span>{messages.totalSpend.metric(metric)}</span>
          <ChevronDown size={9} />
        </button>
        <span className="uc-inline-icon uc-secondary" aria-label={messages.totalSpend.onlyIncludes(names)} {...tooltipProps(messages.totalSpend.onlyIncludes(names))}>
          <InfoCircle size={12} />
        </span>
      </div>
      <div className="uc-card uc-spend-card">
        <PeriodPicker period={period} language={language} />
        {projection.slices.length === 0 ? (
          <p className="uc-spend-empty">{messages.totalSpend.empty(metric)}</p>
        ) : (
          <RingContent projection={projection} language={language} providers={providers} />
        )}
      </div>
    </section>
  );
}

function PeriodPicker({ period, language }: { period: TotalSpendPeriod; language: Language }) {
  const messages = messagesFor(language).totalSpend;
  return (
    <div className="uc-capsule-picker" role="radiogroup" aria-label={messages.periodLabel}>
      {TOTAL_SPEND_PERIODS.map((candidate) => (
        <button
          key={candidate}
          type="button"
          role="radio"
          aria-checked={candidate === period}
          className={`uc-capsule-segment${candidate === period ? " is-selected" : ""}`}
          onClick={() => updateSettings({ totalSpendPeriod: candidate })}
        >
          {messages.period(candidate)}
        </button>
      ))}
    </div>
  );
}

function RingContent({ projection, language, providers }: { projection: TotalSpendProjection; language: Language; providers: Provider[] }) {
  const dark = useIsDark();
  const messages = messagesFor(language);
  const arcs = ringArcs(projection);
  const center = totalSpendRingCenter(projection.center, projection.metric, language);
  const exact = formatAmount(projection.center, projection.metric, "full", language);
  const centerTooltip = projection.estimated ? `${exact} · ${messages.meter.localEstimateNote}` : exact;
  const count = projection.slices.length;
  const aria =
    projection.metric === "cost"
      ? messages.totalSpend.totalCostAria(exact, count)
      : projection.metric === "tokens"
        ? messages.totalSpend.totalTokensAria(exact, count)
        : messages.totalSpend.blendedRateAria(exact, count);
  const byId = new Map(providers.map((provider) => [provider.id, provider]));
  const colorOf = (providerId: string) => brandColor(providerBrand(byId.get(providerId) ?? { id: providerId, displayName: providerId, icon: "" }), dark);
  return (
    <div className="uc-ring-content">
      <div className="uc-ring" role="img" aria-label={aria}>
        <svg width={RING_SIZE} height={RING_SIZE} viewBox={`0 0 ${RING_SIZE} ${RING_SIZE}`} aria-hidden="true">
          {arcs.map((arc) => (
            <path key={arc.providerId} d={ringSectorPath(arc.start, arc.end, { size: RING_SIZE })} fill={colorOf(arc.providerId)} />
          ))}
        </svg>
        <div className="uc-ring-center" {...tooltipProps(centerTooltip)}>
          <span className="uc-ring-primary uc-num">{center.primary}</span>
          <span className="uc-ring-unit">{center.unit}</span>
        </div>
      </div>
      <ul className="uc-legend">
        {projection.slices.map((slice) => (
          <li key={slice.provider.id} className="uc-legend-row">
            <span className="uc-legend-dot" style={{ background: colorOf(slice.provider.id) }} />
            <span className="uc-legend-name uc-truncate">{spendLegendName(slice.provider, language)}</span>
            <span className="uc-legend-value uc-num">{formatAmount(slice.amount, projection.metric, projection.metric === "tokens" ? "row" : "full", language)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
