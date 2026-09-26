/**
 * Wire types shared with the Rust core (`crates/uc-core/src/model`). Field names match the serde
 * output exactly; upstream OpenUsage `Codable` keys are preserved where they differ from camelCase
 * (`providerID`, `costUSD`, …). Dates arrive as ISO-8601 strings.
 */

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

export interface ProviderSnapshot {
  providerID: string;
  displayName: string;
  plan?: string;
  lines: MetricLine[];
  refreshedAt: string;
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

/** Where a sign-in page opened: Google Chrome when it is installed, otherwise the default browser. */
export type LoginBrowser = "chrome" | "default";

export interface AccountLogin {
  flowId: string;
  authorizationUrl: string;
  expiresInSeconds: number;
  browser: LoginBrowser;
}

/** How a browser sign-in ended. The core finishes it on its own and reports it through `account-login`. */
export interface AccountLoginResult {
  flowId: string;
  provider: AccountProvider;
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
}

/** Where `usagectl` stands on this computer (`cli_install.rs`). */
export type CliState = "unavailable" | "managed" | "installed" | "notInstalled" | "conflict";

export interface CliStatus {
  state: CliState;
  command: string;
  /** The installed command, the package's copy, or the file in the way. */
  location?: string;
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
}
