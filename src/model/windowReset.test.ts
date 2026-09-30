import { describe, expect, it } from "vitest";
import type { EngineState, MetricLine, ProgressLine } from "@/lib/types";
import { makeWidget } from "./testHelpers";
import { nextWindowReset, rolledOverReading, rollOverPassedWindows } from "./windowReset";

const now = new Date("2026-09-27T06:10:01Z");

function meter(label: string, used: number, resetsAt?: string): ProgressLine {
  return { type: "progress", label, used, limit: 100, format: { kind: "percent" }, periodDurationMs: 5 * 3_600_000, resetsAt };
}

function engineWith(lines: Record<string, MetricLine[]>): EngineState {
  return {
    providers: Object.fromEntries(
      Object.entries(lines).map(([id, providerLines]) => [
        id,
        { snapshot: { providerID: id, displayName: id, lines: providerLines, refreshedAt: "2026-09-27T06:09:08.016Z" }, refreshing: false },
      ]),
    ),
    lastRefreshAt: "2026-09-27T06:09:09.139Z",
    refreshIntervalMs: 300_000,
  };
}

describe("rollOverPassedWindows", () => {
  it("shows a window whose reset has passed as a fresh one: nothing used, no countdown", () => {
    const state = engineWith({
      "claude@cbdd": [meter("Session", 100, "2026-09-27T06:10:00.119Z"), meter("Weekly", 68, "2026-09-29T03:00:00.119Z")],
    });
    const rolled = rollOverPassedWindows(state, now);
    const [session, weekly] = rolled.providers["claude@cbdd"]!.snapshot!.lines;
    expect(session).toEqual({ type: "progress", label: "Session", used: 0, limit: 100, format: { kind: "percent" }, periodDurationMs: 5 * 3_600_000 });
    expect(session).not.toHaveProperty("resetsAt");
    expect(weekly).toBe(state.providers["claude@cbdd"]!.snapshot!.lines[1]);
    expect(rolled.lastRefreshAt).toBe(state.lastRefreshAt);
    expect(state.providers["claude@cbdd"]!.snapshot!.lines[0]).toMatchObject({ used: 100, resetsAt: "2026-09-27T06:10:00.119Z" });
  });

  it("counts a reset at this very moment as passed", () => {
    const state = engineWith({ codex: [meter("Weekly", 90, now.toISOString())] });
    expect(rollOverPassedWindows(state, now).providers.codex!.snapshot!.lines[0]).toMatchObject({ used: 0 });
  });

  it("returns the same state when no window has reset, so nothing redraws", () => {
    const state = engineWith({
      "claude@a458": [meter("Session", 36, "2026-09-27T10:20:00.167Z"), meter("Session start", 0), { type: "badge", label: "Extra usage spent", text: "Disabled" }],
      codex: [meter("Weekly", 36, "not a date"), { type: "values", label: "Credits", values: [{ number: 0, kind: "dollars", estimated: false }] }],
    });
    state.providers.pending = { refreshing: true };
    expect(rollOverPassedWindows(state, now)).toBe(state);
  });

  it("keeps the providers and snapshots it does not touch", () => {
    const state = engineWith({ "claude@cbdd": [meter("Session", 100, "2026-09-27T06:10:00Z")], codex: [meter("Weekly", 36, "2026-10-04T02:42:09Z")] });
    const rolled = rollOverPassedWindows(state, now);
    expect(rolled).not.toBe(state);
    expect(rolled.providers.codex).toBe(state.providers.codex);
  });
});

describe("rolledOverReading", () => {
  it("reads a row as its window rolled over, nothing used and no countdown, and leaves the row itself alone", () => {
    const row = makeWidget("Session", "percent", 82, 100, { resetsAt: new Date("2026-09-27T06:10:00Z"), periodDurationMs: 5 * 3_600_000 });
    expect(rolledOverReading(row)).toEqual({ ...row, used: 0, resetsAt: null });
    expect(row).toMatchObject({ used: 82, resetsAt: new Date("2026-09-27T06:10:00Z") });
  });
});

describe("nextWindowReset", () => {
  it("finds the earliest reset still ahead across every provider", () => {
    const state = engineWith({
      "claude@cbdd": [meter("Session", 100, "2026-09-27T06:10:00Z"), meter("Weekly", 68, "2026-09-29T03:00:00Z")],
      "claude@a458": [meter("Session", 36, "2026-09-27T10:20:00.167Z")],
      codex: [meter("Weekly", 36, "junk")],
    });
    expect(nextWindowReset(state, now)?.toISOString()).toBe("2026-09-27T10:20:00.167Z");
  });

  it("is null when every reset has passed or none is known", () => {
    expect(nextWindowReset(engineWith({ "claude@cbdd": [meter("Session", 100, "2026-09-27T06:10:00Z"), meter("Weekly", 5)] }), now)).toBeNull();
    expect(nextWindowReset(engineWith({}), now)).toBeNull();
  });
});
