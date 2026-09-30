import { createHash } from "node:crypto";
import { fixtureCatalog, fixtureSnapshots } from "@/lib/fixtures";
import { FEED_FIXTURES } from "@/lib/insightsFeedFixtures";
import type { ProviderSnapshot } from "@/lib/types";
import { buildGlance, CLAUDE_RESETS_PROVIDER_ID, CODEX_RESETS_PROVIDER_ID, glanceMetric, GLANCE_VERSION, isClaudeResetsWing, type GlanceAlert, type GlanceResets, type GlanceWingChoice } from "./glance";
import { buildClaudeGlanceResets } from "./glanceClaudeResets";
import { buildGlanceResets, parseResetFeeds } from "./glanceResets";
import { dailyReliability } from "./insights/claudePresentation";
import { parseClaudeResets } from "./insights/claudeResets";
import { glanceGroups, reconcileLayout } from "./layout";
import { barKind, platformKey } from "./platform";
import { cardIdentity } from "./providerText";
import { DEFAULT_SETTINGS, type GlanceContent, type IslandSettings } from "./settings";
import { makeWidget, NOW, resetsAt, WEEK_SECONDS } from "./testHelpers";
import { setSystemTimeZone } from "./timeZone";
import { DEFAULT_DISPLAY, widgetDataFor, type DisplayOptions } from "./widgetData";

const FETCHED = Date.UTC(2026, 8, 26, 3);
const NOW_GLANCE = new Date(FETCHED + 60_000);
const catalog = fixtureCatalog();
const snapshots = fixtureSnapshots(FETCHED);
const layout = reconcileLayout(null, catalog);
const descriptors = new Map(catalog.flatMap((entry) => entry.descriptors.map((descriptor) => [descriptor.id, descriptor] as const)));
const providers = new Map(catalog.map((entry) => [entry.provider.id, entry.provider] as const));

interface Options {
  display?: DisplayOptions;
  data?: Readonly<Record<string, ProviderSnapshot | undefined>>;
  alert?: GlanceAlert | null;
  island?: Partial<IslandSettings>;
  widget?: Partial<typeof DEFAULT_SETTINGS.widget>;
  wings?: [GlanceWingChoice, GlanceWingChoice];
  hour12?: boolean | null;
  resets?: GlanceResets | null;
  claudeResets?: GlanceResets | null;
  markArt?: Readonly<Record<string, string>>;
}

function glance({ display = DEFAULT_DISPLAY, data = snapshots, alert = null, island = {}, widget = {}, wings = [null, null], hour12 = null, resets = null, claudeResets, markArt }: Options = {}) {
  const islandSettings = { ...DEFAULT_SETTINGS.island, ...island };
  const widgetSettings = { ...DEFAULT_SETTINGS.widget, ...widget };
  const groups = (content: GlanceContent, metrics: readonly string[]) => glanceGroups(content, metrics, layout, catalog, () => true);
  return buildGlance({
    island: { groups: groups(islandSettings.content, islandSettings.metrics), settings: islandSettings, enabled: true, wings },
    widget: { groups: groups(widgetSettings.content, widgetSettings.metrics), settings: widgetSettings },
    dataFor: (descriptor) => widgetDataFor(descriptor, data[descriptor.providerId], display),
    describe: (provider) => {
      const snapshot = data[provider.id];
      return cardIdentity(provider, snapshot ? { snapshot, refreshing: false } : undefined, display.language);
    },
    providerOf: (providerId) => providers.get(providerId),
    refreshedAt: (providerId) => data[providerId]?.refreshedAt,
    language: display.language,
    hour12,
    appName: "Quota Control",
    alert,
    resets,
    claudeResets,
    markArt,
    now: NOW_GLANCE,
  });
}

const ids = (list: readonly { id: string }[]) => list.map((entry) => entry.id);

