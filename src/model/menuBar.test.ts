import { fixtureCatalog, fixtureSnapshots } from "@/lib/fixtures";
import type { ProviderSnapshot } from "@/lib/types";
import { stripText } from "@/strip/render";
import { glanceGroups, pinnedGroups, reconcileLayout } from "./layout";
import { barFill, buildStripContent, isStripEmpty, MAX_BARS, periodLabel, stripSummary, visualFraction } from "./menuBar";
import { providerTitle } from "./providerText";
import { DEFAULT_DISPLAY, widgetDataFor } from "./widgetData";

const catalog = fixtureCatalog();
const snapshots = fixtureSnapshots(Date.UTC(2026, 8, 26, 3));
const layout = reconcileLayout(null, catalog);

function content(display = DEFAULT_DISPLAY, data: Readonly<Record<string, ProviderSnapshot | undefined>> = snapshots) {
  return buildStripContent(
    pinnedGroups(layout, catalog, () => true),
    (descriptor) => widgetDataFor(descriptor, data[descriptor.providerId], display),
    (provider) => providerTitle(provider, display.language),
  );
}

describe("taskbar strip content", () => {
  it("shows every starred metric with data, remaining by default", () => {
    const strip = content();
    expect(strip.groups.map((group) => group.displayName)).toEqual(["Claude · Công ty", "Claude · Cá nhân", "Codex"]);
    expect(strip.groups[0]!.metrics.map((metric) => metric.value)).toEqual(["88%", "42%"]);
    expect(stripSummary(strip)).toContain("Claude · Công ty: Phiên 88%, Tuần 42%");
    expect(stripText(strip)).toContain("Codex 18% 36%");
  });

  it("follows the Used/Left mode and caps the Bars glyph at four meters", () => {
    const strip = content({ ...DEFAULT_DISPLAY, displayMode: "used" });
    expect(strip.groups[0]!.metrics[0]!.value).toBe("12%");
    expect(strip.bars.length).toBe(MAX_BARS);
  });

  it("lists every Hạn mức card with one or two readings each when asked", () => {
    const dataFor = (descriptor: Parameters<typeof widgetDataFor>[0]) => widgetDataFor(descriptor, snapshots[descriptor.providerId], DEFAULT_DISPLAY);
    const name = (provider: Parameters<typeof providerTitle>[0]) => providerTitle(provider, "vi");
    const dashboard = glanceGroups("dashboard", [], layout, catalog, () => true);
    const two = buildStripContent(dashboard, dataFor, name, 2);
    expect(two.groups.map((group) => group.providerId)).toEqual(["claude@7c1e", "claude@a93f", "codex@52d0"]);
    expect(two.groups.every((group) => group.metrics.length === 2)).toBe(true);
    const one = buildStripContent(dashboard, dataFor, name, 1);
    expect(one.groups.map((group) => group.metrics.map((metric) => metric.value))).toEqual([["88%"], ["100%"], ["18%"]]);
    const picked = buildStripContent(glanceGroups("custom", ["codex@52d0.weekly"], layout, catalog, () => true), dataFor, name);
    expect(picked.groups.map((group) => group.metrics.map((metric) => metric.id))).toEqual([["codex@52d0.weekly"]]);
  });

  it("names each reading's limit window, so the five-hour and weekly limits stay apart", () => {
    const strip = content();
    expect(strip.groups[0]!.metrics.map((metric) => [metric.period, metric.value])).toEqual([
      ["5h", "88%"],
      ["week", "42%"],
    ]);
  });

  it("drops a provider whose stars have no data", () => {
    const strip = content(DEFAULT_DISPLAY, { ...snapshots, "codex@52d0": undefined });
    expect(strip.groups.map((group) => group.providerId)).not.toContain("codex@52d0");
    expect(isStripEmpty(content(DEFAULT_DISPLAY, {}))).toBe(true);
  });
});

describe("limit window names", () => {
  const HOUR = 3_600_000;
  const DAY = 24 * HOUR;

  it("names the five-hour and weekly windows the way the strip shows them", () => {
    expect(periodLabel(5 * HOUR)).toBe("5h");
    expect(periodLabel(7 * DAY)).toBe("week");
  });

  it("names other whole windows and leaves the rest unnamed", () => {
    expect(periodLabel(HOUR)).toBe("1h");
    expect(periodLabel(DAY)).toBe("day");
    expect(periodLabel(30 * DAY)).toBe("month");
    expect(periodLabel(90 * 60_000)).toBeNull();
    expect(periodLabel(3 * DAY)).toBeNull();
    expect(periodLabel(undefined)).toBeNull();
    expect(periodLabel(0)).toBeNull();
  });
});

describe("bars geometry", () => {
  it("quantizes near-full bars so a sliver of the remainder stays visible", () => {
    expect(visualFraction(0.97)).toBeCloseTo(0.85, 10);
    expect(visualFraction(0.5)).toBe(0.5);
    expect(visualFraction(1)).toBe(1);
    expect(visualFraction(Number.NaN)).toBe(0);
  });

  it("keeps a minimum remainder and a divider unless the bar is full", () => {
    expect(barFill(28, 0)).toEqual({ fillW: 0, remainderW: 0, dividerX: null });
    expect(barFill(28, 1)).toEqual({ fillW: 28, remainderW: 0, dividerX: null });
    const fill = barFill(28, 0.97);
    expect(fill.remainderW).toBeGreaterThanOrEqual(6);
    expect(fill.dividerX).toBe(28 - fill.remainderW);
  });
});
