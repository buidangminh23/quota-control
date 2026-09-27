/**
 * The single place a number or a deadline becomes display text. Port of upstream
 * `Support/MetricFormatter.swift` and `Support/Formatters.swift`, localized: English matches upstream
 * en_US output, Vietnamese follows vi-VN.
 */
import { messagesFor, translate, type Language } from "@/i18n";
import type { DeadlineVerb, RestoreDay, When } from "@/i18n/messages";
import { compact, compactDollars, compactDong, decimal, dollars, dong, localeOf } from "@/i18n/numbers";
import type { MetricKind, MetricValue } from "@/lib/types";
import { roundHalfAwayFromZero } from "./decimal";
import { calendarDaysBetween, deviceTimeZone } from "./timeZone";

/** `tray` (taskbar strip): shortest. `row` (popup row): abbreviated, money keeps cents. `full`: every digit. */
export type FormatStyle = "tray" | "row" | "full";

export type ResetDisplayMode = "relative" | "absolute";

export type TimeFormat = "auto" | "12h" | "24h";

export type TotalSpendMetric = "cost" | "costPerMtok" | "tokens";

export function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(Math.max(value, 0), 100);
}

let dongRate: number | null = null;

/**
 * The USD→VND rate money shows at on the Vietnamese UI (the core's Vietcombank selling rate), or `null`
 * to keep dollars. The store sets it whenever the rate changes.
 */
export function setDongRate(usdToVnd: number | null): void {
  dongRate = usdToVnd !== null && Number.isFinite(usdToVnd) && usdToVnd > 0 ? usdToVnd : null;
}

/** A dollar amount in đồng on the Vietnamese UI when a rate is known, else `null`. */
function inDong(usd: number, style: FormatStyle, language: Language): string | null {
  if (language !== "vi" || dongRate === null) return null;
  const amount = usd * dongRate;
  if (style === "full") return dong(language, amount, 0);
  return style === "tray" || Math.abs(amount) >= 1000 ? compactDong(language, amount) : dong(language, amount, 0);
}

/** USD with a fixed number of fractional digits, e.g. `$2,059.07` / `2.059,07 $` (đồng on the Vietnamese UI). */
export function currency(amount: number, fractionDigits: number, language: Language): string {
  return inDong(amount, "row", language) ?? dollars(language, amount, fractionDigits);
}

/** A bare number in the given kind and style (no unit label). */
export function formatNumber(value: number, kind: MetricKind, style: FormatStyle, language: Language): string {
  switch (kind) {
    case "percent":
      return `${roundHalfAwayFromZero(clampPercent(value))}%`;
    case "dollars": {
      const local = inDong(value, style, language);
      if (local !== null) return local;
      if (Math.abs(value) >= 1000 && style !== "full") return compactDollars(language, value);
      return dollars(language, value, style === "tray" ? 0 : 2);
    }
    case "count":
      if (style !== "full" && Math.abs(value) >= 1000) return compact(language, value);
      return decimal(language, value, 0, 1);
  }
}

/** A value with its (translated) unit label appended, e.g. `772 credits` / `772 tín dụng`. */
export function formatValue(value: MetricValue, style: FormatStyle, language: Language): string {
  const text = formatNumber(value.number, value.kind, style, language);
  return value.label ? `${text} ${translate(value.label, language)}` : text;
}

/** Dollars per million tokens, e.g. `$1.37/MTok` / `1,37 $/triệu token`. */
export function formatCostPerMtok(value: number, style: FormatStyle, language: Language): string {
  return messagesFor(language).totalSpend.costPerMtok(formatNumber(value, "dollars", style, language));
}

export interface RingCenter {
  primary: string;
  unit: string;
}

/** The Total Spend ring's two-line center: a short figure over a quiet unit. */
export function totalSpendRingCenter(value: number, metric: TotalSpendMetric, language: Language): RingCenter {
  const unit = messagesFor(language).totalSpend.ringUnit;
  switch (metric) {
    case "cost":
      return { primary: formatNumber(value, "dollars", "tray", language), unit: unit("dollars") };
    case "costPerMtok":
      return {
        primary: Math.abs(value) >= 1000 ? compactDollars(language, value) : dollars(language, value, 2),
        unit: unit("perMtok"),
      };
    case "tokens": {
      const magnitude = Math.abs(value);
      const scaled = (divisor: number, key: "billion" | "million" | "thousand" | "tokens"): RingCenter => ({
        primary: decimal(language, value / divisor, 0, 1),
        unit: unit(key),
      });
      if (magnitude >= 1e9) return scaled(1e9, "billion");
      if (magnitude >= 1e6) return scaled(1e6, "million");
      if (magnitude >= 1e3) return scaled(1e3, "thousand");
      return scaled(1, "tokens");
    }
  }
}

const HOUR_MS = 3_600_000;

/** A limit window of whole hours under a day as `5h`, the same in every language; `null` otherwise. */
export function hourWindowLabel(periodMs: number | undefined): string | null {
  if (periodMs === undefined || !(periodMs > 0) || periodMs >= 24 * HOUR_MS || periodMs % HOUR_MS !== 0) return null;
  return `${periodMs / HOUR_MS}h`;
}

