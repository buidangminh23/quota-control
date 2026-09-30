import { FEED_FIXTURES } from "@/lib/insightsFeedFixtures";
import type { Language } from "@/i18n";
import { compactDuration, timeOnDayLabel, type TimeFormat } from "./format";
import type { GlanceResetRow } from "./glance";
import { claudeResetRow, codexResetRow, freeResetRow, MOMENT_PLACEHOLDER, resetRowAvatars } from "./glanceResetRows";
import { bankedResetFor, bankedResetLines } from "./insights/bankedResetLines";
import { parseClaudeResets } from "./insights/claudeResets";
import { freeResetLines } from "./insights/freeResetLines";
import { parseResetStatus } from "./insights/resets";
import type { ResetTiming, UpcomingReset } from "./insights/upcomingReset";
import { setSystemTimeZone } from "./timeZone";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
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

/**
 * A row as the island and the widgets fill it at `now` (`GlanceResetRow.lines` in
 * GlanceAccountRows.swift): the countdown's words with the time left, or its `after` once passed,
 * and `{at}` as the moment's clock time and day.
 */
function fill(row: GlanceResetRow, now: Date, timeFormat: TimeFormat, language: Language) {
  const countdown = row.countdown;
  const awaiting = countdown !== undefined && Date.parse(countdown.at) <= now.getTime();
  const value = countdown
    ? awaiting
      ? (countdown.after ?? countdown.text.replace("{d}", "0"))
      : countdown.text.replace("{d}", compactDuration((Date.parse(countdown.at) - now.getTime()) / 1000, language)!)
    : (row.value ?? "");
  const moment = row.at ? timeOnDayLabel(new Date(row.at), now, timeFormat, language) : "";
  const caption = ((awaiting ? row.captionAfter : undefined) ?? row.caption).replace(MOMENT_PLACEHOLDER, moment);
  return { value, caption, note: row.note ?? null, awaiting };
}

beforeEach(() => setSystemTimeZone("Asia/Saigon"));
afterEach(() => setSystemTimeZone(null));

describe("the free-reset row a Codex account starts with", () => {
  const timings: Record<string, UpcomingReset> = {
    exact: reset({ kind: "exact", at: new Date("2026-09-28T01:00:00Z"), from: "site" }),
    soon: reset({ kind: "exact", at: new Date("2026-09-27T16:30:00Z"), from: "post" }),
    day: reset({ kind: "day", at: new Date("2026-09-27T21:41:35Z"), day: { kind: "tomorrow" } }),
    watch: reset({ kind: "by", at: new Date("2026-09-28T10:00:00Z") }, { origin: "watch", kind: null, chancePercent: 65 }),
    window: reset({ kind: "window", window: "nextWeek", ends: new Date("2026-10-05T07:00:00Z") }),
    untimed: reset({ kind: "unknown" }),
    banked: reset({ kind: "exact", at: new Date("2026-09-28T01:00:00Z"), from: "site" }, { kind: "banked" }),
  };

  it("reads as the popup's row reads at every moment until it goes, in either language and clock", () => {
    for (const [name, next] of Object.entries(timings)) {
      for (const language of ["vi", "en"] as const) {
        for (const timeFormat of ["auto", "12h", "24h"] as const) {
          const row = freeResetRow(next, NOW, timeFormat, language, true);
          const popup = freeResetLines(next, NOW, timeFormat, language);
          expect(row.title, name).toBe(popup.title);
          for (const offset of [0, 30 * MINUTE, 5 * HOUR, 17 * HOUR + 1000, 26 * HOUR, 30 * HOUR, 2 * DAY]) {
            const now = new Date(NOW.getTime() + offset);
            if (now.getTime() >= Date.parse(row.hideAt)) continue;
            const lines = freeResetLines(next, now, timeFormat, language);
            expect(fill(row, now, timeFormat, language), `${name} ${language} ${timeFormat} +${offset / MINUTE}m`).toEqual({
              value: lines.value,
              caption: lines.caption,
              note: lines.note,
              awaiting: lines.awaiting,
            });
          }
        }
      }
    }
  });

  it("goes when the popup's row goes: a day after its time, at a watch's deadline, at a window's end, a week after an untimed post", () => {
    expect(freeResetRow(timings.exact!, NOW, "auto", "vi", false).hideAt).toBe("2026-09-29T01:00:00.000Z");
    expect(freeResetRow(timings.day!, NOW, "auto", "vi", false).hideAt).toBe("2026-09-28T21:41:35.000Z");
    expect(freeResetRow(timings.watch!, NOW, "auto", "vi", false).hideAt).toBe("2026-09-28T10:00:00.000Z");
    expect(freeResetRow(timings.window!, NOW, "auto", "vi", false).hideAt).toBe("2026-10-05T07:00:00.000Z");
    expect(freeResetRow(timings.untimed!, NOW, "auto", "vi", false).hideAt).toBe("2026-10-03T21:41:35.000Z");
  });

  it("colors an announced reset positive and a watch as a notice, with @thsottiaux's picture", () => {
    expect(freeResetRow(timings.exact!, NOW, "auto", "vi", false)).toMatchObject({ tracker: "codex", tone: "positive", author: "@thsottiaux", title: "Reset free" });
    expect(freeResetRow(timings.watch!, NOW, "auto", "vi", false)).toMatchObject({ tone: "notice", title: "Có thể reset" });
    expect(freeResetRow(timings.banked!, NOW, "auto", "vi", false).title).toBe("Tặng lượt để dành");
  });

  it("keeps the popup's hover words, and opens the Reset tab only while that tab is on", () => {
    const popup = freeResetLines(timings.exact!, NOW, "auto", "vi");
    const open = freeResetRow(timings.exact!, NOW, "auto", "vi", true);
    expect(open.opens).toBe(true);
    expect(open.details).toBe(`${popup.details}\nBấm để mở tab Reset.`);
    const closed = freeResetRow(timings.exact!, NOW, "auto", "vi", false);
    expect("opens" in closed).toBe(false);
    expect(closed.details).toBe(popup.details);
  });

  it("follows codex-resets.com's status: the fixture's announcement has no time, and nothing shows once it is a week old", () => {
    const status = parseResetStatus(FEED_FIXTURES.codexResetStatus);
    const announced = new Date("2026-09-26T00:07:13Z");
    expect(codexResetRow(status, new Date(announced.getTime() + HOUR), "auto", "vi", true)).toMatchObject({
      tracker: "codex",
      title: "Reset free",
      value: "chưa rõ giờ",
      caption: "Bài đăng chưa nói khi nào",
      hideAt: "2026-10-03T00:07:13.000Z",
      opens: true,
    });
    expect(codexResetRow(status, new Date(announced.getTime() + 7 * DAY), "auto", "vi", true)).toBeNull();
    expect(codexResetRow(null, NOW, "auto", "vi", true)).toBeNull();
  });
});

