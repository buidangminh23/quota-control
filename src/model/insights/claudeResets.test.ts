import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { FEED_FIXTURES } from "@/lib/insightsFeedFixtures";
import { setSystemTimeZone } from "@/model/timeZone";
import { bankedResetFor, bankedResetLines } from "./bankedResetLines";
import { buildClaudePresentation, scopeText } from "./claudePresentation";
import { insightsFor } from "@/i18n/insights";
import {
  compareTrackers,
  covers,
  detectorBehind,
  forecastSkill,
  latestFor,
  latestForEveryone,
  openBanked,
  parseClaudeResets,
  planFamily,
  readScope,
  type ClaudeReset,
} from "./claudeResets";
import type { CodexReset } from "./resets";

/** Tuesday 29/09/2026 20:00 in Vietnam. */
const NOW = new Date("2026-09-29T13:00:00Z");
const BANKED = "2102438800836489554";

function event(overrides: Record<string, unknown>): Record<string, unknown> {
  return { id: "1", date: "2026-09-20T10:00:00Z", kind: "reset", scope: "all", note: "Reset for all.", url: "https://x.com/ClaudeDevs/status/1", verification: "curated", ...overrides };
}

function body(events: Record<string, unknown>[], overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({ account: "ClaudeDevs", product: "Claude Code", events, live: true, detector: "fresh", provisionalEventIds: [], provisionalPolicyIds: [], ...overrides });
}

function plain(id: string, date: string, kind: CodexReset["kind"] = "regular"): CodexReset {
  return { id, kind, announcedAt: new Date(date), text: "", source: { kind: "x_post", url: null } };
}

beforeEach(() => setSystemTimeZone("Asia/Saigon"));
afterEach(() => setSystemTimeZone(null));

describe("parseClaudeResets", () => {
  it("reads the catalog the core stored: resets apart from limit changes, newest first", () => {
    const feed = parseClaudeResets(FEED_FIXTURES.claudeResets)!;
    expect(feed.live).toBe(true);
    expect(feed.detector).toBe("fresh");
    expect(feed.resets).toHaveLength(14);
    expect(feed.changes).toHaveLength(6);
    expect(feed.resets[0]).toMatchObject({ id: BANKED, kind: "banked", account: "ClaudeDevs", scope: "Pro, Max + Team", provisional: false });
    expect(feed.resets[0]!.usableUntil?.toISOString()).toBe("2026-10-22T23:59:59.000Z");
    expect(feed.resets[1]).toMatchObject({ account: "lydiahallie", scope: "Max", kind: "regular" });
    expect(feed.resets[1]!.source.url).toBe("https://x.com/lydiahallie/status/2095967323412930677");
    expect(feed.resets.every((reset, index, rows) => index === 0 || rows[index - 1]!.announcedAt >= reset.announcedAt)).toBe(true);
    expect(feed.changes.every((change) => change.text.length > 0)).toBe(true);
  });

  it("marks what the site has not reviewed, keeps only links to X and drops what it cannot read", () => {
    const feed = parseClaudeResets(
      body(
        [
          event({ id: "1", verification: "provisional" }),
          event({ id: "2", url: "https://evil.example/claude" }),
          event({ id: "3", account: "not a handle!" }),
          event({ id: "4", kind: "policy" }),
          event({ id: "5", kind: "rumour" }),
          event({ id: "6", date: "soon" }),
          event({ id: "", date: "2026-09-01T00:00:00Z" }),
          event({ id: "1", note: "A second row under the same id." }),
          "not an event" as unknown as Record<string, unknown>,
        ],
        { provisionalPolicyIds: ["4"] },
      ),
    )!;
    expect(feed.resets.map((reset) => reset.id).sort()).toEqual(["1", "2", "3"]);
    expect(feed.resets.find((reset) => reset.id === "1")).toMatchObject({ provisional: true, text: "Reset for all." });
    expect(feed.resets.find((reset) => reset.id === "2")!.source.url).toBeNull();
    expect(feed.resets.find((reset) => reset.id === "3")!.account).toBe("ClaudeDevs");
    expect(feed.changes).toEqual([expect.objectContaining({ id: "4", provisional: true })]);
  });

  it("answers null for anything that is not the stored catalog", () => {
    for (const bad of [null, "", "<html>", "{}", '{"events":"none"}', '{"providers":{"claude":{"events":[]}}}']) expect(parseClaudeResets(bad)).toBeNull();
  });

  it("reports a detector that is behind, but not an unknown one or the published copy", () => {
    expect(detectorBehind(parseClaudeResets(body([event({})], { detector: "stale" })))).toBe(true);
    expect(detectorBehind(parseClaudeResets(body([event({})])))).toBe(false);
    expect(detectorBehind(parseClaudeResets(body([event({})], { detector: null })))).toBe(false);
    expect(detectorBehind(parseClaudeResets(body([event({})], { live: false, detector: null })))).toBe(false);
  });
});

