import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setSystemTimeZone } from "@/model/timeZone";
import { buildGlanceResets, parseResetFeeds, type GlanceResetsInput, type ResetFeeds } from "./glanceResets";

/** Friday 25/09/2026 10:00 in Vietnam. */
const NOW = new Date("2026-09-25T03:00:00Z");
/** Saturday 26/09/2026 14:41 in San Francisco, Sunday 04:41 in Vietnam. */
const POSTED = "2026-09-26T21:41:35Z";
/** Sunday 27/09/2026 07:00 in Vietnam. */
const LATER = new Date("2026-09-27T00:00:00Z");

function post(id: string, kind: "regular" | "banked", announcedAt: string) {
  return { id, reset_type: kind, announced_at: announcedAt, text: `reset ${id}`, source: { type: "x_post", author: "thsottiaux", url: `https://x.com/thsottiaux/status/${id}` } };
}

const HISTORY = JSON.stringify({
  data: [
    post("1", "regular", "2026-09-24T18:17:54Z"),
    post("2", "banked", "2026-09-22T18:23:37Z"),
    post("3", "regular", "2026-09-12T08:09:17Z"),
    post("4", "regular", "2026-09-08T01:56:57Z"),
    post("5", "regular", "2026-08-30T03:00:00Z"),
  ],
  pagination: { has_more: false, next_cursor: null },
  meta: { api_version: "v1", generated_at: "2026-09-25T02:00:00Z" },
});

function status(scheduled: Record<string, unknown> | null = null, watch: Record<string, unknown> | null = null, latest: Record<string, unknown> | null = null): string {
  return JSON.stringify({
    data: { latest_reset: latest, scheduled_reset: scheduled, active_watch: watch, stats: { total: 5 } },
    meta: { api_version: "v1", generated_at: "2026-09-27T00:00:00Z" },
  });
}

function scheduled(text: string, scheduledFor: string | null = null) {
  return { ...post("9", "regular", POSTED), status: "scheduled", scheduled_for: scheduledFor, text };
}

const WATCH = {
  level: "strong",
  reset_chance_percent: 65,
  forecast_window: "next 24 hours",
  observed_at: "2026-09-26T20:00:00Z",
  expires_at: "2026-09-27T20:00:00Z",
  text: "Heads up, something is coming",
  source: { type: "x_post", author: "thsottiaux", url: "https://x.com/thsottiaux/status/10" },
};

function build(feeds: ResetFeeds, overrides: Partial<GlanceResetsInput> = {}) {
  return buildGlanceResets({ feeds, stale: false, now: NOW, language: "vi", timeFormat: "auto", ...overrides });
}

const plain = (value: string | undefined) => value?.replace(/\s/g, " ");

beforeEach(() => setSystemTimeZone("Asia/Saigon"));
afterEach(() => setSystemTimeZone(null));

