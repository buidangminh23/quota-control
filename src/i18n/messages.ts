/**
 * The shape every language catalog implements. Parameterized entries are functions so each language
 * controls word order (English "95% left" vs Vietnamese "Còn 95%").
 */
import type { ErrorCategory, LimitResetResult, UpdateFailureReason, UpdateFailureStage } from "@/lib/types";
import type { PlanTermLeft } from "@/model/planTerm";
import type { SpecialWing } from "@/model/glance";
import type { GlanceContent, IslandLayout, IslandStyle, IslandView, TaskbarDisplay } from "@/model/settings";
import type { PriceMessages, UsageMessages } from "./usageMessages";

export type DisplayModeKey = "used" | "remaining";
export type ResetModeKey = "relative" | "absolute";
export type DeadlineVerb = "resets" | "limit" | "resetExpires";
export type SpendPeriodKey = "today" | "last30" | "last365" | "all";
export type DashboardTabKey = "quota" | "tokens" | "prices" | "benchmark" | "resets";
export type SpendMetricKey = "cost" | "costPerMtok" | "tokens";
export type RingUnitKey = "dollars" | "perMtok" | "billion" | "million" | "thousand" | "tokens";

export type When =
  | { kind: "in"; duration: string }
  | { kind: "today"; time: string }
  | { kind: "tomorrow"; time: string }
  | { kind: "on"; date: string; time: string }
  | { kind: "soon" };

/** The day of a limit's exact restore time, next to its clock time. */
export type RestoreDay = { kind: "today" } | { kind: "tomorrow" } | { kind: "on"; date: Date };

export interface FormatMessages {
  duration(days: number, hours: number, minutes: number): string;
  monthDay(date: Date): string;
  /** A date with its year, e.g. `31/07/2026` / `Jul 31, 2026`. */
  calendarDate(date: Date): string;
  when(when: When): string;
  deadline(verb: DeadlineVerb, when: When): string;
  /** The line under a reset countdown, e.g. `Hồi lại lúc 13:05 · T6 02/10`. */
  restoresAt(time: string, day: RestoreDay): string;
  /** A clock time and its day, e.g. `13:05 · T6 02/10` / `1:05 PM · tomorrow`. */
  timeOnDay(time: string, day: RestoreDay): string;
  expiryListHeader(mode: ResetModeKey): string;
  list(items: string[]): string;
}

export interface MeterMessages {
  displayMode(mode: DisplayModeKey): string;
  headline(value: string, mode: DisplayModeKey): string;
  leftAtReset(percent: number): string;
  usedAtReset(percent: number): string;
  overLimitAtReset(percent: number): string;
  fullAtReset: string;
  limitReached: string;
  spare(percent: number): string;
  notStarted: string;
  freshSessionTooltip: string;
  noData: string;
  /** A session row's title with its window (`5h`), so it reads apart from the weekly limit. */
  sessionTitle(window: string): string;
  dollarLimit(amount: string, noun: string | undefined): string;
  valueWithWord(value: string, word: string): string;
  noUsageInPeriod: string;
  localEstimateNote: string;
  unknownModels(count: number): string;
  outdated: string;
  lastUpdated(duration: string): string;
  refreshTimedOut(seconds: number): string;
  errors: Record<ErrorCategory, string>;
}

export interface DashboardMessages {
  emptyState: string;
  welcomeTitle: string;
  welcomeMessage: string;
  openCustomize: string;
  dismiss: string;
  showMore: string;
  showLess: string;
  refreshing: string;
  hide: string;
  hideProvider(name: string): string;
  starFor(bar: BarKind): string;
  unstar: string;
  refreshProvider(name: string): string;
  customizeEllipsis: string;
  pinLimit(max: number): string;
  peak(readout: string): string;
  tokensReadout(count: string): string;
  otherModels: string;
  inputTokens: string;
  outputTokens: string;
  cacheReadTokens: string;
  cacheWriteTokens: string;
  resetsEmpty: string;
  resetsUnknownExpiries(count: number): string;
  expiringSoon: string;
  noAccountsTitle: string;
  noAccountsMessage: string;
  noAccountsShort: string;
  addAccount: string;
  tabsLabel: string;
  tab(key: DashboardTabKey): string;
  openChat(product: string): string;
  trendRange(days: number, first: string, last: string): string;
  expiryStatus(severity: "normal" | "warning" | "critical"): string;
  unknownPricingWarning: string;
  /** The card header's right corner: time left in the plan's paid period, e.g. `còn 19 ngày`. */
  planTermLeft(left: PlanTermLeft, estimated: boolean): string;
  /** Under it, the day the period ends, e.g. `tới T7 17/10` / `tới ~T4 30/09`; `ended` names the past day alone. */
  planTermDay(day: RestoreDay, estimated: boolean, ended: boolean): string;
  /** Hover notes; `time` is a clock time with its day, `offset` the device zone's GMT offset. */
  planTermStatedNote(time: string, offset: string, checked: string | null): string;
  planTermEndedNote(time: string, offset: string): string;
  planTermEstimateNote(time: string, offset: string, started: string): string;
}