describe("glance document", () => {
  it("carries a brand's official color logo beside its path mark, and nothing for other brands", () => {
    const document = glance({ markArt: { claude: "cGljdHVyZQ==" } });
    const claude = document.widget.providers[0]!;
    expect(claude.mark?.art).toBe("cGljdHVyZQ==");
    expect(claude.mark?.paths.length).toBeGreaterThan(0);
    const codex = document.widget.providers.find((entry) => entry.brand === "codex")!;
    expect(codex.mark?.art).toBeUndefined();
    expect(glance().widget.providers[0]!.mark?.art).toBeUndefined();
  });

  it("lists every Hạn mức card by default, with its heading, plan and readings", () => {
    const document = glance();
    expect(document.version).toBe(GLANCE_VERSION);
    expect(document.locale).toBe("vi_VN");
    expect(ids(document.widget.providers)).toEqual(["claude@7c1e", "claude@a93f", "codex@52d0"]);
    expect(ids(document.providers)).toEqual(["claude@7c1e", "claude@a93f", "codex@52d0"]);
    const claude = document.widget.providers[0]!;
    expect(claude.name).toBe("Claude · Công ty");
    expect(claude.plan).toBe("Max 5x");
    expect(claude.brand).toBe("claude");
    expect(claude.color).toBe("#DE7356");
    expect(claude.mark?.paths.length).toBeGreaterThan(0);
    expect(claude.metrics.length).toBeGreaterThan(2);
    expect(claude.metrics[0]!.headline).toBe("Còn 88%");
    expect(claude.metrics[0]!.fraction).toBeCloseTo(0.88, 5);
    expect(claude.notice).toBeUndefined();
    expect(document.widget.shows).toEqual({ account: true, plan: true, resets: true });
    expect(document.labels.resetsIn).toBe("Đặt lại sau");
    expect(document.alert).toBeUndefined();
  });

  it("keeps an account without readings as a line saying why, unless told not to", () => {
    const signedOut = { ...snapshots, "claude@a93f": undefined };
    const document = glance({ data: signedOut });
    const personal = document.widget.providers.find((provider) => provider.id === "claude@a93f");
    expect(personal?.metrics).toEqual([]);
    expect(personal?.notice).toBe("Chưa có số liệu");
    expect(ids(glance({ data: signedOut, widget: { showProblems: false } }).widget.providers)).not.toContain("claude@a93f");
    expect(glance({ data: {} }).widget.providers.every((provider) => provider.metrics.length === 0)).toBe(true);
    expect(glance({ data: {}, widget: { showProblems: false } }).widget.providers).toEqual([]);
  });

  it("shows only the starred metrics or a hand-picked set when asked", () => {
    const starred = glance({ widget: { content: "starred" } });
    expect(starred.widget.providers[0]!.metrics.map((metric) => metric.value)).toEqual(["88%", "42%"]);
    expect(starred.widget.empty).toBe("Gắn sao chỉ số trong Quota Control để hiện ở đây.");
    const custom = glance({ widget: { content: "custom", metrics: ["codex@52d0.weekly", "codex@52d0.session"] } });
    expect(ids(custom.widget.providers)).toEqual(["codex@52d0"]);
    expect(custom.widget.providers[0]!.metrics.map((metric) => metric.id)).toEqual(["codex@52d0.session", "codex@52d0.weekly"]);
    expect(glance({ widget: { content: "custom", metrics: [] } }).widget.providers).toEqual([]);
  });

  it("dates itself by the newest reading, so an unchanged reading keeps an unchanged document", () => {
    const first = glance();
    const newest = Math.max(...Object.values(snapshots).map((snapshot) => Date.parse(snapshot?.refreshedAt ?? "")).filter(Number.isFinite));
    expect(first.generatedAt).toBe(new Date(newest).toISOString());
    expect(JSON.stringify(glance())).toBe(JSON.stringify(first));
  });

  it("follows Used/Left, the language and the clock, and passes alerts with the alerts switch", () => {
    const english = glance({ display: { ...DEFAULT_DISPLAY, displayMode: "used", language: "en" } });
    expect(english.locale).toBe("en_US");
    expect(english.widget.providers[0]!.metrics[0]!.headline).toBe("12% used");
    expect(english.labels.updated).toBe("Updated");
    const alert = { id: "a", title: "Claude", body: "Almost out", brand: "claude", severity: "critical" as const };
    expect(glance({ alert }).alert).toEqual(alert);
    const quiet = glance({ alert, island: { alerts: false } });
    expect(quiet.alert).toEqual(alert);
    expect(quiet.island.alerts).toBe(false);
    expect(glance().hour12).toBeUndefined();
    expect(glance({ hour12: false }).hour12).toBe(false);
  });
});

