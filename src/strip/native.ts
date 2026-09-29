/**
 * The menu bar strip as a description instead of a picture, for macOS to draw itself
 * (`macos/Host/MenuBarStrip.swift`): the same groups, window names and readings `renderTextStrip`
 * draws, with the menu bar style's own sizes, so the two are one design (Rules.md §0.58). The
 * system then draws the text at every display's own resolution and in the menu bar's look of the
 * moment. The picture still travels with it, for a core that cannot draw the description.
 */
import { PROVIDER_MARKS, type ProviderMark } from "@/assets/providerMarks";
import { stripSummary, type StripContent } from "@/model/menuBar";
import { knownBrandColor } from "@/model/totalSpend";
import { MARK_INSET, SINGLE_WEIGHT, STACKED_WEIGHT, STRIP_METRICS, type StripStyle } from "./render";

export interface NativeStripRow {
  /** The window's short name (`5h`, `week`), or `null` for a reading without one. */
  label: string | null;
  value: string;
}

export interface NativeStripGroup {
  brand: string;
  /** The mark's color on a light and on a dark bar; `null` takes the text color. */
  light: string | null;
  dark: string | null;
  /** The mark's paths, with the brand's color logo as a base64 PNG (`art`) when it has one. */
  mark?: ProviderMark & { art?: string };
  rows: NativeStripRow[];
}

export interface NativeStripMetrics {
  singleSize: number;
  singleWeight: number;
  stackedSize: number;
  stackedWeight: number;
  markSide: number;
  markGap: number;
  markInset: number;
  groupGap: number;
  sidePadding: number;
  labelSize: number | null;
  labelGap: number;
  labelWeight: number;
  labelAlpha: number;
  rowGap: number;
  edge: number;
  minValue: string;
}

export interface NativeStrip {
  version: 1;
  /** Device pixels per point the picture is laid out at, and the description with it. */
  scale: number;
  /** The band's height in those pixels. */
  height: number;
  metrics: NativeStripMetrics;
  groups: NativeStripGroup[];
  /** The readings in words, for a screen reader. */
  text: string;
}

/**
 * The description of `content` for a band `height` device pixels tall, or `null` when the style
 * places its rows another way than by baselines (only the menu bar does) or nothing shows.
 * `art` holds the color logos already drawn, by brand.
 */
export function nativeStrip(
  content: StripContent,
  height: number,
  scale: number,
  art: Readonly<Record<string, string>> = {},
  style: StripStyle = "menuBar",
): NativeStrip | null {
  const metrics = STRIP_METRICS[style];
  if (!metrics.baselines || content.groups.length === 0) return null;
  const named = metrics.labelSize !== null;
  return {
    version: 1,
    scale,
    height,
    metrics: {
      singleSize: metrics.singleSize,
      singleWeight: SINGLE_WEIGHT,
      stackedSize: metrics.stackedSize,
      stackedWeight: STACKED_WEIGHT,
      markSide: metrics.markSide,
      markGap: metrics.markGap,
      markInset: MARK_INSET,
      groupGap: metrics.groupGap,
      sidePadding: metrics.sidePadding,
      labelSize: metrics.labelSize,
      labelGap: metrics.labelGap,
      labelWeight: metrics.labelWeight ?? 500,
      labelAlpha: metrics.labelAlpha ?? 0.8,
      rowGap: metrics.baselines.rowGap,
      edge: metrics.baselines.edge,
      minValue: metrics.baselines.minValue,
    },
    groups: content.groups.map((group) => {
      const mark = PROVIDER_MARKS[group.brand];
      const picture = art[group.brand];
      const entry: NativeStripGroup = {
        brand: group.brand,
        light: knownBrandColor(group.brand, false),
        dark: knownBrandColor(group.brand, true),
        rows: group.metrics.slice(0, 2).map((metric) => ({ label: named ? metric.period : null, value: metric.value })),
      };
      if (mark) entry.mark = picture ? { ...mark, art: picture } : mark;
      return entry;
    }),
    text: stripSummary(content).replaceAll("\n", "; "),
  };
}