describe("the banked-reset row a Claude account starts with", () => {
  const feed = parseClaudeResets(FEED_FIXTURES.claudeResets)!;
  const BANKED = "2102438800836489554";
  const AT = new Date("2026-09-29T13:00:00Z");

  it("reads as the popup's row reads for the card's plan, at every moment until its deadline", () => {
    for (const language of ["vi", "en"] as const) {
      for (const timeFormat of ["auto", "12h", "24h"] as const) {
        const row = claudeResetRow(feed.resets, "Max 5x", [], AT, timeFormat, language, true)!;
        const banked = bankedResetFor(feed.resets, "Max 5x", [], AT)!;
        for (const now of [AT, new Date("2026-10-21T23:30:00Z"), new Date("2026-10-22T17:30:00Z"), new Date("2026-10-22T23:50:00Z")]) {
          const popup = bankedResetLines(banked, now, timeFormat, language)!;
          expect(fill(row, now, timeFormat, language), `${language} ${timeFormat} ${now.toISOString()}`).toEqual({ value: popup.value, caption: popup.caption, note: null, awaiting: false });
          expect(row.title).toBe(popup.title);
        }
      }
    }
  });

  it("counts down to the banked reset's deadline in the accent color, with the poster's picture", () => {
    expect(claudeResetRow(feed.resets, "Pro", [], AT, "auto", "vi", false)).toEqual({
      tracker: "claude",
      title: "Lượt reset để dành",
      tone: "accent",
      author: "@ClaudeDevs",
      countdown: { at: "2026-10-22T23:59:59.000Z", text: "còn {d}" },
      caption: "Dùng trước {at} · GMT+7",
      at: "2026-10-22T23:59:59.000Z",
      details: bankedResetLines(bankedResetFor(feed.resets, "Pro", [], AT)!, AT, "auto", "vi")!.details,
      hideAt: "2026-10-22T23:59:59.000Z",
    });
  });

  it("is left out for a plan the reset leaves out, once marked as used, and after its deadline", () => {
    expect(claudeResetRow(feed.resets, "Free", [], AT, "auto", "vi", true)).toBeNull();
    expect(claudeResetRow(feed.resets, "Max 5x", [BANKED], AT, "auto", "vi", true)).toBeNull();
    expect(claudeResetRow(feed.resets, "Max 5x", [], new Date("2026-10-23T00:00:00Z"), "auto", "vi", true)).toBeNull();
    expect(claudeResetRow(feed.resets, undefined, [], AT, "auto", "vi", true)).not.toBeNull();
  });
});

describe("the pictures a document carries for its rows", () => {
  const row = (author: string): GlanceResetRow => ({ tracker: "claude", title: "", tone: "accent", author, caption: "", details: "", hideAt: "" });
  const entry = (author?: string) => ({ id: "x", name: "", brand: "", color: "", metrics: [], ...(author ? { resetRow: row(author) } : {}) });

  it("sends each named account's picture once, and none for an account without one", () => {
    const avatars = resetRowAvatars([entry("@thsottiaux"), entry("@ClaudeDevs"), entry("@ClaudeDevs"), entry("@lydiahallie"), entry()])!;
    expect(Object.keys(avatars)).toEqual(["@thsottiaux", "@claudedevs"]);
    expect(avatars["@thsottiaux"]).toMatch(/^data:image\//);
    expect(resetRowAvatars([entry("@lydiahallie"), entry()])).toBeNull();
  });
});
