/**
 * The shape every language catalog implements. Parameterized entries are functions so each language
 * controls word order (English "95% left" vs Vietnamese "Còn 95%").
 */
import type { ErrorCategory } from "@/lib/types";

export type DisplayModeKey = "used" | "remaining";
export type ResetModeKey = "relative" | "absolute";
export type DeadlineVerb = "resets" | "limit" | "resetExpires";
export type SpendPeriodKey = "today" | "yesterday" | "last30";
export type SpendMetricKey = "cost" | "costPerMtok" | "tokens";
export type RingUnitKey = "dollars" | "perMtok" | "billion" | "million" | "thousand" | "tokens";

export type When =
  | { kind: "in"; duration: string }
  | { kind: "today"; time: string }
  | { kind: "tomorrow"; time: string }
  | { kind: "on"; date: string; time: string }
  | { kind: "soon" };

export interface FormatMessages {
  duration(days: number, hours: number, minutes: number): string;
  monthDay(date: Date): string;
  when(when: When): string;
  deadline(verb: DeadlineVerb, when: When): string;
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
  copyScreenshot(name: string): string;
  copiedToClipboard: string;
  hide: string;
  hideProvider(name: string): string;
  starForTaskbar: string;
  unstar: string;
  refreshProvider(name: string): string;
  customizeEllipsis: string;
  shareScreenshot: string;
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
  localUsageTitle(brand: string): string;
  noAccountsTitle: string;
  noAccountsMessage: string;
  addAccount: string;
  openChat(product: string): string;
  trendRange(days: number, first: string, last: string): string;
  expiryStatus(severity: "normal" | "warning" | "critical"): string;
  unknownPricingWarning: string;
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
  noEnabledProviders: string;
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
  shareScreenshot: string;
  copiedToClipboard: string;
  copyFailed: string;
  aboutDescription: string;
  openRepository: string;
  close: string;
}

export interface CustomizeMessages {
  alwaysVisible: string;
  onDemand: string;
  dragHere: string;
  metricCount(count: number): string;
  starred: string;
  unstarred: string;
  star: string;
  unstar: string;
  enable(name: string): string;
  reorder: string;
  settingsLinkTitle: string;
  settingsLinkSubtitle: string;
  customizeLinkTitle: string;
  customizeLinkSubtitle: string;
  undo: string;
}

export type SettingsSectionKey = "general" | "appearance" | "usageDisplay" | "taskbar" | "notifications" | "advanced";
export type NotificationKey = "almostOut" | "cuttingItClose" | "willRunOut";

export interface SettingsMessages {
  section(key: SettingsSectionKey): string;
  language: string;
  showTotalSpend: string;
  launchAtLogin(platform: "windows" | "linux" | "other"): string;
  launchAtLoginError: string;
  iconStyle: string;
  iconStyleOption(style: "text" | "bars"): string;
  theme: string;
  themeOption(theme: "system" | "light" | "dark"): string;
  density: string;
  densityOption(density: "regular" | "compact"): string;
  reduceAnimations: string;
  timeFormat: string;
  timeFormatOption(format: "auto" | "12h" | "24h"): string;
  showUsageAs: string;
  resetTimes: string;
  resetTimesOption(mode: "relative" | "absolute"): string;
  alwaysShowPacing: string;
  alwaysShowPacingNote: string;
  showOnTaskbar: string;
  taskbarNote(supported: boolean): string;
  notification(key: NotificationKey): string;
  notificationNote(key: NotificationKey): string;
  notificationsDenied: string;
  allowNotifications: string;
  copyLogPath: string;
  revealLog(platform: "windows" | "linux" | "other"): string;
  logActionFailed: string;
  copied: string;
  resetAllSettings: string;
  resetAllSettingsTitle: string;
  resetAllSettingsMessage: string;
  resetAllSettingsConfirm: string;
}

export interface AccountsMessages {
  connected: string;
  none: string;
  mode(mode: "shared_cli" | "managed_oauth"): string;
  status(kind: "ok" | "refreshing" | "error" | "unknown"): string;
  add: string;
  labelPlaceholder: string;
  signIn(brand: string): string;
  importCurrent(brand: string): string;
  waitingForBrowser: string;
  pasteCode: string;
  codePlaceholder: string;
  complete: string;
  cancel: string;
  openSignInPage: string;
  remove: string;
  removeTitle(label: string): string;
  removeMessage: string;
  removeConfirm: string;
  failed(detail: string): string;
  added(label: string): string;
  chats: string;
  chatsNote: string;
  chatsAuthNote: string;
  newChat(product: string): string;
  noChats: string;
  open: string;
  chatOpenFailed: string;
  createdOn(date: string): string;
}

export interface StripMessages {
  tooltipEmpty: string;
}

export interface NotifyMessages {
  title(provider: string, metric: string): string;
  almostOut(leftPercent: number, reset: string | null): string;
  cuttingItClose(projectedPercent: number): string;
  willRunOut(eta: string | null): string;
}

export interface Messages {
  language: string;
  format: FormatMessages;
  meter: MeterMessages;
  dashboard: DashboardMessages;
  totalSpend: TotalSpendMessages;
  chrome: ChromeMessages;
  customize: CustomizeMessages;
  settings: SettingsMessages;
  accounts: AccountsMessages;
  strip: StripMessages;
  notify: NotifyMessages;
  /** Backend English text → this language; `undefined` keeps the source text. */
  term(text: string): string | undefined;
}
