import type { Backend, DocumentName, Unsubscribe } from "./backend";
import type {
  AccountLogin,
  AccountLoginResult,
  AccountProvider,
  AppInfo,
  ChatSession,
  ConnectedAccount,
  ContextWindowSession,
  EngineState,
  ExchangeRate,
  LimitResetResult,
  LoginBrowser,
  PopoverScreen,
  ProviderEntry,
  SavedKey,
  ServiceEntry,
  SignInMethod,
  UsageGroupRow,
  UsageLedgerInfo,
  UsageQuery,
} from "./types";
import type { PublicFeedName, PublicFeedSnapshot, QualityInfo, QualityQuery, QualitySummary } from "./insightsTypes";
import type { Language } from "@/i18n/language";

const READ_ONLY = "Bản web chỉ để xem. Thao tác này làm trong app Quota Control.";
const ENGINE_POLL_MS = 15_000;
const LEDGER_POLL_MS = 60_000;
const FEED_POLL_MS = 5 * 60_000;
const STORAGE_PREFIX = "quota-control-web:";

function readStorage(name: DocumentName): string | null {
  try {
    return window.localStorage.getItem(STORAGE_PREFIX + name);
  } catch {
    return null;
  }
}

function writeStorage(name: DocumentName, value: string): void {
  try {
    window.localStorage.setItem(STORAGE_PREFIX + name, value);
  } catch {
    return;
  }
}

export function isWebPopup(location: Location): boolean {
  return /\/web\/?($|index\.html$)/.test(location.pathname);
}

export class HttpBackend implements Backend {
  private readonly base: URL;

  constructor(base = new URL("../v1/app/", document.baseURI)) {
    this.base = base;
  }

  private async call<T>(command: string, args?: unknown): Promise<T> {
    const url = new URL(command, this.base);
    if (args !== undefined) url.searchParams.set("q", JSON.stringify(args));
    const response = await fetch(url, { cache: "no-store" });
    if (!response.ok) throw new Error(`Quota Control trả lỗi ${response.status}`);
    return (await response.json()) as T;
  }

  private poll<T>(interval: number, read: () => Promise<T>, listener: (value: T) => void): Unsubscribe {
    const timer = window.setInterval(() => {
      read().then(listener, () => undefined);
    }, interval);
    return () => window.clearInterval(timer);
  }

  private refused<T>(): Promise<T> {
    return Promise.reject(new Error(READ_ONLY));
  }

  usageSummary(query: UsageQuery): Promise<UsageGroupRow[]> {
    return this.call("usage_summary", { query });
  }

  usageLedgerInfo(): Promise<UsageLedgerInfo> {
    return this.call("usage_ledger_info");
  }

  onUsageLedgerChanged(listener: (info: UsageLedgerInfo) => void): Unsubscribe {
    return this.poll(LEDGER_POLL_MS, () => this.usageLedgerInfo(), listener);
  }

  exchangeRate(): Promise<ExchangeRate | null> {
    return this.call("exchange_rate");
  }

  contextWindows(): Promise<ContextWindowSession[]> {
    return this.call("context_windows");
  }

  appInfo(): Promise<AppInfo> {
    return this.call("app_info");
  }

  catalog(): Promise<ProviderEntry[]> {
    return this.call("catalog");
  }

  onCatalogChanged(listener: (catalog: ProviderEntry[]) => void): Unsubscribe {
    return this.poll(ENGINE_POLL_MS, () => this.catalog(), listener);
  }

  listAccounts(): Promise<ConnectedAccount[]> {
    return this.call("list_accounts");
  }

  beginAccountLogin(_provider: string, _language: Language, _method?: SignInMethod): Promise<AccountLogin> {
    return this.refused();
  }

  beginCliLogin(_provider: AccountProvider): Promise<AccountLogin> {
    return this.refused();
  }

  reopenAccountLogin(_flowId: string): Promise<LoginBrowser> {
    return this.refused();
  }

