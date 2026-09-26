/**
 * What the taskbar strip shows: the chosen metrics with real data (the Hạn mức cards, the starred
 * metrics or a hand-picked set), per provider, plus the first four bounded ones for the Bars glyph.
 * Port of upstream `Models/MenuBarContent.swift` and the pure fill geometry of
 * `Support/MenuBarStripRenderer.swift` (`MenuBarBarGeometry`).
 *
 * The strip is dynamic: a metric without data is dropped, and a provider whose metrics all lack data
 * contributes nothing (no orphan mark). Nothing left means the app icon.
 */
import type { Provider, WidgetDescriptor } from "@/lib/types";
import type { ProviderMetrics } from "./layout";
import { brandOf } from "./layout";
import { fraction, isBounded, menuBarValue, type WidgetData } from "./widgetData";

export const MAX_BARS = 4;

export interface StripMetric {
  id: string;
  /** Full metric title in the display language (tooltips and accessibility). */
  label: string;
  /** The strip reading: `42%` for bounded metrics, the compact value otherwise. */
  value: string;
  /** Meter fill 0...1 (bounded metrics only). */
  fraction: number;
  bounded: boolean;
}

export interface StripGroup {
  providerId: string;
  displayName: string;
  brand: string;
  metrics: StripMetric[];
}

export interface StripContent {
  groups: StripGroup[];
  bars: StripMetric[];
}

export function buildStripContent(
  groups: readonly ProviderMetrics[],
  dataFor: (descriptor: WidgetDescriptor) => WidgetData,
  displayName: (provider: Provider) => string,
  perGroup = 2,
): StripContent {
  const resolved = groups.flatMap((group) => {
    const metrics = [...group.always, ...group.onDemand].flatMap((descriptor): StripMetric[] => {
      const data = dataFor(descriptor);
      if (!data.hasData) return [];
      return [{ id: descriptor.id, label: data.title, value: menuBarValue(data), fraction: fraction(data), bounded: isBounded(data) }];
    }).slice(0, Math.max(1, perGroup));
    if (metrics.length === 0) return [];
    return [{ providerId: group.provider.id, displayName: displayName(group.provider), brand: brandOf(group.provider.icon || group.provider.id), metrics }];
  });
  const bars = resolved.flatMap((group) => group.metrics).filter((metric) => metric.bounded).slice(0, MAX_BARS);
  return { groups: resolved, bars };
}

export function isStripEmpty(content: StripContent): boolean {
  return content.groups.length === 0;
}

/** One line per provider, e.g. `Claude: Phiên 12%, Tuần 58%` (tooltip and accessibility text). */
export function stripSummary(content: StripContent): string {
  return content.groups
    .map((group) => `${group.displayName}: ${group.metrics.map((metric) => `${metric.label} ${metric.value}`).join(", ")}`)
    .join("\n");
}

/** Quantize near-full (0.7–1.0) bars by remainder in 15% steps, so 97% still shows a visible tail. */
export function visualFraction(value: number): number {
  if (!Number.isFinite(value)) return 0;
  const clamped = Math.min(1, Math.max(0, value));
  if (clamped > 0.7 && clamped < 1) {
    const quantized = Math.min(1, Math.ceil((1 - clamped) / 0.15) * 0.15);
    return Math.max(0, 1 - quantized);
  }
  return clamped;
}

export interface BarFill {
  fillW: number;
  remainderW: number;
  dividerX: number | null;
}

/** Fill geometry for one bar of the glyph (upstream `MenuBarBarGeometry.fill`). */
export function barFill(trackW: number, value: number): BarFill {
  if (!Number.isFinite(value) || !(value > 0)) return { fillW: 0, remainderW: 0, dividerX: null };
  const visual = visualFraction(value);
  if (visual >= 1) return { fillW: trackW, remainderW: 0, dividerX: null };
  const minVisible = Math.max(4, Math.round(trackW * 0.2));
  const maxFillW = Math.max(1, trackW - minVisible);
  const fillW = Math.max(1, Math.min(maxFillW, Math.round(trackW * visual)));
  const remainderW = Math.min(trackW - 1, Math.max(trackW - fillW, minVisible));
  return { fillW, remainderW, dividerX: trackW - remainderW };
}
