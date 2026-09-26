/**
 * Tổng quan: the live context windows, the ring for a period, every year's total with the all-time sum,
 * then each source's usage trend and period rows.
 */
import { useMemo } from "react";
import { messagesFor } from "@/i18n";
import { formatMoney } from "@/model/currency";
import { tokenGroups } from "@/model/layout";
import { VIEW_COLORS } from "@/model/palette";
import { grandTotal, series, totalsBySource, yearSpan } from "@/model/usage";
import { useIsEnabled, useSettings } from "@/state/hooks";
import { useApp } from "@/state/store";
import { useToday, useUsageRows } from "@/state/usage";
import { ProviderSections } from "../dashboard/ProviderSections";
import { CalendarIcon } from "../ui/icons";
import { ContextWindowsCard } from "./ContextWindows";
import { ChipHeader, dayText, SourceSplit, tokenCount, UsageNote } from "./parts";
import { UsageRingCard } from "./UsageRing";

function YearsCard() {
  const language = useSettings().language;
  const messages = messagesFor(language).usage;
  const today = useToday();
  const rate = useApp((state) => state.exchangeRate);
  const firstDay = useApp((state) => state.ledgerInfo?.firstDay ?? null);
  const { rows, failed } = useUsageRows({ to: today, groupBy: "year" });
  const years = useMemo(() => (rows ? series(rows, yearSpan(rows, today)).reverse() : []), [rows, today]);

  const content = () => {
    if (!rows) return <UsageNote text={failed ? messages.failed : messages.loading} />;
    if (rows.length === 0) return <UsageNote text={messages.noData} />;
    const total = grandTotal(rows);
    const peak = Math.max(...years.map((point) => point.total.totalTokens), 1);
    const currentYear = today.slice(0, 4);
    return (
      <>
        <div className="uc-years-total">
          <span className="uc-years-caption">
            {messages.allTime}
            {firstDay ? <span className="uc-secondary"> · {messages.since(dayText(firstDay, language))}</span> : null}
          </span>
          <span className="uc-years-figure uc-num">{messages.tokens(tokenCount(total.totalTokens, language))}</span>
          {total.costUSD !== undefined ? <span className="uc-years-cost uc-num">{formatMoney(total.costUSD, language, rate)}</span> : null}
          <SourceSplit totals={totalsBySource(rows)} scale={1} />
        </div>
        <ul className="uc-years-list">
          {years.map((point) => (
            <li key={point.key} className="uc-years-row">
              <div className="uc-years-row-head">
                <span className={point.key === currentYear ? "uc-years-label is-current" : "uc-years-label"}>{messages.year(point.key, point.key === currentYear)}</span>
                <span className="uc-years-value uc-num">
                  {tokenCount(point.total.totalTokens, language)}
                  {point.total.costUSD !== undefined ? <span className="uc-secondary"> · {formatMoney(point.total.costUSD, language, rate)}</span> : null}
                </span>
              </div>
              <SourceSplit totals={point.bySource} scale={point.total.totalTokens / peak} />
            </li>
          ))}
        </ul>
      </>
    );
  };

  return (
    <section className="uc-section">
      <ChipHeader icon={<CalendarIcon size={11} />} color={VIEW_COLORS.years} title={messages.yearsTitle} />
      <div className="uc-card uc-years-card">{content()}</div>
    </section>
  );
}

function SourceSections() {
  const catalog = useApp((state) => state.catalog);
  const layout = useApp((state) => state.layout);
  const isEnabled = useIsEnabled();
  const groups = useMemo(() => tokenGroups(layout, catalog, isEnabled), [layout, catalog, isEnabled]);
  return <ProviderSections groups={groups} />;
}

export function OverviewView() {
  return (
    <>
      <ContextWindowsCard />
      <UsageRingCard />
      <YearsCard />
      <SourceSections />
    </>
  );
}
