import { createHash } from "node:crypto";
import { accountDescriptors, fixtureCatalog, fixtureSnapshots } from "@/lib/fixtures";
import { FEED_FIXTURES } from "@/lib/insightsFeedFixtures";
import type { Provider, ProviderSnapshot } from "@/lib/types";
import {
  buildGlance,
  CLAUDE_RESETS_PROVIDER_ID,
  CODEX_RESETS_PROVIDER_ID,
  glanceMetric,
  glancePlanTerm,
  glancePlanTermWords,
  GLANCE_VERSION,
  isClaudeResetsWing,
  type GlanceAlert,
  type GlanceDocument,
  type GlancePlanTerm,
  type GlancePlanTermWords,
  type GlanceResets,
  type GlanceWingChoice,
} from "./glance";
import { buildClaudeGlanceResets } from "./glanceClaudeResets";
import { buildGlanceResets, parseResetFeeds } from "./glanceResets";
import { parseClaudeResets } from "./insights/claudeResets";
import { glanceGroups, reconcileLayout } from "./layout";
import { barKind, platformKey } from "./platform";
import { cardIdentity } from "./providerText";
import { DEFAULT_SETTINGS, type GlanceContent, type IslandSettings, type ThemeSetting } from "./settings";
import { planTermLines } from "./planTermLines";
import { makeWidget, NOW, resetsAt, WEEK_SECONDS } from "./testHelpers";
import { calendarDaysBetween, setSystemTimeZone } from "./timeZone";
import { DEFAULT_DISPLAY, widgetDataFor, type DisplayOptions } from "./widgetData";

const FETCHED = Date.UTC(2026, 8, 26, 3);
const NOW_GLANCE = new Date(FETCHED + 60_000);
const REFRESH_INTERVAL_MS = 300_000;
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
  theme?: ThemeSetting;
  now?: Date;
  /** The refresh error each provider's runtime carries beside its last good snapshot. */
  errors?: Readonly<Record<string, string>>;
}

