/**
 * Wire types shared with the Rust core (`crates/uc-core/src/model`). Field names match the serde
 * output exactly; upstream OpenUsage `Codable` keys are preserved where they differ from camelCase
 * (`providerID`, `costUSD`, …). Dates arrive as ISO-8601 strings.
 */

import type { NativeStrip } from "@/strip/native";

export type MetricKind = "percent" | "dollars" | "count";

export type ProgressFormat = { kind: "percent" } | { kind: "dollars" } | { kind: "count"; suffix: string };

export interface MetricValue {
  number: number;
  kind: MetricKind;
  label?: string;
  estimated: boolean;
}

export interface MetricChartPoint {
  value: number;
  label: string;
  valueLabel?: string;
}

export interface ModelUsageVariant {
  model: string;
  totalTokens: number;
  costUSD?: number;
}

export interface ModelUsageEntry {
  model: string;
  totalTokens: number;
  costUSD?: number;
  variants?: ModelUsageVariant[];
  tokenUsage?: TokenUsage;
}

export interface ModelUsageBreakdown {
  totalTokens: number;
  totalCostUSD?: number;
  models: ModelUsageEntry[];
  sourceNote: string;
  tokenUsage?: TokenUsage;
}

export interface TextLine {
  type: "text";
  label: string;
  value: string;
  colorHex?: string;
  subtitle?: string;
}

export interface ChartLine {
  type: "chart";
  label: string;
  points: MetricChartPoint[];
  note?: string;
}

export interface ValuesLine {
  type: "values";
  label: string;
  values: MetricValue[];
  colorHex?: string;
  expiriesAt?: string[];
  unknownModels?: string[];
  modelBreakdown?: ModelUsageBreakdown;
}

export interface ProgressLine {
  type: "progress";
  label: string;
  used: number;
  limit: number;
  format: ProgressFormat;
  resetsAt?: string;
  periodDurationMs?: number;
  colorHex?: string;
}

export interface BadgeLine {
  type: "badge";
  label: string;
  text: string;
  colorHex?: string;
  subtitle?: string;
}

export type MetricLine = TextLine | ChartLine | ValuesLine | ProgressLine | BadgeLine;

export const ERROR_BADGE_LABEL = "Error";

export interface TokenUsage {
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  cacheCreationInputTokens: number;
}

export interface DailyUsageEntry {
  date: string;
  totalTokens: number;
  costUSD?: number;
  tokenUsage?: TokenUsage;
}

export interface DailyModelUsageEntry {
  date: string;
  models: ModelUsageEntry[];
}

export interface ProviderUsageHistory {
  series: { daily: DailyUsageEntry[] };
  modelUsage?: { daily: DailyModelUsageEntry[] };
  unknownModelsByDay: Record<string, string[]>;
  fallbackPricingModelsByDay?: Record<string, string[]>;
}

export type ErrorCategory =
  | "not_logged_in"
  | "auth_expired"
  | "auth_invalid"
  | "credential_access"
  | "network"
  | "decoding"
  | "http_4xx"
  | "http_5xx"
  | "rate_limited"
  | "not_available"
  | "other";

/** What the provider says about the plan's current paid period. */
export type PlanTerm =
  /** The provider states when the period ends (ChatGPT's login carries it). */
  | { basis: "stated"; endsAt: string; checkedAt?: string }
  /** A monthly estimate from a verified paid subscription, bounded by its last confirmation. */
  | { basis: "monthlyFrom"; startedAt: string; checkedAt?: string };

export interface ProviderSnapshot {
  providerID: string;
  displayName: string;
  plan?: string;
  planCheckedAt?: string;
  planTerm?: PlanTerm;
  /** The account's email as the provider's API reports it. */
  account?: string;
  lines: MetricLine[];
  refreshedAt: string;
  usageUnavailable?: boolean;
  usageHistory?: ProviderUsageHistory;
  warning?: string;
  errorCategory?: ErrorCategory;
}

export interface ProviderLink {
  label: string;
  url: string;
}

export interface Provider {
  id: string;
  displayName: string;
  /** Provider family whose mark and brand color the card uses (`claude` for `claude@…`). */
  icon: string;
  links?: ProviderLink[];
}

export type SessionStartSignal = "zeroUsage" | "missingResetDate";

export interface WidgetTemplate {
  title: string;
  kind: MetricKind;
  limit?: number;
  countSuffix?: string;
  valuePrefix?: string;
  limitNoun?: string;
  unboundedValueWord?: string;
  /** Which of a `values` row's numbers the widget renders; absent = every value. */
  selectionKind?: MetricKind;
  isUsagePeriod?: boolean;
  traySuffix?: string;
  showsResetExpiries?: boolean;
  sessionStartSignal?: SessionStartSignal;
  isChart?: boolean;
  valueTooltipNote?: string;
  infoNote?: string;
  periodDurationMs?: number;
}

