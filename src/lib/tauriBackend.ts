import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Backend, DocumentName, Unsubscribe } from "./backend";
import type { ContextWindowSession, ExchangeRate, UsageGroupRow, UsageLedgerInfo, UsageQuery } from "./types";
import type { LimitResetResult } from "./types";
import type { PublicFeedName, PublicFeedSnapshot, QualityInfo, QualityQuery, QualitySummary } from "./insightsTypes";
import type { Language } from "@/i18n/language";
import type { AccountLogin, AccountLoginResult, AccountProvider, AppInfo, ChatSession, ConnectedAccount, EngineState, LoginBrowser, PopoverScreen, ProviderEntry, StripFrame, TaskbarInfo, UpdateStatus } from "./types";
import type { GlanceDocument } from "@/model/glance";

/** Subscribe to a Tauri event synchronously; the returned function tears the listener down. */
function subscribe<T>(event: string, listener: (payload: T) => void, replay?: () => Promise<T>): Unsubscribe {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  let received = 0;
  listen<T>(event, (message) => {
    received += 1;
    if (!disposed) listener(message.payload);
  })
    .then((stop) => {
      if (disposed) stop();
      else {
        unlisten = stop;
        const revision = received;
        void replay?.().then((payload) => {
          if (!disposed && revision === received) listener(payload);
        }).catch((error: unknown) => console.error(`replay(${event}) failed`, error));
      }
    })
    .catch((error: unknown) => console.error(`listen(${event}) failed`, error));
  return () => {
    disposed = true;
    unlisten?.();
  };
}

export class TauriBackend implements Backend {
  usageSummary(query: UsageQuery): Promise<UsageGroupRow[]> {
    return invoke<UsageGroupRow[]>("usage_summary", { query });
  }

  usageLedgerInfo(): Promise<UsageLedgerInfo> {
    return invoke<UsageLedgerInfo>("usage_ledger_info");
  }

  onUsageLedgerChanged(listener: (info: UsageLedgerInfo) => void): Unsubscribe {
    return subscribe<UsageLedgerInfo>("usage-ledger-changed", listener, () => this.usageLedgerInfo());
  }

  exchangeRate(): Promise<ExchangeRate | null> {
    return invoke<ExchangeRate | null>("exchange_rate");
  }

  contextWindows(): Promise<ContextWindowSession[]> {
    return invoke<ContextWindowSession[]>("context_windows");
  }

  appInfo(): Promise<AppInfo> {
    return invoke<AppInfo>("app_info");
  }

  catalog(): Promise<ProviderEntry[]> {
    return invoke<ProviderEntry[]>("catalog");
  }

  onCatalogChanged(listener: (catalog: ProviderEntry[]) => void): Unsubscribe {
    return subscribe<ProviderEntry[]>("catalog-changed", listener, () => this.catalog());
  }

  listAccounts(): Promise<ConnectedAccount[]> {
    return invoke("list_accounts");
  }

  beginAccountLogin(provider: AccountProvider, language: Language): Promise<AccountLogin> {
    return invoke("begin_account_login", { provider, language });
  }

  reopenAccountLogin(flowId: string): Promise<LoginBrowser> {
    return invoke("reopen_account_login", { flowId });
  }

  cancelAccountLogin(flowId: string): Promise<void> {
    return invoke("cancel_account_login", { flowId });
  }

  onAccountLogin(listener: (result: AccountLoginResult) => void): Unsubscribe {
    return subscribe<AccountLoginResult>("account-login", listener);
  }

  removeAccount(accountId: string): Promise<void> {
    return invoke("remove_account", { accountId });
  }

  listChatSessions(): Promise<ChatSession[]> {
    return invoke("list_chat_sessions");
  }

  onChatSessionsChanged(listener: (sessions: ChatSession[]) => void): Unsubscribe {
    return subscribe<ChatSession[]>("chat-sessions-changed", listener, () => this.listChatSessions());
  }

  createChatSession(provider: AccountProvider, label?: string): Promise<ChatSession> {
    return invoke("create_chat_session", { provider, label: label ?? null });
  }

  openChatSession(sessionId: string): Promise<void> {
    return invoke("open_chat_session", { sessionId });
  }