/** Compact duration (`1d 6h` / `1 ngày 6 giờ`); `null` for non-finite or non-positive spans. */
export function compactDuration(seconds: number, language: Language): string | null {
  if (!Number.isFinite(seconds) || seconds <= 0) return null;
  const totalMinutes = Math.max(1, Math.ceil(seconds / 60));
  const days = Math.floor(totalMinutes / (24 * 60));
  const hours = Math.floor((totalMinutes % (24 * 60)) / 60);
  const minutes = totalMinutes % 60;
  return messagesFor(language).format.duration(days, hours, minutes);
}

let systemUses24Hour: boolean | null = null;

/** The operating system's clock preference, reported by the core; `null` uses the language's convention. */
export function setSystemClockPreference(uses24Hour: boolean | null): void {
  systemUses24Hour = uses24Hour;
}

/** Short wall-clock time in the device's zone, honoring the Time Format setting (`5:30 PM` / `17:30`). */
export function shortTime(date: Date, format: TimeFormat, language: Language, timeZone: string = deviceTimeZone()): string {
  const uses24Hour = format === "12h" ? false : format === "24h" ? true : systemUses24Hour;
  const options: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit", timeZone };
  if (uses24Hour !== null) options.hourCycle = uses24Hour ? "h23" : "h12";
  return date.toLocaleTimeString(localeOf(language), options);
}

function daysBetween(now: Date, date: Date): number {
  return calendarDaysBetween(now, date);
}

/** The day of `date` next to a clock time: today, tomorrow or its weekday and date in the device's zone. */
export function restoreDayOf(date: Date, now: Date, relative = true): RestoreDay {
  if (!relative) return { kind: "on", date };
  const dayDiff = daysBetween(now, date);
  return dayDiff <= 0 ? { kind: "today" } : dayDiff === 1 ? { kind: "tomorrow" } : { kind: "on", date };
}

/** The structured "when" of a deadline, or `null` when the duration is not finite. */
export function whenOf(date: Date, mode: ResetDisplayMode, now: Date, timeFormat: TimeFormat, language: Language): When | null {
  const seconds = (date.getTime() - now.getTime()) / 1000;
  if (mode === "relative") {
    if (seconds <= 5 * 60) return { kind: "soon" };
    const duration = compactDuration(seconds, language);
    return duration === null ? null : { kind: "in", duration };
  }
  if (seconds <= 0) return { kind: "soon" };
  const dayDiff = daysBetween(now, date);
  const time = shortTime(date, timeFormat, language);
  if (dayDiff <= 0) return { kind: "today", time };
  if (dayDiff === 1) return { kind: "tomorrow", time };
  return { kind: "on", date: messagesFor(language).format.monthDay(date), time };
}

/** The verb-less phrase: `2d 6h`, `today at 5:30 PM`, `soon` (and their Vietnamese forms). */
export function whenLabel(date: Date, mode: ResetDisplayMode, now: Date, timeFormat: TimeFormat, language: Language): string | null {
  const when = whenOf(date, mode, now, timeFormat, language);
  return when === null ? null : messagesFor(language).format.when(when);
}

/** `Resets in 2d 6h`, `Limit today at 5:30 PM`, `Resets soon` (and their Vietnamese forms). */
export function deadlineLabel(
  verb: DeadlineVerb,
  date: Date,
  mode: ResetDisplayMode,
  now: Date,
  timeFormat: TimeFormat,
  language: Language,
): string | null {
  const when = whenOf(date, mode, now, timeFormat, language);
  return when === null ? null : messagesFor(language).format.deadline(verb, when);
}

export function resetRelativeLabel(resetsAt: Date, now: Date, timeFormat: TimeFormat, language: Language): string | null {
  return deadlineLabel("resets", resetsAt, "relative", now, timeFormat, language);
}

export function resetAbsoluteLabel(resetsAt: Date, now: Date, timeFormat: TimeFormat, language: Language): string | null {
  return deadlineLabel("resets", resetsAt, "absolute", now, timeFormat, language);
}

/** The exact wall-clock moment a limit comes back, e.g. `Hồi lại lúc 13:05 · T6 02/10`; `null` once it has passed. */
export function restoreLabel(resetsAt: Date, now: Date, timeFormat: TimeFormat, language: Language): string | null {
  if (!(resetsAt.getTime() > now.getTime())) return null;
  return messagesFor(language).format.restoresAt(shortTime(resetsAt, timeFormat, language), restoreDayOf(resetsAt, now));
}

/**
 * A clock time with its day in the device's zone, e.g. `13:05 · ngày mai` / `1:05 PM · Fri, Oct 2`;
 * `relative: false` always names the weekday and date.
 */
export function timeOnDayLabel(date: Date, now: Date, timeFormat: TimeFormat, language: Language, relative = true): string {
  return messagesFor(language).format.timeOnDay(shortTime(date, timeFormat, language), restoreDayOf(date, now, relative));
}
