/**
 * The meter's visual state (color, even-pace tick, warning copy) and the bounded row's trailing text.
 * Port of the "Pace (meter state)" half of upstream `Models/WidgetData.swift`, localized through `@/i18n`.
 *
 * Precedence, highest first: no data → spent → live pace verdict → absolute level bands. Every band keys
 * off the share used, never the displayed fraction, so color and copy never flip with Used/Left.
 */
import { messagesFor, translate, type Language } from "@/i18n";
import { deadlineLabel, resetAbsoluteLabel, resetRelativeLabel } from "./format";
import { evaluatePace, minimumElapsed, secondsToRunOut } from "./pace";
import {
  boundedSubtitle,
  formatWidgetValue,
  isBounded,
  type MeterSeverity,
  roundedAtDisplayPrecision,
  type WidgetData,
} from "./widgetData";

export type MeterState =
  | { kind: "noData" }
  | { kind: "spent" }
  | { kind: "runningOut"; eta: string | null; projectedFraction: number }
  | { kind: "closeToLimit"; sparePercent: number; projectedFraction: number }
  | { kind: "healthy"; projectedFraction: number }
  | { kind: "level"; severity: MeterSeverity };

/** Share of usage below which a pace verdict is not trusted (a fresh window extrapolates wildly). */
const PACE_DISTRUST_SHARE = 0.05;

/** Bar fill severity, or `null` for `noData` (the track stays gray). */
export function meterSeverity(state: MeterState): MeterSeverity | null {
  switch (state.kind) {
    case "noData":
      return null;
    case "spent":
    case "runningOut":
      return "critical";
    case "closeToLimit":
      return "warning";
    case "healthy":
      return "normal";
    case "level":
      return state.severity;
  }
}

/** Hover projection shared by the bar, the spare note and the flame. */
export function meterTooltip(state: MeterState, language: Language): string | null {
  const meter = messagesFor(language).meter;
  switch (state.kind) {
    case "noData":
    case "level":
      return null;
    case "spent":
      return meter.limitReached;
    case "healthy":
      return meter.leftAtReset(Math.round((1 - state.projectedFraction) * 100));
    case "closeToLimit":
      return meter.usedAtReset(Math.round(state.projectedFraction * 100));
    case "runningOut":
      if (!(state.projectedFraction > 1)) return meter.fullAtReset;
      return meter.overLimitAtReset(Math.max(1, Math.round((state.projectedFraction - 1) * 100)));
  }
}

/** The warning note under a close-to-limit meter, e.g. `~8% spare` / `Dư ~8%`. */
export function spareText(state: MeterState, language: Language): string | null {
  return state.kind === "closeToLimit" ? messagesFor(language).meter.spare(state.sparePercent) : null;
}

interface PaceContext {
  limit: number;
  resetsAt: Date;
  periodSeconds: number;
}

function paceContext(data: WidgetData): PaceContext | null {
  if (!data.hasData || data.limit === null || !(data.limit > 0) || !data.resetsAt) return null;
  if (data.periodDurationMs === undefined || !(data.periodDurationMs > 0)) return null;
  return { limit: data.limit, resetsAt: data.resetsAt, periodSeconds: data.periodDurationMs / 1000 };
}

function absoluteLevelState(used: number, limit: number): MeterState {
  const percentUsed = Math.round(Math.min(Math.max(used / limit, 0), 1) * 100);
  if (percentUsed >= 90) return { kind: "level", severity: "critical" };
  if (percentUsed >= 80) return { kind: "level", severity: "warning" };
  return { kind: "level", severity: "normal" };
}

/** A session meter whose rolling window has not begun yet ("Not started"). */
export function isFreshSessionWindow(data: WidgetData, now: Date): boolean {
  if (!data.sessionStartSignal || !data.hasData || data.limit === null || data.used > 0) return false;
  if (data.sessionStartSignal === "zeroUsage") return data.resetsAt !== null && now.getTime() < data.resetsAt.getTime();
  return data.resetsAt === null;
}

function runningOutEta(data: WidgetData, context: PaceContext, now: Date): string | null {
  const seconds = secondsToRunOut(data.used, context.limit, context.resetsAt, context.periodSeconds, now);
  if (seconds === null) return null;
  const runsOutAt = new Date(now.getTime() + seconds * 1000);
  return deadlineLabel("limit", runsOutAt, data.resetDisplayMode, now, data.timeFormat, data.language);
}