export interface TotalSpendMessages {
  metric(key: SpendMetricKey): string;
  metricMenuLabel: string;
  periodLabel: string;
  period(key: SpendPeriodKey): string;
  empty(key: SpendMetricKey): string;
  onlyIncludes(names: string): string;
  ringUnit(key: RingUnitKey): string;
  costPerMtok(amount: string): string;
  totalCostAria(value: string, count: number): string;
  totalTokensAria(value: string, count: number): string;
  blendedRateAria(value: string, count: number): string;
}

export interface ChromeMessages {
  appName: string;
  identity(name: string, version: string): string;
  updating: string;
  nextUpdateMinutes(minutes: number): string;
  nextUpdateSeconds(seconds: number): string;
  refreshNow: string;
  options: string;
  customize: string;
  settings: string;
  about(name: string): string;
  quit(name: string): string;
  back: string;
  resetProvider(name: string): string;
  resetAll: string;
  resetAllTitle: string;
  resetAllMessage: string;
  resetAllConfirm: string;
  cancel: string;
  accounts: string;
  /** The Options menu's update entry, worded like the tray menu's. */
  checkForUpdates: string;
  installUpdate(version: string): string;
  aboutDescription: string;
  openRepository: string;
  close: string;
}

export interface CustomizeMessages {
  alwaysVisible: string;
  onDemand: string;
  dragHere: string;
  metricCount(count: number): string;
  starred(bar: BarKind): string;
  unstarred(bar: BarKind): string;
  star(bar: BarKind): string;
  unstar: string;
  enable(name: string): string;
  reorder: string;
  settingsLinkTitle: string;
  settingsLinkSubtitle: string;
  customizeLinkTitle: string;
  customizeLinkSubtitle: string;
  undo(platform: PlatformKey): string;
}

export type SettingsSectionKey =
  | "general"
  | "appearance"
  | "usageDisplay"
  | "taskbar"
  | "menuBar"
  | "island"
  | "widget"
  | "notifications"
  | "updates"
  | "advanced";
/** The operating system a string is phrased for. */
export type PlatformKey = "windows" | "linux" | "macos" | "other";
/** Where starred metrics show: the taskbar (Windows, Linux) or the macOS menu bar. */
export type BarKind = "taskbar" | "menuBar";
export type NotificationKey = "almostOut" | "cuttingItClose" | "willRunOut";

