import { evaluatePace, secondsToRunOut } from "./pace";
import { setSystemClockPreference } from "./format";
import {
  boundedDetailText,
  boundedDetailTooltip,
  boundedResetMoment,
  boundedTrailingText,
  hasResetLabel,
  isFreshSessionWindow,
  meterSeverity,
  meterState,
  meterStyleTooltip,
  meterTooltip,
  paceTick,
  resetCountdownText,
  spareText,
  type MeterState,
} from "./meterState";
import { makeWidget, NOW, plainSpaces, resetsAt, WEEK_SECONDS } from "./testHelpers";
import { setSystemTimeZone } from "./timeZone";
import type { DisplayMode, WidgetData } from "./widgetData";

const now = NOW;
const week = WEEK_SECONDS;
const FRESH_SESSION_TOOLTIP = "Sessions start after you send your first message.";

function paced(used: number, elapsed = 0.5, period = week, displayMode: DisplayMode = "used", alwaysShowPacing = false): WidgetData {
  return makeWidget("Weekly", "percent", used, 100, {
    displayMode,
    alwaysShowPacing,
    resetsAt: resetsAt(elapsed, period),
    periodDurationMs: period * 1000,
  });
}

function tick(data: WidgetData): number | null {
  return paceTick(data, meterState(data, now), now);
}

function spare(data: WidgetData): string | null {
  return spareText(meterState(data, now), data.language);
}

const level = (severity: "normal" | "warning" | "critical"): MeterState => ({ kind: "level", severity });

describe("Pace", () => {
  it("has no signal at zero usage", () => {
    expect(evaluatePace(0, 100, resetsAt(0.5, week), week, now)).toBeNull();
  });

  it("projects early in the window", () => {
    expect(evaluatePace(5, 100, resetsAt(0.02, week), week, now)?.status).toBe("behind");
  });

  it("classifies ahead / on track / behind", () => {
    const reset = resetsAt(0.5, week);
    const cases: Array<[number, string]> = [
      [44, "ahead"],
      [46, "onTrack"],
      [50, "onTrack"],
      [60, "behind"],
      [100, "behind"],
      [130, "behind"],
    ];
    for (const [used, status] of cases) expect(evaluatePace(used, 100, reset, week, now)?.status).toBe(status);
  });

  it("projects end-of-period usage", () => {
    const result = evaluatePace(30, 100, resetsAt(0.5, week), week, now);
    expect(result?.status).toBe("ahead");
    expect(result?.projectedUsage).toBeCloseTo(60, 2);
  });

  it("returns null once the window reset", () => {
    expect(evaluatePace(50, 100, new Date(now.getTime() - 60_000), week, now)).toBeNull();
    expect(evaluatePace(50, 100, now, week, now)).toBeNull();
  });

  it("runs out only when behind and before reset", () => {
    const reset = resetsAt(0.33, week);
    expect(secondsToRunOut(50, 100, reset, week, now)!).toBeCloseTo(0.33 * week, -4);
    expect(secondsToRunOut(30, 100, reset, week, now)).toBeNull();
  });

  it("waits until the window has materially started", () => {
    const session = 5 * 3600;
    expect(evaluatePace(1, 100, resetsAt(60 / session, session), session, now)).toBeNull();
  });
});