describe("buildGlanceResets", () => {
  it("is absent without a status and without any history", () => {
    expect(build(parseResetFeeds(null, null))).toBeNull();
    expect(build(parseResetFeeds("{}", "not json"))).toBeNull();
  });

  it("words the tracker the way the Reset tab does, with the Codex mark and color", () => {
    const resets = build(parseResetFeeds(status(), HISTORY))!;
    expect(resets).toMatchObject({ title: "Reset Codex", source: "Theo codex-resets.com", brand: "codex", color: "#10A37F" });
    expect(resets.mark?.paths.length).toBeGreaterThan(0);
    expect(resets.stale).toBeUndefined();
    expect(resets.upcoming).toBeUndefined();
    expect(resets.latest).toEqual({
      at: "2026-09-24T18:17:54.000Z",
      kind: "regular",
      label: "Lần reset gần nhất",
      kindLabel: "Reset",
      since: { at: "2026-09-24T18:17:54.000Z", text: "Đã {d} chưa có reset", since: true },
      when: "1:17 · T6 25/09",
    });
  });

  it("gives the three chances as whole percents with the tab's horizon labels", () => {
    const resets = build(parseResetFeeds(status(), HISTORY))!;
    expect(resets.forecastTitle).toBe("Khả năng có reset");
    expect(resets.forecast.map((chance) => [chance.days, chance.label])).toEqual([
      [1, "24 giờ tới"],
      [3, "3 ngày tới"],
      [7, "7 ngày tới"],
    ]);
    for (const chance of resets.forecast) expect(Number.isInteger(chance.percent)).toBe(true);
    const [day, three, week] = resets.forecast.map((chance) => chance.percent);
    expect(day! < three! && three! < week! && week! <= 100).toBe(true);
    expect(resets.forecastNote).toBe("Ước tính từ lịch sử, không phải tin chính thức.");
    expect(resets.wait).toBe("Đã 0,4 ngày chưa có reset. Trước đây, 0% số lần chờ ngắn hơn thế này.");
    expect(resets.median).toMatch(/^Bình thường cứ 6,6 ngày lại reset một lần, nên khoảng 15:52 01\/10 là tới lượt\.$/);
  });

  it("says why there is no forecast when the history is too short", () => {
    const resets = build(parseResetFeeds(status(null, null, post("1", "regular", "2026-09-24T18:17:54Z")), null))!;
    expect(resets.forecast).toEqual([]);
    expect(resets.forecastNote).toBe("Chưa đủ lịch sử để ước tính.");
    expect(resets.latest?.at).toBe("2026-09-24T18:17:54.000Z");
    expect(resets.wait).toBeUndefined();
  });

  it("lays the calendar out as one character per day, Monday first, with today and the days to come", () => {
    const calendar = build(parseResetFeeds(status(), HISTORY))!.calendar!;
    const empty = ".......";
    expect(calendar).toMatchObject({ title: "Lịch reset 20 tuần qua", weeks: 20, today: 137 });
    expect(calendar.cells).toBe(`${empty.repeat(15)}......r${empty}.r...r.${empty}..b.r--`);
    expect(calendar.weekdays).toEqual(["T2", "T3", "T4", "T5", "T6", "T7", "CN"]);
    expect(calendar.months).toEqual([
      { week: 0, label: "Th5" },
      { week: 3, label: "Th6" },
      { week: 8, label: "Th7" },
      { week: 12, label: "Th8" },
      { week: 17, label: "Th9" },
    ]);
    expect(calendar.legend).toEqual({ regular: "Reset", banked: "Lượt để dành", today: "Hôm nay" });
  });

  it("counts announcements by weekday and four-hour block in the device's zone", () => {
    const rhythm = build(parseResetFeeds(status(), HISTORY))!.rhythm!;
    expect(rhythm).toMatchObject({ title: "Thói quen thông báo", total: 5, weekdayTitle: "Theo thứ", hourTitle: "Theo giờ (giờ máy này)" });
    expect(rhythm.weekdays.map((day) => day.count)).toEqual([0, 1, 1, 0, 1, 1, 1]);
    expect(rhythm.hours).toEqual([
      { label: "0h", count: 2 },
      { label: "4h", count: 0 },
      { label: "8h", count: 2 },
      { label: "12h", count: 1 },
      { label: "16h", count: 0 },
      { label: "20h", count: 0 },
    ]);
  });

  it("stays byte-identical minute to minute while the tracker's numbers do not change", () => {
    const feeds = parseResetFeeds(status(scheduled("Resets coming tomorrow!")), HISTORY);
    const first = JSON.stringify(build(feeds, { now: LATER }));
    expect(JSON.stringify(build(feeds, { now: new Date(LATER.getTime() + 60_000) }))).toBe(first);
    expect(JSON.stringify(build(parseResetFeeds(status(), HISTORY)))).toBe(JSON.stringify(build(parseResetFeeds(status(), HISTORY), { now: new Date(NOW.getTime() + 60_000) })));
  });

  it("marks a tracker whose feed could not be refreshed", () => {
    expect(build(parseResetFeeds(status(), HISTORY), { stale: true })!.stale).toBe("Lần tải gần nhất bị lỗi, đang hiện bản đã lưu.");
  });
});

