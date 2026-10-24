import type { ProviderSnapshot, WidgetDescriptor } from "@/lib/types";
import { makeWidget } from "./testHelpers";
import { DEFAULT_DISPLAY, descriptorTitle, metricTitle, offeredRows, widgetDataFor } from "./widgetData";

const HOUR = 3_600_000;

function descriptor(title: string): WidgetDescriptor {
  return {
    id: `codex@52d0.${title.toLowerCase()}`,
    providerId: "codex@52d0",
    metricLabel: title,
    template: { title, kind: "percent", limit: 100 },
    pinnable: true,
    isSpendTile: false,
  };
}

function snapshot(lines: ProviderSnapshot["lines"]): ProviderSnapshot {
  return { providerID: "codex@52d0", displayName: "Codex · codex", lines, refreshedAt: "2026-09-27T10:00:00Z" };
}

function row(title: string, hasData: boolean) {
  return { id: title, data: makeWidget(title, "percent", 10, 100, { hasData }) };
}

describe("metric titles", () => {
  it("names a session's window so it reads apart from the weekly limit", () => {
    expect(metricTitle("Session", 5 * HOUR, "vi")).toBe("Phiên 5h");
    expect(metricTitle("Session", 5 * HOUR, "en")).toBe("5h Session");
    expect(metricTitle("Session", 3 * HOUR, "vi")).toBe("Phiên 3h");
  });

  it("keeps the plain title without a whole-hour window, and for every other metric", () => {
    expect(metricTitle("Session", undefined, "vi")).toBe("Phiên");
    expect(metricTitle("Session", 90 * 60_000, "vi")).toBe("Phiên");
    expect(metricTitle("Session", 7 * 24 * HOUR, "vi")).toBe("Phiên");
    expect(metricTitle("Weekly", 7 * 24 * HOUR, "vi")).toBe("Tuần");
    expect(metricTitle("Spark", 5 * HOUR, "vi")).toBe("Spark");
  });

  it("takes a descriptor's window from the provider's last reading", () => {
    const session = descriptor("Session");
    const reading = snapshot([{ type: "progress", label: "Session", used: 20, limit: 100, format: { kind: "percent" }, periodDurationMs: 5 * HOUR }]);
    expect(descriptorTitle(session, reading, "vi")).toBe("Phiên 5h");
    expect(descriptorTitle(session, snapshot([]), "vi")).toBe("Phiên");
    expect(descriptorTitle(session, undefined, "vi")).toBe("Phiên");
  });
});

describe("offered rows", () => {
  it("drops the rows a successful read does not carry, before and behind the caret", () => {
    const offered = offeredRows([row("Session", false), row("Weekly", true)], [row("Sonnet", false), row("Fable", true)], true);
    expect(offered.always.map((entry) => entry.id)).toEqual(["Weekly"]);
    expect(offered.onDemand.map((entry) => entry.id)).toEqual(["Fable"]);
  });

  it("keeps every row until a read succeeds", () => {
    const always = [row("Session", false), row("Weekly", false)];
    const onDemand = [row("Sonnet", false)];
    expect(offeredRows(always, onDemand, false)).toEqual({ always, onDemand });
  });

  it("keeps every row when the read has data for none, rather than leave the card empty", () => {
    const always = [row("Session", false)];
    const onDemand = [row("Sonnet", false)];
    expect(offeredRows(always, onDemand, true)).toEqual({ always, onDemand });
  });

  it("keeps pending reported windows beside valid readings and omits unsupported windows", () => {
    const pending = { ...row("Session", false), data: makeWidget("Session", "percent", 100, 100, { hasData: false, hasReading: true }) };
    const unsupported = { ...row("Sonnet", false), data: makeWidget("Sonnet", "percent", 0, 100, { hasData: false, hasReading: false }) };
    const weekly = row("Weekly", true);
    expect(offeredRows([pending, weekly], [unsupported], true)).toEqual({ always: [pending, weekly], onDemand: [] });
    expect(offeredRows([weekly], [pending, unsupported], true)).toEqual({ always: [weekly], onDemand: [pending] });
    expect(offeredRows([pending], [unsupported], true)).toEqual({ always: [pending], onDemand: [] });
  });
});

describe("quota availability", () => {
  it("withholds unavailable numeric readings while retaining meter geometry and labels", () => {
    const reading = snapshot([{ type: "progress", label: "Session", used: 82, limit: 100, format: { kind: "percent" }, periodDurationMs: 5 * HOUR, resetsAt: "2026-09-27T15:00:00Z" }]);
    const available = widgetDataFor(descriptor("Session"), reading, DEFAULT_DISPLAY);
    expect(available.hasData).toBe(true);
    expect(widgetDataFor(descriptor("Session"), { ...reading, usageUnavailable: true }, DEFAULT_DISPLAY)).toEqual({ ...available, hasData: false });
    expect(widgetDataFor(descriptor("Session"), { ...reading, errorCategory: "rate_limited" }, DEFAULT_DISPLAY).hasData).toBe(false);
    expect(reading.lines[0]).toMatchObject({ used: 82 });
  });

  it("withholds cached credit counts and disabled badges on a failed account reading", () => {
    for (const line of [
      { type: "values", label: "Session", values: [{ number: 8, kind: "count", estimated: false }] },
      { type: "badge", label: "Session", text: "Disabled" },
    ] as ProviderSnapshot["lines"]) {
      const reading = { ...snapshot([line]), usageUnavailable: true };
      expect(widgetDataFor(descriptor("Session"), reading, DEFAULT_DISPLAY).hasData).toBe(false);
    }
  });

  it("withholds only the expired window while keeping sibling windows and credits available", () => {
    const reading = { ...snapshot([
      { type: "progress", label: "Session", used: 82, limit: 100, format: { kind: "percent" }, periodDurationMs: 5 * HOUR, resetsAt: "2026-09-27T10:00:00Z" },
      { type: "progress", label: "Weekly", used: 64, limit: 100, format: { kind: "percent" }, periodDurationMs: 7 * 24 * HOUR, resetsAt: "2026-10-01T10:00:00Z" },
      { type: "values", label: "Rate Limit Resets", values: [{ number: 3, kind: "count", estimated: false }] },
    ]), unavailableMetrics: ["Session"] };
    const session = widgetDataFor(descriptor("Session"), reading, DEFAULT_DISPLAY);
    expect(session).toMatchObject({ hasData: false, hasReading: true, used: 82, periodDurationMs: 5 * HOUR });
    expect(widgetDataFor(descriptor("Weekly"), reading, DEFAULT_DISPLAY)).toMatchObject({ hasData: true, hasReading: true, used: 64 });
    expect(widgetDataFor(descriptor("Rate Limit Resets"), reading, DEFAULT_DISPLAY)).toMatchObject({ hasData: true, hasReading: true, values: [{ number: 3 }] });
    expect(widgetDataFor(descriptor("Sonnet"), reading, DEFAULT_DISPLAY)).toMatchObject({ hasData: false, hasReading: false });
    for (const label of ["Session", "Weekly", "Rate Limit Resets"]) {
      expect(widgetDataFor(descriptor(label), { ...reading, usageUnavailable: true }, DEFAULT_DISPLAY).hasData).toBe(false);
    }
  });

  it("preserves cached local token history", () => {
    const local = { ...descriptor("Session"), providerId: "codex-local" };
    const reading = { ...snapshot([{ type: "values", label: "Session", values: [{ number: 2048, kind: "count" as const, estimated: false }] }]), errorCategory: "network" as const };
    expect(widgetDataFor(local, reading, DEFAULT_DISPLAY).hasData).toBe(true);
  });
});
