/**
 * Building blocks the Token views and the price list share: a colorful capsule picker, a card header
 * with a tinted icon chip, stacked columns, ranked bars, a source legend and the loading / empty line.
 */
import type { CSSProperties, ReactNode } from "react";
import { messagesFor, type Language } from "@/i18n";
import { compact, localeOf } from "@/i18n/numbers";
import { dayDate } from "@/lib/days";
import type { ExchangeRate, UsageSource, UsageTotals } from "@/lib/types";
import { formatMoney } from "@/model/currency";
import { SOURCE_COLORS } from "@/model/palette";
import { USAGE_SOURCES } from "@/model/usage";
import { tooltipProps } from "../ui/tooltip";

export interface CapsuleOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
  /** The segment's accent while selected. */
  color?: string;
}

export function Capsules<T extends string>({
  label,
  options,
  value,
  onChange,
  small,
  stacked,
}: {
  label: string;
  options: ReadonlyArray<CapsuleOption<T>>;
  value: T;
  onChange: (value: T) => void;
  /** A compact picker that sits beside a title. */
  small?: boolean;
  /** Icons above their labels, so four views fit the popup's width. */
  stacked?: boolean;
}) {
  return (
    <div className={`uc-capsule-picker is-fit${small ? " is-small" : ""}${stacked ? " is-stacked" : ""}`} role="radiogroup" aria-label={label}>
      {options.map((option) => {
        const selected = option.value === value;
        const style = option.color ? ({ "--uc-segment-accent": option.color } as CSSProperties) : undefined;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={selected}
            className={`uc-capsule-segment${selected ? " is-selected" : ""}${option.color ? " is-tinted" : ""}`}
            style={style}
            onClick={() => onChange(option.value)}
          >
            {option.icon ? <span className="uc-segment-icon">{option.icon}</span> : null}
            <span className="uc-truncate">{option.label}</span>
          </button>
        );
      })}
    </div>
  );
}

/** A section header with a colored icon chip, like the grouped lists of the system settings. */
export function ChipHeader({ icon, color, title, trailing }: { icon: ReactNode; color: string; title: string; trailing?: ReactNode }) {
  return (
    <div className="uc-section-header is-chip">
      <span className="uc-chip-icon" style={{ background: color }} aria-hidden="true">
        {icon}
      </span>
      <h2 className="uc-chip-title uc-truncate">{title}</h2>
      {trailing}
    </div>
  );
}

/** Loading, failure and empty states share one quiet line. */
export function UsageNote({ text }: { text: string }) {
  return <p className="uc-usage-note">{text}</p>;
}

/** A day as the UI writes dates: `03/07/2026` / `Jul 3, 2026`. */
export function dayText(day: string, language: Language): string {
  const options: Intl.DateTimeFormatOptions = language === "vi" ? { day: "2-digit", month: "2-digit", year: "numeric" } : { month: "short", day: "numeric", year: "numeric" };
  return dayDate(day).toLocaleDateString(localeOf(language), options);
}

export function tokenCount(value: number, language: Language): string {
  return Math.abs(value) >= 1000 ? compact(language, value) : String(Math.round(value));
}

/** `12,3 Tr token · 1,2 Tr ₫`, or tokens alone when the totals carry no price. */
export function totalsText(totals: UsageTotals, language: Language, rate: ExchangeRate | null): string {
  const messages = messagesFor(language).usage;
  const tokens = messages.tokens(tokenCount(totals.totalTokens, language));
  return totals.costUSD === undefined ? tokens : `${tokens} · ${formatMoney(totals.costUSD, language, rate)}`;
}

export interface StackSegment {
  color: string;
  value: number;
}

export interface StackColumn {
  key: string;
  segments: StackSegment[];
  tooltip: string;
  /** Drawn a little stronger, e.g. today. */
  current?: boolean;
}

