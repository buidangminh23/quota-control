import type { ProviderSnapshot, WidgetDescriptor } from "@/lib/types";
import { makeWidget } from "./testHelpers";
import { descriptorTitle, metricTitle, offeredRows } from "./widgetData";

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
});
