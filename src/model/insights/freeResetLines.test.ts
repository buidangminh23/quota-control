import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setSystemTimeZone } from "@/model/timeZone";
import { freeResetLines } from "./freeResetLines";
import type { ResetTiming, UpcomingReset } from "./upcomingReset";

/** Sunday 27/09/2026 07:00 in Vietnam. */
const NOW = new Date("2026-09-27T00:00:00Z");
const POSTED = new Date("2026-09-26T21:41:35Z");

function reset(timing: ResetTiming, overrides: Partial<UpcomingReset> = {}): UpcomingReset {
  return {
    origin: "scheduled",
    kind: "regular",
    timing,
    chancePercent: null,
    announcedAt: POSTED,
    text: "Resets coming tomorrow! https://t.co/abc",
    source: { kind: "x_post", url: "https://x.com/thsottiaux/status/1" },
    ...overrides,
  };
}

beforeEach(() => setSystemTimeZone("Asia/Saigon"));
afterEach(() => setSystemTimeZone(null));

describe("freeResetLines", () => {
  it("counts down to an exact time and shows it in the device's zone", () => {
    const lines = freeResetLines(reset({ kind: "exact", at: new Date("2026-09-28T01:00:00Z"), from: "site" }), NOW, "auto", "vi");
    expect(lines).toMatchObject({ title: "Reset free", value: "sau 1 ngày 1 giờ", caption: "Lúc 8:00 · ngày mai · GMT+7", note: null, awaiting: false });
    expect(lines.details).toContain("Codex Resets ghi giờ hẹn cụ thể");
    expect(lines.details).toContain("GMT+7");
  });

  it("keeps the poster's day on its own line under an estimated local time", () => {
    const lines = freeResetLines(reset({ kind: "day", at: new Date("2026-09-27T21:41:35Z"), day: { kind: "tomorrow" } }), POSTED, "auto", "vi");
    expect(lines.value).toBe("sau ~1 ngày 0 giờ");
    expect(lines.caption).toBe("Khoảng 4:41 · T2 28/09 · GMT+7");
    expect(lines.note).toBe("“Ngày mai” theo giờ Mỹ");
    expect(lines.details).toMatch(/^@thsottiaux đăng lúc 4:41 · hôm nay:\n“Resets coming tomorrow!”\n“Ngày mai” tính theo giờ San Francisco/);
  });

  it("marks a passed time as waiting for confirmation", () => {
    const lines = freeResetLines(reset({ kind: "exact", at: new Date("2026-09-26T23:00:00Z"), from: "post" }), NOW, "24h", "vi");
    expect(lines).toMatchObject({ value: "chờ xác nhận", caption: "Hẹn 6:00 · hôm nay · GMT+7", awaiting: true });
  });

  it("words a watch as a deadline with its chance", () => {
    const watch = reset({ kind: "by", at: new Date("2026-09-27T10:00:00Z") }, { origin: "watch", kind: null, chancePercent: 65 });
    const lines = freeResetLines(watch, NOW, "auto", "vi");
    expect(lines).toMatchObject({ title: "Có thể reset", value: "trong 10 giờ tới", caption: "65% · trước 17:00 · hôm nay · GMT+7" });
    expect(lines.details).toContain("Codex Resets thấy dấu hiệu");
  });

  it("has no countdown for a window or a post without a time", () => {
    const week = freeResetLines(reset({ kind: "window", window: "nextWeek", ends: new Date("2026-10-05T07:00:00Z") }), NOW, "auto", "vi");
    expect(week).toMatchObject({ value: "chưa rõ giờ", caption: "Tuần sau giờ Mỹ", awaiting: false });
    expect(freeResetLines(reset({ kind: "unknown" }), NOW, "auto", "vi")).toMatchObject({ value: "chưa rõ giờ", caption: "Bài đăng chưa nói khi nào" });
  });

  it("speaks English and follows the device's zone", () => {
    setSystemTimeZone("America/New_York");
    const lines = freeResetLines(reset({ kind: "exact", at: new Date("2026-09-28T01:00:00Z"), from: "site" }, { kind: "banked" }), NOW, "auto", "en");
    expect(lines).toMatchObject({ title: "Banked reset", value: "in 1d 1h" });
    expect(lines.caption.replace(/\s/g, " ")).toBe("At 9:00 PM · tomorrow · GMT-4");
    const monday = freeResetLines(reset({ kind: "day", at: new Date("2026-09-28T21:41:35Z"), day: { kind: "weekday", weekday: 1 } }), POSTED, "12h", "en");
    expect(monday.caption.replace(/\s/g, " ")).toBe("Around 5:41 PM · Mon, Sep 28 · GMT-4");
    expect(monday.note).toBe("“Monday” in US Pacific time");
  });
});