describe("the announced reset", () => {
  const upcoming = (statusBody: string, now = LATER) => build(parseResetFeeds(statusBody, HISTORY), { now })!.upcoming;

  it("counts down to an exact time, then waits a day for confirmation", () => {
    expect(upcoming(status(scheduled("Resetting at the time below", "2026-09-28T01:00:00Z")))).toEqual({
      title: "Reset free",
      tone: "positive",
      countdown: { at: "2026-09-28T01:00:00.000Z", text: "sau {d}", after: "chờ xác nhận" },
      caption: "Lúc 8:00 · T2 28/09 · GMT+7",
      captionAfter: "Hẹn 8:00 · T2 28/09 · GMT+7",
      hideAt: "2026-09-29T01:00:00.000Z",
    });
  });

  it("estimates a named day and keeps the poster's word", () => {
    expect(upcoming(status(scheduled("Resets coming tomorrow!")))).toEqual({
      title: "Reset free",
      tone: "positive",
      countdown: { at: "2026-09-27T21:41:35.000Z", text: "sau ~{d}", after: "chờ xác nhận" },
      caption: "Khoảng 4:41 · T2 28/09 · GMT+7",
      captionAfter: "Hẹn 4:41 · T2 28/09 · GMT+7",
      note: "“Ngày mai” theo giờ Mỹ",
      hideAt: "2026-09-28T21:41:35.000Z",
    });
  });

  it("counts a watch down to its deadline with its chance", () => {
    expect(upcoming(status(null, WATCH))).toEqual({
      title: "Có thể reset",
      tone: "notice",
      countdown: { at: "2026-09-27T20:00:00.000Z", text: "trong {d} tới" },
      caption: "65% · trước 3:00 · T2 28/09 · GMT+7",
      hideAt: "2026-09-27T20:00:00.000Z",
      chancePercent: 65,
    });
  });

  it("shows a fixed value for a window or a post without a time, until the tab would drop it", () => {
    expect(upcoming(status(scheduled("More resets coming next week")))).toEqual({
      title: "Reset free",
      tone: "positive",
      value: "chưa rõ giờ",
      caption: "Tuần sau giờ Mỹ",
      hideAt: "2026-10-05T07:00:00.000Z",
    });
    expect(upcoming(status(scheduled("Something nice is on the way")))).toEqual({
      title: "Reset free",
      tone: "positive",
      value: "chưa rõ giờ",
      caption: "Bài đăng chưa nói khi nào",
      hideAt: "2026-10-03T21:41:35.000Z",
    });
  });

  it("is gone once the tab would no longer show it", () => {
    expect(upcoming(status(scheduled("Resetting soon", "2026-09-28T01:00:00Z")), new Date("2026-09-29T02:00:00Z"))).toBeUndefined();
    expect(upcoming(status(null, WATCH), new Date("2026-09-27T21:00:00Z"))).toBeUndefined();
  });
});

describe("English", () => {
  it("words every part in English", () => {
    const resets = build(parseResetFeeds(status(scheduled("Resetting at the time below", "2026-09-28T01:00:00Z")), HISTORY), { now: LATER, language: "en" })!;
    expect(resets).toMatchObject({ title: "Codex Resets", source: "From codex-resets.com", forecastTitle: "Chance of a reset" });
    expect(resets.forecastNote).toBe("An estimate from past resets, not official word.");
    expect(resets.upcoming).toMatchObject({ title: "Free reset", countdown: { text: "in {d}", after: "awaiting confirmation" } });
    expect(plain(resets.upcoming?.caption)).toBe("At 8:00 AM · Mon, Sep 28 · GMT+7");
    expect(resets.latest).toMatchObject({ label: "Latest reset", kindLabel: "Reset", since: { text: "{d} since the last reset" } });
    expect(resets.forecast.map((chance) => chance.label)).toEqual(["next 24 hours", "next 3 days", "next 7 days"]);
    expect(resets.calendar?.weekdays).toEqual(["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]);
    expect(resets.calendar?.legend).toEqual({ regular: "Reset", banked: "Banked", today: "Today" });
  });
});

describe("parseResetFeeds", () => {
  it("folds the status's latest reset into the history, newest first", () => {
    const feeds = parseResetFeeds(status(null, null, post("0", "banked", "2026-09-25T01:00:00Z")), HISTORY);
    expect(feeds.status?.latest?.id).toBe("0");
    expect(feeds.resets.map((reset) => reset.id)).toEqual(["0", "1", "2", "3", "4", "5"]);
  });
});
