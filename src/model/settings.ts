/**
 * The popup's preferences, persisted in the shared `settings` document. Port of upstream's scattered
 * `@AppStorage` settings (Appearance, Density, Time Format, Usage Display, Total Spend, Notifications,
 * Privacy, Logging) as one typed record.
 *
 * The document is shared with the Rust core: it reads `language` for native menus and
 * `automaticUpdateChecks` for its update schedule, and owns `enabledProviders`. Saving always merges
 * onto the stored document, so keys the popup does not own survive untouched.
 */
import { DEFAULT_LANGUAGE, isLanguage, type Language } from "@/i18n";
import type { ResetDisplayMode, TimeFormat, TotalSpendMetric } from "./format";
import type { DisplayMode } from "./widgetData";

export type ThemeSetting = "system" | "light" | "dark";
export type DensitySetting = "regular" | "compact";
/** How starred metrics render on the taskbar: provider mark plus values, or the compact bars glyph. */
export type IconStyle = "text" | "bars";
/** The Taskbar picker: the starred numbers as a strip, the bars glyph in the tray icon, or only the app icon. */
export type TaskbarDisplay = IconStyle | "icon";
/** A span of the usage history, ending today: one day, 30 days, 365 days or everything recorded. */
export type TotalSpendPeriod = "today" | "last30" | "last365" | "all";
export const TOTAL_SPEND_PERIODS: readonly TotalSpendPeriod[] = ["today", "last30", "last365", "all"];
/**
 * The dashboard's tabs, left to right: account limits, the token use, the official price list, model
 * quality and benchmarks, then Codex resets.
 */
export type DashboardTab = "quota" | "tokens" | "prices" | "benchmark" | "resets";
export const DASHBOARD_TABS: readonly DashboardTab[] = ["quota", "tokens", "prices", "benchmark", "resets"];
/** The Token tab's views, left to right. */
export type TokenView = "overview" | "history" | "charts" | "projects";
export const TOKEN_VIEWS: readonly TokenView[] = ["overview", "history", "charts", "projects"];
/** What the Biểu đồ view compares: time buckets (stacked by source) or a ranking of models or projects. */
export type TokenChart = "day" | "month" | "year" | "model" | "project";
export const TOKEN_CHARTS: readonly TokenChart[] = ["day", "month", "year", "model", "project"];
export type TokenChartMetric = "tokens" | "cost";
export const TOKEN_CHART_METRICS: readonly TokenChartMetric[] = ["tokens", "cost"];
/** How the Token ring splits the total. */
export type TokenRingBy = "source" | "model" | "project";
export const TOKEN_RING_BYS: readonly TokenRingBy[] = ["source", "model", "project"];
export type PriceProvider = "claude" | "openai";
export const PRICE_PROVIDERS: readonly PriceProvider[] = ["claude", "openai"];
/** Processing tiers on the official price pages; each provider lists only some of them. */
export type PriceTier = "standard" | "batch" | "flex" | "fast";
export const PRICE_TIERS: readonly PriceTier[] = ["standard", "batch", "flex", "fast"];
export type PriceCurrency = "vnd" | "usd";

/** What a macOS glance surface lists: the Hạn mức cards, the starred metrics, or a hand-picked set. */
export type GlanceContent = "dashboard" | "starred" | "custom";
export const GLANCE_CONTENTS: readonly GlanceContent[] = ["dashboard", "starred", "custom"];
/** How the closed Dynamic Island shows a reading beside the notch. */
export type IslandStyle = "percent" | "ring" | "bar";
export const ISLAND_STYLES: readonly IslandStyle[] = ["percent", "ring", "bar"];
/** At most this many metrics can be picked by hand; more would not fit any surface. */
export const MAX_GLANCE_METRICS = 64;

/** What the desktop widget (and the open Dynamic Island) lists, and how each account reads. */
export interface GlanceSurfaceSettings {
  content: GlanceContent;
  /** Metric ids shown when `content` is `custom`. */
  metrics: string[];
  showAccount: boolean;
  showPlan: boolean;
  showResets: boolean;
  /** Accounts without readings (signed out, session expired) still get a line saying why. */
  showProblems: boolean;
}

/** What the taskbar strip (the macOS menu bar item) lists. */
export interface StripSettings {
  content: GlanceContent;
  /** Metric ids shown when `content` is `custom`. */
  metrics: string[];
  /** Readings per account: two stacked, like upstream's menu bar, or one. */
  values: 1 | 2;
}

