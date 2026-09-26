/**
 * Lịch sử: any past month's days, newest first, each with its tokens, cost and a Claude / Codex bar;
 * a day opens its split by source, model and project.
 */
import { useMemo, useState } from "react";
import { messagesFor, type Language } from "@/i18n";
import { addMonths, dayDate, monthOf } from "@/lib/days";
import type { ExchangeRate, UsageGroupRow } from "@/lib/types";
import { formatMoney } from "@/model/currency";
import { colorOf, SOURCE_COLORS, VIEW_COLORS } from "@/model/palette";
import { grandTotal, modelLabel, monthQuery, series, totalsByKey, totalsBySource, USAGE_SOURCES } from "@/model/usage";
import { useSettings } from "@/state/hooks";
import { useApp } from "@/state/store";
import { useToday, useUsageColors, useUsageRows, type UsageColors } from "@/state/usage";
import { ChevronLeft, ChevronRight, ClockIcon } from "../ui/icons";
import { tooltipProps } from "../ui/tooltip";
import { ChipHeader, RankedBars, SourceSplit, tokenCount, totalsText, UsageNote, type RankedBar } from "./parts";

let rememberedMonth: string | null = null;

function monthLabel(month: string, language: Language): string {
  const [year, index] = month.split("-").map(Number);
  return messagesFor(language).usage.monthTitle(year!, index!);
}

export function HistoryView() {
  const language = useSettings().language;
  const today = useToday();
  const firstDay = useApp((state) => state.ledgerInfo?.firstDay ?? null);
  const [month, setMonth] = useState(() => rememberedMonth ?? monthOf(today));
  const [day, setDay] = useState<string | null>(null);
  const current = monthOf(today);
  const first = firstDay ? monthOf(firstDay) : current;
  const shown = month > current ? current : month;
  const move = (step: number) => {
    const next = addMonths(shown, step);
    rememberedMonth = next;
    setMonth(next);
  };
  if (day) return <DayDetail day={day} language={language} onBack={() => setDay(null)} />;
  return <MonthDays month={shown} today={today} language={language} canBack={shown > first} canForward={shown < current} onMove={move} onDay={setDay} />;
}

