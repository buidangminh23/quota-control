import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { insightsFor } from "@/i18n/insights";
import { FEED_FIXTURES } from "@/lib/insightsFeedFixtures";
import { timeOnDayLabel } from "./format";
import { buildClaudeGlanceResets, CLAUDE_RESETS_SITE, pendingBanked, type ClaudeGlanceResetsInput } from "./glanceClaudeResets";
import { parseClaudeResets, type ClaudeResetFeed } from "./insights/claudeResets";
import { SOURCE_COLORS } from "./palette";
import { offsetLabel, setSystemTimeZone } from "./timeZone";

/** Tuesday 29/09/2026 20:00 in Vietnam. */
const NOW = new Date("2026-09-29T13:00:00Z");
const BANKED = "2102438800836489554";
const DEADLINE = "2026-10-22T23:59:59.000Z";
const HOUR = 3_600_000;

function event(overrides: Record<string, unknown>): Record<string, unknown> {
  return { id: "1", date: "2026-09-20T10:00:00Z", kind: "reset", scope: "all", note: "Reset for all.", url: "https://x.com/ClaudeDevs/status/1", verification: "curated", ...overrides };
}

function feedOf(events: Record<string, unknown>[]): ClaudeResetFeed {
  return parseClaudeResets(JSON.stringify({ account: "ClaudeDevs", events, live: true, detector: "fresh", provisionalEventIds: [], provisionalPolicyIds: [] }))!;
}

const FIXTURE = parseClaudeResets(FEED_FIXTURES.claudeResets)!;

function build(feed: ClaudeResetFeed = FIXTURE, overrides: Partial<ClaudeGlanceResetsInput> = {}) {
  return buildClaudeGlanceResets({ feed, accounts: ["max"], used: [], stale: false, now: NOW, language: "vi", timeFormat: "24h", ...overrides });
}

beforeEach(() => setSystemTimeZone("Asia/Saigon"));
afterEach(() => setSystemTimeZone(null));