describe("scope", () => {
  it("reads who an announcement covered from the site's words", () => {
    expect(readScope("all")).toEqual({ reach: "everyone" });
    expect(readScope("affected users")).toEqual({ reach: "affected" });
    expect(readScope("paid plans")).toEqual({ reach: "plans", plans: ["pro", "max", "team", "enterprise"] });
    expect(readScope("Pro + Max")).toEqual({ reach: "plans", plans: ["pro", "max"] });
    expect(readScope("Pro, Max + Team")).toEqual({ reach: "plans", plans: ["pro", "max", "team"] });
    expect(readScope("Max")).toEqual({ reach: "plans", plans: ["max"] });
    expect(readScope("API customers")).toEqual({ reach: "unknown" });
    expect(readScope(null)).toEqual({ reach: "unknown" });
  });

  it("settles a plan only when the scope can", () => {
    expect(planFamily("Max 20x")).toBe("max");
    expect(planFamily("Pro")).toBe("pro");
    expect(planFamily("prolite")).toBeNull();
    expect(planFamily(undefined)).toBeNull();
    expect(covers("all", "free")).toBe(true);
    expect(covers("Max", "max")).toBe(true);
    expect(covers("Max", "pro")).toBe(false);
    expect(covers("paid plans", "free")).toBe(false);
    expect(covers("affected users", "max")).toBeNull();
    expect(covers(null, "max")).toBeNull();
  });

  it("words the scope, keeping the site's own words when it does not know them", () => {
    const text = insightsFor("vi").claude;
    expect(scopeText("all", text)).toBe("Mọi người dùng");
    expect(scopeText("paid plans", text)).toBe("Các gói trả phí");
    expect(scopeText("Pro, Max + Team", text)).toBe("Gói Pro, Max, Team");
    expect(scopeText("affected users", text)).toBe("Người dùng bị ảnh hưởng");
    expect(scopeText("API customers", text)).toBe("API customers");
    expect(scopeText(null, text)).toBeNull();
  });

  it("finds the latest reset for a plan and for everyone", () => {
    const feed = parseClaudeResets(FEED_FIXTURES.claudeResets)!;
    expect(latestFor(feed.resets, "max")!.id).toBe(BANKED);
    expect(latestFor(feed.resets, "free")!.id).toBe("2094856679250919746");
    expect(latestForEveryone(feed.resets)!.id).toBe("2094856679250919746");
  });
});

describe("banked resets", () => {
  const feed = parseClaudeResets(FEED_FIXTURES.claudeResets)!;

  it("lists the ones that can still be applied until their deadline", () => {
    expect(openBanked(feed.resets, NOW).map((reset) => reset.id)).toEqual([BANKED]);
    expect(openBanked(feed.resets, new Date("2026-10-22T23:59:58Z"))).toHaveLength(1);
    expect(openBanked(feed.resets, new Date("2026-10-22T23:59:59Z"))).toHaveLength(0);
    expect(openBanked(feed.resets, new Date("2026-09-22T16:00:00Z"))).toHaveLength(0);
    const open: ClaudeReset = { ...feed.resets[0]!, usableUntil: null };
    expect(openBanked([open], NOW)).toHaveLength(0);
  });

  it("reminds a card whose plan is covered or cannot be named, until it is marked as applied", () => {
    expect(bankedResetFor(feed.resets, "Max 20x", [], NOW)?.id).toBe(BANKED);
    expect(bankedResetFor(feed.resets, undefined, [], NOW)?.id).toBe(BANKED);
    expect(bankedResetFor(feed.resets, "Free", [], NOW)).toBeNull();
    expect(bankedResetFor(feed.resets, "Enterprise", [], NOW)).toBeNull();
    expect(bankedResetFor(feed.resets, "Max 20x", [BANKED], NOW)).toBeNull();
  });

  it("words the row with the time left and the deadline in the device's zone", () => {
    const lines = bankedResetLines(feed.resets[0]!, NOW, "24h", "vi")!;
    expect(lines).toMatchObject({ title: "Lượt reset để dành", value: "còn 23 ngày 11 giờ", caption: "Dùng trước 6:59 · T6 23/10 · GMT+7" });
    expect(lines.details).toMatch(/^@ClaudeDevs đăng lúc 23:44 · T3 22\/09:\n“Launched Opus 5\.5/);
    expect(lines.details).toContain("Settings → Usage");
    expect(bankedResetLines(feed.resets[0]!, new Date("2026-10-23T00:00:00Z"), "24h", "vi")).toBeNull();
  });
});