  async cancelAccountLogin(_flowId: string): Promise<void> {}

  onAccountLogin(_listener: (result: AccountLoginResult) => void): Unsubscribe {
    return () => undefined;
  }

  removeAccount(_accountId: string): Promise<void> {
    return this.refused();
  }

  async listRemovedLogins(): Promise<ConnectedAccount[]> {
    return [];
  }

  restoreRemovedLogins(_provider: AccountProvider): Promise<void> {
    return this.refused();
  }

  listServices(): Promise<ServiceEntry[]> {
    return this.call("list_services");
  }

  addApiKey(_serviceId: string, _key: string, _label?: string, _fields?: Record<string, string>): Promise<SavedKey> {
    return this.refused();
  }

  removeApiKey(_keyId: string): Promise<void> {
    return this.refused();
  }

  dismissDetectedCard(_cardId: string): Promise<void> {
    return this.refused();
  }

  restoreDismissedCards(_serviceId: string): Promise<void> {
    return this.refused();
  }

  async listChatSessions(): Promise<ChatSession[]> {
    return [];
  }

  createChatSession(_provider: AccountProvider, _label?: string): Promise<ChatSession> {
    return this.refused();
  }

  openChatSession(_sessionId: string): Promise<void> {
    return this.refused();
  }

  engineState(): Promise<EngineState> {
    return this.call("engine_state");
  }

  onEngineState(listener: (state: EngineState) => void): Unsubscribe {
    return this.poll(ENGINE_POLL_MS, () => this.engineState(), listener);
  }

  async refresh(_providerId?: string): Promise<void> {}

  redeemLimitReset(_providerId: string): Promise<LimitResetResult> {
    return this.refused();
  }

  async setEnabledProviders(_providerIds: string[]): Promise<void> {}

  async loadDocument<T>(name: DocumentName): Promise<T | null> {
    const saved = readStorage(name);
    return saved !== null ? (JSON.parse(saved) as T) : this.call<T | null>("load_document", { name });
  }

  async saveDocument(name: DocumentName, value: unknown): Promise<void> {
    writeStorage(name, JSON.stringify(value));
  }

  async resizePopup(_height: number): Promise<void> {}

  async hidePopup(): Promise<void> {}

  onPopupVisibility(listener: (shown: boolean) => void): Unsubscribe {
    const timer = window.setTimeout(() => listener(true), 0);
    return () => window.clearTimeout(timer);
  }

  onNavigate(_listener: (screen: PopoverScreen) => void): Unsubscribe {
    return () => undefined;
  }

  async openUrl(url: string): Promise<void> {
    window.open(url, "_blank", "noopener,noreferrer");
  }

  async copyText(text: string): Promise<void> {
    await navigator.clipboard.writeText(text);
  }

  systemTimeZone(): Promise<string | null> {
    return this.call("system_time_zone");
  }

  async setTrayIcon(_png: Uint8Array | null, _tooltip: string): Promise<void> {}

  modelQuality(query: QualityQuery): Promise<QualitySummary> {
    return this.call("model_quality", { query });
  }

  async rescanModelQuality(): Promise<void> {}

  onModelQualityChanged(_listener: (info: QualityInfo) => void): Unsubscribe {
    return () => undefined;
  }

  publicFeed(name: PublicFeedName): Promise<PublicFeedSnapshot> {
    return this.call("public_feed", { name });
  }

  refreshPublicFeed(name: PublicFeedName): Promise<PublicFeedSnapshot> {
    return this.publicFeed(name);
  }

  onPublicFeedChanged(listener: (name: PublicFeedName) => void): Unsubscribe {
    const names: PublicFeedName[] = ["codexResetStatus", "codexResets", "claudeResets"];
    return this.poll(FEED_POLL_MS, async () => names, (list) => list.forEach(listener));
  }

  async quit(): Promise<void> {}
}
