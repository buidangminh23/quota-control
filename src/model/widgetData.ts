/**
 * Everything a row needs to render one metric. Port of upstream `Models/WidgetData.swift` (the value
 * text half; pacing lives in `meterState.ts`) and the `resolve` half of `Stores/WidgetDataStore.swift`,
 * localized through `@/i18n`.
 *
 * A metric with a `limit` renders as a meter row; without one it is a single right-aligned text line.
 */
import { messagesFor, translate, type Language } from "@/i18n";
import type {
  MetricChartPoint,
  MetricKind,
  MetricLine,
  MetricValue,
  ModelUsageBreakdown,
  ProviderSnapshot,
  SessionStartSignal,
  WidgetDescriptor,
  WidgetTemplate,
} from "@/lib/types";
import { roundHalfAwayFromZero } from "./decimal";
import {
  clampPercent,
  compactDuration,
  currency,
  deadlineLabel,
  formatNumber,
  formatValue,
  type ResetDisplayMode,
  type TimeFormat,
  whenLabel,
} from "./format";

export type DisplayMode = "used" | "remaining";

/** Source-text sentinel for locally estimated spend; displayed through the catalog. */
export const LOCAL_ESTIMATE_NOTE = "Estimated locally, so it may be off";
/** Headline shown on a tile with no real backing metric (em dash). */
export const NO_DATA_HEADLINE = "—";

/** The global display choices every row is stamped with (upstream `WidgetDataStore.data(for:)`). */
export interface DisplayOptions {
  displayMode: DisplayMode;
  resetDisplayMode: ResetDisplayMode;
  alwaysShowPacing: boolean;
  timeFormat: TimeFormat;
  language: Language;
}

export interface WidgetData extends DisplayOptions {
  /** Display title, already in `language`. */
  title: string;
  kind: MetricKind;
  used: number;
  limit: number | null;
  countSuffix?: string;
  valuePrefix?: string;
  resetsAt: Date | null;
  expiriesAt: Date[];
  showsResetExpiries: boolean;
  unknownModels: string[];
  modelBreakdown?: ModelUsageBreakdown;
  periodDurationMs?: number;
  valueTextOverride?: string;
  subtitleOverride?: string;
  limitNoun?: string;
  unboundedValueWord?: string;
  infoNote?: string;
  valueTooltipNote?: string;
  hasData: boolean;
  values: MetricValue[];
  /** Which values the tile renders; absent = every value. */
  selectionKind?: MetricKind;
  isUsagePeriod: boolean;
  traySuffix?: string;
  sessionStartSignal?: SessionStartSignal;
  isChart: boolean;
  chartPoints: MetricChartPoint[];
  chartNote?: string;
}

export const DEFAULT_DISPLAY: DisplayOptions = {
  displayMode: "remaining",
  resetDisplayMode: "relative",
  alwaysShowPacing: false,
  timeFormat: "auto",
  language: "vi",
};

/** The descriptor's template as a tile (upstream `descriptor.sample`); its numbers are placeholders. */
export function sampleFromTemplate(template: WidgetTemplate, display: DisplayOptions): WidgetData {
  return {
    ...display,
    title: translate(template.title, display.language),
    kind: template.kind,
    used: 0,
    limit: template.limit ?? null,
    countSuffix: template.countSuffix,
    valuePrefix: template.valuePrefix,
    resetsAt: null,
    expiriesAt: [],
    showsResetExpiries: template.showsResetExpiries ?? false,
    unknownModels: [],
    periodDurationMs: template.periodDurationMs,
    limitNoun: template.limitNoun,
    unboundedValueWord: template.unboundedValueWord,
    infoNote: template.infoNote,
    valueTooltipNote: template.valueTooltipNote,
    hasData: true,
    values: [],
    selectionKind: template.selectionKind,
    isUsagePeriod: template.isUsagePeriod ?? false,
    traySuffix: template.traySuffix,
    sessionStartSignal: template.sessionStartSignal,
    isChart: template.isChart ?? false,
    chartPoints: [],
  };
}