describe("meter state", () => {
  it("shows the even-pace tick for amber and red, not blue by default", () => {
    expect(tick(paced(46))).toBeCloseTo(0.5, 3);
    expect(tick(paced(60))).toBeCloseTo(0.5, 3);
    expect(tick(paced(30))).toBeNull();
  });

  it("has no tick without a reset window", () => {
    const data = makeWidget("Credits", "dollars", 12, 20);
    expect(tick(data)).toBeNull();
    expect(tick({ ...data, resetsAt: new Date(now.getTime() + week * 1000) })).toBeNull();
  });

  it("shows the numeric projection at reset", () => {
    expect(meterTooltip(meterState(paced(30), now), "en")).toBe("~40% left at reset");
    expect(meterTooltip(meterState(paced(46), now), "en")).toBe("~92% used at reset");
    expect(meterTooltip(meterState(paced(60), now), "en")).toBe("~20% over limit at reset");
    expect(meterTooltip(meterState(paced(50.2), now), "en")).toBe("~1% over limit at reset");
  });

  it("falls back to a plain level bar at zero usage", () => {
    expect(meterState(paced(0), now)).toEqual(level("normal"));
    expect(meterTooltip(meterState(paced(0), now), "en")).toBeNull();
  });

  it("reads Limit reached when spent", () => {
    expect(meterState(paced(100), now)).toEqual({ kind: "spent" });
    expect(meterTooltip(meterState(paced(100), now), "en")).toBe("Limit reached");
    expect(meterState(makeWidget("Credits", "dollars", 99.999, 100), now)).toEqual({ kind: "spent" });
    expect(meterTooltip(meterState(makeWidget("Credits", "dollars", 99, 100), now), "en")).toBeNull();
  });

  it("shows spare copy only when amber", () => {
    expect(spare(paced(46))).toBe("~8% spare");
    expect(spare(paced(30))).toBeNull();
    expect(spare(paced(60))).toBeNull();
    expect(spare(paced(49))).toBe("~2% spare");
  });

  it("lets spent outrank close-to-limit", () => {
    const data = makeWidget("Weekly", "percent", 99.6, 100, { resetsAt: resetsAt(0.997, week), periodDurationMs: week * 1000 });
    expect(meterState(data, now)).toEqual({ kind: "spent" });
    expect(tick(data)).toBeNull();
    expect(spare(data)).toBeNull();
  });

  it("turns a projection at the limit red without an ETA", () => {
    for (const used of [49.8, 50]) {
      const state = meterState(paced(used), now);
      expect(state.kind).toBe("runningOut");
      if (state.kind === "runningOut") expect(state.eta).toBeNull();
      expect(tick(paced(used))).not.toBeNull();
      expect(meterTooltip(state, "en")).toBe("~100% used at reset");
    }
  });

  it("carries a run-out time before reset", () => {
    const state = meterState(paced(60), now);
    expect(state.kind === "runningOut" && state.eta !== null).toBe(true);
  });

  it("distrusts projections below five percent used", () => {
    const session = 5 * 3600;
    expect(meterState(paced(2, 240 / session, session), now)).toEqual(level("normal"));
    expect(meterState(paced(1, 0.01, session), now)).toEqual(level("normal"));
    expect(meterState(paced(6, 240 / session, session), now).kind).toBe("runningOut");
  });

  it("adds the tick to healthy bars with Always Show Pacing", () => {
    expect(tick(paced(30, 0.4, week, "used", true))).toBeCloseTo(0.4, 3);
    expect(tick(paced(30, 0.4, week, "remaining", true))).toBeCloseTo(0.6, 3);
    expect(tick(paced(76, 0.8, week, "used", true))).toBeCloseTo(0.8, 3);
    expect(tick(paced(76, 0.8, week, "remaining", true))).toBeCloseTo(0.2, 3);
    expect(tick(paced(60, 0.5, week, "used", true))).toBeCloseTo(0.5, 3);
    expect(tick(paced(100, 0.5, week, "used", true))).toBeNull();
    expect(spare(paced(46, 0.5, week, "used", true))).toBe("~8% spare");
  });

  it("stays silent on untouched meters and rows without a window", () => {
    const untouched = paced(0, 0.03, week, "used", true);
    expect(meterState(untouched, now)).toEqual(level("normal"));
    expect(tick(untouched)).toBeNull();
    const credits = makeWidget("Credits", "dollars", 12, 20, { alwaysShowPacing: true });
    expect(tick(credits)).toBeNull();
    expect(meterTooltip(meterState(credits, now), "en")).toBeNull();
  });
});