function MonthDays({
  month,
  today,
  language,
  canBack,
  canForward,
  onMove,
  onDay,
}: {
  month: string;
  today: string;
  language: Language;
  canBack: boolean;
  canForward: boolean;
  onMove: (step: number) => void;
  onDay: (day: string) => void;
}) {
  const messages = messagesFor(language).usage;
  const rate = useApp((state) => state.exchangeRate);
  const { rows, failed } = useUsageRows(monthQuery(month, today, "day"));
  const days = useMemo(() => {
    if (!rows) return [];
    const keys = [...new Set(rows.map((row) => row.key))].sort().reverse();
    return series(rows, keys);
  }, [rows]);
  const peak = Math.max(...days.map((point) => point.total.totalTokens), 1);

  return (
    <section className="uc-section">
      <ChipHeader
        icon={<ClockIcon size={11} />}
        color={VIEW_COLORS.history}
        title={monthLabel(month, language)}
        trailing={
          <span className="uc-month-nav">
            <button type="button" className="uc-icon-button" aria-label={messages.previousMonth} disabled={!canBack} onClick={() => onMove(-1)} {...tooltipProps(messages.previousMonth)}>
              <ChevronLeft size={10} />
            </button>
            <button type="button" className="uc-icon-button" aria-label={messages.nextMonth} disabled={!canForward} onClick={() => onMove(1)} {...tooltipProps(messages.nextMonth)}>
              <ChevronRight size={10} />
            </button>
          </span>
        }
      />
      <div className="uc-card uc-history-card">
        {!rows ? (
          <UsageNote text={failed ? messages.failed : messages.loading} />
        ) : days.length === 0 ? (
          <UsageNote text={messages.noData} />
        ) : (
          <>
            <div className="uc-history-total">
              <span>{messages.monthTotal}</span>
              <span className="uc-num">{totalsText(grandTotal(rows), language, rate)}</span>
            </div>
            <ul className="uc-history-days">
              {days.map((point) => (
                <li key={point.key}>
                  <button type="button" className="uc-history-day" onClick={() => onDay(point.key)}>
                    <span className="uc-history-day-head">
                      <span className="uc-history-day-label">{messages.dayShort(dayDate(point.key))}</span>
                      <span className="uc-history-day-value uc-num">
                        {tokenCount(point.total.totalTokens, language)}
                        {point.total.costUSD !== undefined ? <span className="uc-secondary"> · {formatMoney(point.total.costUSD, language, rate)}</span> : null}
                      </span>
                      <ChevronRight size={8} />
                    </span>
                    <SourceSplit totals={point.bySource} scale={point.total.totalTokens / peak} />
                  </button>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>
    </section>
  );
}

function bars(rows: readonly UsageGroupRow[], label: (key: string) => string, color: (key: string) => string, language: Language, rate: ExchangeRate | null): RankedBar[] {
  return totalsByKey(rows)
    .sort((a, b) => b.total.totalTokens - a.total.totalTokens)
    .map((item) => ({
      key: item.key,
      label: label(item.key),
      color: color(item.key),
      value: item.total.totalTokens,
      valueText: tokenCount(item.total.totalTokens, language),
      detail: item.total.costUSD !== undefined ? formatMoney(item.total.costUSD, language, rate) : undefined,
    }));
}

function DayDetail({ day, language, onBack }: { day: string; language: Language; onBack: () => void }) {
  const messages = messagesFor(language).usage;
  const rate = useApp((state) => state.exchangeRate);
  const colors: UsageColors = useUsageColors();
  const models = useUsageRows({ from: day, to: day, groupBy: "model" });
  const projects = useUsageRows({ from: day, to: day, groupBy: "project" });
  const date = dayDate(day);
  const month = monthLabel(monthOf(day), language);

  const content = () => {
    if (!models.rows || !projects.rows) return <UsageNote text={models.failed || projects.failed ? messages.failed : messages.loading} />;
    if (models.rows.length === 0) return <UsageNote text={messages.noUsage} />;
    const bySource = totalsBySource(models.rows);
    const sourceBars: RankedBar[] = USAGE_SOURCES.filter((source) => bySource[source].totalTokens > 0).map((source) => ({
      key: source,
      label: messages.source(source),
      color: SOURCE_COLORS[source],
      value: bySource[source].totalTokens,
      valueText: tokenCount(bySource[source].totalTokens, language),
      detail: bySource[source].costUSD !== undefined ? formatMoney(bySource[source].costUSD!, language, rate) : undefined,
    }));
    return (
      <>
        <div className="uc-history-total">
          <span>{messages.dayTitle(date)}</span>
          <span className="uc-num">{totalsText(grandTotal(models.rows), language, rate)}</span>
        </div>
        <h3 className="uc-subheading">{messages.bySource}</h3>
        <RankedBars bars={sourceBars} />
        <h3 className="uc-subheading">{messages.byModel}</h3>
        <RankedBars bars={bars(models.rows, modelLabel, (key) => colorOf(colors.models, key), language, rate)} />
        <h3 className="uc-subheading">{messages.byProject}</h3>
        <RankedBars bars={bars(projects.rows, (key) => key || messages.unknownProject, (key) => colorOf(colors.projects, key), language, rate)} />
      </>
    );
  };

  return (
    <section className="uc-section">
      <ChipHeader
        icon={<ClockIcon size={11} />}
        color={VIEW_COLORS.history}
        title={messages.dayShort(date)}
        trailing={
          <button type="button" className="uc-link-button" onClick={onBack} aria-label={messages.back(month)}>
            <ChevronLeft size={9} />
            {month}
          </button>
        }
      />
      <div className="uc-card uc-history-card">{content()}</div>
    </section>
  );
}
