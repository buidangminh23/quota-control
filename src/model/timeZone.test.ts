import { afterEach, describe, expect, it } from "vitest";
import {
  calendarDaysBetween,
  deviceTimeZone,
  instantAt,
  isKnownTimeZone,
  offsetLabel,
  offsetMinutes,
  setSystemTimeZone,
  startOfDayIn,
  webviewTimeZone,
  zonedParts,
  zoneName,
} from "./timeZone";

const SAIGON = "Asia/Saigon";
const LOS_ANGELES = "America/Los_Angeles";

afterEach(() => setSystemTimeZone(null));

describe("zonedParts", () => {
  it("reads one moment as each zone's own date, time and weekday", () => {
    const moment = new Date("2026-09-27T01:19:37Z");
    expect(zonedParts(moment, SAIGON)).toEqual({ year: 2026, month: 9, day: 27, hour: 8, minute: 19, weekday: 0 });
    expect(zonedParts(moment, LOS_ANGELES)).toEqual({ year: 2026, month: 9, day: 26, hour: 18, minute: 19, weekday: 6 });
  });

  it("reads midnight as hour 0", () => {
    expect(zonedParts(new Date("2026-09-27T07:00:00Z"), LOS_ANGELES).hour).toBe(0);
  });
});

describe("offsets", () => {
  it("follows daylight saving time", () => {
    expect(offsetMinutes(new Date("2026-09-27T00:00:00Z"), SAIGON)).toBe(420);
    expect(offsetMinutes(new Date("2026-09-27T00:00:00Z"), LOS_ANGELES)).toBe(-420);
    expect(offsetMinutes(new Date("2026-01-15T00:00:00Z"), LOS_ANGELES)).toBe(-480);
  });

  it("labels whole, half and zero offsets", () => {
    const moment = new Date("2026-09-27T00:00:00Z");
    expect(offsetLabel(moment, SAIGON)).toBe("GMT+7");
    expect(offsetLabel(moment, LOS_ANGELES)).toBe("GMT-7");
    expect(offsetLabel(moment, "Asia/Kolkata")).toBe("GMT+5:30");
    expect(offsetLabel(moment, "UTC")).toBe("GMT");
  });
});

describe("instantAt", () => {
  it("turns a wall-clock time in a zone into the moment it names", () => {
    expect(instantAt({ year: 2026, month: 9, day: 27, hour: 0, minute: 0 }, LOS_ANGELES).toISOString()).toBe("2026-09-27T07:00:00.000Z");
    expect(instantAt({ year: 2026, month: 9, day: 28, hour: 8, minute: 0 }, SAIGON).toISOString()).toBe("2026-09-28T01:00:00.000Z");
  });

  it("takes the first of a repeated hour and a neighbour of a skipped one", () => {
    expect(instantAt({ year: 2026, month: 11, day: 1, hour: 1, minute: 30 }, LOS_ANGELES).toISOString()).toBe("2026-11-01T08:30:00.000Z");
    const skipped = instantAt({ year: 2026, month: 3, day: 8, hour: 2, minute: 30 }, LOS_ANGELES).toISOString();
    expect(["2026-03-08T09:30:00.000Z", "2026-03-08T10:30:00.000Z"]).toContain(skipped);
  });
});

describe("calendar days", () => {
  it("counts days in the zone asked for", () => {
    const now = new Date("2026-09-26T20:00:00Z");
    const later = new Date("2026-09-27T10:00:00Z");
    expect(calendarDaysBetween(now, later, LOS_ANGELES)).toBe(1);
    expect(calendarDaysBetween(now, later, SAIGON)).toBe(0);
  });

  it("finds the midnight that starts a day in a zone", () => {
    const post = new Date("2026-09-26T21:41:35Z");
    expect(startOfDayIn(post, LOS_ANGELES).toISOString()).toBe("2026-09-26T07:00:00.000Z");
    expect(startOfDayIn(post, LOS_ANGELES, 1).toISOString()).toBe("2026-09-27T07:00:00.000Z");
    expect(startOfDayIn(new Date("2026-11-01T12:00:00Z"), LOS_ANGELES, 1).toISOString()).toBe("2026-11-02T08:00:00.000Z");
  });
});

describe("device time zone", () => {
  it("uses the zone the core reports and falls back to the webview's", () => {
    setSystemTimeZone(LOS_ANGELES);
    expect(deviceTimeZone()).toBe(LOS_ANGELES);
    setSystemTimeZone("Not/A_Zone");
    expect(deviceTimeZone()).toBe(webviewTimeZone());
    setSystemTimeZone(null);
    expect(deviceTimeZone()).toBe(webviewTimeZone());
  });

  it("knows real zone names only", () => {
    expect(isKnownTimeZone(SAIGON)).toBe(true);
    expect(isKnownTimeZone("")).toBe(false);
    expect(isKnownTimeZone("Mars/Olympus")).toBe(false);
  });

  it("names a zone in the reader's language", () => {
    expect(zoneName(new Date("2026-09-27T00:00:00Z"), SAIGON, "vi-VN")).toMatch(/Đông Dương/);
    expect(zoneName(new Date("2026-09-27T00:00:00Z"), LOS_ANGELES, "en-US")).toBe("Pacific Daylight Time");
  });
});