/** The meter's full visual state for `now`. */
export function meterState(data: WidgetData, now: Date): MeterState {
  if (!data.hasData || data.limit === null || !(data.limit > 0)) {
    return data.hasData ? { kind: "level", severity: "normal" } : { kind: "noData" };
  }
  const limit = data.limit;
  if (roundedAtDisplayPrecision(data, limit - data.used) <= 0) return { kind: "spent" };
  if (isFreshSessionWindow(data, now)) return absoluteLevelState(data.used, limit);

  const context = paceContext(data);
  const result = context && evaluatePace(data.used, context.limit, context.resetsAt, context.periodSeconds, now);
  if (!context || !result) return absoluteLevelState(data.used, limit);

  const projected = result.projectedUsage / context.limit;
  if (result.status === "ahead") return { kind: "healthy", projectedFraction: projected };
  if (data.used / context.limit < PACE_DISTRUST_SHARE) return absoluteLevelState(data.used, limit);
  if (result.status === "behind") {
    return { kind: "runningOut", eta: runningOutEta(data, context, now), projectedFraction: projected };
  }
  const sparePercent = Math.round((1 - projected) * 100);
  if (sparePercent < 1) return { kind: "runningOut", eta: null, projectedFraction: projected };
  return { kind: "closeToLimit", sparePercent, projectedFraction: projected };
}

/** Even-pace tick position 0...1, or `null` when hidden. */
export function paceTick(data: WidgetData, state: MeterState, now: Date): number | null {
  switch (state.kind) {
    case "spent":
    case "noData":
    case "level":
      return null;
    case "healthy":
      if (!data.alwaysShowPacing) return null;
      break;
    case "closeToLimit":
    case "runningOut":
      break;
  }
  const context = paceContext(data);
  if (!context) return null;
  const windowStart = context.resetsAt.getTime() - context.periodSeconds * 1000;
  const elapsed = (now.getTime() - windowStart) / 1000;
  if (elapsed < minimumElapsed(context.periodSeconds) || now.getTime() >= context.resetsAt.getTime()) return null;
  const elapsedFraction = Math.min(Math.max(elapsed / context.periodSeconds, 0), 1);
  return data.displayMode === "remaining" ? 1 - elapsedFraction : elapsedFraction;
}

function resetLabel(data: WidgetData, resetsAt: Date, now: Date, absolute: boolean): string | null {
  return absolute
    ? resetAbsoluteLabel(resetsAt, now, data.timeFormat, data.language)
    : resetRelativeLabel(resetsAt, now, data.timeFormat, data.language);
}

/** Trailing text on the bounded row, honoring Countdown/Exact Time. */
export function boundedTrailingText(data: WidgetData, now: Date): string | null {
  const meter = messagesFor(data.language).meter;
  if (!data.hasData) return meter.noData;
  if (data.subtitleOverride !== undefined) return translate(data.subtitleOverride, data.language);
  if (isFreshSessionWindow(data, now)) return meter.notStarted;
  if (data.resetsAt) return resetLabel(data, data.resetsAt, now, data.resetDisplayMode === "absolute");
  return boundedSubtitle(data, now);
}

/** Whether the trailing text is a concrete reset countdown (a click flips Countdown/Exact Time). */
export function hasResetLabel(data: WidgetData, now: Date): boolean {
  return data.hasData && data.subtitleOverride === undefined && data.resetsAt !== null && !isFreshSessionWindow(data, now);
}

/** The opposite reset format from the one shown, or the "Not started" explanation. */
export function resetTooltip(data: WidgetData, now: Date): string | null {
  if (isFreshSessionWindow(data, now)) return messagesFor(data.language).meter.freshSessionTooltip;
  if (!hasResetLabel(data, now) || !data.resetsAt) return null;
  return resetLabel(data, data.resetsAt, now, data.resetDisplayMode !== "absolute");
}

/** Whether the headline is a flippable Used/Left reading. */
export function hasMeterStyleToggle(data: WidgetData): boolean {
  return data.hasData && isBounded(data) && data.valueTextOverride === undefined;
}

/** The opposite Used/Left reading, e.g. headline `95% left` → `5% used` (`Còn 95%` → `Đã dùng 5%`). */
export function meterStyleTooltip(data: WidgetData): string | null {
  if (!hasMeterStyleToggle(data) || data.limit === null) return null;
  const oppositeMode = data.displayMode === "remaining" ? "used" : "remaining";
  const opposite = data.displayMode === "remaining" ? data.used : Math.max(0, data.limit - data.used);
  const value = (data.valuePrefix ?? "") + formatWidgetValue(data, opposite);
  return messagesFor(data.language).meter.headline(value, oppositeMode);
}