function glance({ display = DEFAULT_DISPLAY, data = snapshots, alert = null, island = {}, widget = {}, wings = [null, null], hour12 = null, resets = null, claudeResets, markArt, theme = "system", now = NOW_GLANCE, errors = {} }: Options = {}) {
  const islandSettings = { ...DEFAULT_SETTINGS.island, ...island };
  const widgetSettings = { ...DEFAULT_SETTINGS.widget, ...widget };
  const groups = (content: GlanceContent, metrics: readonly string[]) => glanceGroups(content, metrics, layout, catalog, () => true);
  return buildGlance({
    island: { groups: groups(islandSettings.content, islandSettings.metrics), settings: islandSettings, enabled: true, wings },
    widget: { groups: groups(widgetSettings.content, widgetSettings.metrics), settings: widgetSettings },
    dataFor: (descriptor) => widgetDataFor(descriptor, data[descriptor.providerId], display),
    describe: (provider) => {
      const snapshot = data[provider.id];
      const error = errors[provider.id];
      const runtime = snapshot || error ? { refreshing: false, ...(snapshot ? { snapshot } : {}), ...(error ? { error } : {}) } : undefined;
      return cardIdentity(provider, runtime, display.language);
    },
    providerOf: (providerId) => providers.get(providerId),
    refreshedAt: (providerId) => data[providerId]?.refreshedAt,
    refreshIntervalMs: REFRESH_INTERVAL_MS,
    language: display.language,
    hour12,
    theme,
    appName: "Quota Control",
    alert,
    resets,
    claudeResets,
    markArt,
    now,
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

  it("keeps an account without readings as a line in the popup's words, unless told not to", () => {
    const signedOut = { ...snapshots, "claude@a93f": undefined };
    const document = glance({ data: signedOut });
    const personal = document.widget.providers.find((provider) => provider.id === "claude@a93f");
    expect(personal?.metrics).toEqual([]);
    expect(personal?.notice).toBe("Không có dữ liệu");
    expect(personal?.problem).toBeUndefined();
    expect(document.labels.noData).toBe("Không có dữ liệu");
    const english = glance({ data: signedOut, display: { ...DEFAULT_DISPLAY, language: "en" } });
    expect(english.widget.providers.find((provider) => provider.id === "claude@a93f")?.notice).toBe("No data");
    expect(english.labels.noData).toBe("No data");
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
    const island = glance({ island: { style: "ring", expandOnHover: false, showPlan: false } }).island;
    expect(island.style).toBe("ring");
    expect(island.expandOnHover).toBe(false);
    expect(island.shows.plan).toBe(false);
  });

  it("shows the plan by default, as the popup's cards and the widgets do", () => {
    expect(glance().island.shows).toEqual({ account: true, plan: true, resets: true });
  });
});

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** How the island and the widgets word the plan period's corner (`GlancePlanTerm.lines` in Glance.swift). */
function fillPlanTerm(term: GlancePlanTerm, words: GlancePlanTermWords, now: Date): { left: string; day: string; soon: boolean } | null {
  const about = term.estimated ? "~" : "";
  const remaining = Date.parse(term.endsAt) - now.getTime();
  const days = calendarDaysBetween(now, new Date(term.endsAt));
  if (remaining <= 0) return term.estimated ? null : { left: words.due, day: days >= 0 ? words.today : term.on, soon: true };
  const word = days <= 0 ? words.today : days === 1 ? words.tomorrow : term.on;
  const [forms, count] =
    remaining < HOUR ? [words.minutes, Math.ceil(remaining / MINUTE)] : remaining < DAY ? [words.hours, Math.floor(remaining / HOUR)] : [words.days, Math.floor(remaining / DAY)];
  const form = (count === 1 ? forms[0] : forms[forms.length - 1]) ?? "";
  return { left: form.replace("{n}", `${about}${count}`), day: words.until.replace("{d}", `${about}${word}`), soon: now.getTime() > Date.parse(term.soonAt) };
}

describe("the account header", () => {
  beforeEach(() => setSystemTimeZone("Asia/Saigon"));
  afterEach(() => setSystemTimeZone(null));

  const account = (document: GlanceDocument, id: string) => document.widget.providers.find((entry) => entry.id === id)!;
  const english = { ...DEFAULT_DISPLAY, language: "en" as const };

  it("carries each account's plan period as moments, with the day it ends as the popup names a date", () => {
    const document = glance();
    expect(account(document, "claude@7c1e").term).toEqual({ endsAt: "2026-09-30T03:00:00.000Z", soonAt: "2026-09-26T03:00:00.000Z", on: "T4 30/09", estimated: true });
    expect(account(document, "codex@52d0").term).toEqual({ endsAt: "2026-10-16T15:00:00.000Z", soonAt: "2026-10-12T15:00:00.000Z", on: "T6 16/10" });
    expect(account(document, "claude@a93f").term).toBeUndefined();
    expect(document.providers.find((entry) => entry.id === "codex@52d0")?.term).toEqual(account(document, "codex@52d0").term);
    expect(account(glance({ display: english }), "codex@52d0").term?.on).toBe("Fri, Oct 16");
  });

  it("carries the popup's plan-period words, with a place for the count and one for the day, only while an account has a period", () => {
    expect(glance().labels.planTerm).toEqual({
      days: ["còn {n} ngày", "còn {n} ngày"],
      hours: ["còn {n} giờ", "còn {n} giờ"],
      minutes: ["còn {n} phút", "còn {n} phút"],
      due: "đã tới hạn",
      until: "tới {d}",
      today: "hôm nay",
      tomorrow: "ngày mai",
    });
    expect(glance({ display: english }).labels.planTerm).toEqual({
      days: ["{n} day left", "{n} days left"],
      hours: ["{n} hour left", "{n} hours left"],
      minutes: ["{n} min left", "{n} min left"],
      due: "Period ended",
      until: "{d}",
      today: "today",
      tomorrow: "tomorrow",
    });
    const { planTerm: _claude, ...claude } = snapshots["claude@7c1e"]!;
    const { planTerm: _codex, ...codex } = snapshots["codex@52d0"]!;
    expect(glance({ data: { ...snapshots, "claude@7c1e": claude, "codex@52d0": codex } }).labels.planTerm).toBeUndefined();
  });

  it("words the corner as the popup does at any moment, filled in the way the island and the widgets fill it", () => {
    const stated = { basis: "stated", endsAt: "2026-10-16T15:00:00Z" } as const;
    const estimate = { basis: "monthlyFrom", startedAt: "2026-08-30T03:00:00Z" } as const;
    const lines = (term: typeof stated | typeof estimate, now: Date, language: "vi" | "en") => {
      const popup = planTermLines(term, now, "auto", language);
      return popup ? { left: popup.left, day: popup.day, soon: popup.soon } : null;
    };
    for (const zone of ["Asia/Saigon", "America/Los_Angeles"]) {
      setSystemTimeZone(zone);
      for (const language of ["vi", "en"] as const) {
        const words = glancePlanTermWords(language);
        const statedTerm = glancePlanTerm(stated, NOW_GLANCE, language)!;
        const end = Date.parse(statedTerm.endsAt);
        for (const offset of [-25 * DAY, -4 * DAY - 1000, -4 * DAY + 1000, -2 * DAY, -30 * HOUR, -23.5 * HOUR, -HOUR - 1000, -HOUR + 1000, -90_000, -MINUTE, -30_000, HOUR, 2 * DAY]) {
          const now = new Date(end + offset);
          expect(fillPlanTerm(statedTerm, words, now), `${zone} ${language} ${offset}`).toEqual(lines(stated, now, language));
        }
        const estimatedTerm = glancePlanTerm(estimate, NOW_GLANCE, language)!;
        const renewal = Date.parse(estimatedTerm.endsAt);
        for (const offset of [-4 * DAY + MINUTE, -3 * DAY, -25 * HOUR, -5 * HOUR, -40 * MINUTE, -1000]) {
          const now = new Date(renewal + offset);
          expect(fillPlanTerm(estimatedTerm, words, now), `${zone} ${language} ~${offset}`).toEqual(lines(estimate, now, language));
        }
        expect(fillPlanTerm(estimatedTerm, words, new Date(renewal + 1000))).toBeNull();
      }
    }
  });

  it("flags an account whose refresh failed with the warning triangle and its reason, with readings or without", () => {
    expect(glance().widget.providers.some((entry) => entry.problem)).toBe(false);
    const codex = account(glance({ errors: { "codex@52d0": "Refresh failed" } }), "codex@52d0");
    expect(codex.problem).toBe("Làm mới thất bại");
    expect(codex.metrics.length).toBeGreaterThan(0);
    expect(codex.notice).toBeUndefined();
    const signedOut = glance({ data: { ...snapshots, "claude@a93f": undefined }, errors: { "claude@a93f": "Refresh failed" } });
    expect(account(signedOut, "claude@a93f")).toMatchObject({ metrics: [], problem: "Làm mới thất bại", notice: "Làm mới thất bại" });
    const expired = { ...snapshots["claude@a93f"]!, errorCategory: "auth_expired" as const, lines: [] };
    expect(account(glance({ data: { ...snapshots, "claude@a93f": expired } }), "claude@a93f")).toMatchObject({
      metrics: [],
      problem: "Phiên đăng nhập đã hết hạn",
      notice: "Phiên đăng nhập đã hết hạn",
    });
  });

  it("says a reading is outdated two refresh intervals after it was taken, as beside the card's name", () => {
    expect(glance().widget.providers.some((entry) => entry.outdated)).toBe(false);
    expect(glance({ now: new Date(FETCHED + 8 * MINUTE) }).widget.providers.some((entry) => entry.outdated)).toBe(false);
    const later = glance({ now: new Date(FETCHED + 11 * MINUTE) });
    expect(later.widget.providers.map((entry) => entry.outdated)).toEqual(["Dữ liệu cũ", "Dữ liệu cũ", "Dữ liệu cũ"]);
    expect(account(glance({ now: new Date(FETCHED + 11 * MINUTE), display: english }), "codex@52d0").outdated).toBe("Outdated");
  });

  it("carries the email the provider reports when the card's label is not one, keeping the heading", () => {
    const reported = {
      ...snapshots,
      "codex@52d0": { ...snapshots["codex@52d0"]!, account: "dev@example.com" },
      "claude@7c1e": { ...snapshots["claude@7c1e"]!, account: "work@example.com" },
    };
    const document = glance({ data: reported });
    expect(account(document, "codex@52d0")).toMatchObject({ name: "Codex", account: "dev@example.com" });
    expect(account(document, "claude@7c1e")).toMatchObject({ name: "Claude · Công ty", account: "work@example.com" });
    expect(account(glance(), "codex@52d0").account).toBeUndefined();
  });

  it("gives a mark its color on a light background where the brand's differs there, as the popup's light theme does", () => {
    expect(glance().widget.providers.some((entry) => "lightColor" in entry)).toBe(false);
    const single = (provider: Provider) => {
      const snapshot: ProviderSnapshot = {
        providerID: provider.id,
        displayName: provider.displayName,
        refreshedAt: new Date(FETCHED).toISOString(),
        lines: [{ type: "progress", label: "Session", used: 40, limit: 100, format: { kind: "percent" } }],
      };
      const groups = [{ provider, always: accountDescriptors(provider, "codex").slice(0, 1), onDemand: [] }];
      return buildGlance({
        island: { groups, settings: DEFAULT_SETTINGS.island, enabled: true, wings: [null, null] },
        widget: { groups, settings: DEFAULT_SETTINGS.widget },
        dataFor: (descriptor) => widgetDataFor(descriptor, snapshot, DEFAULT_DISPLAY),
        describe: (entry) => cardIdentity(entry, { snapshot, refreshing: false }, "vi"),
        providerOf: () => provider,
        refreshedAt: () => snapshot.refreshedAt,
        refreshIntervalMs: REFRESH_INTERVAL_MS,
        language: "vi",
        hour12: null,
        theme: "system",
        appName: "Quota Control",
        alert: null,
        resets: null,
        now: NOW_GLANCE,
      }).widget.providers[0]!;
    };
    expect(single({ id: "cursor@1", displayName: "Cursor", icon: "cursor" })).toMatchObject({ color: "#F5F5F7", lightColor: "#13120A" });
    expect(single({ id: "openai@1", displayName: "OpenAI", icon: "openai" })).toMatchObject({ color: "#ECECEC", lightColor: "#0D0D0D" });
    expect("lightColor" in single({ id: "copilot@1", displayName: "Copilot", icon: "copilot" })).toBe(false);
    const plain = single({ id: "mystery@1", displayName: "Mystery", icon: "mystery" });
    expect(plain.color).toBe("#FFFFFF");
    expect("lightColor" in plain).toBe(false);
  });

  it("carries the app's theme for every widget, and nothing while it follows the Mac", () => {
    expect("theme" in glance()).toBe(false);
    expect(glance({ theme: "dark" }).theme).toBe("dark");
    expect(glance({ theme: "light" }).theme).toBe("light");
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

  const codex = (language: "vi" | "en") =>
    buildGlanceResets({ feeds: parseResetFeeds(FEED_FIXTURES.codexResetStatus, FEED_FIXTURES.codexResets), stale: false, now: NOW_GLANCE, language, timeFormat: "auto", theme: "system" });
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
  /**
   * SHA-256 of the documents built from these same inputs, without the quoted announcement: what
   * 0.3.16 (`be761f5`) built, apart from the account header 0.3.20 brought in line with the popup's
   * card: the plan period (`term` on the accounts that have one, `labels.planTerm`), the plan the
   * island now shows by default (`island.shows.plan`) and the rows' `Không có dữ liệu`
   * (`labels.noData`).
   */
  const PINNED: Readonly<Record<string, string>> = {
    defaults: "507d2a501a52d96fb25a64500ac5c5e54f4a3a126c5858095bcb528973950174",
    wings: "a70d17d7fd4da2c9c8cc8fdf11021735b5d1e37be43754c304d7dd2e24c8c00f",
    tuned: "fbd30bf8aa1c0673b4c83461c6a5ba01cc3b7130b86dc04e6ec28169bb3d9337",
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

  it("stays byte for byte what 0.3.16 sent apart from the quoted announcement and the account header, even with a Claude tracker at hand", () => {
    const tracker = claude();
    expect(tracker).not.toBeNull();
    for (const [name, options] of Object.entries(scenarios())) {
      const document = glance(options);
      expect(document.resets?.presentation?.latest, name).toMatchObject({ excerpt: expect.stringMatching(/^GPT-6 Sol and Luna are out\./), url: "https://x.com/thsottiaux/status/2102463847714247142" });
      expect(digest(unquoted(document)), name).toBe(PINNED[name]);
      expect(digest(unquoted(glance({ ...options, claudeResets: tracker }))), name).toBe(PINNED[name]);
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
