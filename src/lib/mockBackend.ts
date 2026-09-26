/**
 * Browser-only backend for building the popup without the Rust core. Serves static fixtures and
 * simulates refreshes, accounts and chat sessions in memory. Never bundled into the Tauri app's startup
 * path (loaded lazily by `main.tsx`).
 */
import type { Backend, DocumentName, Unsubscribe } from "./backend";
import type {
  AccountLogin,
  AccountLoginResult,
  AccountProvider,
  AppInfo,
  AvailableUpdate,
  ChatSession,
  CliStatus,
  ConnectedAccount,
  EngineState,
  LoginBrowser,
  PopoverScreen,
  ProviderEntry,
  UpdateStatus,
} from "./types";
import { accountDescriptors, accountProvider, fixtureAccounts, fixtureCatalog, fixtureEngineState } from "./fixtures";

const REFRESH_DELAY_MS = 600;
const LOGIN_EXPIRY_SECONDS = 600;
const LOGIN_DELAY_MS = 2_500;
const UPDATE_STEP_MS = 120;
const UPDATE_SIZE_BYTES = 12_000_000;

const ACCOUNT_LABELS: Record<AccountProvider, string> = { claude: "Claude", codex: "Codex" };

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function mockId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `mock-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export class MockBackend implements Backend {
  private state: EngineState = fixtureEngineState();
  private entries: ProviderEntry[] = fixtureCatalog();
  private readonly engineListeners = new Set<(state: EngineState) => void>();
  private readonly catalogListeners = new Set<(catalog: ProviderEntry[]) => void>();
  private readonly visibilityListeners = new Set<(shown: boolean) => void>();
  private readonly navigateListeners = new Set<(screen: PopoverScreen) => void>();
  private readonly documents = new Map<DocumentName, unknown>();
  private readonly accounts: ConnectedAccount[] = fixtureAccounts();
  private readonly pendingLogins = new Map<string, { provider: AccountProvider; timer?: ReturnType<typeof setTimeout> }>();
  private readonly loginListeners = new Set<(result: AccountLoginResult) => void>();
  private readonly chatSessions: ChatSession[] = [];
  private shortcut: string | null = null;
  private cli: CliStatus = { state: "notInstalled", command: "usagectl" };
  private version = "0.1.0";
  private updateState: UpdateStatus = { supported: true, currentVersion: "0.1.0", phase: "idle", manual: false, downloaded: 0 };
  private readonly updateListeners = new Set<(status: UpdateStatus) => void>();
  /** Test hook: how long the simulated browser takes to finish a sign-in; `null` waits for `finishLogin`. */
  loginDelayMs: number | null = LOGIN_DELAY_MS;
  /** Test hook: the release the next check finds; `null` means the running version is the newest. */
  nextRelease: AvailableUpdate | null = { version: "0.2.0", notes: "Faster refresh and a new update card." };

  async appInfo(): Promise<AppInfo> {
    return { name: "Quota Control", version: this.version, platform: "web" };
  }

  async catalog(): Promise<ProviderEntry[]> {
    return structuredClone(this.entries);
  }

  onCatalogChanged(listener: (catalog: ProviderEntry[]) => void): Unsubscribe {
    this.catalogListeners.add(listener);
    return () => this.catalogListeners.delete(listener);
  }

  async listAccounts(): Promise<ConnectedAccount[]> {
    return structuredClone(this.accounts);
  }

  async beginAccountLogin(provider: AccountProvider): Promise<AccountLogin> {
    const flowId = mockId();
    const timer = this.loginDelayMs === null ? undefined : setTimeout(() => this.finishLogin(flowId), this.loginDelayMs);
    this.pendingLogins.set(flowId, { provider, timer });
    return {
      flowId,
      authorizationUrl: `https://example.invalid/oauth/${provider}?flow=${flowId}`,
      expiresInSeconds: LOGIN_EXPIRY_SECONDS,
      browser: "chrome",
    };
  }

  async reopenAccountLogin(flowId: string): Promise<LoginBrowser> {
    if (!this.pendingLogins.has(flowId)) throw new Error("This login is no longer active. Start again.");
    return "chrome";
  }

  async cancelAccountLogin(flowId: string): Promise<void> {
    const pending = this.takeLogin(flowId);
    if (pending) this.emitLogin({ flowId, provider: pending.provider, status: "cancelled" });
  }

  onAccountLogin(listener: (result: AccountLoginResult) => void): Unsubscribe {
    this.loginListeners.add(listener);
    return () => this.loginListeners.delete(listener);
  }

  /** Test hook: end a waiting sign-in as if the browser returned, the way the core reports it. */
  finishLogin(flowId: string, error?: string): void {
    const pending = this.takeLogin(flowId);
    if (!pending) return;
    if (error !== undefined) {
      this.emitLogin({ flowId, provider: pending.provider, status: "failed", error });
      return;
    }
    const account = this.addAccount(pending.provider, pending.provider, "managed_oauth");
    this.emitLogin({ flowId, provider: pending.provider, status: "connected", accountId: account.id });
  }

  async removeAccount(accountId: string): Promise<void> {
    const index = this.accounts.findIndex((account) => account.id === accountId);
    if (index >= 0) this.accounts.splice(index, 1);
    this.entries = this.entries.filter((entry) => entry.provider.id !== accountId);
    this.update((state) => {
      delete state.providers[accountId];
    });
    this.emitCatalog();
  }

  async listChatSessions(): Promise<ChatSession[]> {
    return structuredClone(this.chatSessions);
  }

  async createChatSession(provider: AccountProvider, label?: string): Promise<ChatSession> {
    const session: ChatSession = {
      id: mockId(),
      provider,
      label: label ?? ACCOUNT_LABELS[provider],
      createdAt: new Date().toISOString(),
    };
    this.chatSessions.push(session);
    return structuredClone(session);
  }

  async openChatSession(sessionId: string): Promise<void> {
    if (!this.chatSessions.some((session) => session.id === sessionId)) throw new Error("Chat session does not exist");
  }

  async engineState(): Promise<EngineState> {
    return this.state;
  }

  onEngineState(listener: (state: EngineState) => void): Unsubscribe {
    this.engineListeners.add(listener);
    return () => this.engineListeners.delete(listener);
  }

  async refresh(providerId?: string): Promise<void> {
    const ids = providerId ? [providerId] : Object.keys(this.state.providers);
    this.update((state) => {
      for (const id of ids) {
        const entry = state.providers[id];
        if (entry) entry.refreshing = true;
      }
    });
    await new Promise((resolve) => setTimeout(resolve, REFRESH_DELAY_MS));
    this.update((state) => {
      for (const id of ids) {
        const entry = state.providers[id];
        if (entry) entry.refreshing = false;
      }
      state.lastRefreshAt = new Date().toISOString();
    });
  }

  async setEnabledProviders(providerIds: string[]): Promise<void> {
    const settings = (this.documents.get("settings") as Record<string, unknown> | undefined) ?? {};
    this.documents.set("settings", { ...settings, enabledProviders: [...providerIds] });
  }

  async loadDocument<T>(name: DocumentName): Promise<T | null> {
    return (this.documents.get(name) as T | undefined) ?? null;
  }

  async saveDocument(name: DocumentName, value: unknown): Promise<void> {
    this.documents.set(name, structuredClone(value));
  }

  async resizePopup(): Promise<void> {}

  async hidePopup(): Promise<void> {
    for (const listener of this.visibilityListeners) listener(false);
  }

  onPopupVisibility(listener: (shown: boolean) => void): Unsubscribe {
    this.visibilityListeners.add(listener);
    return () => this.visibilityListeners.delete(listener);
  }

  onNavigate(listener: (screen: PopoverScreen) => void): Unsubscribe {
    this.navigateListeners.add(listener);
    return () => this.navigateListeners.delete(listener);
  }

  /** Test hook: behaves like a tray menu item asking the popup to open `screen`. */
  navigate(screen: PopoverScreen): void {
    for (const listener of this.navigateListeners) listener(screen);
  }

  async openUrl(url: string): Promise<void> {
    window.open(url, "_blank", "noopener");
  }

  async copyImagePng(): Promise<void> {}

  async copyText(text: string): Promise<void> {
    await navigator.clipboard?.writeText(text);
  }

  async setTrayIcon(): Promise<void> {}

  async globalShortcut(): Promise<string | null> {
    return this.shortcut;
  }

  async setGlobalShortcut(shortcut: string | null): Promise<string | null> {
    this.shortcut = shortcut;
    return shortcut;
  }

  async pauseGlobalShortcut(): Promise<void> {}

  async cliStatus(): Promise<CliStatus> {
    return { ...this.cli };
  }

  async installCli(): Promise<CliStatus> {
    this.cli = { state: "installed", command: "usagectl", location: "~/.local/bin/usagectl" };
    return { ...this.cli };
  }

  async uninstallCli(): Promise<CliStatus> {
    this.cli = { state: "notInstalled", command: "usagectl" };
    return { ...this.cli };
  }

  async updateStatus(): Promise<UpdateStatus> {
    return structuredClone(this.updateState);
  }

  onUpdateStatus(listener: (status: UpdateStatus) => void): Unsubscribe {
    this.updateListeners.add(listener);
    return () => this.updateListeners.delete(listener);
  }

  async checkForUpdate(): Promise<UpdateStatus> {
    this.setUpdate({ phase: "checking", manual: true, failure: undefined });
    await wait(UPDATE_STEP_MS);
    const checkedAt = new Date().toISOString();
    const release = this.nextRelease;
    this.setUpdate(release ? { phase: "available", available: { ...release }, checkedAt } : { phase: "upToDate", available: undefined, checkedAt });
    return this.updateStatus();
  }

  /** Simulates the download, then the relaunch on the new version (the core exits or restarts here). */
  async installUpdate(): Promise<void> {
    if (!this.updateState.available) await this.checkForUpdate();
    const release = this.updateState.available;
    if (!release) throw new Error("Quota Control is already up to date");
    this.setUpdate({ phase: "downloading", manual: true, downloaded: 0, total: UPDATE_SIZE_BYTES, failure: undefined });
    for (let step = 1; step <= 4; step += 1) {
      await wait(UPDATE_STEP_MS);
      this.setUpdate({ downloaded: (UPDATE_SIZE_BYTES * step) / 4 });
    }
    this.setUpdate({ phase: "installing" });
    await wait(UPDATE_STEP_MS);
    this.version = release.version;
    this.nextRelease = null;
    this.setUpdateStatus({ supported: true, currentVersion: release.version, phase: "idle", manual: false, downloaded: 0 });
  }

  /** Test hook: publish `status` the way the core pushes `update-status`. */
  setUpdateStatus(status: UpdateStatus): void {
    this.updateState = structuredClone(status);
    const snapshot = structuredClone(this.updateState);
    for (const listener of this.updateListeners) listener(snapshot);
  }

  async quit(): Promise<void> {}

  private setUpdate(patch: Partial<UpdateStatus>): void {
    this.setUpdateStatus({ ...this.updateState, ...patch });
  }

  private addAccount(provider: AccountProvider, label: string, credentialMode: ConnectedAccount["credentialMode"]): ConnectedAccount {
    const timestamp = new Date().toISOString();
    const hash = mockId().replace(/[^a-f0-9]/g, "").slice(0, 8);
    const card = accountProvider(provider, hash, label);
    const account: ConnectedAccount = { id: card.id, provider, label, connectedAt: timestamp, updatedAt: timestamp, credentialMode };
    this.accounts.push(account);
    const localIndex = this.entries.findIndex((entry) => entry.provider.id === `${provider}-local`);
    const entry = { provider: card, descriptors: accountDescriptors(card, provider) };
    if (localIndex >= 0) this.entries.splice(localIndex, 0, entry);
    else this.entries.push(entry);
    this.update((state) => {
      state.providers[card.id] = { refreshing: true };
    });
    this.emitCatalog();
    setTimeout(() => {
      this.update((state) => {
        state.providers[card.id] = {
          refreshing: false,
          snapshot: {
            providerID: card.id,
            displayName: card.displayName,
            refreshedAt: new Date().toISOString(),
            lines: [
              { type: "progress", label: "Session", used: 4, limit: 100, format: { kind: "percent" }, resetsAt: new Date(Date.now() + 4 * 3_600_000).toISOString(), periodDurationMs: 5 * 3_600_000 },
              { type: "progress", label: "Weekly", used: 11, limit: 100, format: { kind: "percent" }, resetsAt: new Date(Date.now() + 5 * 86_400_000).toISOString(), periodDurationMs: 7 * 86_400_000 },
            ],
          },
        };
      });
    }, REFRESH_DELAY_MS);
    return structuredClone(account);
  }

  private takeLogin(flowId: string): { provider: AccountProvider } | undefined {
    const pending = this.pendingLogins.get(flowId);
    if (!pending) return undefined;
    this.pendingLogins.delete(flowId);
    clearTimeout(pending.timer);
    return pending;
  }

  private emitLogin(result: AccountLoginResult): void {
    for (const listener of this.loginListeners) listener({ ...result });
  }

  private emitCatalog(): void {
    const catalog = structuredClone(this.entries);
    for (const listener of this.catalogListeners) listener(catalog);
  }

  private update(mutate: (state: EngineState) => void): void {
    const next = structuredClone(this.state);
    mutate(next);
    this.state = next;
    for (const listener of this.engineListeners) listener(next);
  }
}