describe("island wings", () => {
  it("takes each account's first reading by default", () => {
    const wings = glance().island.wings;
    expect(wings.map((wing) => `${wing.id}|${wing.metrics[0]!.id}`)).toEqual(["claude@7c1e|claude@7c1e.session", "claude@a93f|claude@a93f.session"]);
  });

  it("shows a picked metric and fills the other slot automatically", () => {
    const weekly = descriptors.get("codex@52d0.weekly")!;
    const wings = glance({ wings: [null, weekly] }).island.wings;
    expect(wings.map((wing) => wing.metrics[0]!.id)).toEqual(["claude@7c1e.session", "codex@52d0.weekly"]);
    expect(wings[1]!.metrics[0]!.value).toBe("36%");
  });

  it("falls back to the automatic reading when the picked metric has no data", () => {
    const weekly = descriptors.get("codex@52d0.weekly")!;
    const wings = glance({ wings: [weekly, null], data: { ...snapshots, "codex@52d0": undefined } }).island.wings;
    expect(wings.map((wing) => wing.metrics[0]!.id)).toEqual(["claude@7c1e.session", "claude@a93f.session"]);
  });

  it("carries the closed style and the hover choice", () => {
    const island = glance({ island: { style: "ring", expandOnHover: false, showPlan: true } }).island;
    expect(island.style).toBe("ring");
    expect(island.expandOnHover).toBe(false);
    expect(island.shows.plan).toBe(true);
  });
});

const TRACKER: GlanceResets = {
  title: "Reset Codex",
  source: "Theo codex-resets.com",
  brand: "codex",
  color: "#10A37F",
  upcoming: {
    title: "Reset free",
    tone: "positive",
    countdown: { at: new Date(FETCHED + 3 * 3_600_000).toISOString(), text: "sau {d}", after: "chờ xác nhận" },
    caption: "Lúc 13:00 · CN 27/09 · GMT+7",
    captionAfter: "Hẹn 13:00 · CN 27/09 · GMT+7",
    hideAt: new Date(FETCHED + 27 * 3_600_000).toISOString(),
  },
  latest: {
    at: "2026-09-24T18:17:54.000Z",
    kind: "regular",
    label: "Lần reset gần nhất",
    kindLabel: "Reset",
    since: { at: "2026-09-24T18:17:54.000Z", text: "Đã {d} chưa có reset", since: true },
    when: "1:17 · T6 25/09",
  },
  forecastTitle: "Khả năng có reset",
  forecast: [
    { days: 1, percent: 22, label: "24 giờ tới" },
    { days: 3, percent: 52, label: "3 ngày tới" },
    { days: 7, percent: 82, label: "7 ngày tới" },
  ],
  forecastNote: "Ước tính từ lịch sử, không phải tin chính thức.",
};

const DEADLINE = "2026-10-22T23:59:59.000Z";

const CLAUDE_TRACKER: GlanceResets = {
  title: "Reset Claude",
  source: "Theo claude-resets.com",
  brand: "claude",
  color: "#DE7356",
  site: "https://claude-resets.com",
  upcoming: {
    title: "Lượt reset để dành",
    tone: "positive",
    countdown: { at: DEADLINE, text: "còn {d}" },
    caption: "Dùng trước 6:59 · T6 23/10 · GMT+7",
    hideAt: DEADLINE,
  },
  latest: {
    at: "2026-09-22T16:44:06.000Z",
    kind: "banked",
    label: "Lần reset gần nhất",
    kindLabel: "Lượt để dành",
    since: { at: "2026-09-22T16:44:06.000Z", text: "Đã {d} chưa có reset", since: true },
    when: "23:44 · T3 22/09",
  },
  forecastTitle: "Khả năng có reset",
  forecast: [
    { days: 1, percent: 9, label: "24 giờ tới" },
    { days: 3, percent: 25, label: "3 ngày tới" },
    { days: 7, percent: 49, label: "7 ngày tới" },
  ],
  forecastNote: "Ước tính từ lịch sử, không phải tin chính thức.",
};