describe("meter severity", () => {
  const severity = (data: WidgetData) => meterSeverity(meterState(data, now));
  const percent = (used: number, limit: number | null = 100, displayMode: DisplayMode = "used") =>
    makeWidget("Session", "percent", used, limit, { displayMode });

  it("lets the pace verdict override absolute bands", () => {
    expect(severity(paced(66, 0.363))).toBe("critical");
    expect(severity(paced(85, 0.96))).toBe("normal");
    expect(severity(paced(88, 0.9))).toBe("warning");
    expect(severity(paced(85, 0.02))).toBe("critical");
  });

  it("treats a remainder that rounds to zero as spent", () => {
    expect(meterState(paced(100, 0.5), now)).toEqual({ kind: "spent" });
    expect(meterState(paced(130, 0.02), now)).toEqual({ kind: "spent" });
    expect(meterState(paced(99.6, 0.997), now)).toEqual({ kind: "spent" });
    expect(meterState(makeWidget("Credits", "dollars", 99.996, 100), now)).toEqual({ kind: "spent" });
  });

  it("bands absolute levels on rounded percentages", () => {
    const cases: Array<[number, string]> = [
      [0, "normal"],
      [79, "normal"],
      [79.6, "warning"],
      [80, "warning"],
      [89.4, "warning"],
      [89.6, "critical"],
      [90, "critical"],
    ];
    for (const [used, expected] of cases) expect(severity(percent(used))).toBe(expected);
  });

  it("ignores the Used/Left mode", () => {
    expect(severity(percent(95, 100, "remaining"))).toBe("critical");
    expect(severity(percent(85, 100, "remaining"))).toBe("warning");
    expect(severity(percent(5, 100, "remaining"))).toBe("normal");
  });

  it("bands other kinds on their share of the limit", () => {
    expect(severity(makeWidget("Credits", "dollars", 45, 50))).toBe("critical");
    expect(severity(makeWidget("Requests", "count", 400, 500, { countSuffix: "requests" }))).toBe("warning");
  });

  it("keeps unbounded, zero-limit and no-data meters calm", () => {
    expect(severity(percent(99, null))).toBe("normal");
    expect(severity(percent(99, 0))).toBe("normal");
    const noData = { ...percent(50), hasData: false };
    expect(meterState(noData, now)).toEqual({ kind: "noData" });
    expect(severity(noData)).toBeNull();
  });
});

describe("reset display", () => {
  it("honors the mode in the one-phrase text a notification says", () => {
    const current = new Date();
    const data = makeWidget("Weekly", "percent", 50, 100, {
      resetsAt: new Date(current.getTime() + (4 * 24 + 17) * 3_600_000),
      periodDurationMs: week * 1000,
    });
    expect(hasResetLabel(data, current)).toBe(true);
    expect(boundedTrailingText(data, current)?.startsWith("Resets in ")).toBe(true);
    const absolute = { ...data, resetDisplayMode: "absolute" as const };
    expect(boundedTrailingText(absolute, current)?.startsWith("Resets in ")).toBe(false);
    expect(boundedTrailingText(absolute, current)?.startsWith("Resets ")).toBe(true);
  });

  it("reads Not started for a fresh zero-usage session and suppresses pacing", () => {
    const at = new Date(1_800_000_000_000);
    const period = 5 * 3600;
    const data = makeWidget("Session", "percent", 0, 100, {
      sessionStartSignal: "zeroUsage",
      periodDurationMs: period * 1000,
      resetsAt: new Date(at.getTime() + (period / 2) * 1000),
      alwaysShowPacing: true,
    });
    expect(boundedTrailingText(data, at)).toBe("Not started");
    expect(hasResetLabel(data, at)).toBe(false);
    expect(resetCountdownText(data, at)).toBeNull();
    expect(boundedDetailText(data, at)).toBe("Not started");
    expect(boundedDetailTooltip(data, at)).toBe(FRESH_SESSION_TOOLTIP);
    const state = meterState(data, at);
    expect(state).toEqual(level("normal"));
    expect(paceTick(data, state, at)).toBeNull();
  });

  it("keeps the countdown for a sub-one-percent session with a reset date", () => {
    const at = new Date(1_800_000_000_000);
    const data = makeWidget("Session", "percent", 0, 100, {
      sessionStartSignal: "missingResetDate",
      periodDurationMs: 5 * 3600 * 1000,
      resetsAt: new Date(at.getTime() + 2 * 3_600_000),
    });
    expect(isFreshSessionWindow(data, at)).toBe(false);
    expect(hasResetLabel(data, at)).toBe(true);
    expect(boundedTrailingText(data, at)?.startsWith("Resets in")).toBe(true);
    const fresh = { ...data, resetsAt: null };
    expect(isFreshSessionWindow(fresh, at)).toBe(true);
    expect(boundedTrailingText(fresh, at)).toBe("Not started");
  });

  it("never reads Not started without the session signal", () => {
    const at = new Date(1_800_000_000_000);
    const data = makeWidget("Weekly", "percent", 0, 100, {
      periodDurationMs: week * 1000,
      resetsAt: new Date(at.getTime() + (week / 2) * 1000),
    });
    expect(isFreshSessionWindow(data, at)).toBe(false);
    expect(boundedTrailingText(data, at)?.startsWith("Resets")).toBe(true);
  });

  it("honors the mode in the run-out time", () => {
    const current = new Date();
    const data = makeWidget("Session", "percent", 90, 100, {
      resetsAt: new Date(current.getTime() + 5 * 3_600_000),
      periodDurationMs: 10 * 3_600_000,
    });
    const relative = meterState(data, current);
    expect(relative.kind === "runningOut" && relative.eta?.startsWith("Limit in ") && relative.eta.endsWith("m")).toBe(true);
    const absolute = meterState({ ...data, resetDisplayMode: "absolute" }, current);
    expect(absolute.kind === "runningOut" && /^Limit (today|tomorrow) at /.test(absolute.eta ?? "")).toBe(true);
  });

  it("falls back to limit context without a reset date", () => {
    const data = makeWidget("Credits", "dollars", 12, 20, { resetDisplayMode: "absolute" });
    expect(hasResetLabel(data, now)).toBe(false);
    expect(boundedDetailTooltip(data, now)).toBeNull();
    expect(boundedTrailingText(data, now)).toBe("$20 limit");
    expect(resetCountdownText(data, now)).toBeNull();
    expect(boundedDetailText(data, now)).toBe("$20 limit");
  });

  it("flips the Used/Left reading in the meter style tooltip", () => {
    expect(meterStyleTooltip(makeWidget("Weekly", "percent", 5, 100, { displayMode: "remaining" }))).toBe("5% used");
    expect(meterStyleTooltip(makeWidget("Weekly", "percent", 5, 100))).toBe("95% left");
    expect(meterStyleTooltip(makeWidget("Weekly", "percent", 5, null))).toBeNull();
  });
});

