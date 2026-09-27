import { describe, expect, it } from "vitest";
import { parseResetStatus } from "./resets";
import { AWAITING_MS, timingFromText, timingMoment, upcomingReset, type ResetTiming } from "./upcomingReset";

/** Saturday 26/09/2026 14:41 in San Francisco, Sunday 04:41 in Vietnam. */
const POSTED = new Date("2026-09-26T21:41:35Z");

function iso(timing: ResetTiming): string | null {
  return timingMoment(timing)?.toISOString() ?? null;
}

function status(scheduled: Record<string, unknown> | null, watch: Record<string, unknown> | null = null): string {
  return JSON.stringify({
    data: { latest_reset: null, scheduled_reset: scheduled, active_watch: watch, stats: { total: 55, last_reset_at: null, days_since_last: null, avg_interval_days: 6.9 } },
    meta: { api_version: "v1", generated_at: "2026-09-27T01:19:37.070Z" },
  });
}

function scheduled(text: string, scheduledFor: string | null = null, kind = "regular") {
  return {
    id: "2103963215885701493",
    status: "scheduled",
    reset_type: kind,
    announced_at: POSTED.toISOString(),
    scheduled_for: scheduledFor,
    text,
    source: { type: "x_post", author: "thsottiaux", url: "https://x.com/thsottiaux/status/2103963215885701493" },
  };
}

const WATCH = {
  level: "strong",
  reset_chance_percent: 65,
  forecast_window: "next 24 hours",
  observed_at: "2026-09-26T20:00:00Z",
  expires_at: "2026-09-27T20:00:00Z",
  text: "Heads up, something is coming",
  source: { type: "x_post", author: "thsottiaux", url: "https://x.com/thsottiaux/status/1" },
};

describe("timingFromText", () => {
  it("counts 'tomorrow' down 24 hours from the post, inside tomorrow in San Francisco", () => {
    const timing = timingFromText("Resets coming tomorrow!", POSTED);
    expect(timing).toMatchObject({ kind: "day", day: { kind: "tomorrow" } });
    expect(iso(timing)).toBe("2026-09-27T21:41:35.000Z");
  });

  it("keeps a late-night 'tomorrow' inside that day across the end of daylight saving time", () => {
    expect(iso(timingFromText("more resets tomorrow", new Date("2026-11-01T06:30:00Z")))).toBe("2026-11-02T06:30:00.000Z");
  });

  it("runs 'today' and 'tonight' to midnight in San Francisco", () => {
    expect(timingFromText("Resetting limits tonight", POSTED)).toMatchObject({ kind: "day", day: { kind: "tonight" } });
    expect(iso(timingFromText("Resetting limits tonight", POSTED))).toBe("2026-09-27T07:00:00.000Z");
    expect(iso(timingFromText("one more reset later today", POSTED))).toBe("2026-09-27T07:00:00.000Z");
  });

  it("counts a weekday as that many days after the post", () => {
    const timing = timingFromText("We will reset on Monday", POSTED);
    expect(timing).toMatchObject({ kind: "day", day: { kind: "weekday", weekday: 1 } });
    expect(iso(timing)).toBe("2026-09-28T21:41:35.000Z");
  });

  it("reads 'in N hours' and a clock time as exact", () => {
    expect(timingFromText("Resetting in 2 hours", POSTED)).toEqual({ kind: "exact", at: new Date("2026-09-26T23:41:35Z"), from: "post" });
    expect(iso(timingFromText("resets in ~30 min", POSTED))).toBe("2026-09-26T22:11:35.000Z");
    expect(iso(timingFromText("Reset in half an hour", POSTED))).toBe("2026-09-26T22:11:35.000Z");
    expect(iso(timingFromText("Resetting tomorrow at 10am PT", POSTED))).toBe("2026-09-27T17:00:00.000Z");
    expect(iso(timingFromText("Resets at 5pm ET", POSTED))).toBe("2026-09-27T21:00:00.000Z");
    expect(iso(timingFromText("Resetting at 4:30 pm", POSTED))).toBe("2026-09-26T23:30:00.000Z");
  });

  it("gives no countdown for a week or a weekend", () => {
    expect(timingFromText("@giadotai Sorry Gia. More resets coming next week", POSTED)).toEqual({
      kind: "window",
      window: "nextWeek",
      ends: new Date("2026-10-05T07:00:00Z"),
    });
    expect(timingFromText("Another reset this weekend", POSTED)).toEqual({ kind: "window", window: "weekend", ends: new Date("2026-09-28T07:00:00Z") });
  });

  it("does not mistake numbers for times", () => {
    expect(timingFromText("Resets are coming, with 2x usage on GPT-5.5 for 50% of plans", POSTED)).toEqual({ kind: "unknown" });
    expect(timingFromText("Resetting everyone at 9", POSTED)).toEqual({ kind: "unknown" });
  });
});

describe("upcomingReset", () => {
  it("counts down to scheduled_for when the site knows the time", () => {
    const parsed = parseResetStatus(status(scheduled("Resets at noon", "2026-09-27T01:00:00.000Z")));
    const next = upcomingReset(parsed, new Date("2026-09-26T22:00:00Z"));
    expect(next).toMatchObject({ origin: "scheduled", kind: "regular", timing: { kind: "exact", at: new Date("2026-09-27T01:00:00Z"), from: "site" } });
  });

  it("reads the post when scheduled_for is missing, as the live data on 27/09 does", () => {
    const parsed = parseResetStatus(status(scheduled("@giadotai Sorry Gia. More resets coming next week")));
    expect(upcomingReset(parsed, new Date("2026-09-27T01:19:37Z"))?.timing).toMatchObject({ kind: "window", window: "nextWeek" });
    expect(upcomingReset(parsed, new Date("2026-10-05T07:00:00Z"))).toBeNull();
  });

  it("keeps a passed time for a day, then lets go", () => {
    const parsed = parseResetStatus(status(scheduled("Resetting in 1 hour")));
    const due = POSTED.getTime() + 3_600_000;
    expect(upcomingReset(parsed, new Date(due + AWAITING_MS - 60_000))).not.toBeNull();
    expect(upcomingReset(parsed, new Date(due + AWAITING_MS))).toBeNull();
  });

  it("falls back to the site's watch and ends with it", () => {
    const parsed = parseResetStatus(status(null, WATCH));
    expect(upcomingReset(parsed, new Date("2026-09-27T01:00:00Z"))).toMatchObject({
      origin: "watch",
      kind: null,
      chancePercent: 65,
      timing: { kind: "by", at: new Date("2026-09-27T20:00:00Z") },
    });
    expect(upcomingReset(parsed, new Date("2026-09-27T20:00:00Z"))).toBeNull();
  });

  it("prefers an announcement over a watch", () => {
    const parsed = parseResetStatus(status(scheduled("Resets coming tomorrow"), WATCH));
    expect(upcomingReset(parsed, new Date("2026-09-27T01:00:00Z"))?.origin).toBe("scheduled");
  });

  it("is empty without data", () => {
    expect(upcomingReset(null, POSTED)).toBeNull();
    expect(upcomingReset(parseResetStatus(status(null)), POSTED)).toBeNull();
  });
});
