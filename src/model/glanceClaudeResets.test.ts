import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { messagesFor } from "@/i18n";
import { insightsFor } from "@/i18n/insights";
import { FEED_FIXTURES } from "@/lib/insightsFeedFixtures";
import { timeOnDayLabel } from "./format";
import { buildClaudeGlanceResets, CLAUDE_RESETS_SITE, pendingBanked, type ClaudeGlanceResetsInput } from "./glanceClaudeResets";
import { buildClaudePresentation } from "./insights/claudePresentation";
import { parseClaudeResets, type ClaudeResetFeed } from "./insights/claudeResets";
import { parseResets } from "./insights/resets";
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
/** The Codex history as the Reset tab reads it for its comparison: the list alone. */
const CODEX = parseResets(FEED_FIXTURES.codexResets);

function build(feed: ClaudeResetFeed = FIXTURE, overrides: Partial<ClaudeGlanceResetsInput> = {}) {
  return buildClaudeGlanceResets({ feed, accounts: ["max"], used: [], stale: false, now: NOW, language: "vi", timeFormat: "24h", ...overrides });
}

beforeEach(() => setSystemTimeZone("Asia/Saigon"));
afterEach(() => setSystemTimeZone(null));

describe("buildClaudeGlanceResets", () => {
  it("keeps a feed without resets as a tracker with no reset card, as the Reset tab keeps its Claude view", () => {
    for (const feed of [feedOf([]), feedOf([event({ id: "p1", kind: "policy", scope: "paid plans", note: "Raised the weekly limits." })])]) {
      const resets = build(feed);
      expect(resets).toMatchObject({ title: "Reset Claude", brand: "claude", forecast: [] });
      for (const key of ["latest", "upcoming", "calendar", "rhythm", "wait", "median"] as const) expect(resets[key], key).toBeUndefined();
      expect(resets.presentation).toMatchObject({ statuses: [], stats: [], history: [], forecast: { chances: [] } });
      expect(resets.presentation?.latest).toBeUndefined();
      expect(resets.presentation?.method).toEqual(insightsFor("vi").claude.method);
      expect(resets.presentation?.changes ?? []).toHaveLength(feed.changes.length);
    }
    expect(build(feedOf([event({ id: "r1" })])).latest?.at).toBe("2026-09-20T10:00:00.000Z");
  });

  it("says above the source when the feed was read, like the Reset tab's Claude view", () => {
    const presentation = build(FIXTURE, { fetchedAt: "2026-09-29T12:57:00Z" }).presentation!;
    expect(presentation.fetched).toEqual({ at: "2026-09-29T12:57:00.000Z", text: "Tải {d} trước", since: true, recent: "Vừa tải" });
    expect(Object.keys(presentation).indexOf("fetched")).toBe(Object.keys(presentation).indexOf("source") - 1);
    expect("fetched" in build(FIXTURE).presentation!).toBe(false);
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

  it("says Codex only where the Reset tab's method and its comparison do, in either language", () => {
    for (const language of ["vi", "en"] as const) {
      for (const used of [[], [BANKED]]) {
        for (const codex of [[], CODEX]) {
          const resets = build(FIXTURE, { language, used, codex })!;
          expect(resets.presentation!.method).toEqual(insightsFor(language).claude.method);
          const { method: _method, compare, ...rest } = resets.presentation!;
          expect(compare === undefined, `${language} ${codex.length}`).toBe(codex.length === 0);
          expect(JSON.stringify({ ...resets, presentation: rest })).not.toMatch(/codex/i);
        }
      }
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
    expect(card).toMatchObject({ resetId: BANKED, how: text.bankedHow });
    expect(card.meta).toContain("Gói Max của bạn: có áp dụng");
    expect(card.meta.some((line) => line.startsWith("Dùng được đến "))).toBe(true);
    expect(card.meta).not.toContain(text.bankedHow);
    for (const key of ["due", "used"]) expect(key in card, key).toBe(false);
    const later = build(FIXTURE, { now: new Date(NOW.getTime() + 5 * HOUR) })!.presentation!.statuses;
    expect(later).toEqual(statuses);
  });

  it("keeps a banked card marked as used, folded, above the quiet card, as the Reset tab lists them", () => {
    const presentation = build(FIXTURE, { used: [BANKED] })!.presentation!;
    const popup = buildClaudePresentation({ feed: FIXTURE, codex: [], plans: ["max"], accounts: ["max"], used: [BANKED], now: NOW, language: "vi", timeFormat: "24h" });
    expect(presentation.statuses.map((card) => card.id)).toEqual([...popup.banked, ...popup.statuses].map((card) => card.id));
    expect(presentation.statuses.map((card) => card.id)).toEqual([`banked:${BANKED}`, "quiet"]);
    expect(presentation.statuses[0]).toMatchObject({ kind: "banked", resetId: BANKED, used: true, hideAt: DEADLINE, how: insightsFor("vi").claude.bankedHow });
    expect(presentation.statuses[1]).toEqual({ id: "quiet", kind: "quiet", title: insightsFor("vi").quietTitle, meta: [] });
    expect(presentation.quietTitle).toBe(insightsFor("vi").quietTitle);
    const open = build()!.presentation!;
    const { used: _used, ...folded } = presentation.statuses[0]!;
    expect(folded).toEqual(open.statuses[0]);
  });

  it("words the banked cards' buttons and their confirmation as the Reset tab's BankedCards does", () => {
    for (const language of ["vi", "en"] as const) {
      const text = insightsFor(language).claude;
      const words = { markUsed: text.bankedMarkUsed, title: text.bankedConfirmTitle, message: text.bankedConfirmMessage, confirm: text.bankedConfirm, cancel: messagesFor(language).chrome.cancel, used: text.bankedUsed, undo: text.bankedUndo };
      expect(build(FIXTURE, { language })!.presentation!.bankedActions).toEqual(words);
      expect(build(FIXTURE, { language, used: [BANKED] })!.presentation!.bankedActions).toEqual(words);
    }
    expect(build(FIXTURE, { now: new Date(Date.parse(DEADLINE) + 1_000) })!.presentation!).not.toHaveProperty("bankedActions");
    expect(build(FIXTURE, { accounts: ["free"] })!.presentation!).not.toHaveProperty("bankedActions");
    expect(build(feedOf([event({ id: "r1" })])).presentation!).not.toHaveProperty("bankedActions");
  });

  it("leaves the buttons off a banked reset whose id a press could not carry, and keeps the words that send the user to the Reset tab", () => {
    const feed = feedOf([event({ id: "not an id", date: "2026-09-25T10:00:00Z", resetType: "banked", usableUntil: "2026-11-30T00:00:00Z" })]);
    const presentation = build(feed)!.presentation!;
    const card = presentation.statuses[0]!;
    expect(card).toMatchObject({ id: "banked:not an id", kind: "banked", how: insightsFor("vi").claude.glanceBankedHow });
    expect("resetId" in card).toBe(false);
    expect(presentation.bankedActions?.used).toBe(insightsFor("vi").claude.bankedUsed);
    expect(build(feed)!.upcoming?.hideAt).toBe("2026-11-30T00:00:00.000Z");
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
    for (const key of ["notices", "banked", "compare"]) expect(key in presentation, key).toBe(false);
    expect(presentation.forecast.chances).toHaveLength(3);
    expect(presentation.method).toEqual(text.claude.method);
    expect(presentation.source).toBe(text.claude.source);
    expect(presentation.authorAvatar).toMatch(/^data:image\//);
    expect(presentation.avatarHandle).toBe("@ClaudeDevs");
    expect(presentation.latest).toMatchObject({ at: "2026-09-22T16:44:06.000Z", author: { handle: "@ClaudeDevs" } });
    expect(presentation.history).toHaveLength(FIXTURE.resets.length);
    expect(presentation.history.find((item) => item.author?.handle === "@lydiahallie")).toMatchObject({ scope: "Gói Max" });
    expect(presentation.stats.length).toBeGreaterThan(0);
  });

  it("carries the Reset tab's self-check under the chances, the same all day", () => {
    const popup = buildClaudePresentation({ feed: FIXTURE, codex: [], plans: ["max"], accounts: ["max"], used: [], now: NOW, language: "vi", timeFormat: "24h" });
    const forecast = build()!.presentation!.forecast;
    expect(forecast.reliability).toMatch(/^Thử lại trên \d+ ngày đã qua \(\d+ lần reset\)/);
    expect(forecast.reliability).toBe(popup.forecast.reliability);
    const morning = new Date("2026-09-29T01:00:00Z");
    const night = new Date("2026-09-29T16:30:00Z");
    expect(build(FIXTURE, { now: morning })!.presentation!.forecast.reliability).toBe(build(FIXTURE, { now: night })!.presentation!.forecast.reliability);
  });

  it("lists the limit changes apart from the history, as the Reset tab's Claude view does", () => {
    const popup = buildClaudePresentation({ feed: FIXTURE, codex: [], plans: ["max"], accounts: ["max"], used: [], now: NOW, language: "vi", timeFormat: "24h" });
    const presentation = build()!.presentation!;
    const text = insightsFor("vi").claude;
    expect(presentation.changes).toHaveLength(6);
    expect(presentation.changes).toEqual(popup.changes);
    expect(presentation).toMatchObject({ changesTitle: "Thay đổi hạn mức", changeBadge: "Hạn mức", changesNote: text.changesNote });
    expect(presentation.changes![0]).toMatchObject({ scope: "Các gói trả phí", author: { handle: "@ClaudeDevs" } });
    const keys = Object.keys(presentation);
    expect(keys.indexOf("changes")).toBeGreaterThan(keys.indexOf("history"));
    expect(presentation.history.map((item) => item.id)).not.toContain(presentation.changes![0]!.id);
    const english = build(FIXTURE, { language: "en" }).presentation!;
    expect(english).toMatchObject({ changesTitle: "Limit changes", changeBadge: "Limits" });
    for (const key of ["changes", "changesTitle", "changeBadge", "changesNote"]) expect(key in build(feedOf([event({ id: "r1" })])).presentation!, key).toBe(false);
  });

  it("says above the cards when the site is behind or only its published copy could be read", () => {
    const text = insightsFor("vi").claude;
    const body = (extra: Record<string, unknown>) => parseClaudeResets(JSON.stringify({ ...JSON.parse(FEED_FIXTURES.claudeResets), ...extra }))!;
    expect("notices" in build(body({ live: true, detector: "fresh" })).presentation!).toBe(false);
    expect(build(body({ live: true, detector: "stale" })).presentation!.notices).toEqual([text.detectorBehind]);
    expect(build(body({ live: false, detector: null })).presentation!.notices).toEqual([text.datasetNote]);
    const popup = buildClaudePresentation({ feed: body({ live: false, detector: null }), codex: [], plans: ["max"], accounts: ["max"], used: [], now: NOW, language: "vi", timeFormat: "24h" });
    expect(build(body({ live: false, detector: null })).presentation!.notices).toEqual(popup.notices);
  });

  it("sets Claude against Codex once the Codex history is at hand, each column named and marked as the Reset tab heads it", () => {
    const popup = buildClaudePresentation({ feed: FIXTURE, codex: CODEX, plans: ["max"], accounts: ["max"], used: [], now: NOW, language: "vi", timeFormat: "24h" });
    const compare = build(FIXTURE, { codex: CODEX }).presentation!.compare!;
    const { columns, ...rest } = compare;
    expect(rest).toEqual(popup.compare);
    expect(columns).toMatchObject({ claude: { name: "Claude", color: SOURCE_COLORS.claude }, codex: { name: "Codex", color: SOURCE_COLORS.codex } });
    expect(columns!.claude.mark?.paths.length).toBeGreaterThan(0);
    expect(columns!.codex.mark?.paths.length).toBeGreaterThan(0);
    expect(compare.rows[0]).toMatchObject({ label: "Số lần reset" });
    expect(compare.months.length).toBeGreaterThan(0);
    const keys = Object.keys(build(FIXTURE, { codex: CODEX }).presentation!);
    expect(keys.indexOf("compare")).toBeGreaterThan(keys.indexOf("changes"));
    expect("compare" in build(FIXTURE, { codex: [] }).presentation!).toBe(false);
  });
});