/** Vertical stacked columns scaled to the tallest one, with a peak label and a three-point axis. */
export function StackedColumns({ columns, peak, axis }: { columns: StackColumn[]; peak: string; axis: string[] }) {
  const totals = columns.map((column) => column.segments.reduce((sum, segment) => sum + Math.max(segment.value, 0), 0));
  const max = Math.max(...totals, 0);
  return (
    <div className="uc-columns">
      <span className="uc-columns-peak">{peak}</span>
      <div className="uc-columns-plot">
        <span className="uc-columns-grid is-top" />
        <span className="uc-columns-grid is-middle" />
        {columns.map((column, index) => {
          const total = totals[index]!;
          const height = max > 0 ? (total / max) * 100 : 0;
          return (
            <div key={column.key} className={`uc-column${column.current ? " is-current" : ""}`} {...tooltipProps(column.tooltip)}>
              <div className="uc-column-stack" style={{ height: `${height}%` }}>
                {column.segments.map((segment, part) =>
                  segment.value > 0 ? <span key={part} className="uc-column-part" style={{ background: segment.color, flexGrow: segment.value }} /> : null,
                )}
              </div>
            </div>
          );
        })}
      </div>
      <div className="uc-columns-axis">
        {axis.map((label, index) => (
          <span key={index}>{label}</span>
        ))}
      </div>
    </div>
  );
}

export interface RankedBar {
  key: string;
  label: string;
  color: string;
  value: number;
  valueText: string;
  detail?: string;
  tooltip?: string;
}

/** Ranked rows with a colored bar under each, scaled to the largest. */
export function RankedBars({ bars }: { bars: RankedBar[] }) {
  const max = Math.max(...bars.map((bar) => bar.value), 0);
  return (
    <ul className="uc-ranked">
      {bars.map((bar) => (
        <li key={bar.key} className="uc-ranked-row" {...tooltipProps(bar.tooltip)}>
          <div className="uc-ranked-head">
            <span className="uc-legend-dot" style={{ background: bar.color }} />
            <span className="uc-ranked-label uc-truncate">{bar.label}</span>
            <span className="uc-ranked-value uc-num">{bar.valueText}</span>
          </div>
          <div className="uc-ranked-track">
            <span className="uc-ranked-fill" style={{ width: `${max > 0 ? Math.max((bar.value / max) * 100, 1.5) : 0}%`, background: bar.color }} />
          </div>
          {bar.detail ? <span className="uc-ranked-detail">{bar.detail}</span> : null}
        </li>
      ))}
    </ul>
  );
}

/** Claude and Codex with their brand dots and, optionally, a figure each. */
export function SourceLegend({ language, values }: { language: Language; values?: Partial<Record<UsageSource, string>> }) {
  const messages = messagesFor(language).usage;
  return (
    <div className="uc-source-legend">
      {USAGE_SOURCES.map((source) => (
        <span key={source} className="uc-source-legend-item">
          <span className="uc-legend-dot" style={{ background: SOURCE_COLORS[source] }} />
          <span>{messages.source(source)}</span>
          {values?.[source] ? <span className="uc-secondary uc-num">{values[source]}</span> : null}
        </span>
      ))}
    </div>
  );
}

/** A thin bar split between the sources, e.g. under a day or a year. */
export function SourceSplit({ totals, scale }: { totals: Record<UsageSource, UsageTotals>; scale: number }) {
  const sum = USAGE_SOURCES.reduce((acc, source) => acc + totals[source].totalTokens, 0);
  return (
    <div className="uc-split-track">
      <div className="uc-split-fill" style={{ width: `${Math.min(Math.max(scale, 0), 1) * 100}%` }}>
        {USAGE_SOURCES.map((source) =>
          totals[source].totalTokens > 0 && sum > 0 ? (
            <span key={source} style={{ background: SOURCE_COLORS[source], flexGrow: totals[source].totalTokens }} />
          ) : null,
        )}
      </div>
    </div>
  );
}
