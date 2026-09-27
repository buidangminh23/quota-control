/**
 * Small pieces shared by the Benchmark and Reset tabs: the capsule switch, a rate bar with its
 * confidence band, the source line with its link, the fetch time with its refresh button, a
 * fold-out explanation and number helpers.
 */
import { useState, type ReactNode } from "react";
import type { Language } from "@/i18n";
import type { InsightsMessages } from "@/i18n/insights";
import { decimal } from "@/i18n/numbers";
import { backend } from "@/lib/backend";
import type { PublicFeedName, PublicFeedSnapshot } from "@/lib/insightsTypes";
import { compactDuration } from "@/model/format";
import { deviceTimeZone } from "@/model/timeZone";
import { useNow } from "@/state/hooks";
import { refreshFeeds, useInsights } from "@/state/insights";
import { Button } from "../ui/controls";
import { ChevronDown, ChevronUp, ExternalIcon } from "../ui/icons";
import { tooltipProps } from "../ui/tooltip";

const MINUTE_MS = 60_000;

export function Segmented<T extends string>({
  value,
  options,
  label,
  onChange,
  ariaLabel,
}: {
  value: T;
  options: readonly T[];
  label: (option: T) => string;
  onChange: (option: T) => void;
  ariaLabel: string;
}) {
  return (
    <div className="uc-capsule-picker" role="radiogroup" aria-label={ariaLabel}>
      {options.map((option) => (
        <button
          key={option}
          type="button"
          role="radio"
          aria-checked={option === value}
          className={`uc-capsule-segment${option === value ? " is-selected" : ""}`}
          onClick={() => onChange(option)}
        >
          {label(option)}
        </button>
      ))}
    </div>
  );
}

/**
 * A 0..1 rate. With an interval, the fill runs to its lower end, a lighter band covers the interval
 * and a tick marks the observed rate, so the bar never looks surer than the data.
 */
export function RateBar({ rate, low, high }: { rate: number; low: number | null; high: number | null }) {
  const percent = (value: number) => `${Math.min(100, Math.max(0, value * 100))}%`;
  const banded = low !== null && high !== null;
  return (
    <div className="uc-meter uc-rate-bar" aria-hidden="true">
      <span className="uc-meter-fill uc-rate-fill" style={{ width: percent(banded ? low : rate) }} />
      {banded ? <span className="uc-rate-band" style={{ left: percent(low), width: percent(Math.max(0, high - low)) }} /> : null}
      {banded ? <span className="uc-rate-tick" style={{ left: `calc(${percent(rate)} - 1px)` }} /> : null}
    </div>
  );
}

export function openExternal(url: string): void {
  void backend()
    .openUrl(url)
    .catch((error: unknown) => console.error("Opening the link failed", error));
}

export function LinkButton({ url, label, children }: { url: string; label: string; children?: ReactNode }) {
  return (
    <button type="button" className="uc-insight-link" aria-label={label} onClick={() => openExternal(url)} {...tooltipProps(children ? null : label)}>
      {children}
      <ExternalIcon size={10} />
    </button>
  );
}

export function SourceLine({ text, url, linkLabel }: { text: string; url?: string | null; linkLabel?: string }) {
  return (
    <p className="uc-insight-source">
      <span>{text}</span>
      {url ? <LinkButton url={url} label={linkLabel ?? text} /> : null}
    </p>
  );
}

/**
 * When the copy on screen was fetched, and a button that asks the sources for their latest
 * published data now. After a failed attempt the time is that of the last good download, because
 * that copy is what the view still shows.
 */
export function FeedStatus({
  names,
  shown,
  language,
  text,
  tooltip,
}: {
  names: readonly PublicFeedName[];
  shown: PublicFeedSnapshot | undefined;
  language: Language;
  text: InsightsMessages;
  tooltip?: string;
}) {
  const now = useNow();
  const refreshing = useInsights((state) => names.some((name) => state.refreshing[name] === true));
  const at = shown?.error ? shown.fetchedAt : (shown?.checkedAt ?? shown?.fetchedAt);
  const ago = agoText(at, now, language);
  const recent = at ? now.getTime() - new Date(at).getTime() < MINUTE_MS : false;
  return (
    <div className="uc-insight-status">
      <span className="uc-insight-status-text">{ago === null ? "" : recent ? text.justNow : text.fetchedAgo(ago)}</span>
      <Button onClick={() => void refreshFeeds(names)} className="is-small" disabled={refreshing} tooltip={tooltip}>
        {refreshing ? text.refreshing : text.refresh}
      </Button>
    </div>
  );
}

export function Disclosure({ title, children }: { title: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="uc-disclosure">
      <button type="button" className="uc-disclosure-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span>{title}</span>
        {open ? <ChevronUp size={10} /> : <ChevronDown size={10} />}
      </button>
      {open ? <div className="uc-disclosure-body">{children}</div> : null}
    </div>
  );
}

export function percentText(language: Language, rate: number, digits = 1): string {
  return `${decimal(language, rate * 100, digits, digits)}%`;
}

export function numberText(language: Language, value: number, digits = 0): string {
  return decimal(language, value, digits, digits);
}

/** How long ago `iso` was, as a bare duration (`5 phút`), or `null` for a missing or future time. */
export function agoText(iso: string | Date | null | undefined, now: Date, language: Language): string | null {
  if (!iso) return null;
  const time = iso instanceof Date ? iso.getTime() : new Date(iso).getTime();
  if (Number.isNaN(time)) return null;
  return compactDuration(Math.max(60, (now.getTime() - time) / 1000), language);
}

/** A calendar date in the device's zone (or `timeZone`): `26/09/2026` or `9/26/2026`. */
export function dateText(date: Date, language: Language, timeZone: string = deviceTimeZone()): string {
  return date.toLocaleDateString(language === "vi" ? "vi-VN" : "en-US", { day: "2-digit", month: "2-digit", year: "numeric", timeZone });
}

/** A `YYYY-MM-DD` day as written, whatever the zone. */
export function dayText(day: string, language: Language): string {
  const [year, month, date] = day.split("-").map(Number);
  if (!year || !month || !date) return day;
  return dateText(new Date(Date.UTC(year, month - 1, date, 12)), language, "UTC");
}
