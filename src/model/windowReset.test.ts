import { describe, expect, it } from "vitest";
import type { EngineState, MetricLine, ProgressLine } from "@/lib/types";
import { makeWidget } from "./testHelpers";
import { nextUsageDeadline, nextWindowReset, rolledOverReading, rollOverPassedWindows } from "./windowReset";

const now = new Date("2026-09-27T06:10:01Z");

function meter(label: string, used: number, resetsAt?: string): ProgressLine {
  return { type: "progress", label, used, limit: 100, format: { kind: "percent" }, periodDurationMs: 5 * 3_600_000, resetsAt };
}

function engineWith(lines: Record<string, MetricLine[]>): EngineState {
  return {
    providers: Object.fromEntries(Object.entries(lines).map(([id, providerLines]) => [
      id,
      { snapshot: { providerID: id, displayName: id, plan: "Plus", planCheckedAt: now.toISOString(), lines: providerLines, refreshedAt: "2026-09-27T06:09:08.016Z" }, refreshing: false },
    ])),
    lastRefreshAt: "2026-09-27T06:09:09.139Z",
    refreshIntervalMs: 300_000,
  };
}

describe("quota validity", () => {
  it.each(["claude@work", "codex@personal", "cursor", "glm@account"])("withholds %s readings after reset without fabricating restored allowance", (id) => {
    const state = engineWith({ [id]: [meter("Session", 100, "2026-09-27T06:10:00Z"), meter("Weekly", 68, "2026-09-29T03:00:00Z")] });
    const snapshot = state.providers[id]!.snapshot!;
    const unavailable = rollOverPassedWindows(state, now).providers[id]!.snapshot!;
    expect(unavailable).toEqual({ ...snapshot, usageUnavailable: true });
    expect(unavailable.lines).toBe(snapshot.lines);
    expect(snapshot.usageUnavailable).toBeUndefined();
    expect(unavailable.plan).toBe("Plus");
    expect(unavailable.planCheckedAt).toBe(now.toISOString());
  });

  it("withholds a reset at the exact deadline", () => {
    const state = engineWith({ codex: [meter("Weekly", 90, now.toISOString())] });
    expect(rollOverPassedWindows(state, now).providers.codex!.snapshot).toMatchObject({ usageUnavailable: true, lines: [{ used: 90 }] });
  });

  it.each(["network", "rate_limited", "auth_expired"] as const)("withholds cached readings on a %s failure", (errorCategory) => {
    const state = engineWith({ codex: [meter("Session", 23)] });
    state.providers.codex!.error = errorCategory;
    expect(rollOverPassedWindows(state, now).providers.codex!.snapshot!.usageUnavailable).toBe(true);
    delete state.providers.codex!.error;
    state.providers.codex!.snapshot!.errorCategory = errorCategory;
    expect(rollOverPassedWindows(state, now).providers.codex!.snapshot!.usageUnavailable).toBe(true);
  });

  it.each(["invalid", "2026-09-27T06:10:02Z", "2026-09-27T06:00:01Z"])("withholds invalid, future or obsolete timestamps: %s", (refreshedAt) => {
    const state = engineWith({ codex: [meter("Session", 23)] });
    state.providers.codex!.snapshot!.refreshedAt = refreshedAt;
    expect(rollOverPassedWindows(state, now).providers.codex!.snapshot!.usageUnavailable).toBe(true);
  });

  it("preserves a valid reading while a refresh is in progress", () => {
    const state = engineWith({ codex: [meter("Session", 23)] });
    state.providers.codex!.refreshing = true;
    expect(rollOverPassedWindows(state, now)).toBe(state);
  });

  it("restores readings only from a valid successful provider snapshot", () => {
    const stale = engineWith({ codex: [meter("Session", 100, "2026-09-27T06:10:00Z")] });
    expect(rollOverPassedWindows(stale, now).providers.codex!.snapshot!.usageUnavailable).toBe(true);
    const confirmed = structuredClone(stale);
    confirmed.providers.codex!.snapshot!.refreshedAt = now.toISOString();
    confirmed.providers.codex!.snapshot!.lines = [meter("Session", 9, "2026-09-27T11:10:00Z")];
    expect(rollOverPassedWindows(confirmed, now)).toBe(confirmed);
    expect(confirmed.providers.codex!.snapshot!.lines[0]).toMatchObject({ used: 9 });
  });

  it("preserves local token history and unrelated provider objects", () => {
    const state = engineWith({ "claude@work": [meter("Session", 100, "2026-09-27T06:10:00Z")], "claude-local": [meter("Today", 75, "2026-09-27T06:10:00Z")], codex: [meter("Weekly", 36, "2026-10-04T02:42:09Z")] });
    state.providers["claude-local"]!.error = "network";
    state.providers["claude-local"]!.snapshot!.refreshedAt = "2025-01-01T00:00:00Z";
    const validated = rollOverPassedWindows(state, now);
    expect(validated.providers["claude-local"]).toBe(state.providers["claude-local"]);
    expect(validated.providers.codex).toBe(state.providers.codex);
    expect(validated.lastRefreshAt).toBe(state.lastRefreshAt);
  });

  it("keeps repeated validation stable", () => {
    const state = engineWith({ codex: [meter("Weekly", 36, now.toISOString())] });
    const unavailable = rollOverPassedWindows(state, now);
    expect(rollOverPassedWindows(unavailable, now)).toBe(unavailable);
  });
});

describe("rolledOverReading", () => {
  it("marks the window unavailable and preserves the actual provider reading", () => {
    const row = makeWidget("Session", "percent", 82, 100, { resetsAt: new Date("2026-09-27T06:10:00Z"), periodDurationMs: 5 * 3_600_000 });
    expect(rolledOverReading(row)).toEqual({ ...row, hasData: false });
    expect(row).toMatchObject({ hasData: true, used: 82, resetsAt: new Date("2026-09-27T06:10:00Z") });
  });
});

describe("validity deadlines", () => {
  it("finds the earliest future reset", () => {
    const state = engineWith({ claude: [meter("Session", 100, "2026-09-27T06:10:00Z"), meter("Weekly", 68, "2026-09-29T03:00:00Z")], codex: [meter("Session", 36, "2026-09-27T10:20:00.167Z")] });
    expect(nextWindowReset(state, now)?.toISOString()).toBe("2026-09-27T10:20:00.167Z");
  });

  it("wakes when a successful reading expires even without any reset date", () => {
    const state = engineWith({ codex: [meter("Session", 23)] });
    expect(nextUsageDeadline(state, now)?.toISOString()).toBe("2026-09-27T06:19:08.016Z");
  });

  it("wakes at reset before freshness expiry", () => {
    const state = engineWith({ codex: [meter("Session", 23, "2026-09-27T06:11:00Z")] });
    expect(nextUsageDeadline(state, now)?.toISOString()).toBe("2026-09-27T06:11:00.000Z");
  });

  it("does not schedule a local-history expiry or a deadline already passed", () => {
    const state = engineWith({ "claude-local": [meter("Today", 2, "2026-09-28T00:00:00Z")] });
    expect(nextUsageDeadline(state, now)).toBeNull();
    const old = engineWith({ codex: [meter("Session", 23, "2026-09-27T06:10:00Z")] });
    old.providers.codex!.snapshot!.refreshedAt = "2025-01-01T00:00:00Z";
    expect(nextUsageDeadline(old, now)).toBeNull();
  });
});