describe("buildClaudeGlanceResets", () => {
  it("is absent until the feed records a reset: limit changes alone are none", () => {
    expect(build(feedOf([]))).toBeNull();
    expect(build(feedOf([event({ id: "p1", kind: "policy", scope: "paid plans", note: "Raised the weekly limits." })]))).toBeNull();
    expect(build(feedOf([event({ id: "r1" })]))).not.toBeNull();
  });

  it("words the tracker like the Codex one, with the Claude mark, color and site", () => {
    const resets = build()!;
    expect(resets).toMatchObject({ title: "Reset Claude", source: "Theo claude-resets.com", brand: "claude", color: SOURCE_COLORS.claude, site: CLAUDE_RESETS_SITE });
    expect(resets.color).toBe("#DE7356");
    expect(resets.mark?.paths.length).toBeGreaterThan(0);
    expect(resets.mark?.art).toBeUndefined();
    expect(resets.stale).toBeUndefined();
    expect(build(FIXTURE, { language: "en" })).toMatchObject({ title: "Claude Resets", source: "Via claude-resets.com" });
    expect(build(FIXTURE, { theme: "dark" })!.theme).toBe("dark");
    expect(build(FIXTURE, { stale: true })!.stale).toBe(insightsFor("vi").staleNote);
  });

  it("never says Codex, in either language", () => {
    for (const language of ["vi", "en"] as const) {
      expect(JSON.stringify(build(FIXTURE, { language }))).not.toMatch(/codex/i);
      expect(JSON.stringify(build(FIXTURE, { language, used: [BANKED] }))).not.toMatch(/codex/i);
    }
  });

  it("quotes the latest announcement in its card and keeps the banked card's words for a widget that hides it", () => {
    const presentation = build()!.presentation!;
    expect(presentation.latest?.excerpt).toBeTruthy();
    expect(presentation.latest?.url).toBe(`https://x.com/ClaudeDevs/status/${BANKED}`);
    const card = presentation.statuses.find((status) => status.id === `banked:${BANKED}`)!;
    expect(card).toMatchObject({ sameAsLatest: true, excerpt: presentation.latest!.excerpt, author: { handle: "@ClaudeDevs" } });
  });

  it("counts down to the banked reset still to apply, the way a Claude card reads it", () => {
    const upcoming = build()!.upcoming!;
    expect(upcoming).toEqual({
      title: "Lượt reset để dành",
      tone: "positive",
      countdown: { at: DEADLINE, text: "còn {d}" },
      caption: "Dùng trước 6:59 · T6 23/10 · GMT+7",
      hideAt: DEADLINE,
    });
    expect(build(FIXTURE, { language: "en" })!.upcoming).toMatchObject({ title: "Banked reset", countdown: { at: DEADLINE, text: "{d} left" } });
    expect(build(FIXTURE, { now: new Date("2026-10-22T20:00:00Z") })!.upcoming!.caption).toBe("Dùng trước 6:59 · T6 23/10 · GMT+7");
  });

  it("names the offset in force at the deadline, not today's", () => {
    setSystemTimeZone("Europe/London");
    const until = new Date("2026-11-30T00:00:00Z");
    const feed = feedOf([event({ id: "b1", date: "2026-09-25T10:00:00Z", resetType: "banked", usableUntil: until.toISOString() })]);
    expect(offsetLabel(until, "Europe/London")).not.toBe(offsetLabel(NOW, "Europe/London"));
    const upcoming = build(feed, { accounts: [], language: "en" })!.upcoming!;
    expect(upcoming.caption).toBe(`Use before ${timeOnDayLabel(until, NOW, "24h", "en", false)} · GMT`);
  });

  it("drops the banked reset once it is marked as used, past its deadline, or for accounts it leaves out", () => {
    expect(build(FIXTURE, { used: [BANKED] })!.upcoming).toBeUndefined();
    expect(build(FIXTURE, { now: new Date(Date.parse(DEADLINE) + 1_000) })!.upcoming).toBeUndefined();
    expect(build(FIXTURE, { accounts: ["free"] })!.upcoming).toBeUndefined();
    expect(build(FIXTURE, { accounts: ["free", "free"] })!.upcoming).toBeUndefined();
    expect(build(FIXTURE, { accounts: ["free", "team"] })!.upcoming?.hideAt).toBe(DEADLINE);
    expect(build(FIXTURE, { accounts: ["free", null] })!.upcoming?.hideAt).toBe(DEADLINE);
    expect(build(FIXTURE, { accounts: [] })!.upcoming?.hideAt).toBe(DEADLINE);
    expect(build(FIXTURE, { used: ["2095967323412930677"] })!.upcoming?.hideAt).toBe(DEADLINE);
  });

  it("follows the soonest banked reset still to apply", () => {
    const feed = feedOf([
      event({ id: "late", date: "2026-09-25T10:00:00Z", resetType: "banked", usableUntil: "2026-11-30T00:00:00Z", scope: "all" }),
      event({ id: "soon", date: "2026-09-24T10:00:00Z", resetType: "banked", usableUntil: "2026-10-05T00:00:00Z", scope: "Max" }),
      event({ id: "gone", date: "2026-09-01T10:00:00Z", resetType: "banked", usableUntil: "2026-09-10T00:00:00Z", scope: "all" }),
      event({ id: "plain", date: "2026-09-20T10:00:00Z" }),
    ]);
    expect(pendingBanked(feed.resets, ["max"], [], NOW).map((reset) => reset.id)).toEqual(["soon", "late"]);
    expect(build(feed)!.upcoming!.hideAt).toBe("2026-10-05T00:00:00.000Z");
    expect(build(feed, { used: ["soon"] })!.upcoming!.hideAt).toBe("2026-11-30T00:00:00.000Z");
    expect(build(feed, { accounts: ["pro"] })!.upcoming!.hideAt).toBe("2026-11-30T00:00:00.000Z");
    expect(build(feed, { used: ["soon", "late"] })!.upcoming).toBeUndefined();
    const statuses = build(feed)!.presentation!.statuses;
    expect(statuses.map((card) => card.id)).toEqual(["banked:soon", "banked:late"]);
  });

  it("never counts a limit change: the history reads as if there were none", () => {
    const resets = [event({ id: "r1", date: "2026-09-20T10:00:00Z" }), event({ id: "r2", date: "2026-09-10T10:00:00Z" }), event({ id: "r3", date: "2026-08-28T10:00:00Z" }), event({ id: "r4", date: "2026-08-15T10:00:00Z" })];
    const changes = [event({ id: "p1", kind: "policy", date: "2026-09-27T10:00:00Z", note: "Raised the limits." }), event({ id: "p2", kind: "policy", date: "2026-09-05T10:00:00Z", note: "Lowered the limits." })];
    const withChanges = build(feedOf([...changes, ...resets]))!;
    const without = build(feedOf(resets))!;
    for (const key of ["latest", "forecast", "forecastNote", "wait", "median", "calendar", "rhythm", "upcoming"] as const) {
      expect(withChanges[key], key).toEqual(without[key]);
    }
    expect(withChanges.latest!.at).toBe("2026-09-20T10:00:00.000Z");
    expect(withChanges.rhythm!.total).toBe(4);
    expect(withChanges.calendar!.cells.replace(/[^rb]/g, "")).toHaveLength(4);
    expect(withChanges.presentation!.history.map((item) => item.id)).toEqual(["r1", "r2", "r3", "r4"]);
  });

  it("reads the history the way the Codex tracker does", () => {
    const resets = build()!;
    const text = insightsFor("vi");
    expect(resets.latest).toMatchObject({ at: "2026-09-22T16:44:06.000Z", kind: "banked", label: text.latestTitle, kindLabel: text.kind("banked") });
    expect(resets.latest!.since).toEqual({ at: "2026-09-22T16:44:06.000Z", text: text.glanceSinceLast("{d}"), since: true });
    expect(resets.forecastTitle).toBe(text.glanceChanceTitle);
    expect(resets.forecast.map((chance) => chance.days)).toEqual([1, 3, 7]);
    expect(resets.forecast.every((chance) => Number.isInteger(chance.percent) && chance.percent >= 0 && chance.percent <= 100)).toBe(true);
    expect(resets.forecastNote).toBe(text.glanceForecastNote);
    expect(resets.wait).toBeDefined();
    expect(resets.median).toBeDefined();
    expect(resets.calendar!.cells).toHaveLength(resets.calendar!.weeks * 7);
    expect(resets.rhythm!.total).toBe(FIXTURE.resets.length);
  });

  it("draws the banked reset as a status card whose time left moves by itself", () => {
    const statuses = build()!.presentation!.statuses;
    expect(statuses).toHaveLength(1);
    const card = statuses[0]!;
    const text = insightsFor("vi").claude;
    expect(card).toMatchObject({ id: `banked:${BANKED}`, kind: "banked", title: text.bankedTitle, hideAt: DEADLINE, author: { handle: "@ClaudeDevs" } });
    expect(card.dueCountdown).toEqual({ at: DEADLINE, text: "Còn {d}" });
    expect(card.meta.at(-1)).toBe(text.glanceBankedHow);
    expect(card.meta).toContain("Gói Max của bạn: có áp dụng");
    expect(card.meta.some((line) => line.startsWith("Dùng được đến "))).toBe(true);
    for (const key of ["due", "used", "resetId", "how"]) expect(key in card, key).toBe(false);
    const later = build(FIXTURE, { now: new Date(NOW.getTime() + 5 * HOUR) })!.presentation!.statuses;
    expect(later).toEqual(statuses);
  });

  it("shows the quiet card instead once no banked reset is left to apply", () => {
    const presentation = build(FIXTURE, { used: [BANKED] })!.presentation!;
    expect(presentation.statuses).toEqual([{ id: "quiet", kind: "quiet", title: insightsFor("vi").quietTitle, meta: [] }]);
    expect(presentation.quietTitle).toBe(insightsFor("vi").quietTitle);
  });

  it("keeps each chance's meter to the whole percent it shows, so the document holds still minute to minute", () => {
    const chances = build()!.presentation!.forecast.chances;
    expect(chances).toHaveLength(3);
    for (const chance of chances) {
      expect(Math.round(chance.fraction * 100)).toBeCloseTo(chance.fraction * 100, 9);
      expect(chance.percent).toBe(`${Math.round(chance.fraction * 100)}%`);
    }
    const later = build(FIXTURE, { now: new Date(NOW.getTime() + 60_000) })!.presentation!.forecast;
    expect(later).toEqual(build()!.presentation!.forecast);
  });

  it("carries the Claude view cut down to what the island and the widgets draw", () => {
    const presentation = build()!.presentation!;
    const text = insightsFor("vi");
    for (const key of ["notices", "banked", "changes", "changesTitle", "changesNote", "compare"]) expect(key in presentation, key).toBe(false);
    expect("reliability" in presentation.forecast).toBe(false);
    expect(presentation.forecast.chances).toHaveLength(3);
    expect(presentation.method).toEqual(text.claude.glanceMethod);
    expect(presentation.source).toBe(text.claude.source);
    expect(presentation.authorAvatar).toMatch(/^data:image\//);
    expect(presentation.avatarHandle).toBe("@ClaudeDevs");
    expect(presentation.latest).toMatchObject({ at: "2026-09-22T16:44:06.000Z", author: { handle: "@ClaudeDevs" } });
    expect(presentation.history).toHaveLength(FIXTURE.resets.length);
    expect(presentation.history.find((item) => item.author?.handle === "@lydiahallie")).toMatchObject({ scope: "Gói Max" });
    expect(presentation.stats.length).toBeGreaterThan(0);
  });

  it("keeps the method's shared paragraphs word for word from the Reset tab's Claude view", () => {
    for (const language of ["vi", "en"] as const) {
      const claude = insightsFor(language).claude;
      expect(claude.glanceMethod.slice(0, 3)).toEqual(claude.method.slice(0, 3));
      expect(claude.glanceMethod.join(" ")).not.toMatch(/codex/i);
    }
  });
});
