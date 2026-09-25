/**
 * The popup's only door to the outside world. `TauriBackend` talks to the Rust core; `MockBackend`
 * serves fixtures so the interface can be built and tested in a plain browser (`pnpm dev`).
 *
 * Rust command names live in `src-tauri/src/commands.rs`; keep both sides in sync.
 */
import type { AccountLogin, AccountProvider, AppInfo, ChatSession, ConnectedAccount, EngineState, PopoverScreen, ProviderEntry } from "./types";

export type Unsubscribe = () => void;

/** Named JSON documents the core persists atomically in the app config directory. */
export type DocumentName = "settings" | "layout";

export interface Backend {
  appInfo(): Promise<AppInfo>;
  catalog(): Promise<ProviderEntry[]>;
  onCatalogChanged(listener: (catalog: ProviderEntry[]) => void): Unsubscribe;
  listAccounts(): Promise<ConnectedAccount[]>;
  importCurrentAccount(provider: AccountProvider, label?: string): Promise<ConnectedAccount>;
  beginAccountLogin(provider: AccountProvider, label?: string): Promise<AccountLogin>;
  completeAccountLogin(flowId: string, callback?: string): Promise<ConnectedAccount>;
  cancelAccountLogin(flowId: string): Promise<void>;
  removeAccount(accountId: string): Promise<void>;
  listChatSessions(): Promise<ChatSession[]>;
  createChatSession(provider: AccountProvider, label?: string): Promise<ChatSession>;
  openChatSession(sessionId: string): Promise<void>;
  engineState(): Promise<EngineState>;
  onEngineState(listener: (state: EngineState) => void): Unsubscribe;
  /** Force-refresh one provider, or every enabled provider when `providerId` is omitted. */
  refresh(providerId?: string): Promise<void>;
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
  copyImagePng(png: Uint8Array): Promise<void>;
  copyText(text: string): Promise<void>;
  /** Replace the tray icon (PNG bytes) and tooltip; `null` restores the app icon. */
  setTrayIcon(png: Uint8Array | null, tooltip: string): Promise<void>;
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
