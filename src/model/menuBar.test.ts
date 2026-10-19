import { fixtureCatalog, fixtureEngineState, fixtureSnapshots } from "@/lib/fixtures";
import type { ProviderSnapshot } from "@/lib/types";
import { stripText } from "@/strip/render";
import { glanceGroups, pinnedGroups, reconcileLayout } from "./layout";
import { barFill, buildStripContent, isStripEmpty, MAX_BARS, periodLabel, stripSummary, visualFraction } from "./menuBar";
import { providerTitle } from "./providerText";
import { DEFAULT_DISPLAY, widgetDataFor } from "./widgetData";
import { rollOverPassedWindows } from "./windowReset";

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
    expect(stripSummary(strip)).toContain("Claude · Công ty: Phiên 5h 88%, Tuần 42%");
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

  it("keeps selected accounts visible before their first reading", () => {
    const strip = content(DEFAULT_DISPLAY, { ...snapshots, "codex@52d0": undefined });
    expect(strip.groups.map((group) => group.providerId)).toEqual(content().groups.map((group) => group.providerId));
    expect(strip.groups[2]!.metrics.map((metric) => metric.value)).toEqual(["—", "—"]);
    expect(isStripEmpty(content(DEFAULT_DISPLAY, {}))).toBe(false);
    expect(isStripEmpty(buildStripContent([], () => { throw new Error("No selected metrics"); }, () => ""))).toBe(true);
  });

  it.each(["rate_limited", "network", "auth_expired"])("keeps an account in place through a %s error and recovery", (error) => {
    const now = new Date("2026-10-19T05:30:00Z");
    const state = fixtureEngineState(now.getTime());
    const readings = () => Object.fromEntries(Object.entries(rollOverPassedWindows(state, now).providers).map(([id, runtime]) => [id, runtime.snapshot]));
    const before = content(DEFAULT_DISPLAY, readings());
    state.providers["claude@a93f"]!.error = error;
    const failed = content(DEFAULT_DISPLAY, readings());
    expect(failed.groups.map((group) => group.providerId)).toEqual(before.groups.map((group) => group.providerId));
    expect(failed.groups[0]).toEqual(before.groups[0]);
    expect(failed.groups[2]).toEqual(before.groups[2]);
    expect(failed.groups[1]!.metrics.map(({ id, period }) => ({ id, period }))).toEqual(before.groups[1]!.metrics.map(({ id, period }) => ({ id, period })));
    expect(failed.groups[1]!.metrics).toEqual([
      expect.objectContaining({ value: "—", fraction: 0, bounded: false }),
      expect.objectContaining({ value: "—", fraction: 0, bounded: false }),
    ]);
    expect(failed.bars.every((metric) => !metric.id.startsWith("claude@a93f."))).toBe(true);
    delete state.providers["claude@a93f"]!.error;
    expect(content(DEFAULT_DISPLAY, readings())).toEqual(before);
  });

  it.each(["expired", "stale"])("keeps an account visible while its reading is %s", (reason) => {
    const now = new Date("2026-10-19T05:30:00Z");
    const state = fixtureEngineState(now.getTime());
    const snapshot = state.providers["codex@52d0"]!.snapshot!;
    if (reason === "stale") snapshot.refreshedAt = new Date(now.getTime() - 600_000).toISOString();
    else snapshot.lines = snapshot.lines.map((line) => line.type === "progress" && line.label === "Session" ? { ...line, resetsAt: now.toISOString() } : line);
    const validated = rollOverPassedWindows(state, now);
    const strip = content(DEFAULT_DISPLAY, Object.fromEntries(Object.entries(validated.providers).map(([id, runtime]) => [id, runtime.snapshot])));
    expect(strip.groups.map((group) => group.providerId)).toEqual(content().groups.map((group) => group.providerId));
    expect(strip.groups[2]!.metrics.map((metric) => metric.value)).toEqual(["—", "—"]);
  });

  it("keeps only the plan's reported windows during an unavailable reading", () => {
    const codex = snapshots["codex@52d0"]!;
    const weeklyOnly = { ...codex, lines: codex.lines.filter((line) => line.label !== "Session") };
    const before = content(DEFAULT_DISPLAY, { ...snapshots, "codex@52d0": weeklyOnly });
    const unavailable = content(DEFAULT_DISPLAY, { ...snapshots, "codex@52d0": { ...weeklyOnly, usageUnavailable: true } });
    expect(before.groups[2]!.metrics.map((metric) => metric.period)).toEqual(["week"]);
    expect(unavailable.groups[2]!.metrics).toEqual([expect.objectContaining({ period: "week", value: "—", bounded: false })]);
  });

  it("still removes accounts excluded by the user's selection", () => {
    const groups = glanceGroups("dashboard", [], layout, catalog, (id) => id !== "claude@a93f");
    const strip = buildStripContent(groups, (descriptor) => widgetDataFor(descriptor, undefined, DEFAULT_DISPLAY), (provider) => providerTitle(provider, "vi"));
    expect(strip.groups.map((group) => group.providerId)).toEqual(["claude@7c1e", "codex@52d0"]);
  });

  it("omits an unsupported window even when it is the only selected metric", () => {
    const codex = snapshots["codex@52d0"]!;
    const weeklyOnly = { ...codex, lines: codex.lines.filter((line) => line.label !== "Session"), usageUnavailable: true };
    const groups = glanceGroups("custom", ["codex@52d0.session"], layout, catalog, () => true);
    expect(groups).toHaveLength(1);
    const strip = buildStripContent(groups, (descriptor) => widgetDataFor(descriptor, weeklyOnly, DEFAULT_DISPLAY), (provider) => providerTitle(provider, "vi"));
    expect(strip.groups).toEqual([]);
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
