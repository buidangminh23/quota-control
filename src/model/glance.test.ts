import { fixtureCatalog, fixtureSnapshots } from "@/lib/fixtures";
import type { ProviderSnapshot, WidgetDescriptor } from "@/lib/types";
import { buildGlance, glanceMetric, GLANCE_VERSION, type GlanceAlert } from "./glance";
import { glanceGroups, reconcileLayout } from "./layout";
import { barKind, platformKey } from "./platform";
import { cardIdentity } from "./providerText";
import { DEFAULT_SETTINGS, type GlanceContent, type IslandSettings } from "./settings";
import { makeWidget, NOW, resetsAt, WEEK_SECONDS } from "./testHelpers";
import { DEFAULT_DISPLAY, widgetDataFor, type DisplayOptions } from "./widgetData";

const FETCHED = Date.UTC(2026, 8, 26, 3);
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
  wings?: [WidgetDescriptor | null, WidgetDescriptor | null];
  hour12?: boolean | null;
}

function glance({ display = DEFAULT_DISPLAY, data = snapshots, alert = null, island = {}, widget = {}, wings = [null, null], hour12 = null }: Options = {}) {
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
    now: new Date(FETCHED + 60_000),
  });
}

const ids = (list: readonly { id: string }[]) => list.map((entry) => entry.id);

describe("glance document", () => {
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