describe("the Claude reset tracker", () => {
  it("rides along for the surface that chose it, named for it, beside the Codex one", () => {
    const island = glance({ island: { resetsProvider: "claude" }, resets: TRACKER, claudeResets: CLAUDE_TRACKER });
    expect(island.island.resetsProvider).toBe("claude");
    expect(island.widget.resetsProvider).toBeUndefined();
    expect(island.labels.claudeResetsTab).toBe("Reset Claude");
    expect(island.labels.claudeResetsOff).toBe("Bật tab Reset hoặc thông báo khi Claude reset trong Quota Control để xem dự báo.");
    expect(island.labels.resetsOff).toBe("Bật tab Reset hoặc thông báo reset trong Quota Control để xem dự báo.");
    expect(island.labels.tabs.resets).toBe("Reset Codex");
    expect(island.claudeResets).toBe(CLAUDE_TRACKER);
    expect(island.resets).toBe(TRACKER);
    const widget = glance({ widget: { resetsProvider: "claude" }, display: { ...DEFAULT_DISPLAY, language: "en" }, claudeResets: CLAUDE_TRACKER });
    expect(widget.widget.resetsProvider).toBe("claude");
    expect(widget.island.resetsProvider).toBeUndefined();
    expect(widget.labels.claudeResetsTab).toBe("Claude Resets");
    expect(widget.labels.claudeResetsOff).toBe("Turn on the Resets tab or Claude reset notifications in Quota Control to see the forecast.");
    expect(widget.claudeResets).toBe(CLAUDE_TRACKER);
  });

  it("names the Claude view even while its tracker is off", () => {
    const document = glance({ island: { resetsProvider: "claude" }, resets: TRACKER, claudeResets: null });
    expect(document.labels.claudeResetsTab).toBe("Reset Claude");
    expect(document.labels.claudeResetsOff).toBe("Bật tab Reset hoặc thông báo khi Claude reset trong Quota Control để xem dự báo.");
    expect("claudeResets" in document).toBe(false);
    expect(document.resets).toBe(TRACKER);
  });

  it("stays out of a document when only a wing reads it, which carries its own reading", () => {
    const document = glance({ wings: ["claude-resets:next", null], claudeResets: CLAUDE_TRACKER });
    expect("claudeResets" in document).toBe(false);
    expect(document.labels.claudeResetsTab).toBeUndefined();
    expect(document.labels.claudeResetsOff).toBeUndefined();
    expect(document.island.wings[0]).toMatchObject({ id: CLAUDE_RESETS_PROVIDER_ID, brand: "claude" });
  });

  it("tells the Claude wings apart from every other wing", () => {
    expect(["claude-resets:next", "claude-resets:chance-1", "claude-resets:chance-3", "claude-resets:chance-7", "claude-resets:since"].every(isClaudeResetsWing)).toBe(true);
    expect(["", "quota:next", "codex-resets:next", "claude-resets:soon", "claude@7c1e.session", "claude-resets"].some(isClaudeResetsWing)).toBe(false);
  });
});

describe("island sections and labels", () => {
  it("carries the chosen tabs, how they are arranged and each view's options, as copies of the settings", () => {
    const tabs: ("quota" | "resets" | "upcoming")[] = ["quota", "upcoming"];
    const document = glance({ island: { tabs, layout: "separate", upcomingLimit: 3 } });
    expect(document.island.tabs).toEqual(["quota", "upcoming"]);
    expect(document.island.sections).toEqual({ quota: true, resets: false, upcoming: true });
    expect(document.island.arrangement).toBe("tabs");
    expect(document.island.upcomingLimit).toBe(3);
    tabs.push("resets");
    expect(document.island.tabs).toEqual(["quota", "upcoming"]);
    expect(glance({ island: { layout: "combined" } }).island.arrangement).toBe("stacked");
  });

  it("carries the widget's Overview parts and reset parts", () => {
    const resetParts = { next: true, latest: false, chances: true, wait: false, calendar: true, rhythm: false };
    const document = glance({ widget: { tabs: ["resets"], resetParts } });
    expect(document.widget.tabs).toEqual(["resets"]);
    expect(document.widget.resetParts).toEqual(resetParts);
    expect(document.widget.upcomingLimit).toBe(0);
  });

  it("carries the reset and coming-back labels", () => {
    const document = glance();
    expect(document.labels).toMatchObject({
      resetsOff: "Bật tab Reset hoặc thông báo reset trong Quota Control để xem dự báo.",
      upcoming: "Sắp đặt lại",
      upcomingEmpty: "Chưa có hạn mức nào có giờ đặt lại.",
      tabs: { quota: "Hạn mức", resets: "Reset Codex", upcoming: "Sắp đặt lại" },
    });
    expect(glance({ display: { ...DEFAULT_DISPLAY, language: "en" } }).labels.upcoming).toBe("Coming back");
  });

  it("attaches the tracker only when there is one", () => {
    expect(glance().resets).toBeUndefined();
    expect(glance({ resets: TRACKER }).resets).toBe(TRACKER);
  });

});