export type LimitResourceSource =
  | { type: "progress" }
  | { type: "value"; kind: MetricKind; label?: string }
  | { type: "progressOrValue"; kind: MetricKind; label?: string };

export interface LimitResourceDescriptor {
  key: string;
  kind: "consumption" | "balance";
  unit: string;
  source: LimitResourceSource;
  estimated: boolean;
}

export interface UsageHistoryDescriptor {
  scope: "machineLocal" | "accountWide";
  estimatedCost: boolean;
  sourceNote: string;
}

export interface WidgetDescriptor {
  id: string;
  providerId: string;
  metricLabel: string;
  template: WidgetTemplate;
  pinnable: boolean;
  isSpendTile: boolean;
  limitResources?: LimitResourceDescriptor[];
  historyResource?: UsageHistoryDescriptor;
}

/** One provider the engine knows, with the widgets it can feed in declaration order. */
export interface ProviderEntry {
  provider: Provider;
  descriptors: WidgetDescriptor[];
}

/** Live refresh state for one provider (stale-while-revalidate). */
/** What spending one banked limit reset came to (`limit_resets.rs`). */
export type LimitResetResult =
  | { status: "reset"; resetType: string | null }
  | { status: "rejected"; code: string }
  | { status: "failed"; category: ErrorCategory }
  | { status: "inFlight" };

export interface ProviderRuntimeState {
  /** Last good snapshot; absent until the first cache load or success. */
  snapshot?: ProviderSnapshot;
  /** Latest refresh error text, cleared by the next success. The last good snapshot stays on screen. */
  error?: string;
  refreshing: boolean;
}

export interface EngineState {
  providers: Record<string, ProviderRuntimeState>;
  /** When the most recent full refresh batch finished; drives "Next update in …". */
  lastRefreshAt?: string;
  refreshIntervalMs: number;
}

export type Platform = "windows" | "linux" | "macos" | "web";

export interface AppInfo {
  name: string;
  version: string;
  platform: Platform;
  logFile?: string;
}

export type PopoverScreen = "dashboard" | "customize" | "settings";

export type AccountProvider = "claude" | "codex";

export type UsageSource = "claude" | "codex";
export type UsageGrouping = "day" | "month" | "year" | "model" | "project";

export interface UsageTotals {
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  cacheCreationInputTokens: number;
  totalTokens: number;
  costUSD?: number;
}

export interface UsageQuery {
  from?: string;
  to?: string;
  groupBy: UsageGrouping;
}

export interface UsageGroupRow {
  key: string;
  source: UsageSource;
  totals: UsageTotals;
}

export interface UsageLedgerInfo {
  firstDay: string | null;
  updatedAt: string | null;
  importing: boolean;
}

export interface ExchangeRate {
  usdToVnd: number;
  publishedAt: string;
  fetchedAt: string;
  stale: boolean;
}

/**
 * How full one recent Claude Code or Codex session's context window is, read from its local log. The
 * core never reads or returns message text.
 */
export interface ContextWindowSession {
  source: UsageSource;
  /** The Claude Code session id or the Codex thread id. */
  sessionId: string;
  /** Repository name under the ledger's project rule; empty when unknown. */
  project: string;
  model: string;
  /** Tokens in the context now: the latest request's prompt (input, cache reads and writes) plus its reply. */
  usedTokens: number;
  /** The model's context window for this session, or `null` when the log does not tell. */
  windowTokens: number | null;
  /** The prompt of the session's first request after its latest compaction: system prompt, tools, memory, opening message. */
  baseTokens: number;
  /** What the latest exchange added: `usedTokens` minus the previous request's; 0 after a single request. */
  lastTurnTokens: number;
  /** ISO time of the latest request. */
  updatedAt: string;
}

export interface ChatSession {
  id: string;
  provider: AccountProvider;
  label: string;
  createdAt: string;
}

export interface ConnectedAccount {
  id: string;
  provider: AccountProvider;
  label: string;
  connectedAt: string;
  updatedAt: string;
  /** `cli`: the live login of Claude Code or the Codex CLI on this computer, listed automatically. */
  credentialMode: "shared_cli" | "managed_oauth" | "cli";
}

/** A card of a service beyond Claude and Codex that this computer has without anything saved in Quota Control. */
export interface DetectedCard {
  id: string;
  service: string;
  /** The account's email or name, when the login tells. */
  label: string | null;
  /** The app whose login it is ("Gemini CLI"), or the environment variable holding the key. */
  origin: string;
}

/** A browser sign-in a service offers: with a Google account, or with a GitHub account. */
export type SignInMethod = "google" | "github";

