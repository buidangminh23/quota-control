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
/** The dashboard's tabs, left to right: account limits, the token use, then the official price list. */
export type DashboardTab = "quota" | "tokens" | "prices";
export const DASHBOARD_TABS: readonly DashboardTab[] = ["quota", "tokens", "prices"];
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
  /** Whether the dashboard has its Token tab (upstream "Show Total Spend"). */
  showTotalSpend: boolean;
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
  showTotalSpend: true,
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
    showTotalSpend: flag(stored.showTotalSpend, defaults.showTotalSpend),
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
  return { ...base, ...settings, notifications: { ...settings.notifications } };
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
