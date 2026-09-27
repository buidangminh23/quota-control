import { describe, expect, it } from "vitest";
import { addMonthsClamped, isPlanTermSoon, nextMonthlyRenewal, planTermEnd, planTermLeft } from "./planTerm";

const day = (iso: string) => new Date(iso);

describe("addMonthsClamped", () => {
  it("keeps the day of the month and falls back to a shorter month's last day", () => {
    const start = day("2026-07-31T03:40:09Z");
    expect(addMonthsClamped(start, 1).toISOString()).toBe("2026-08-31T03:40:09.000Z");
    expect(addMonthsClamped(start, 2).toISOString()).toBe("2026-09-30T03:40:09.000Z");
    expect(addMonthsClamped(start, 3).toISOString()).toBe("2026-10-31T03:40:09.000Z");
    expect(addMonthsClamped(start, 7).toISOString()).toBe("2027-02-28T03:40:09.000Z");
    expect(addMonthsClamped(day("2027-12-31T23:59:59Z"), 2).toISOString()).toBe("2028-02-29T23:59:59.000Z");
  });
});

describe("nextMonthlyRenewal", () => {
  it("finds the first renewal after now, counted from the start rather than from the last renewal", () => {
    const start = day("2026-07-31T03:40:09Z");
    expect(nextMonthlyRenewal(start, day("2026-09-27T02:45:00Z")).toISOString()).toBe("2026-09-30T03:40:09.000Z");
    expect(nextMonthlyRenewal(start, day("2026-09-30T03:40:09Z")).toISOString()).toBe("2026-10-31T03:40:09.000Z");
    expect(nextMonthlyRenewal(start, day("2026-08-01T00:00:00Z")).toISOString()).toBe("2026-08-31T03:40:09.000Z");
    expect(nextMonthlyRenewal(day("2026-08-17T01:56:39Z"), day("2026-10-17T01:56:40Z")).toISOString()).toBe("2026-11-17T01:56:39.000Z");
  });
});

describe("planTermEnd", () => {
  const now = day("2026-09-27T02:45:00Z");

  it("takes a stated end as it is and estimates a monthly renewal from a start", () => {
    expect(planTermEnd({ basis: "stated", endsAt: "2026-10-17T01:56:39+00:00", checkedAt: "2026-09-25T13:56:24Z" }, now)).toEqual({
      endsAt: day("2026-10-17T01:56:39Z"),
      estimated: false,
      checkedAt: day("2026-09-25T13:56:24Z"),
      startedAt: null,
    });
    expect(planTermEnd({ basis: "monthlyFrom", startedAt: "2026-07-31T03:40:09Z" }, now)).toEqual({
      endsAt: day("2026-09-30T03:40:09Z"),
      estimated: true,
      checkedAt: null,
      startedAt: day("2026-07-31T03:40:09Z"),
    });
  });

  it("gives up on dates it cannot read", () => {
    expect(planTermEnd({ basis: "stated", endsAt: "soon" }, now)).toBeNull();
    expect(planTermEnd({ basis: "monthlyFrom", startedAt: "" }, now)).toBeNull();
  });
});

describe("planTermLeft", () => {
  const now = day("2026-09-27T00:00:00Z");
  const at = (ms: number) => new Date(now.getTime() + ms);

  it("names whole days, then hours, then minutes, and says when the end has passed", () => {
    expect(planTermLeft(at(19.99 * 86_400_000), now)).toEqual({ kind: "days", count: 19 });
    expect(planTermLeft(at(86_400_000), now)).toEqual({ kind: "days", count: 1 });
    expect(planTermLeft(at(86_399_000), now)).toEqual({ kind: "hours", count: 23 });
    expect(planTermLeft(at(3_600_000), now)).toEqual({ kind: "hours", count: 1 });
    expect(planTermLeft(at(3_599_000), now)).toEqual({ kind: "minutes", count: 60 });
    expect(planTermLeft(at(1_000), now)).toEqual({ kind: "minutes", count: 1 });
    expect(planTermLeft(now, now)).toEqual({ kind: "due" });
  });

  it("turns to the warning color at three whole days, the same count the header shows", () => {
    expect(isPlanTermSoon({ kind: "days", count: 4 })).toBe(false);
    expect(isPlanTermSoon({ kind: "days", count: 3 })).toBe(true);
    expect(isPlanTermSoon({ kind: "hours", count: 5 })).toBe(true);
    expect(isPlanTermSoon({ kind: "due" })).toBe(true);
  });
});