export interface SettingsMessages {
  section(key: SettingsSectionKey): string;
  language: string;
  showTotalSpend: string;
  launchAtLogin(platform: PlatformKey): string;
  launchAtLoginError: string;
  globalShortcut: string;
  globalShortcutTooltip: string;
  recordShortcut: string;
  pressShortcut: string;
  clearShortcut: string;
  shortcutNeedsModifier(platform: PlatformKey): string;
  shortcutUnsupported: string;
  shortcutUnavailable: string;
  theme: string;
  themeOption(theme: "system" | "light" | "dark"): string;
  density: string;
  densityOption(density: "regular" | "compact"): string;
  reduceAnimations: string;
  timeFormat: string;
  timeFormatOption(format: "auto" | "12h" | "24h"): string;
  timeZone: string;
  /** The detected zone, e.g. `Giờ Đông Dương · GMT+7`. */
  timeZoneValue(name: string, offset: string): string;
  timeZoneNote(zone: string): string;
  showUsageAs: string;
  resetTimes: string;
  resetTimesOption(mode: "relative" | "absolute"): string;
  alwaysShowPacing: string;
  alwaysShowPacingNote: string;
  barDisplay(bar: BarKind): string;
  taskbarDisplayOption(display: TaskbarDisplay): string;
  barNote(display: TaskbarDisplay, bar: BarKind): string;
  dynamicIsland: string;
  dynamicIslandNote: string;
  desktopWidget: string;
  desktopWidgetNote: string;
  /** The widgets beyond the limit ones and which of them the content settings shape. */
  desktopWidgetKindsNote: string;
  glanceContent: string;
  glanceContentOption(content: GlanceContent): string;
  glanceContentNote(content: GlanceContent): string;
  glanceMetrics: string;
  glanceMetricsNote: string;
  glanceMetricsNone: string;
  glanceShowAccount: string;
  glanceShowPlan: string;
  glanceShowResets: string;
  glanceShowProblems: string;
  glanceShowProblemsNote: string;
  stripValues: string;
  stripValuesOption(count: 1 | 2): string;
  islandStyle: string;
  islandStyleOption(style: IslandStyle): string;
  islandWing(side: "left" | "right"): string;
  islandWingAuto: string;
  /** A wing choice that is not one metric: the soonest limit reset or a Codex reset reading. */
  islandWingSpecial(wing: SpecialWing): string;
  islandLayout: string;
  islandLayoutOption(layout: IslandLayout): string;
  islandLayoutNote(layout: IslandLayout): string;
  /** The heading of the switches that pick what a combined island shows. */
  islandSections: string;
  islandView: string;
  islandViewOption(view: IslandView): string;
  /** `trackerOff`: neither the Reset tab nor reset notifications is on, so the tracker has no data. */
  islandViewNote(view: IslandView, trackerOff: boolean): string;
  islandExpandOnHover: string;
  islandExpandOnHoverNote: string;
  islandAlerts: string;
  islandAlertsNote: string;
  notification(key: NotificationKey): string;
  notificationNote(key: NotificationKey): string;
  notificationsDenied: string;
  allowNotifications: string;
  copied: string;
  resetAllSettings: string;
  resetAllSettingsTitle: string;
  resetAllSettingsMessage: string;
  resetAllSettingsConfirm: string;
}

export interface AccountsMessages {
  connected: string;
  none: string;
  /** `cliName`: Claude Code or the Codex CLI, whose login a `cli` account follows. */
  mode(mode: "shared_cli" | "managed_oauth" | "cli", cliName: string): string;
  status(kind: "ok" | "refreshing" | "error" | "unknown"): string;
  add: string;
  signInWithGoogle: string;
  signInWithGitHub: string;
  signInNote(brand: string): string;
  /** Under a service's sign-in button: where the page opens and what to choose there. */
  serviceSignInNote(service: string, method: "google" | "github"): string;
  /** The Add Account panel's picker between a service's ways to connect. */
  methodsLabel: string;
  /** The plus button of an Add Account row that opens the provider's sign-in page at once. */
  quickSignIn(service: string, method: string): string;
  /** The plus button of an Add Account row whose provider connects with a key or its own app. */
  quickAdd(service: string): string;
  /** A device sign-in's code, which the user types on the page that opened. */
  userCodeLabel: string;
  copyCode: string;
  userCodeNote: string;
  cliNote: string;
  starting: string;
  waiting(brand: string, browser: "chrome" | "default"): string;
  waitingNote: string;
  cancel: string;
  openSignInPage: string;
  connectedNotice(brand: string): string;
  notConnectedNotice(brand: string): string;
  loginFailed(brand: string, detail: string): string;
  remove: string;
  removeTitle(label: string): string;
  removeMessage: string;
  removeConfirm: string;
  failed(detail: string): string;
  chatOpenFailed: string;
  /** Where a service card reads its credentials: an app's login, an environment variable, a saved key, or a sign-in made here. */
  serviceSource(kind: "login" | "env" | "key" | "google" | "github", detail: string): string;
  /** How the Add Account list says a provider is added: a Google or GitHub sign-in, an API key or a cookie. */
  kindGoogle: string;
  kindGitHub: string;
  kindApiKey: string;
  kindCookie: string;
  /** A service that only reads `app`'s login on this computer. */
  appLoginNote(service: string, app: string): string;
  /** A service that takes a key and also reads `app`'s login. */
  alsoAppLogin(app: string): string;
  searchService: string;
  noServiceMatch: string;
  keyLabel: string;
  keyPlaceholder: string;
  /** The key field's hint for a value other than an API key (a session cookie). */
  pasteValue(what: string): string;
  getKey: string;
  /** The button that opens the page a session cookie or another kind of key is copied from. */
  openServicePage(service: string): string;
  saveKey: string;
  keySaved(service: string): string;
  keyStoredNote: string;
  keyEnvNote(variables: string): string;
  /** How to copy one session cookie's value from the browser. */
  cookieNote: string;
  /** How to copy a request's whole Cookie header from the browser. */
  cookieHeaderNote: string;
  changeService: string;
  hiddenNote: string;
  removeKeyTitle(label: string): string;
  removeKeyMessage: string;
  removeKeyConfirm: string;
}