describe("compareTrackers", () => {
  it("counts both histories over the time both were tracked", () => {
    const claude = [plain("c1", "2026-06-10T00:00:00Z"), plain("c2", "2026-06-20T00:00:00Z"), plain("c3", "2026-09-19T12:00:00Z", "banked")];
    const codex = [plain("x0", "2026-01-05T00:00:00Z"), plain("x1", "2026-06-12T00:00:00Z"), plain("x2", "2026-07-02T00:00:00Z"), plain("x3", "2026-09-28T13:00:00Z")];
    const result = compareTrackers(claude, codex, NOW, "UTC")!;
    expect(result.from.toISOString()).toBe("2026-06-10T00:00:00.000Z");
    expect(result.claude).toMatchObject({ resets: 3, banked: 1, medianGapDays: 50.75, longestGapDays: 91.5, last30Days: 1 });
    expect(result.claude.averageGapDays).toBeCloseTo(50.75);
    expect(result.claude.daysSinceLast).toBeCloseTo(10.0417, 3);
    expect(result.codex).toMatchObject({ resets: 3, banked: 0, last30Days: 1, daysSinceLast: 1 });
    expect(result.months).toEqual([
      { year: 2026, month: 5, claude: 2, codex: 1 },
      { year: 2026, month: 6, claude: 0, codex: 1 },
      { year: 2026, month: 7, claude: 0, codex: 0 },
      { year: 2026, month: 8, claude: 1, codex: 1 },
    ]);
  });

  it("waits for both histories", () => {
    expect(compareTrackers([], [plain("x", "2026-06-12T00:00:00Z")], NOW)).toBeNull();
    expect(compareTrackers([plain("c", "2026-06-12T00:00:00Z")], [], NOW)).toBeNull();
  });
});

describe("forecastSkill", () => {
  it("finds the estimate no better than the plain average on Claude's short history", () => {
    const feed = parseClaudeResets(FEED_FIXTURES.claudeResets)!;
    const skill = forecastSkill(feed.resets, NOW)!;
    expect(skill.verdict).toBe("same");
    expect(Math.abs(skill.skill)).toBeLessThan(0.03);
    expect(skill.days).toBeGreaterThan(120);
  });

  it("finds it better on a history whose pace changed, where recent weeks tell more", () => {
    const day = (days: number) => new Date(NOW.getTime() - days * 86_400_000).toISOString();
    const slow = Array.from({ length: 8 }, (_, index) => plain(`s${index}`, day(300 - index * 25)));
    const fast = Array.from({ length: 33 }, (_, index) => plain(`f${index}`, day(100 - index * 3)));
    const skill = forecastSkill([...slow, ...fast], NOW)!;
    expect(skill.verdict).toBe("better");
    expect(skill.skill).toBeGreaterThan(0.05);
    expect(skill.byHorizon[7]).toBeGreaterThan(0);
  });

  it("says nothing on a history too short to try", () => {
    expect(forecastSkill([plain("a", "2026-09-01T00:00:00Z"), plain("b", "2026-09-10T00:00:00Z")], NOW)).toBeNull();
    const month = Array.from({ length: 6 }, (_, index) => plain(String(index), new Date(NOW.getTime() - (index + 1) * 6 * 86_400_000).toISOString()));
    expect(forecastSkill(month, NOW)).toBeNull();
  });
});