describe("special wings", () => {
  const wingIds = (wings: { id: string; metrics: { id: string }[] }[]) => wings.map((wing) => `${wing.id}|${wing.metrics[0]!.id}`);

  it("counts down to the island's soonest limit reset", () => {
    const document = glance({ wings: ["quota:next", null] });
    const now = NOW_GLANCE.getTime();
    const soonest = document.providers
      .flatMap((provider) => provider.metrics.map((metric) => ({ provider, metric })))
      .filter(({ metric }) => metric.resetsAt && Date.parse(metric.resetsAt) > now)
      .sort((a, b) => Date.parse(a.metric.resetsAt!) - Date.parse(b.metric.resetsAt!))[0]!;
    const wing = document.island.wings[0]!;
    expect(wing.id).toBe(soonest.provider.id);
    expect(wing.metrics[0]).toEqual({ ...soonest.metric, countdown: { at: soonest.metric.resetsAt, text: "sau {d}" } });
    expect(document.island.wings[1]!.metrics[0]!.countdown).toBeUndefined();
  });

  it("falls back to the automatic reading when no limit has a reset ahead", () => {
    const island = { content: "custom" as const, metrics: ["claude@7c1e.extra", "codex@52d0.credits"] };
    const automatic = wingIds(glance({ island }).island.wings);
    expect(automatic).toEqual(["claude@7c1e|claude@7c1e.extra", "codex@52d0|codex@52d0.credits"]);
    expect(wingIds(glance({ island, wings: ["quota:next", null] }).island.wings)).toEqual(automatic);
    expect(wingIds(glance({ wings: ["quota:next", null] }).island.wings)).toEqual(["codex@52d0|codex@52d0.session", "claude@7c1e|claude@7c1e.session"]);
  });

  it("shows the announced free reset's countdown, then the 24-hour chance once it has passed", () => {
    const wing = glance({ resets: TRACKER, wings: ["codex-resets:next", null] }).island.wings[0]!;
    expect(wing).toMatchObject({ id: CODEX_RESETS_PROVIDER_ID, name: "Reset Codex", brand: "codex", color: "#10A37F" });
    expect(wing.metrics[0]).toEqual({
      id: "codex-resets:next",
      label: "Reset free",
      value: "Reset free",
      headline: "Lúc 13:00 · CN 27/09 · GMT+7",
      fraction: null,
      severity: "normal",
      countdown: { at: TRACKER.upcoming!.countdown!.at, text: "sau {d}", after: "chờ xác nhận" },
    });
    const passed = { ...TRACKER, upcoming: { ...TRACKER.upcoming!, countdown: { at: new Date(FETCHED - 60_000).toISOString(), text: "sau {d}" } } };
    const fallback = glance({ resets: passed, wings: ["codex-resets:next", null] }).island.wings[0]!.metrics[0]!;
    expect(fallback).toMatchObject({ id: "codex-resets:next", label: "24 giờ tới", value: "22%", fraction: 0.22, severity: "normal" });
    expect(fallback.countdown).toBeUndefined();
    const untimed = { ...TRACKER, upcoming: { title: "Reset free", tone: "positive" as const, value: "chưa rõ giờ", caption: "Tuần sau giờ Mỹ", hideAt: TRACKER.upcoming!.hideAt } };
    expect(glance({ resets: untimed, wings: ["codex-resets:next", null] }).island.wings[0]!.metrics[0]!.value).toBe("22%");
  });

  it("reads each forecast horizon as a whole-percent meter", () => {
    const wings = glance({ resets: TRACKER, wings: ["codex-resets:chance-3", "codex-resets:chance-7"] }).island.wings;
    expect(wings.map((wing) => wing.metrics[0])).toEqual([
      { id: "codex-resets:chance-3", label: "3 ngày tới", value: "52%", headline: "52% · 3 ngày tới", fraction: 0.52, severity: "normal" },
      { id: "codex-resets:chance-7", label: "7 ngày tới", value: "82%", headline: "82% · 7 ngày tới", fraction: 0.82, severity: "normal" },
    ]);
    expect(glance({ resets: TRACKER, wings: ["codex-resets:chance-1", null] }).island.wings[0]!.metrics[0]!.value).toBe("22%");
  });

  it("counts the time since the last reset", () => {
    const metric = glance({ resets: TRACKER, wings: [null, "codex-resets:since"] }).island.wings[1]!.metrics[0]!;
    expect(metric).toMatchObject({ id: "codex-resets:since", label: "Chưa reset", value: "1:17 · T6 25/09", fraction: null, severity: "normal" });
    expect(metric.countdown).toEqual({ at: TRACKER.latest!.at, text: "đã {d}", since: true });
  });

  it("behaves like an automatic slot when the tracker has nothing for it", () => {
    const automatic = wingIds(glance().island.wings);
    expect(wingIds(glance({ wings: ["codex-resets:next", "codex-resets:since"] }).island.wings)).toEqual(automatic);
    const bare = { ...TRACKER, forecast: [], latest: undefined, upcoming: undefined };
    expect(wingIds(glance({ resets: bare, wings: ["codex-resets:chance-1", "codex-resets:since"] }).island.wings)).toEqual(automatic);
    expect(wingIds(glance({ resets: bare, wings: ["codex-resets:next", null] }).island.wings)).toEqual(automatic);
  });

  it("counts a Claude wing down to the banked reset's deadline, then shows the 24-hour chance", () => {
    const wing = glance({ claudeResets: CLAUDE_TRACKER, wings: ["claude-resets:next", null] }).island.wings[0]!;
    expect(wing).toMatchObject({ id: CLAUDE_RESETS_PROVIDER_ID, name: "Reset Claude", brand: "claude", color: "#DE7356" });
    expect(wing.metrics[0]).toEqual({
      id: "claude-resets:next",
      label: "Lượt reset để dành",
      value: "Lượt reset để dành",
      headline: "Dùng trước 6:59 · T6 23/10 · GMT+7",
      fraction: null,
      severity: "normal",
      countdown: { at: DEADLINE, text: "còn {d}" },
    });
    const none = glance({ claudeResets: { ...CLAUDE_TRACKER, upcoming: undefined }, wings: ["claude-resets:next", null] }).island.wings[0]!.metrics[0]!;
    expect(none).toMatchObject({ id: "claude-resets:next", label: "24 giờ tới", value: "9%", fraction: 0.09 });
    expect(none.countdown).toBeUndefined();
  });

  it("reads Claude's chances and the time since its last reset from the Claude tracker alone", () => {
    const wings = glance({ resets: TRACKER, claudeResets: CLAUDE_TRACKER, wings: ["claude-resets:chance-7", "claude-resets:since"] }).island.wings;
    expect(wings.map((wing) => wing.brand)).toEqual(["claude", "claude"]);
    expect(wings[0]!.metrics[0]).toEqual({ id: "claude-resets:chance-7", label: "7 ngày tới", value: "49%", headline: "49% · 7 ngày tới", fraction: 0.49, severity: "normal" });
    expect(wings[1]!.metrics[0]).toMatchObject({ id: "claude-resets:since", label: "Chưa reset", value: "23:44 · T3 22/09" });
    expect(wings[1]!.metrics[0]!.countdown).toEqual({ at: CLAUDE_TRACKER.latest!.at, text: "đã {d}", since: true });
    const codexOnly = wingIds(glance({ resets: TRACKER, wings: ["claude-resets:chance-1", "claude-resets:since"] }).island.wings);
    expect(codexOnly).toEqual(wingIds(glance().island.wings));
    const claudeOnly = wingIds(glance({ claudeResets: CLAUDE_TRACKER, wings: ["codex-resets:chance-1", "codex-resets:since"] }).island.wings);
    expect(claudeOnly).toEqual(wingIds(glance().island.wings));
  });
});

