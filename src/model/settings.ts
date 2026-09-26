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
export type TotalSpendPeriod = "today" | "yesterday" | "last30";
/** The dashboard's tabs, left to right: account limits, then the total token use. */
export type DashboardTab = "quota" | "tokens";
export const DASHBOARD_TABS: readonly DashboardTab[] = ["quota", "tokens"];

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
    totalSpendPeriod: oneOf(stored.totalSpendPeriod, ["today", "yesterday", "last30"], defaults.totalSpendPeriod),
    totalSpendMetric: oneOf(stored.totalSpendMetric, ["cost", "costPerMtok", "tokens"], defaults.totalSpendMetric),
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
