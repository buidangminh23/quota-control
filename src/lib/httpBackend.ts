import type { Backend, DocumentName, Unsubscribe } from "./backend";
import type {
  PendingLogin,
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
import { withCoreOwnedKeys } from "@/model/settings";

const SERVER_ONLY = "Đăng nhập CLI chạy lệnh trên máy cài Quota Control, nên làm trong app trên máy đó.";
const LOGIN_POLL_MS = 3_000;
const ACTION_HEADER = { "x-quota-control": "write", "content-type": "application/json" };
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
  private readonly loginPages = new Map<string, string>();
  readonly remote = true;

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

  private async act<T>(command: string, args: unknown = {}): Promise<T> {
    const response = await fetch(new URL(command, this.base), {
      method: "POST",
      cache: "no-store",
      headers: ACTION_HEADER,
      body: JSON.stringify(args),
    });
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    if (!response.ok) throw new Error(body?.error ?? `Quota Control trả lỗi ${response.status}`);
    return body as T;
  }

  private poll<T>(interval: number, read: () => Promise<T>, listener: (value: T) => void): Unsubscribe {
    const timer = window.setInterval(() => {
      read().then(listener, () => undefined);
    }, interval);
    return () => window.clearInterval(timer);
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

  async beginAccountLogin(provider: string, language: Language, method?: SignInMethod): Promise<AccountLogin> {
    const page = window.open("", "_blank");
    try {
      const login = await this.act<AccountLogin>("begin_account_login", { provider, language, method: method ?? null });
      this.loginPages.set(login.flowId, login.authorizationUrl);
      if (page) {
        page.opener = null;
        page.location.href = login.authorizationUrl;
      }
      return login;
    } catch (error) {
      page?.close();
      throw error;
    }
  }

  beginCliLogin(_provider: AccountProvider): Promise<AccountLogin> {
    return Promise.reject(new Error(SERVER_ONLY));
  }

  async reopenAccountLogin(flowId: string): Promise<LoginBrowser> {
    const page = this.loginPages.get(flowId);
    if (!page) throw new Error("Phiên đăng nhập này đã hết. Bắt đầu lại.");
    window.open(page, "_blank", "noopener,noreferrer");
    return "remote";
  }

  async listPendingLogins(): Promise<PendingLogin[]> {
    const pending = await this.call<PendingLogin[]>("pending_logins");
    for (const login of pending) this.loginPages.set(login.flowId, login.authorizationUrl);
    return pending;
  }

  async completeAccountLogin(flowId: string, address: string): Promise<void> {
    await this.act("complete_account_login", { flowId, address });
  }

  async cancelAccountLogin(flowId: string): Promise<void> {
    this.loginPages.delete(flowId);
    await this.act("cancel_account_login", { flowId });
  }

  onAccountLogin(listener: (result: AccountLoginResult) => void): Unsubscribe {
    const seen = new Set<string>();
    let primed = false;
    const check = () => {
      this.call<AccountLoginResult[]>("login_results").then((results) => {
        for (const result of results) {
          const key = `${result.flowId}:${result.status}`;
          if (seen.has(key)) continue;
          seen.add(key);
          if (primed) listener(result);
        }
        primed = true;
      }, () => undefined);
    };
    check();
    const timer = window.setInterval(check, LOGIN_POLL_MS);
    return () => window.clearInterval(timer);
  }

  async removeAccount(accountId: string): Promise<void> {
    await this.act("remove_account", { accountId });
  }

  listRemovedLogins(): Promise<ConnectedAccount[]> {
    return this.call("list_removed_logins");
  }

  async restoreRemovedLogins(provider: AccountProvider): Promise<void> {
    await this.act("restore_removed_logins", { provider });
  }

  listServices(): Promise<ServiceEntry[]> {
    return this.call("list_services");
  }

  addApiKey(serviceId: string, key: string, label?: string, fields?: Record<string, string>): Promise<SavedKey> {
    return this.act("add_api_key", { serviceId, key, label: label ?? null, fields: fields ?? null });
  }

  async removeApiKey(keyId: string): Promise<void> {
    await this.act("remove_api_key", { keyId });
  }

  async dismissDetectedCard(cardId: string): Promise<void> {
    await this.act("dismiss_detected_card", { cardId });
  }

  async restoreDismissedCards(serviceId: string): Promise<void> {
    await this.act("restore_dismissed_cards", { serviceId });
  }

  async listChatSessions(): Promise<ChatSession[]> {
    return [];
  }

  createChatSession(_provider: AccountProvider, _label?: string): Promise<ChatSession> {
    return Promise.reject(new Error(SERVER_ONLY));
  }

  openChatSession(_sessionId: string): Promise<void> {
    return Promise.reject(new Error(SERVER_ONLY));
  }

  engineState(): Promise<EngineState> {
    return this.call("engine_state");
  }

  onEngineState(listener: (state: EngineState) => void): Unsubscribe {
    return this.poll(ENGINE_POLL_MS, () => this.engineState(), listener);
  }

  async refresh(providerId?: string): Promise<void> {
    await this.act("refresh", { providerId: providerId ?? null });
  }

  redeemLimitReset(providerId: string): Promise<LimitResetResult> {
    return this.act("redeem_limit_reset", { providerId });
  }

  async setEnabledProviders(_providerIds: string[]): Promise<void> {}

  async loadDocument<T>(name: DocumentName): Promise<T | null> {
    const saved = readStorage(name);
    if (saved === null) return this.call<T | null>("load_document", { name });
    const copy = JSON.parse(saved) as T;
    if (name !== "settings") return copy;
    const stored = await this.call<unknown>("load_document", { name }).catch(() => undefined);
    return stored === undefined ? copy : (withCoreOwnedKeys(copy, stored) as T);
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
