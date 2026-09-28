/**
 * The popup's only door to the outside world. `TauriBackend` talks to the Rust core; `MockBackend`
 * serves fixtures so the interface can be built and tested in a plain browser (`pnpm dev`).
 *
 * Rust command names live in `src-tauri/src/commands.rs`; keep both sides in sync.
 */
import type { Language } from "@/i18n/language";
import type { AccountLogin, AccountLoginResult, AccountProvider, AppInfo, ChatSession, ConnectedAccount, EngineState, LoginBrowser, PopoverScreen, ProviderEntry, SavedKey, ServiceEntry, SignInMethod, StripFrame, TaskbarInfo, UpdateStatus } from "./types";

import type { ContextWindowSession, ExchangeRate, UsageGroupRow, UsageLedgerInfo, UsageQuery } from "./types";
import type { LimitResetResult } from "./types";
import type { PublicFeedName, PublicFeedSnapshot, QualityInfo, QualityQuery, QualitySummary } from "./insightsTypes";
import type { GlanceDocument } from "@/model/glance";

export type Unsubscribe = () => void;

/** Named JSON documents the core persists atomically in the app config directory. */
export type DocumentName = "settings" | "layout";

export interface Backend {
  usageSummary(query: UsageQuery): Promise<UsageGroupRow[]>;
  usageLedgerInfo(): Promise<UsageLedgerInfo>;
  onUsageLedgerChanged(listener: (info: UsageLedgerInfo) => void): Unsubscribe;
  exchangeRate(): Promise<ExchangeRate | null>;
  /** Recent sessions (a request in the last 24 hours), newest first; cheap enough to poll every 10 seconds. */
  contextWindows?(): Promise<ContextWindowSession[]>;
  appInfo(): Promise<AppInfo>;
  catalog(): Promise<ProviderEntry[]>;
  onCatalogChanged(listener: (catalog: ProviderEntry[]) => void): Unsubscribe;
  listAccounts(): Promise<ConnectedAccount[]>;
  /**
   * Open the provider's sign-in page, in Google Chrome when it is installed. The core finishes the
   * login itself, even while the popup is hidden, and reports how it ended through `onAccountLogin`.
   * `provider` is `claude`, `codex` or a service's id; a service signs in with `method`.
   */
  beginAccountLogin(provider: string, language: Language, method?: SignInMethod): Promise<AccountLogin>;
  /** Show the sign-in page of a login that is still waiting. */
  reopenAccountLogin(flowId: string): Promise<LoginBrowser>;
  cancelAccountLogin(flowId: string): Promise<void>;
  onAccountLogin(listener: (result: AccountLoginResult) => void): Unsubscribe;
  removeAccount(accountId: string): Promise<void>;
  /** Every service beyond Claude and Codex, with the logins found on this computer and the keys saved. */
  listServices(): Promise<ServiceEntry[]>;
  /** Save an API key (and the values its service asks for beside it) and add its card. */
  addApiKey(serviceId: string, key: string, label?: string, fields?: Record<string, string>): Promise<SavedKey>;
  removeApiKey(keyId: string): Promise<void>;
  /** Remove a card found on this computer (another app's login, an environment key) without touching the login. */
  dismissDetectedCard(cardId: string): Promise<void>;
  /** Show again the removed cards a service found on this computer. */
  restoreDismissedCards(serviceId: string): Promise<void>;
  listChatSessions(): Promise<ChatSession[]>;
  onChatSessionsChanged?(listener: (sessions: ChatSession[]) => void): Unsubscribe;
  createChatSession(provider: AccountProvider, label?: string): Promise<ChatSession>;
  openChatSession(sessionId: string): Promise<void>;
  engineState(): Promise<EngineState>;
  onEngineState(listener: (state: EngineState) => void): Unsubscribe;
  /** Force-refresh one provider, or every enabled provider when `providerId` is omitted. */
  refresh(providerId?: string): Promise<void>;
  /**
   * Spend the account's banked limit reset that expires first (Codex only). Only when the user
   * asks; the regular limits keep resetting on the provider's own schedule.
   */
  redeemLimitReset?(providerId: string): Promise<LimitResetResult>;
  /** Which providers the engine refreshes; the popup owns enablement (Customize). */
  setEnabledProviders(providerIds: string[]): Promise<void>;
  loadDocument<T>(name: DocumentName): Promise<T | null>;
  saveDocument(name: DocumentName, value: unknown): Promise<void>;
  /** Report the content height (CSS px) so the core can size and re-anchor the popup window. */
  resizePopup(height: number): Promise<void>;
  hidePopup(): Promise<void>;
  onPopupVisibility(listener: (shown: boolean) => void): Unsubscribe;
  /** Tray menu items ask the popup to open a screen (e.g. Settings). */
  onNavigate(listener: (screen: PopoverScreen) => void): Unsubscribe;
  openUrl(url: string): Promise<void>;
  copyText(text: string): Promise<void>;
  /** The operating system's current IANA time zone, read afresh each call; `null` when it names none. */
  systemTimeZone?(): Promise<string | null>;
  /** Replace the tray icon (PNG bytes) and tooltip; `null` restores the app icon. */
  setTrayIcon(png: Uint8Array | null, tooltip: string): Promise<void>;
  /** The taskbar band the live strip is rendered for; absent where the core has no strip. */
  taskbarInfo?(): Promise<TaskbarInfo>;
  onTaskbarInfo?(listener: (info: TaskbarInfo) => void): Unsubscribe;
  /** Show a strip frame on the taskbar (tray title on Linux); `null` removes the strip. */
  setTaskbarStrip?(frame: StripFrame | null): Promise<void>;
  /** Send the starred metrics to the macOS Dynamic Island and desktop widget; absent elsewhere. */
  setGlance?(document: GlanceDocument): Promise<void>;
  /** The accelerator that toggles the popup from anywhere (`Ctrl+Alt+KeyU`), or `null` for none. */
  globalShortcut?(): Promise<string | null>;
  /** Register and save a new accelerator, or clear it with `null`; rejects when it is unusable. */
  setGlobalShortcut?(shortcut: string | null): Promise<string | null>;
  /** Release the accelerator while Settings records one, so pressing it records instead of toggling. */
  pauseGlobalShortcut?(paused: boolean): Promise<void>;
  /** The app's self-update state; absent where the core has no updater. */
  updateStatus?(): Promise<UpdateStatus>;
  onUpdateStatus?(listener: (status: UpdateStatus) => void): Unsubscribe;
  /** Look for a newer release now; resolves to the finished status. */
  checkForUpdate?(): Promise<UpdateStatus>;
  /** Download, verify and install the newest release, checking first when none is pending. The app
   * then exits (Windows, relaunched by the installer) or restarts (Linux); a rejection means it stayed. */
  installUpdate?(): Promise<void>;
  /** The popup has shown which version this launch replaced (`UpdateStatus.updatedFrom`). */
  acknowledgeUpdate?(): Promise<void>;
  /** Model quality counted from the local transcripts, per model and project, within the query's days. */
  modelQuality(query: QualityQuery): Promise<QualitySummary>;
  /** Rescan the transcripts now instead of at the next 10-minute pass. */
  rescanModelQuality(): Promise<void>;
  /** Fires when a scan starts (`scanning: true`) and when it finishes. */
  onModelQualityChanged(listener: (info: QualityInfo) => void): Unsubscribe;
  /** The cached copy of a public feed; the core refreshes each on its own schedule. */
  publicFeed(name: PublicFeedName): Promise<PublicFeedSnapshot>;
  /** Ask the source now (at most once a minute per feed); resolves to the resulting snapshot. */
  refreshPublicFeed(name: PublicFeedName): Promise<PublicFeedSnapshot>;
  /** Fires with the feed's name when its body changes. */
  onPublicFeedChanged(listener: (name: PublicFeedName) => void): Unsubscribe;
  quit(): Promise<void>;
}

let current: Backend | null = null;

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function setBackend(backend: Backend): void {
  current = backend;
}

export function backend(): Backend {
  if (!current) {
    throw new Error("Backend not initialized; call setBackend() during startup.");
  }
  return current;
}