describe("meter copy in Vietnamese", () => {
  const vi = (data: WidgetData): WidgetData => ({ ...data, language: "vi", timeFormat: "24h" });

  it("projects the reset in Vietnamese", () => {
    expect(meterTooltip(meterState(vi(paced(30)), now), "vi")).toBe("Còn ~40% khi đặt lại");
    expect(meterTooltip(meterState(vi(paced(46)), now), "vi")).toBe("Dùng ~92% khi đặt lại");
    expect(meterTooltip(meterState(vi(paced(60)), now), "vi")).toBe("Vượt ~20% hạn mức khi đặt lại");
    expect(meterTooltip(meterState(vi(paced(100)), now), "vi")).toBe("Đã hết hạn mức");
    expect(spare(vi(paced(46)))).toBe("Dư ~8%");
  });

  it("reads the run-out time and the reset countdown in Vietnamese", () => {
    const current = new Date();
    const data = vi(
      makeWidget("Session", "percent", 90, 100, {
        resetsAt: new Date(current.getTime() + 5 * 3_600_000),
        periodDurationMs: 10 * 3_600_000,
      }),
    );
    const state = meterState(data, current);
    expect(state.kind === "runningOut" && state.eta?.startsWith("Hết hạn mức sau ")).toBe(true);
    expect(boundedTrailingText(data, current)?.startsWith("Đặt lại sau ")).toBe(true);
    expect(resetCountdownText(data, current)?.startsWith("Đặt lại sau ")).toBe(true);
    expect(boundedDetailText(data, current)?.startsWith("Đặt lại lúc ")).toBe(true);
  });

  it("explains a fresh session and missing data in Vietnamese", () => {
    const at = new Date(1_800_000_000_000);
    const fresh = vi(
      makeWidget("Session", "percent", 0, 100, {
        sessionStartSignal: "zeroUsage",
        periodDurationMs: 5 * 3600 * 1000,
        resetsAt: new Date(at.getTime() + 2.5 * 3_600_000),
      }),
    );
    expect(boundedTrailingText(fresh, at)).toBe("Chưa bắt đầu");
    expect(boundedDetailText(fresh, at)).toBe("Chưa bắt đầu");
    expect(boundedDetailTooltip(fresh, at)).toBe("Phiên chỉ bắt đầu sau khi bạn gửi tin nhắn đầu tiên.");
    expect(boundedTrailingText({ ...fresh, hasData: false }, at)).toBe("Không có dữ liệu");
    expect(boundedDetailText({ ...fresh, hasData: false }, at)).toBe("Không có dữ liệu");
    expect(boundedDetailTooltip({ ...fresh, hasData: false }, at)).toBeNull();
  });

  it("states the limit and flips the reading in Vietnamese", () => {
    const credits = vi(makeWidget("Credits", "dollars", 12, 20));
    expect(plainSpaces(boundedTrailingText(credits, now))).toBe("Hạn mức 20 $");
    expect(meterStyleTooltip(vi(makeWidget("Weekly", "percent", 5, 100, { displayMode: "remaining" })))).toBe("Đã dùng 5%");
    expect(meterStyleTooltip(vi(makeWidget("Weekly", "percent", 5, 100)))).toBe("Còn 95%");
  });
});