function parseDate(value: string | undefined): Date | null {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

/** Resolve one snapshot line against a descriptor, or `null` when no dashboard tile consumes it. */
export function resolveLine(line: MetricLine, descriptor: WidgetDescriptor, display: DisplayOptions): WidgetData | null {
  const template = descriptor.template;
  const sample = sampleFromTemplate(template, display);
  switch (line.type) {
    case "progress": {
      const kind = line.format.kind;
      return {
        ...sampleFromTemplate({ title: template.title, kind }, display),
        used: kind === "percent" ? clampPercent(line.used) : line.used,
        limit: line.limit,
        countSuffix: line.format.kind === "count" ? line.format.suffix : undefined,
        valuePrefix: template.valuePrefix,
        resetsAt: parseDate(line.resetsAt),
        periodDurationMs: line.periodDurationMs,
        limitNoun: template.limitNoun,
        infoNote: template.infoNote,
        sessionStartSignal: template.sessionStartSignal,
      };
    }
    case "text":
      return null;
    case "values": {
      const data: WidgetData = {
        ...sample,
        values: line.values,
        limit: null,
        expiriesAt: (line.expiriesAt ?? []).map(parseDate).filter((date): date is Date => date !== null),
        unknownModels: line.unknownModels ?? [],
        modelBreakdown: line.modelBreakdown,
      };
      const selected = selectedValues(data);
      data.hasData = selected.length > 0;
      data.infoNote = selected.some((value) => value.estimated) ? LOCAL_ESTIMATE_NOTE : template.infoNote;
      return data;
    }
    case "badge":
      return { ...sample, limit: null, valueTextOverride: line.text, subtitleOverride: line.subtitle };
    case "chart":
      return { ...sample, isChart: true, chartPoints: line.points, chartNote: line.note, hasData: line.points.length > 0 };
  }
}

/** The tile for `descriptor` given the provider's snapshot; "No data" when nothing real backs it. */
export function widgetDataFor(descriptor: WidgetDescriptor, snapshot: ProviderSnapshot | undefined, display: DisplayOptions): WidgetData {
  const line = snapshot?.lines.find((candidate) => candidate.label === descriptor.metricLabel);
  const resolved = line ? resolveLine(line, descriptor, display) : null;
  if (resolved) return resolved;
  return { ...sampleFromTemplate(descriptor.template, display), hasData: false };
}

export function isBounded(data: WidgetData): boolean {
  return data.limit !== null;
}

/** `values` projected through the tile's selection. */
export function selectedValues(data: WidgetData): MetricValue[] {
  return data.selectionKind ? data.values.filter((value) => value.kind === data.selectionKind) : data.values;
}

export function hasModelBreakdown(data: WidgetData): boolean {
  return data.hasData && data.isUsagePeriod && (data.modelBreakdown?.models.length ?? 0) > 0;
}

export function displayedValue(data: WidgetData): number {
  if (data.displayMode !== "remaining" || data.limit === null) return data.used;
  return Math.max(0, data.limit - data.used);
}

/** Rounds to the precision the headline prints: whole percent, one-decimal count, or cents. */
export function roundedAtDisplayPrecision(data: WidgetData, value: number): number {
  switch (data.kind) {
    case "percent":
      return roundHalfAwayFromZero(value);
    case "count":
      return roundHalfAwayFromZero(value * 10) / 10;
    case "dollars":
      return roundHalfAwayFromZero(value * 100) / 100;
  }
}

/** Meter fill 0...1, from the same rounded value the headline shows. */
export function fraction(data: WidgetData): number {
  if (data.limit === null || !(data.limit > 0)) return 0;
  return Math.min(Math.max(roundedAtDisplayPrecision(data, displayedValue(data)) / data.limit, 0), 1);
}

/** Remaining share of the limit 0...1, independent of the Used/Left mode. */
export function remainingFraction(data: WidgetData): number {
  if (data.limit === null || !(data.limit > 0)) return 0;
  return Math.min(Math.max((data.limit - data.used) / data.limit, 0), 1);
}

export function formatWidgetValue(data: WidgetData, value: number): string {
  return formatNumber(value, data.kind, "full", data.language);
}

/** A backend note (source text) in the row's language. */
export function noteText(data: WidgetData, note: string): string {
  return note === LOCAL_ESTIMATE_NOTE ? messagesFor(data.language).meter.localEstimateNote : translate(note, data.language);
}

/** Primary value string for unbounded rows and the strip. */
export function valueText(data: WidgetData): string {
  if (!data.hasData) return NO_DATA_HEADLINE;
  if (data.valueTextOverride !== undefined) return translate(data.valueTextOverride, data.language);
  const first = selectedValues(data)[0];
  if (first) return (data.valuePrefix ?? "") + formatNumber(first.number, first.kind, "row", data.language);
  return (data.valuePrefix ?? "") + formatWidgetValue(data, displayedValue(data));
}

/** The taskbar-strip reading: percent meters as `42%`, other meters compact, unbounded rows compact. */
export function menuBarValue(data: WidgetData): string {
  if (!data.hasData) return valueText(data);
  const language = data.language;
  if (data.limit !== null && data.limit > 0) {
    if (data.kind === "percent") {
      const percent = Math.min(100, Math.max(0, roundHalfAwayFromZero((displayedValue(data) / data.limit) * 100)));
      return `${percent}%`;
    }
    return formatNumber(displayedValue(data), data.kind, "tray", language);
  }
  const first = selectedValues(data)[0];
  if (first) {
    if (data.traySuffix && first.kind === "count") {
      return `${formatNumber(first.number, "count", "tray", language)} ${translate(data.traySuffix, language)}`;
    }
    return formatValue(first, "tray", language);
  }
  if (data.valueTextOverride !== undefined) return translate(data.valueTextOverride, language);
  const number = formatNumber(displayedValue(data), data.kind, "tray", language);
  return data.kind === "count" && data.countSuffix ? `${number} ${translate(data.countSuffix, language)}` : number;
}

/** Headline on bounded rows, e.g. `95% left` / `Còn 95%`. */
export function boundedHeadline(data: WidgetData): string {
  if (data.valueTextOverride !== undefined) return translate(data.valueTextOverride, data.language);
  return messagesFor(data.language).meter.headline(valueText(data), data.displayMode);
}

/** Subtitle under a bounded headline: reset timing, cadence, or limit context. */
export function boundedSubtitle(data: WidgetData, now: Date): string | null {
  const language = data.language;
  if (data.subtitleOverride !== undefined) return translate(data.subtitleOverride, language);
  if (data.resetsAt) {
    const label = deadlineLabel("resets", data.resetsAt, "relative", now, data.timeFormat, language);
    if (label) return label;
  }
  if (data.periodDurationMs !== undefined) {
    const duration = compactDuration(data.periodDurationMs / 1000, language);
    if (duration) return messagesFor(language).format.deadline("resets", { kind: "in", duration });
  }
  switch (data.kind) {
    case "percent":
      return null;
    case "dollars": {
      if (data.limit === null) return null;
      const digits = Math.round(data.limit) === data.limit ? 0 : 2;
      return messagesFor(language).meter.dollarLimit(currency(data.limit, digits, language), data.limitNoun);
    }
    case "count":
      return data.countSuffix ? translate(data.countSuffix, language) : null;
  }
}

/** The single headline a row renders; an em dash without data. */
export function headline(data: WidgetData): string {
  if (!data.hasData) return NO_DATA_HEADLINE;
  return isBounded(data) ? boundedHeadline(data) : valueText(data);
}

/** Right-aligned line of an unbounded row: `$4.08 · 1.2M tokens`, `$1,503.00 left`, or `No data`. */
export function unboundedDetail(data: WidgetData): string {
  const language = data.language;
  const meter = messagesFor(language).meter;
  if (!data.hasData) return meter.noData;
  if (data.valueTextOverride !== undefined) return translate(data.valueTextOverride, language);
  const selected = selectedValues(data);
  if (selected.length === 1) {
    const value = selected[0]!;
    if (value.kind === "dollars" && data.unboundedValueWord) {
      return meter.valueWithWord(formatNumber(value.number, "dollars", "row", language), data.unboundedValueWord);
    }
    return formatValue(value, "row", language);
  }
  if (selected.length > 1) return selected.map((value) => formatValue(value, "row", language)).join(" · ");
  const word = data.unboundedValueWord ?? (data.displayMode === "used" ? "used" : "left");
  const text = data.kind === "count" && data.countSuffix ? `${valueText(data)} ${translate(data.countSuffix, language)}` : valueText(data);
  return meter.valueWithWord(text, word);
}

export function unboundedSubtitle(data: WidgetData): string | null {
  return data.hasData && data.subtitleOverride !== undefined ? translate(data.subtitleOverride, data.language) : null;
}

/** True for a period with data whose every shown value is zero (`$0.00 · 0 tokens`). */
export function isZeroUsage(data: WidgetData): boolean {
  if (!data.hasData) return false;
  const selected = selectedValues(data);
  return selected.length > 0 && selected.every((value) => value.number === 0);
}

function tooltipNote(data: WidgetData): string | undefined {
  const note = data.infoNote ?? data.valueTooltipNote;
  return note === undefined ? undefined : noteText(data, note);
}

/** The exact figures a compact unbounded value shortens, or `null` when nothing is abbreviated. */
export function unboundedTooltip(data: WidgetData): string | null {
  if (!data.hasData) return null;
  const selected = selectedValues(data);
  if (!selected.some((value) => Math.abs(value.number) >= 1000) && tooltipNote(data) === undefined) return null;
  return selected.map((value) => formatValue(value, "full", data.language)).join(" · ");
}

export const EXPIRY_WARNING_WINDOW_SECONDS = 7 * 24 * 3600;
export const EXPIRY_CRITICAL_WINDOW_SECONDS = 48 * 3600;

export type MeterSeverity = "normal" | "warning" | "critical";

export function expirySeverityFor(secondsRemaining: number): MeterSeverity {
  if (secondsRemaining <= EXPIRY_CRITICAL_WINDOW_SECONDS) return "critical";
  if (secondsRemaining <= EXPIRY_WARNING_WINDOW_SECONDS) return "warning";
  return "normal";
}

/** Status of the soonest reset-credit expiry, or `null` when the row carries none. */
export function expirySeverity(data: WidgetData, now: Date): MeterSeverity | null {
  if (!data.hasData || data.expiriesAt.length === 0) return null;
  const soonest = Math.min(...data.expiriesAt.map((date) => date.getTime()));
  return expirySeverityFor((soonest - now.getTime()) / 1000);
}

export function resetCreditCount(data: WidgetData): number {
  return Math.floor(selectedValues(data)[0]?.number ?? 0);
}

/** When each reset credit expires, following the Countdown/Exact Time mode. */
export function expiryTooltip(data: WidgetData, now: Date): string | null {
  if (!data.hasData || data.expiriesAt.length === 0) return null;
  const sorted = [...data.expiriesAt].sort((a, b) => a.getTime() - b.getTime());
  if (sorted.length === 1) return deadlineLabel("resetExpires", sorted[0]!, data.resetDisplayMode, now, data.timeFormat, data.language);
  const entries = sorted
    .map((date, index) => {
      const when = whenLabel(date, data.resetDisplayMode, now, data.timeFormat, data.language);
      return when === null ? null : `${index + 1}. ${when}`;
    })
    .filter((entry): entry is string => entry !== null);
  if (entries.length === 0) return null;
  return [messagesFor(data.language).format.expiryListHeader(data.resetDisplayMode), ...entries].join("\n");
}

export function hasUnknownModels(data: WidgetData): boolean {
  return data.hasData && data.unknownModels.length > 0;
}

export function unknownModelTooltip(data: WidgetData): string | null {
  if (!hasUnknownModels(data)) return null;
  const header = messagesFor(data.language).meter.unknownModels(data.unknownModels.length);
  return [header, ...data.unknownModels.map((model) => `- ${model}`)].join("\n");
}

/** Hover text for an unbounded row's value. */
export function unboundedValueTooltip(data: WidgetData, now: Date): string | null {
  const expiry = expiryTooltip(data, now);
  if (expiry) return expiry;
  const note = tooltipNote(data);
  if (isZeroUsage(data) && data.isUsagePeriod) {
    return [messagesFor(data.language).meter.noUsageInPeriod, ...(note ? [note] : [])].join("\n");
  }
  const figures = unboundedTooltip(data);
  if (figures !== null) return [figures, ...(note ? [note] : [])].join("\n");
  return null;
}

/** Row offsets of text-only rows directly under another text-only row (they pull together). */
export function condensedTextRowOffsets(rows: WidgetData[]): Set<number> {
  const offsets = new Set<number>();
  for (let index = 1; index < rows.length; index += 1) {
    if (!isBounded(rows[index - 1]!) && !isBounded(rows[index]!)) offsets.add(index);
  }
  return offsets;
}