describe("a document for someone who never chose Claude", () => {
  beforeEach(() => setSystemTimeZone("Asia/Saigon"));
  afterEach(() => setSystemTimeZone(null));

  const codex = (language: "vi" | "en") => {
    const feeds = parseResetFeeds(FEED_FIXTURES.codexResetStatus, FEED_FIXTURES.codexResets);
    return buildGlanceResets({ feeds, stale: false, now: NOW_GLANCE, language, timeFormat: "auto", theme: "system", reliability: dailyReliability(feeds.resets, NOW_GLANCE, language) });
  };
  const claude = () =>
    buildClaudeGlanceResets({ feed: parseClaudeResets(FEED_FIXTURES.claudeResets)!, accounts: ["max", null], used: [], stale: false, now: NOW_GLANCE, language: "vi", timeFormat: "auto", theme: "system" });
  const scenarios = (): Record<string, Options> => ({
    defaults: { resets: codex("vi") },
    wings: { resets: codex("vi"), wings: ["codex-resets:next", "codex-resets:since"], alert: { id: "a1", title: "Title", body: "Body", brand: "codex", severity: "normal" }, hour12: true },
    tuned: {
      display: { ...DEFAULT_DISPLAY, language: "en" },
      resets: codex("en"),
      island: { layout: "combined", tabs: ["resets", "quota"], resetParts: { ...DEFAULT_SETTINGS.island.resetParts, calendar: false } },
      widget: { content: "starred", upcomingLimit: 3 },
      wings: ["quota:next", "codex-resets:chance-7"],
    },
  });
  /** SHA-256 of the documents 0.3.16 (`be761f5`) built from these same inputs. */
  const RELEASED: Readonly<Record<string, string>> = {
    defaults: "27f79bf299e37eeefc370b5c7c50f50479639888e0cc999596faaa04ec00652f",
    wings: "e1d39d984056f8e5571c619d2146b07637743d83ce6fe62050e215cd3d53f1fa",
    tuned: "d319d2b1be43d61562cff513331df6d8124f0c2c50107cf27a7267073fd52973",
  };
  const digest = (document: object) => createHash("sha256").update(JSON.stringify(document)).digest("hex");
  /** The document without the one thing added since: the announcement the latest reset's card quotes. */
  const unquoted = (document: ReturnType<typeof glance>) => {
    const copy = JSON.parse(JSON.stringify(document)) as ReturnType<typeof glance>;
    const latest = copy.resets?.presentation?.latest;
    if (latest) {
      delete latest.excerpt;
      delete latest.fullText;
      delete latest.url;
      delete latest.observed;
    }
    return copy;
  };

  it("stays byte for byte what 0.3.16 sent apart from the quoted announcement, even with a Claude tracker at hand", () => {
    const tracker = claude();
    expect(tracker).not.toBeNull();
    for (const [name, options] of Object.entries(scenarios())) {
      const document = glance(options);
      expect(document.resets?.presentation?.latest, name).toMatchObject({ excerpt: expect.stringMatching(/^GPT-6 Sol and Luna are out\./), url: "https://x.com/thsottiaux/status/2102463847714247142" });
      expect(digest(unquoted(document)), name).toBe(RELEASED[name]);
      expect(digest(unquoted(glance({ ...options, claudeResets: tracker }))), name).toBe(RELEASED[name]);
    }
  });

  it("carries no Claude key at all", () => {
    const document = glance({ ...scenarios().defaults, claudeResets: claude() });
    expect("claudeResets" in document).toBe(false);
    expect("claudeResetsTab" in document.labels).toBe(false);
    expect("claudeResetsOff" in document.labels).toBe(false);
    expect("resetsProvider" in document.island).toBe(false);
    expect("resetsProvider" in document.widget).toBe(false);
    expect(JSON.stringify(document)).not.toMatch(/claude-resets/);
  });
});

describe("glance metric", () => {
  it("counts down to a reset and colors a meter running out", () => {
    const reset = resetsAt(0.2, WEEK_SECONDS);
    const data = makeWidget("Weekly", "percent", 95, 100, { resetsAt: reset, periodDurationMs: WEEK_SECONDS * 1000 });
    const metric = glanceMetric("claude.weekly", data, NOW);
    expect(metric.resetsAt).toBe(reset.toISOString());
    expect(metric.severity).toBe("critical");
    expect(metric.detail).toBeUndefined();
  });

  it("gives text rows no meter and no pace color", () => {
    const data = makeWidget("Today", "dollars", 0, null, { values: [{ kind: "dollars", number: 4.08, estimated: false }] });
    const metric = glanceMetric("claude.today", data, NOW);
    expect(metric.fraction).toBeNull();
    expect(metric.severity).toBe("none");
    expect(metric.headline).toContain("4.08");
  });
});

describe("platform wording", () => {
  it("names the menu bar only on macOS", () => {
    expect(platformKey("macos")).toBe("macos");
    expect(platformKey("web")).toBe("other");
    expect(barKind("macos")).toBe("menuBar");
    expect(barKind("windows")).toBe("taskbar");
  });
});