export interface IslandSettings extends GlanceSurfaceSettings {
  style: IslandStyle;
  /** The metrics beside the notch, left then right; an empty slot takes the content's next reading. */
  wings: [string, string];
  /** Open the details when the pointer rests on the island; otherwise a click opens them. */
  expandOnHover: boolean;
  /** Open the island for a few seconds when a limit runs low or comes back. */
  alerts: boolean;
}

export interface NotificationSettings {
  /** A metric crosses under 10% remaining. */
  almostOut: boolean;
  /** A metric is projected to finish the period close to its limit. */
  cuttingItClose: boolean;
  /** A metric is projected to run out before it resets. */
  willRunOut: boolean;
}

export interface AppSettings {
  language: Language;
  theme: ThemeSetting;
  density: DensitySetting;
  reduceAnimations: boolean;
  timeFormat: TimeFormat;
  iconStyle: IconStyle;
  showTaskbarStrip: boolean;
  strip: StripSettings;
  /** macOS: readings around the notch (a pill in the menu bar on screens without one). */
  dynamicIsland: boolean;
  island: IslandSettings;
  /** macOS: what the desktop widgets list. */
  widget: GlanceSurfaceSettings;
  /** Whether the dashboard has its Token tab (upstream "Show Total Spend"). */
  showTotalSpend: boolean;
  /** Whether the dashboard has its Benchmark tab (model quality, public leaderboards, comparison). */
  showBenchmarkTab: boolean;
  /** Whether the dashboard has its Codex reset tab. */
  showResetsTab: boolean;
  /** Notify when a Codex reset is announced or scheduled. */
  notifyCodexResets: boolean;
  /** The dashboard tab the popup opens on. */
  dashboardTab: DashboardTab;
  totalSpendPeriod: TotalSpendPeriod;
  totalSpendMetric: TotalSpendMetric;
  tokenView: TokenView;
  tokenRingBy: TokenRingBy;
  tokenChart: TokenChart;
  tokenChartMetric: TokenChartMetric;
  /** The span the model and project charts rank over. */
  tokenChartPeriod: TotalSpendPeriod;
  projectPeriod: TotalSpendPeriod;
  priceProvider: PriceProvider;
  priceTier: PriceTier;
  /** Prices in đồng (Vietnamese only, at the Vietcombank rate) or as published in dollars. */
  priceCurrency: PriceCurrency;
  displayMode: DisplayMode;
  resetDisplayMode: ResetDisplayMode;
  alwaysShowPacing: boolean;
  notifications: NotificationSettings;
  customizeHintDismissed: boolean;
  /** The "connect an account" hint was closed; it stays closed even while no account is connected. */
  accountsHintDismissed: boolean;
  /** The core looks for a new release at launch and every six hours (upstream "Update Automatically"). */
  automaticUpdateChecks: boolean;
}

export const DEFAULT_SETTINGS: AppSettings = {
  language: DEFAULT_LANGUAGE,
  theme: "system",
  density: "regular",
  reduceAnimations: false,
  timeFormat: "auto",
  iconStyle: "text",
  showTaskbarStrip: true,
  strip: { content: "dashboard", metrics: [], values: 2 },
  dynamicIsland: true,
  island: {
    content: "dashboard",
    metrics: [],
    showAccount: true,
    showPlan: false,
    showResets: true,
    showProblems: true,
    style: "percent",
    wings: ["", ""],
    expandOnHover: true,
    alerts: true,
  },
  widget: {
    content: "dashboard",
    metrics: [],
    showAccount: true,
    showPlan: true,
    showResets: true,
    showProblems: true,
  },
  showTotalSpend: true,
  showBenchmarkTab: true,
  showResetsTab: true,
  notifyCodexResets: true,
  dashboardTab: "quota",
  totalSpendPeriod: "today",
  totalSpendMetric: "tokens",
  tokenView: "overview",
  tokenRingBy: "model",
  tokenChart: "day",
  tokenChartMetric: "tokens",
  tokenChartPeriod: "last30",
  projectPeriod: "last30",
  priceProvider: "claude",
  priceTier: "standard",
  priceCurrency: "vnd",
  displayMode: "remaining",
  resetDisplayMode: "relative",
  alwaysShowPacing: false,
  notifications: { almostOut: false, cuttingItClose: false, willRunOut: false },
  customizeHintDismissed: false,
  accountsHintDismissed: false,
  automaticUpdateChecks: true,
};