/** An API key saved in Quota Control, or an account signed in to from it; neither the key nor the token reaches the popup. */
export interface SavedKey {
  id: string;
  service: string;
  label: string;
  addedAt: string;
  /** The key's last four characters; empty for a signed-in account. */
  hint: string;
  /** How a signed-in account was added; absent for a pasted key. */
  signIn?: SignInMethod;
}

/** A service beyond Claude and Codex, with the cards it has (`list_services`). */
export interface ServiceEntry {
  id: string;
  name: string;
  /** The app whose login on this computer the service reads, when it reads one. */
  loginFrom: string | null;
  takesApiKey: boolean;
  /** What the key field asks for, in English: "API key", or a session cookie's name for a web login. */
  keyLabel: string;
  /** Whether the key is one token, one session cookie's value, or a browser's whole Cookie header. */
  keyFormat: "token" | "cookie" | "cookieHeader";
  /** Where to create a key. */
  keyUrl: string | null;
  /** Environment variables read for a key. */
  keyEnv: string[];
  /** Other values asked for beside the key, as `[field, English label]`. */
  keyFields: [string, string][];
  /** Its cards start hidden until turned on in Customize. */
  startsHidden: boolean;
  /** The browser sign-ins it offers, in order. */
  signIn: SignInMethod[];
  detected: DetectedCard[];
  keys: SavedKey[];
  /** Cards found on this computer that were removed here; the logins and keys themselves are untouched. */
  dismissed: DetectedCard[];
}

/** Where a sign-in page opened: Google Chrome when it is installed, otherwise the default browser. */
export type LoginBrowser = "chrome" | "default";

export interface AccountLogin {
  flowId: string;
  authorizationUrl: string;
  expiresInSeconds: number;
  browser: LoginBrowser;
  /** The code to type on the sign-in page, for a device sign-in (GitHub) that cannot fill it in. */
  userCode?: string;
}

/** How a browser sign-in ended. The core finishes it on its own and reports it through `account-login`. */
export interface AccountLoginResult {
  flowId: string;
  /** `claude`, `codex`, or the id of the service signed in to. */
  provider: string;
  /** `expired`: nobody finished the sign-in in the browser in time. */
  status: "connected" | "cancelled" | "expired" | "failed";
  accountId?: string;
  error?: string;
}

/** The taskbar band the strip frames are rendered for (`taskbar_strip.rs`). */
export interface TaskbarInfo {
  supported: boolean;
  /** Device-pixel height of the taskbar band. */
  height: number;
  /** Device pixels per logical pixel on the taskbar's monitor. */
  scale: number;
  /** The taskbar's own (system) theme, which can differ from the app theme. */
  theme: "light" | "dark";
  edge: "bottom" | "top" | "left" | "right";
}

/** One rendered strip frame: PNG in device pixels plus its text and tooltip forms. */
export interface StripFrame {
  png: Uint8Array;
  width: number;
  height: number;
  text: string;
  tooltip: string;
  /** The same strip as a description, which macOS draws itself in place of the picture. */
  native?: NativeStrip;
}

/** The system's own answer about notification access (`system.rs`). */
export type SystemNotificationAccess = "granted" | "denied" | "undetermined";

/** A notification for the system to show (`system.rs`). */
export interface SystemNotification {
  title: string;
  body: string;
  /** What the notification is about: the next one about the same thing replaces it. */
  id?: string;
  /** Notifications with the same group stay together in the system's list. */
  group?: string;
}

/** Where the app's self-update stands (`updates.rs`). */
export type UpdatePhase = "idle" | "checking" | "upToDate" | "available" | "downloading" | "installing" | "failed";
export type UpdateFailureStage = "check" | "download" | "install";
/** `release`: the manifest is missing or has no build for this package; `permission`: elevation was refused. */
export type UpdateFailureReason = "network" | "release" | "signature" | "permission" | "other";

export interface AvailableUpdate {
  version: string;
  notes?: string;
  publishedAt?: string;
}

export interface UpdateStatus {
  /** This installation can replace itself; development builds and other packages cannot. */
  supported: boolean;
  /** A found release can install on its own here; `false` where installing asks for an administrator password. */
  unattended?: boolean;
  currentVersion: string;
  phase: UpdatePhase;
  /** The user started the current check or install; background checks stay quiet until they find something. */
  manual: boolean;
  available?: AvailableUpdate;
  downloaded: number;
  total?: number;
  /** When the last check finished (ISO-8601). */
  checkedAt?: string;
  failure?: { stage: UpdateFailureStage; reason: UpdateFailureReason };
  /** The version this launch replaced, until the popup has shown it (`acknowledgeUpdate`). */
  updatedFrom?: string;
}