  engineState(): Promise<EngineState> {
    return invoke<EngineState>("engine_state");
  }

  onEngineState(listener: (state: EngineState) => void): Unsubscribe {
    return subscribe<EngineState>("engine-state", listener, () => this.engineState());
  }

  refresh(providerId?: string): Promise<void> {
    return invoke("refresh", { providerId: providerId ?? null });
  }

  redeemLimitReset(providerId: string): Promise<LimitResetResult> {
    return invoke<LimitResetResult>("redeem_limit_reset", { providerId });
  }

  setEnabledProviders(providerIds: string[]): Promise<void> {
    return invoke("set_enabled_providers", { providerIds });
  }

  loadDocument<T>(name: DocumentName): Promise<T | null> {
    return invoke<T | null>("load_document", { name });
  }

  saveDocument(name: DocumentName, value: unknown): Promise<void> {
    return invoke("save_document", { name, value });
  }

  resizePopup(height: number): Promise<void> {
    return invoke("resize_popup", { height });
  }

  hidePopup(): Promise<void> {
    return invoke("hide_popup");
  }

  onPopupVisibility(listener: (shown: boolean) => void): Unsubscribe {
    return subscribe<boolean>("popup-visibility", listener);
  }

  onNavigate(listener: (screen: PopoverScreen) => void): Unsubscribe {
    return subscribe<PopoverScreen>("navigate", listener);
  }

  openUrl(url: string): Promise<void> {
    return invoke("open_url", { url });
  }

  copyText(text: string): Promise<void> {
    return invoke("copy_text", { text });
  }

  taskbarInfo(): Promise<TaskbarInfo> {
    return invoke<TaskbarInfo>("taskbar_info");
  }

  onTaskbarInfo(listener: (info: TaskbarInfo) => void): Unsubscribe {
    return subscribe<TaskbarInfo>("taskbar-info", listener, () => this.taskbarInfo());
  }

  setTaskbarStrip(frame: StripFrame | null): Promise<void> {
    return invoke("set_taskbar_strip", { frame: frame ? { ...frame, png: Array.from(frame.png) } : null });
  }

  setGlance(document: GlanceDocument): Promise<void> {
    return invoke("set_glance", { document });
  }

  setTrayIcon(png: Uint8Array | null, tooltip: string): Promise<void> {
    return invoke("set_tray_icon", { png: png ? Array.from(png) : null, tooltip });
  }

  globalShortcut(): Promise<string | null> {
    return invoke<string | null>("global_shortcut");
  }

  setGlobalShortcut(shortcut: string | null): Promise<string | null> {
    return invoke<string | null>("set_global_shortcut", { shortcut });
  }

  pauseGlobalShortcut(paused: boolean): Promise<void> {
    return invoke("pause_global_shortcut", { paused });
  }

  updateStatus(): Promise<UpdateStatus> {
    return invoke<UpdateStatus>("update_status");
  }

  onUpdateStatus(listener: (status: UpdateStatus) => void): Unsubscribe {
    return subscribe<UpdateStatus>("update-status", listener, () => this.updateStatus());
  }

  checkForUpdate(): Promise<UpdateStatus> {
    return invoke<UpdateStatus>("check_for_update");
  }

  installUpdate(): Promise<void> {
    return invoke("install_update");
  }

  modelQuality(query: QualityQuery): Promise<QualitySummary> {
    return invoke<QualitySummary>("model_quality", { query });
  }

  rescanModelQuality(): Promise<void> {
    return invoke("rescan_model_quality");
  }

  onModelQualityChanged(listener: (info: QualityInfo) => void): Unsubscribe {
    return subscribe<QualityInfo>("model-quality-changed", listener);
  }

  publicFeed(name: PublicFeedName): Promise<PublicFeedSnapshot> {
    return invoke<PublicFeedSnapshot>("public_feed", { name });
  }

  refreshPublicFeed(name: PublicFeedName): Promise<PublicFeedSnapshot> {
    return invoke<PublicFeedSnapshot>("refresh_public_feed", { name });
  }

  onPublicFeedChanged(listener: (name: PublicFeedName) => void): Unsubscribe {
    return subscribe<PublicFeedName>("public-feed-changed", listener);
  }

  quit(): Promise<void> {
    return invoke("quit_app");
  }
}