/** Keys the core owns inside the shared document; the popup never writes them from its own copy. */
const CORE_OWNED_KEYS = ["enabledProviders"] as const;

function asRecord(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function oneOf<T extends string>(value: unknown, options: readonly T[], fallback: T): T {
  return typeof value === "string" && (options as readonly string[]).includes(value) ? (value as T) : fallback;
}

function flag(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function metricIds(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const ids = value.filter((id): id is string => typeof id === "string" && id.length > 0 && id.length <= 512);
  return [...new Set(ids)].slice(0, MAX_GLANCE_METRICS);
}

function parseSurface(value: unknown, defaults: GlanceSurfaceSettings): GlanceSurfaceSettings {
  const stored = asRecord(value);
  return {
    content: oneOf(stored.content, GLANCE_CONTENTS, defaults.content),
    metrics: metricIds(stored.metrics),
    showAccount: flag(stored.showAccount, defaults.showAccount),
    showPlan: flag(stored.showPlan, defaults.showPlan),
    showResets: flag(stored.showResets, defaults.showResets),
    showProblems: flag(stored.showProblems, defaults.showProblems),
  };
}

function parseStrip(value: unknown, defaults: StripSettings): StripSettings {
  const stored = asRecord(value);
  return {
    content: oneOf(stored.content, GLANCE_CONTENTS, defaults.content),
    metrics: metricIds(stored.metrics),
    values: stored.values === 1 || stored.values === 2 ? stored.values : defaults.values,
  };
}

function parseIsland(value: unknown, defaults: IslandSettings): IslandSettings {
  const stored = asRecord(value);
  const wings = Array.isArray(stored.wings) ? stored.wings : [];
  const wing = (index: number) => (typeof wings[index] === "string" && wings[index].length <= 512 ? (wings[index] as string) : "");
  return {
    ...parseSurface(value, defaults),
    style: oneOf(stored.style, ISLAND_STYLES, defaults.style),
    wings: [wing(0), wing(1)],
    expandOnHover: flag(stored.expandOnHover, defaults.expandOnHover),
    alerts: flag(stored.alerts, defaults.alerts),
  };
}

/** Read the stored document; every missing or invalid key falls back to its default on its own. */
export function parseSettings(raw: unknown): AppSettings {
  const stored = asRecord(raw);
  const notifications = asRecord(stored.notifications);
  const defaults = DEFAULT_SETTINGS;
  return {
    language: isLanguage(stored.language) ? stored.language : defaults.language,
    theme: oneOf(stored.theme, ["system", "light", "dark"], defaults.theme),
    density: oneOf(stored.density, ["regular", "compact"], defaults.density),
    reduceAnimations: flag(stored.reduceAnimations, defaults.reduceAnimations),
    timeFormat: oneOf(stored.timeFormat, ["auto", "12h", "24h"], defaults.timeFormat),
    iconStyle: oneOf(stored.iconStyle, ["text", "bars"], defaults.iconStyle),
    showTaskbarStrip: flag(stored.showTaskbarStrip, defaults.showTaskbarStrip),
    strip: parseStrip(stored.strip, defaults.strip),
    dynamicIsland: flag(stored.dynamicIsland, defaults.dynamicIsland),
    island: parseIsland(stored.island, defaults.island),
    widget: parseSurface(stored.widget, defaults.widget),
    showTotalSpend: flag(stored.showTotalSpend, defaults.showTotalSpend),
    showBenchmarkTab: flag(stored.showBenchmarkTab, defaults.showBenchmarkTab),
    showResetsTab: flag(stored.showResetsTab, defaults.showResetsTab),
    notifyCodexResets: flag(stored.notifyCodexResets, defaults.notifyCodexResets),
    dashboardTab: oneOf(stored.dashboardTab, DASHBOARD_TABS, defaults.dashboardTab),
    totalSpendPeriod: oneOf(stored.totalSpendPeriod, TOTAL_SPEND_PERIODS, defaults.totalSpendPeriod),
    totalSpendMetric: oneOf(stored.totalSpendMetric, ["cost", "costPerMtok", "tokens"], defaults.totalSpendMetric),
    tokenView: oneOf(stored.tokenView, TOKEN_VIEWS, defaults.tokenView),
    tokenRingBy: oneOf(stored.tokenRingBy, TOKEN_RING_BYS, defaults.tokenRingBy),
    tokenChart: oneOf(stored.tokenChart, TOKEN_CHARTS, defaults.tokenChart),
    tokenChartMetric: oneOf(stored.tokenChartMetric, TOKEN_CHART_METRICS, defaults.tokenChartMetric),
    tokenChartPeriod: oneOf(stored.tokenChartPeriod, TOTAL_SPEND_PERIODS, defaults.tokenChartPeriod),
    projectPeriod: oneOf(stored.projectPeriod, TOTAL_SPEND_PERIODS, defaults.projectPeriod),
    priceProvider: oneOf(stored.priceProvider, PRICE_PROVIDERS, defaults.priceProvider),
    priceTier: oneOf(stored.priceTier, PRICE_TIERS, defaults.priceTier),
    priceCurrency: oneOf(stored.priceCurrency, ["vnd", "usd"], defaults.priceCurrency),
    displayMode: oneOf(stored.displayMode, ["used", "remaining"], defaults.displayMode),
    resetDisplayMode: oneOf(stored.resetDisplayMode, ["relative", "absolute"], defaults.resetDisplayMode),
    alwaysShowPacing: flag(stored.alwaysShowPacing, defaults.alwaysShowPacing),
    notifications: {
      almostOut: flag(notifications.almostOut, defaults.notifications.almostOut),
      cuttingItClose: flag(notifications.cuttingItClose, defaults.notifications.cuttingItClose),
      willRunOut: flag(notifications.willRunOut, defaults.notifications.willRunOut),
    },
    customizeHintDismissed: flag(stored.customizeHintDismissed, defaults.customizeHintDismissed),
    accountsHintDismissed: flag(stored.accountsHintDismissed, defaults.accountsHintDismissed),
    automaticUpdateChecks: flag(stored.automaticUpdateChecks, defaults.automaticUpdateChecks),
  };
}

/**
 * The document to save: the freshly stored one with the popup's keys replaced. Core-owned keys are
 * taken from `stored`, never from an older in-memory copy, and `enabledProviders` is narrowed to
 * providers the core still knows (the core rejects a save naming an unknown provider).
 */
export function mergeSettingsDocument(
  stored: unknown,
  settings: AppSettings,
  knownProviderIds: ReadonlySet<string>,
): Record<string, unknown> {
  const base = { ...asRecord(stored) };
  for (const key of CORE_OWNED_KEYS) {
    if (!(key in base)) continue;
    const ids = base[key];
    if (Array.isArray(ids)) base[key] = ids.filter((id): id is string => typeof id === "string" && knownProviderIds.has(id));
  }
  return {
    ...base,
    ...settings,
    notifications: { ...settings.notifications },
    strip: { ...settings.strip, metrics: [...settings.strip.metrics] },
    island: { ...settings.island, metrics: [...settings.island.metrics], wings: [...settings.island.wings] },
    widget: { ...settings.widget, metrics: [...settings.widget.metrics] },
  };
}

/** The providers the core refreshes, or `null` when the document leaves them at the default (all). */
export function enabledProvidersOf(raw: unknown): string[] | null {
  const ids = asRecord(raw).enabledProviders;
  return Array.isArray(ids) ? ids.filter((id): id is string => typeof id === "string") : null;
}

export function anyNotificationEnabled(settings: Pick<AppSettings, "notifications">): boolean {
  const { almostOut, cuttingItClose, willRunOut } = settings.notifications;
  return almostOut || cuttingItClose || willRunOut;
}

/** The Taskbar picker's value. Where the core hosts no strip, numbers fall back to the bars glyph. */
export function taskbarDisplayOf(settings: Pick<AppSettings, "showTaskbarStrip" | "iconStyle">, stripSupported: boolean): TaskbarDisplay {
  if (!settings.showTaskbarStrip) return "icon";
  return stripSupported ? settings.iconStyle : "bars";
}

/** What a Taskbar picker choice stores: "icon" turns the strip off and keeps the last style. */
export function taskbarDisplayPatch(display: TaskbarDisplay): Partial<Pick<AppSettings, "showTaskbarStrip" | "iconStyle">> {
  return display === "icon" ? { showTaskbarStrip: false } : { showTaskbarStrip: true, iconStyle: display };
}