describe("the reset countdown and exact moment of a row", () => {
  const SECOND = 1000;
  const inZone = (zone: string, body: () => void): void => {
    setSystemTimeZone(zone);
    try {
      body();
    } finally {
      setSystemTimeZone(null);
    }
  };
  const session = (current: Date, leftMs: number, extra: Partial<WidgetData> = {}): WidgetData =>
    makeWidget("Session", "percent", 40, 100, { language: "vi", timeFormat: "24h", resetsAt: new Date(current.getTime() + leftMs), periodDurationMs: 5 * 3_600_000, ...extra });
  const weekly = (current: Date, leftMs: number, extra: Partial<WidgetData> = {}): WidgetData =>
    makeWidget("Weekly", "percent", 28, 100, { language: "vi", timeFormat: "24h", resetsAt: new Date(current.getTime() + leftMs), periodDurationMs: week * 1000, ...extra });

  it("keeps the compact duration above five minutes and counts the last five to the second", () => {
    const current = new Date(2026, 9, 2, 16, 4);
    const cases: Array<[number, string]> = [
      [2 * 3_600_000 + 34 * 60_000, "Đặt lại sau 2 giờ 34 phút"],
      [6 * 60 * SECOND, "Đặt lại sau 6 phút"],
      [300 * SECOND + 1, "Đặt lại sau 6 phút"],
      [300 * SECOND, "Đặt lại sau 05:00"],
      [299 * SECOND + 1, "Đặt lại sau 05:00"],
      [299 * SECOND, "Đặt lại sau 04:59"],
      [136 * SECOND, "Đặt lại sau 02:16"],
      [60 * SECOND, "Đặt lại sau 01:00"],
      [59 * SECOND + 400, "Đặt lại sau 01:00"],
      [SECOND + 1, "Đặt lại sau 00:02"],
      [SECOND, "Đặt lại sau 00:01"],
      [1, "Đặt lại sau 00:01"],
    ];
    for (const row of [session, weekly]) {
      for (const [left, expected] of cases) expect(resetCountdownText(row(current, left), current), `${left} ms left`).toBe(expected);
    }
    expect(resetCountdownText(weekly(current, 25 * 3_600_000), current)).toBe("Đặt lại sau 1 ngày 1 giờ");
    expect(resetCountdownText(session(current, 299 * SECOND, { language: "en" }), current)).toBe("Resets in 04:59");
    expect(resetCountdownText(session(current, 2 * 3_600_000 + 34 * 60_000, { language: "en" }), current)).toBe("Resets in 2h 34m");
  });

  it("never counts below zero or says the limit is back once the moment has passed", () => {
    const current = new Date(2026, 9, 2, 16, 4);
    for (const left of [0, -1, -SECOND, -3_600_000]) {
      const data = session(current, left);
      expect(resetCountdownText(data, current), `${left} ms left`).toBe("Sắp đặt lại");
      expect(resetCountdownText({ ...data, language: "en" }, current)).toBe("Resets soon");
      expect(boundedDetailText(data, current), `${left} ms left`).toBeNull();
      expect(meterState(data, current).kind).not.toBe("noData");
      expect(data.used).toBe(40);
    }
    expect(resetCountdownText(session(current, 0, { resetsAt: new Date(Number.NaN) }), current)).toBeNull();
  });

  it("names the exact moment beside the reading, by the clock setting and the day in the device zone", () => {
    const current = new Date(Date.UTC(2026, 9, 2, 9, 4));
    inZone("Asia/Saigon", () => {
      const today = session(current, 299 * SECOND);
      expect(boundedDetailText(today, current)).toBe("Đặt lại lúc 16:08 · hôm nay");
      expect(plainSpaces(boundedDetailText({ ...today, timeFormat: "12h" }, current))).toBe("Đặt lại lúc 4:08 CH · hôm nay");
      expect(plainSpaces(boundedDetailText({ ...today, language: "en", timeFormat: "12h" }, current))).toBe("Resets at 4:08 PM · today");
      expect(boundedDetailText({ ...today, language: "en", timeFormat: "24h" }, current)).toBe("Resets at 16:08 · today");
      const beforeMidnight = new Date(Date.UTC(2026, 9, 2, 16, 58));
      const overnight = weekly(beforeMidnight, 4 * 60_000);
      expect(resetCountdownText(overnight, beforeMidnight)).toBe("Đặt lại sau 04:00");
      expect(boundedDetailText(overnight, beforeMidnight)).toBe("Đặt lại lúc 0:02 · ngày mai");
      expect(boundedDetailText(overnight, new Date(Date.UTC(2026, 9, 2, 17, 0)))).toBe("Đặt lại lúc 0:02 · hôm nay");
      expect(boundedDetailText(weekly(current, 6 * 86_400_000 + 3_600_000), current)).toBe("Đặt lại lúc 17:04 · T5 08/10");
    });
    inZone("America/Los_Angeles", () => {
      expect(boundedDetailText(session(current, 299 * SECOND), current)).toBe("Đặt lại lúc 2:08 · hôm nay");
    });
    try {
      setSystemClockPreference(false);
      inZone("Asia/Saigon", () => {
        expect(plainSpaces(boundedDetailText(session(current, 299 * SECOND, { timeFormat: "auto" }), current))).toBe("Đặt lại lúc 4:08 CH · hôm nay");
      });
    } finally {
      setSystemClockPreference(null);
    }
  });

  it("keeps both texts in place whatever Reset Times is saved as", () => {
    const current = new Date(2026, 8, 26, 12);
    const data = weekly(current, 0, { resetsAt: new Date(2026, 9, 2, 13, 5) });
    for (const resetDisplayMode of ["relative", "absolute"] as const) {
      const row = { ...data, resetDisplayMode };
      expect(resetCountdownText(row, current)).toBe("Đặt lại sau 6 ngày 1 giờ");
      expect(boundedDetailText(row, current)).toBe("Đặt lại lúc 13:05 · T6 02/10");
      expect(boundedDetailTooltip(row, current)).toBeNull();
    }
    const final = { ...session(current, 136 * SECOND), resetDisplayMode: "absolute" as const };
    expect(resetCountdownText(final, current)).toBe("Đặt lại sau 02:16");
    expect(boundedDetailText(final, current)).toBe("Đặt lại lúc 12:02 · hôm nay");
  });

  it("hands the row the exact moment in two parts, and none where the row says something else there", () => {
    const current = new Date(2026, 8, 26, 12);
    const data = weekly(current, 0, { resetsAt: new Date(2026, 9, 2, 13, 5) });
    expect(boundedResetMoment(data, current)).toEqual({ text: "Đặt lại lúc 13:05 · T6 02/10", lead: "Đặt lại lúc ", moment: "13:05 · T6 02/10" });
    expect(boundedResetMoment(data, current)?.text).toBe(boundedDetailText(data, current));
    expect(boundedResetMoment({ ...data, language: "en" }, current)).toEqual({ text: "Resets at 13:05 · Fri, Oct 2", lead: "Resets at ", moment: "13:05 · Fri, Oct 2" });
    expect(boundedResetMoment({ ...data, hasData: false }, current)).toBeNull();
    expect(boundedResetMoment({ ...data, subtitleOverride: "Paused" }, current)).toBeNull();
    expect(boundedResetMoment({ ...data, resetsAt: null }, current)).toBeNull();
    expect(boundedResetMoment(data, new Date(2026, 9, 2, 13, 5))).toBeNull();
    const fresh = session(current, 2 * 3_600_000, { used: 0, sessionStartSignal: "zeroUsage" });
    expect([boundedResetMoment(fresh, current), boundedDetailText(fresh, current)]).toEqual([null, "Chưa bắt đầu"]);
  });

  it("leaves a row without a reset to count down with its status beside the reading", () => {
    const current = new Date(2026, 8, 26, 12);
    const data = weekly(current, 0, { resetsAt: new Date(2026, 9, 2, 13, 5) });
    const noData = { ...data, hasData: false };
    expect([resetCountdownText(noData, current), boundedDetailText(noData, current)]).toEqual([null, "Không có dữ liệu"]);
    const paused = { ...data, subtitleOverride: "Paused" };
    expect([resetCountdownText(paused, current), boundedDetailText(paused, current)]).toEqual([null, "Paused"]);
    const rolledOver = { ...data, used: 0, resetsAt: null };
    expect([resetCountdownText(rolledOver, current), boundedDetailText(rolledOver, current)]).toEqual(["Đặt lại sau 7 ngày 0 giờ", null]);
    expect(boundedTrailingText(rolledOver, current)).toBe("Đặt lại sau 7 ngày 0 giờ");
    const credits = makeWidget("Credits", "dollars", 12, 20, { language: "vi" });
    expect(resetCountdownText(credits, current)).toBeNull();
    expect(plainSpaces(boundedDetailText(credits, current))).toBe("Hạn mức 20 $");
  });
});
