import { fixtureCatalog } from "@/lib/fixtures";
import type { EngineState, ProviderSnapshot } from "@/lib/types";
import { reconcileLayout } from "@/model/layout";
import { DEFAULT_SETTINGS } from "@/model/settings";
import { DEFAULT_DISPLAY } from "@/model/widgetData";
import { PaceNotifier } from "./paceNotifications";

const HOUR = 3_600_000;
const catalog = fixtureCatalog();
const placed = new Set(reconcileLayout(null, catalog).placed);
const now = new Date(Date.UTC(2026, 8, 26, 12));
const resetsAt = new Date(now.getTime() + 2 * HOUR).toISOString();
const ALL_ON = { almostOut: true, cuttingItClose: true, willRunOut: true };

function runtimes(sessionUsed: number): EngineState["providers"] {
  const snapshot: ProviderSnapshot = {
    providerID: "codex@52d0",
    displayName: "Codex · codex",
    refreshedAt: now.toISOString(),
    lines: [{ type: "progress", label: "Session", used: sessionUsed, limit: 100, format: { kind: "percent" }, resetsAt, periodDurationMs: 5 * HOUR }],
  };
  return { "codex@52d0": { snapshot, refreshing: false } };
}

function evaluate(notifier: PaceNotifier, used: number, toggles = ALL_ON) {
  return notifier.evaluate(catalog, runtimes(used), placed, () => true, toggles, DEFAULT_DISPLAY, (entry, metric) => `${entry.provider.displayName} · ${metric}`, now);
}

describe("PaceNotifier", () => {
  it("records a baseline on the first reading instead of replaying alerts", () => {
    expect(evaluate(new PaceNotifier(), 95)).toEqual([]);
  });

  it("fires once when a limit drops under 10% left, and once per reset window", () => {
    const notifier = new PaceNotifier();
    evaluate(notifier, 20);
    const alerts = evaluate(notifier, 93);
    expect(alerts.map((alert) => alert.milestone)).toContain("almostOut");
    expect(alerts.find((alert) => alert.milestone === "almostOut")!.body).toMatch(/^Chỉ còn 7% hạn mức\./);
    expect(evaluate(notifier, 94)).toEqual([]);
  });

  it("stays quiet for milestones the user did not turn on", () => {
    const notifier = new PaceNotifier();
    evaluate(notifier, 20, DEFAULT_SETTINGS.notifications);
    expect(evaluate(notifier, 93, DEFAULT_SETTINGS.notifications)).toEqual([]);
  });
});
