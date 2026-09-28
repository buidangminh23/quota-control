import { fixtureCatalog, fixtureSnapshots } from "@/lib/fixtures";
import type { ProviderSnapshot } from "@/lib/types";
import { buildGlance, CODEX_RESETS_PROVIDER_ID, glanceMetric, GLANCE_VERSION, type GlanceAlert, type GlanceResets, type GlanceWingChoice } from "./glance";
import { glanceGroups, reconcileLayout } from "./layout";
import { barKind, platformKey } from "./platform";
import { cardIdentity } from "./providerText";
import { DEFAULT_SETTINGS, type GlanceContent, type IslandSettings } from "./settings";
import { makeWidget, NOW, resetsAt, WEEK_SECONDS } from "./testHelpers";
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
  markArt?: Readonly<Record<string, string>>;
}

function glance({ display = DEFAULT_DISPLAY, data = snapshots, alert = null, island = {}, widget = {}, wings = [null, null], hour12 = null, resets = null, markArt }: Options = {}) {
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

describe("island sections and labels", () => {
  it("opens the island on exactly the chosen view, never the reset tracker mixed into the limits", () => {
    expect(glance().island.sections).toEqual({ quota: true, resets: false, upcoming: false });
    expect(glance({ island: { view: "resets" } }).island.sections).toEqual({ quota: false, resets: true, upcoming: false });
    expect(glance({ island: { view: "upcoming" } }).island.sections).toEqual({ quota: false, resets: false, upcoming: true });
    expect(glance({ island: { view: "resets", sections: { quota: true, resets: true, upcoming: true } } }).island.sections).toEqual({ quota: false, resets: true, upcoming: false });
  });

  it("shows every switched-on view together when combined, as a copy of the settings", () => {
    const sections = { quota: true, resets: true, upcoming: false };
    const document = glance({ island: { layout: "combined", view: "upcoming", sections } });
    expect(document.island.sections).toEqual({ quota: true, resets: true, upcoming: false });
    sections.upcoming = true;
    expect(document.island.sections.upcoming).toBe(false);
  });

  it("carries the reset and coming-back labels", () => {
    const document = glance();
    expect(document.labels).toMatchObject({
      resetsOff: "Bật tab Reset hoặc thông báo reset trong Quota Control để xem dự báo.",
      upcoming: "Sắp đặt lại",
      upcomingEmpty: "Chưa có hạn mức nào có giờ đặt lại.",
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