export interface StripMessages {
  tooltipEmpty: string;
}

/** What the macOS Dynamic Island and desktop widget say around the readings. */
export interface GlanceMessages {
  /** What an empty island or widget says, worded for its content choice. */
  empty: Record<GlanceContent, string>;
  /** An account whose card has no readings and no error to explain it. */
  noData: string;
  /** After a count of accounts left out: `+2 tài khoản khác`, `+2 more`. */
  more: string;
  updated: string;
  resetsIn: string;
  resetting: string;
  open: string;
  notRunning: string;
  /** Unit suffixes the island's countdowns use, written like `format.duration` (`4 ngày 3 giờ`, `4d 3h`). */
  units: { day: string; hour: string; minute: string };
  /** What a reset widget or island section says while the Reset tab and reset notifications are off. */
  resetsOff: string;
  /** The heading of the next limits to come back, and what it says when none has a reset time. */
  upcoming: string;
  upcomingEmpty: string;
  /** The label of the wing counting the time since the last Codex reset. */
  sinceReset: string;
  /** A wing counting down to a moment, `span` being the time left: `sau 2 giờ`, `in 2h`. */
  wingIn(span: string): string;
  /** A wing counting the time since a moment: `đã 2 ngày`, `2d ago`. */
  wingSince(span: string): string;
  /** A month on the reset calendar of the island and widgets, 0 for January: `Th9`, `Sep`; worded
   * apart from the weekday names beside it (`T2`…`CN`). */
  calendarMonth(month: number): string;
}

/** The update dialog and the Settings "App Updates" section. */
export interface UpdateMessages {
  availableTitle: string;
  availableMessage(name: string, version: string): string;
  install: string;
  whatsNew: string;
  later: string;
  hide: string;
  close: string;
  updatedTitle(version: string): string;
  updatedMessage(name: string, from: string, to: string): string;
  checking: string;
  upToDateTitle: string;
  upToDateMessage(name: string, version: string): string;
  downloading(version: string): string;
  installing(version: string): string;
  installingNote(platform: PlatformKey): string;
  failedTitle(stage: UpdateFailureStage): string;
  failure(reason: UpdateFailureReason): string;
  retry: string;
  automaticChecks: string;
  automaticChecksNote: string;
  version(version: string): string;
  checkNow: string;
  lastChecked(time: string): string;
  availableStatus(version: string): string;
  unsupported: string;
  openReleases: string;
}

export interface NotifyMessages {
  title(provider: string, metric: string): string;
  almostOut(leftPercent: number, reset: string | null): string;
  cuttingItClose(projectedPercent: number): string;
  willRunOut(eta: string | null): string;
}

export interface LimitResetMessages {
  redeem: string;
  redeeming: string;
  confirmTitle: string;
  /** `expiry` is when the reset about to be spent expires, e.g. `18:37 ngày mai`. */
  confirmMessage(expiry: string | null): string;
  confirm: string;
  result(result: LimitResetResult, errors: Record<ErrorCategory, string>): string;
  failed: string;
}

export interface Messages {
  language: string;
  format: FormatMessages;
  meter: MeterMessages;
  dashboard: DashboardMessages;
  totalSpend: TotalSpendMessages;
  usage: UsageMessages;
  prices: PriceMessages;
  chrome: ChromeMessages;
  customize: CustomizeMessages;
  settings: SettingsMessages;
  accounts: AccountsMessages;
  strip: StripMessages;
  glance: GlanceMessages;
  update: UpdateMessages;
  notify: NotifyMessages;
  limitReset: LimitResetMessages;
  /** Backend English text → this language; `undefined` keeps the source text. */
  term(text: string): string | undefined;
}
