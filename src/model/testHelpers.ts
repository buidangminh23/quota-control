/** Test-only builders mirroring upstream's `WidgetData(...)` initializer defaults (English, 12-hour). */
import type { MetricKind } from "@/lib/types";
import type { WidgetData } from "./widgetData";

export function makeWidget(title: string, kind: MetricKind, used: number, limit: number | null, extra: Partial<WidgetData> = {}): WidgetData {
  return {
    title,
    kind,
    used,
    limit,
    displayMode: "used",
    resetDisplayMode: "relative",
    alwaysShowPacing: false,
    timeFormat: "12h",
    language: "en",
    resetsAt: null,
    expiriesAt: [],
    showsResetExpiries: false,
    unknownModels: [],
    hasData: true,
    values: [],
    isUsagePeriod: false,
    isChart: false,
    chartPoints: [],
    ...extra,
  };
}

export const WEEK_SECONDS = 7 * 24 * 60 * 60;
export const NOW = new Date(1_700_000_000_000);

/** A reset date such that exactly `elapsed` of the window has gone by as of `now`. */
export function resetsAt(elapsed: number, periodSeconds: number, now: Date = NOW): Date {
  return new Date(now.getTime() + periodSeconds * (1 - elapsed) * 1000);
}

/** Replaces the narrow and regular no-break spaces ICU puts before AM/PM and currency symbols. */
export function plainSpaces(text: string | null | undefined): string | null {
  return text === null || text === undefined ? null : text.replace(/[  ]/g, " ");
}