describe("buildClaudePresentation", () => {
  const feed = parseClaudeResets(FEED_FIXTURES.claudeResets)!;
  const codex = [plain("x0", "2026-01-05T00:00:00Z"), plain("x1", "2026-05-01T00:00:00Z"), plain("x2", "2026-09-26T18:17:54Z", "banked")];
  const build = (overrides: Partial<Parameters<typeof buildClaudePresentation>[0]> = {}) =>
    buildClaudePresentation({ feed, codex, plans: ["max"], used: [], now: NOW, language: "vi", timeFormat: "24h", ...overrides });

  it("shows the latest reset with who it covered and the banked reset still open", () => {
    const view = build();
    expect(view.latest).toMatchObject({ title: "Lần reset gần nhất", ago: "6 ngày trước", meta: "23:44 · T3 22/09 · Lượt để dành · Gói Pro, Max, Team", author: { handle: "@ClaudeDevs" } });
    expect(view.latest!.notes).toEqual([]);
    expect(view.statuses).toEqual([]);
    expect(view.banked).toHaveLength(1);
    expect(view.banked[0]).toMatchObject({ kind: "banked", resetId: BANKED, used: false, title: "Có lượt reset để dành", due: "Còn 23 ngày 11 giờ", author: { handle: "@ClaudeDevs" } });
    expect(view.banked[0]!.meta).toEqual(["Gói Pro, Max, Team", "Gói Max của bạn: có áp dụng", "Dùng được đến 6:59 23/10/2026"]);
  });

  it("says when a plan was left out, and falls back to the quiet card once the reset is marked as applied", () => {
    const view = build({ plans: ["free", "max"], used: [BANKED] });
    expect(view.latest!.notes).toEqual(["Gói Max của bạn: có áp dụng", "Gói Free của bạn: không áp dụng"]);
    expect(build({ plans: ["max", "pro"], used: [BANKED] }).latest!.notes).toEqual(["Gói Max, Pro của bạn: có áp dụng"]);
    expect(view.banked[0]).toMatchObject({ used: true });
    expect(view.statuses.map((card) => card.kind)).toEqual(["quiet"]);
    expect(view.stats).toContainEqual({ label: "Reset cho mọi người", value: "27,8 ngày trước · 02/09" });
    expect(view.stats).toContainEqual({ label: "Số lần đổi hạn mức", value: "6" });
    expect(view.stats.some((row) => row.label === "Reset cho gói Max")).toBe(false);
    expect(view.stats.some((row) => row.label === "Reset cho gói Free")).toBe(false);
  });

  it("keeps Claude's own wording for the estimate, the source and the history", () => {
    const view = build();
    expect(view.forecast.chances.map((chance) => chance.days)).toEqual([1, 3, 7]);
    expect(view.forecast.disclaimer).toBe("Chỉ là ước đoán từ lịch sử, không phải thông tin chính thức từ Anthropic.");
    expect(view.forecast.reliability).toMatch(/^Thử lại trên \d+ ngày đã qua: cách ước tính này chỉ ngang mức trung bình của lịch sử/);
    expect(view.source).toContain("claude-resets.com");
    expect(view.method.join(" ")).not.toContain("thsottiaux");
    expect(view.history).toHaveLength(14);
    expect(view.history[1]).toMatchObject({ author: { handle: "@lydiahallie" }, scope: "Gói Max", kindLabel: "Reset" });
    expect(view.history.every((item) => item.provisional === undefined)).toBe(true);
    expect(view.changes).toHaveLength(6);
    expect(view.changes[0]).toMatchObject({ scope: "Các gói trả phí", author: { handle: "@ClaudeDevs" } });
    expect(view.notices).toEqual([]);
  });

  it("flags an entry the site has not reviewed and a source that is behind", () => {
    const fresh = parseClaudeResets(body([event({ id: "9", date: "2026-09-29T12:00:00Z", verification: "provisional" }), event({ id: "8", date: "2026-09-01T12:00:00Z" })], { detector: "stale" }))!;
    const view = build({ feed: fresh });
    expect(view.latest!.meta).toBe("19:00 · T3 29/09 · Mọi người dùng · Chưa kiểm chứng");
    expect(view.latest!.notes).toEqual([insightsFor("vi").claude.provisionalNote]);
    expect(view.history[0]!.provisional).toBe("Chưa kiểm chứng");
    expect(view.notices).toEqual([insightsFor("vi").claude.detectorBehind]);
    const published = build({ feed: { ...fresh, live: false, detector: null } });
    expect(published.notices).toEqual([insightsFor("vi").claude.datasetNote]);
  });

  it("sets Claude against Codex from the day both were tracked", () => {
    const compare = build().compare!;
    expect(compare.since).toBe("Tính từ 17/04/2026, khi cả hai cùng được theo dõi.");
    expect(compare.rows.map((row) => row.label)).toEqual(["Số lần reset", "Trung bình giữa hai lần", "Trung vị giữa hai lần", "Khoảng lặng dài nhất", "Từ lần gần nhất", "30 ngày qua"]);
    expect(compare.rows[0]).toMatchObject({ claude: "14", codex: "2" });
    expect(compare.rows[4]).toMatchObject({ claude: "6,8 ngày", codex: "2,8 ngày" });
    expect(compare.months.map((month) => month.label)).toEqual(["T4", "T5", "T6", "T7", "T8", "T9"]);
    expect(compare.months.reduce((sum, month) => sum + month.claude, 0)).toBe(14);
    expect(build({ codex: [] }).compare).toBeUndefined();
  });
});
