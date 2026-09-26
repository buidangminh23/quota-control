/**
 * The popup's single store: backend data (catalog, engine state, accounts, chat sessions), the
 * user's settings and layout documents, navigation and transient notices. Components read it through
 * `useApp`; every mutation goes through the actions below so persistence and undo stay in one place.
 */
import { create } from "zustand";
import { messagesFor, translate } from "@/i18n";
import { backend } from "@/lib/backend";
import type {
  AccountLoginResult,
  AccountProvider,
  AppInfo,
  ChatSession,
  ConnectedAccount,
  EngineState,
  LoginBrowser,
  PopoverScreen,
  ProviderEntry,
  UpdateStatus,
} from "@/lib/types";
import { hasDashboardCard, reconcileLayout, parseLayout, resetAllLayout, sameLayout, type LayoutDocument } from "@/model/layout";
import { brandName } from "@/model/providerText";
import { DASHBOARD_TABS, DEFAULT_SETTINGS, enabledProvidersOf, mergeSettingsDocument, parseSettings, type AppSettings, type DashboardTab } from "@/model/settings";
import { isTransientBanner, updateBannerKey, updateBannerOf } from "@/model/updateBanner";
import type { DisplayOptions } from "@/model/widgetData";

export type Screen = PopoverScreen | "accounts";

export type NoticeTone = "positive" | "notice";

/** Which way the screen content slides in: from the right (`forward`) or from the left (`back`). */
export type Motion = "forward" | "back";

/** A browser sign-in in progress. It outlives the popup, which hides while the browser is in front. */
export type AccountLogin =
  | { phase: "starting"; provider: AccountProvider }
  | { phase: "waiting"; provider: AccountProvider; flowId: string; browser: LoginBrowser };

export interface AccountLoginError {
  provider: AccountProvider;
  text: string;
}

export interface Notice {
  text: string;
  tone: NoticeTone;
  /** Bumped on every present so a repeated notice replays its entrance. */
  id: number;
}

export interface AppState {
  ready: boolean;
  info: AppInfo | null;
  catalog: ProviderEntry[];
  engine: EngineState | null;
  accounts: ConnectedAccount[];
  accountLogin: AccountLogin | null;
  /** Why the last sign-in did not connect, until the next one starts. */
  accountLoginError: AccountLoginError | null;
  chats: ChatSession[];
  settings: AppSettings;
  /** Providers the core refreshes; `null` means all of them (the core's default). */
  enabledProviders: string[] | null;
  layout: LayoutDocument;
  undoStack: LayoutDocument[];
  screen: Screen;
  /** The screen being left, for the slide direction. */
  previousScreen: Screen;
  /** How the dashboard content enters after a tab switch; cleared by any navigation. */
  tabMotion: Motion | null;
  customizeProviderId: string | null;
  popupVisible: boolean;
  notice: Notice | null;
  /** The core's self-update state; `null` where there is no updater. */
  update: UpdateStatus | null;
  /** Key of the update card the user closed (`updateBannerKey`). */
  dismissedUpdate: string | null;
}

/** Undo depth, matching upstream `LayoutUndoHistory`. */
export const UNDO_DEPTH = 40;
const NOTICE_TIMEOUT_MS = 2500;

export const useApp = create<AppState>(() => ({
  ready: false,
  info: null,
  catalog: [],
  engine: null,
  accounts: [],
  accountLogin: null,
  accountLoginError: null,
  chats: [],
  settings: DEFAULT_SETTINGS,
  enabledProviders: null,
  layout: reconcileLayout(null, []),
  undoStack: [],
  screen: "dashboard",
  previousScreen: "dashboard",
  tabMotion: null,
  customizeProviderId: null,
  popupVisible: true,
  notice: null,
  update: null,
  dismissedUpdate: null,
}));

const get = () => useApp.getState();
const set = useApp.setState;

export function isProviderEnabled(state: Pick<AppState, "enabledProviders">, providerId: string): boolean {
  return state.enabledProviders === null || state.enabledProviders.includes(providerId);
}

/** The providers the Token tab reads: local token history, which has no card or switch of its own. */
export function tokenSourceIds(catalog: readonly ProviderEntry[]): string[] {
  return catalog
    .filter((entry) => !hasDashboardCard(entry.provider.id) && entry.descriptors.some((descriptor) => descriptor.isSpendTile))
    .map((entry) => entry.provider.id);
}

/**
 * The dashboard has its Token tab while "Hiện tab Token" is on and the core can read token history.
 * It never depends on which sources are enabled: `enableTokenSources` turns them back on instead.
 */
export function hasTokensTab(state: Pick<AppState, "settings" | "catalog">): boolean {
  return state.settings.showTotalSpend && tokenSourceIds(state.catalog).length > 0;
}

/** The dashboard tab on screen: the saved one, or Hạn mức while there is no Token tab. */
export function visibleDashboardTab(state: Pick<AppState, "settings" | "catalog">): DashboardTab {
  return hasTokensTab(state) ? state.settings.dashboardTab : "quota";
}

export function displayOptionsOf(settings: AppSettings): DisplayOptions {
  return {
    displayMode: settings.displayMode,
    resetDisplayMode: settings.resetDisplayMode,
    alwaysShowPacing: settings.alwaysShowPacing,
    timeFormat: settings.timeFormat,
    language: settings.language,
  };
}

function logFailure(action: string) {
  return (error: unknown) => console.error(`${action} failed`, error);
}

let settingsWrites: Promise<void> = Promise.resolve();
let layoutWrites: Promise<void> = Promise.resolve();

/** Save the popup's settings over the freshly stored document (core-owned keys preserved). */
function persistSettings(): void {
  settingsWrites = settingsWrites
    .then(async () => {
      const api = backend();
      const stored = await api.loadDocument<unknown>("settings");
      const known = new Set(get().catalog.map((entry) => entry.provider.id));
      await api.saveDocument("settings", mergeSettingsDocument(stored, get().settings, known));
    })
    .catch(logFailure("Saving settings"));
}

function persistLayout(): void {
  layoutWrites = layoutWrites.then(() => backend().saveDocument("layout", get().layout)).catch(logFailure("Saving layout"));
}

export function updateSettings(patch: Partial<AppSettings>): void {
  set({ settings: { ...get().settings, ...patch } });
  persistSettings();
  if (patch.showTotalSpend) enableTokenSources();
}

/**
 * Keep the Token tab's sources enabled while the tab is on. Before 0.1.6 they were dashboard cards,
 * and hiding such a card disabled its source; with no card or switch left to undo that, the tab
 * would stay empty (or, in 0.1.6, missing) for good.
 */
function enableTokenSources(): void {
  const { settings, catalog, enabledProviders } = get();
  if (!settings.showTotalSpend || enabledProviders === null) return;
  const disabled = tokenSourceIds(catalog).filter((providerId) => !enabledProviders.includes(providerId));
  if (disabled.length > 0) setProvidersEnabled(disabled, true);
}

/** Show a dashboard tab; its content slides in from the side the tab sits on. */
export function selectDashboardTab(tab: DashboardTab): void {
  const current = visibleDashboardTab(get());
  if (tab === current) return;
  set({ tabMotion: DASHBOARD_TABS.indexOf(tab) > DASHBOARD_TABS.indexOf(current) ? "forward" : "back" });
  updateSettings({ dashboardTab: tab });
}

/** The tab after (`step` 1) or before (`step` -1) the one on screen, wrapping around. */
export function cycleDashboardTab(step: 1 | -1): void {
  const index = DASHBOARD_TABS.indexOf(visibleDashboardTab(get()));
  selectDashboardTab(DASHBOARD_TABS[(index + step + DASHBOARD_TABS.length) % DASHBOARD_TABS.length]!);
}

/** Apply a layout change; `undoable` records the prior layout (upstream `recordingUndoStep`). */
export function updateLayout(mutate: (layout: LayoutDocument) => LayoutDocument, options: { undoable?: boolean } = {}): boolean {
  const { layout, undoStack } = get();
  const next = mutate(layout);
  if (next === layout || sameLayout(next, layout)) return false;
  set({
    layout: next,
    undoStack: options.undoable === false ? undoStack : [...undoStack, layout].slice(-UNDO_DEPTH),
  });
  persistLayout();
  return true;
}

/** Step back one customization; caret open/closed state is not part of undo. */
export function undoLayout(): boolean {
  const { undoStack, layout } = get();
  const previous = undoStack.at(-1);
  if (!previous) return false;
  set({ layout: { ...previous, openProviders: layout.openProviders }, undoStack: undoStack.slice(0, -1) });
  persistLayout();
  return true;
}

export function resetAllCustomization(): void {
  const layout = resetAllLayout(get().catalog);
  set({ layout, undoStack: [] });
  persistLayout();
  const ids = get().catalog.map((entry) => entry.provider.id);
  set({ enabledProviders: ids });
  void backend().setEnabledProviders(ids).catch(logFailure("Enabling providers"));
}

export function resetAllSettings(): void {
  set({ settings: DEFAULT_SETTINGS });
  persistSettings();
  resetAllCustomization();
}

export function setProviderEnabled(providerId: string, enabled: boolean): void {
  setProvidersEnabled([providerId], enabled);
}

/**
 * The core rewrites the settings document to record the selection, so the call queues behind any
 * settings save in flight: that save must not write back the selection it read before this one.
 */
function setProvidersEnabled(providerIds: readonly string[], enabled: boolean): void {
  const state = get();
  const all = state.catalog.map((entry) => entry.provider.id);
  const current = new Set(state.enabledProviders ?? all);
  for (const providerId of providerIds) {
    if (enabled) current.add(providerId);
    else current.delete(providerId);
  }
  const ids = all.filter((id) => current.has(id));
  set({ enabledProviders: ids });
  settingsWrites = settingsWrites.then(() => backend().setEnabledProviders(ids)).catch(logFailure("Enabling providers"));
}

export function navigate(screen: Screen, customizeProviderId: string | null = null): void {
  const { screen: current } = get();
  set({ screen, previousScreen: current, tabMotion: null, customizeProviderId: screen === "customize" ? customizeProviderId : null });
}

export function openCustomizeDetail(providerId: string | null): void {
  set({ customizeProviderId: providerId });
}

let noticeTimer: ReturnType<typeof setTimeout> | undefined;

export function showNotice(text: string, tone: NoticeTone = "positive"): void {
  clearTimeout(noticeTimer);
  set({ notice: { text, tone, id: (get().notice?.id ?? 0) + 1 } });
  noticeTimer = setTimeout(() => set({ notice: null }), NOTICE_TIMEOUT_MS);
}

export function clearNotice(): void {
  clearTimeout(noticeTimer);
  set({ notice: null });
}

export function refresh(providerId?: string): void {
  void backend().refresh(providerId).catch(logFailure("Refresh"));
}

/** Look for a new release now; the result shows on the update card and in Settings. */
export function checkForUpdates(): void {
  const api = backend();
  if (!api.checkForUpdate) return;
  set({ dismissedUpdate: null });
  void api
    .checkForUpdate()
    .then((update) => set({ update }))
    .catch(logFailure("Checking for updates"));
}

/** Download and install the newest release. On success the core exits or restarts the app. */
export function installUpdate(): void {
  const api = backend();
  if (!api.installUpdate) return;
  set({ dismissedUpdate: null });
  void api.installUpdate().catch(logFailure("Installing the update"));
}

/** Close the update card until something new happens (upstream's banner close button). */
export function dismissUpdate(): void {
  const key = updateBannerKey(get().update);
  if (key) set({ dismissedUpdate: key });
}

export async function reloadAccounts(): Promise<void> {
  try {
    set({ accounts: await backend().listAccounts() });
  } catch (error) {
    logFailure("Listing accounts")(error);
  }
}

function errorText(error: unknown): string {
  const raw = error instanceof Error ? error.message : typeof error === "string" ? error : String(error);
  return translate(raw, get().settings.language);
}

let loginAttempt = 0;

/**
 * Open the provider's sign-in page. The core waits for the browser, saves the account and reports
 * the outcome through `account-login`, so nothing here waits for the sign-in itself.
 */
export async function startAccountLogin(provider: AccountProvider): Promise<void> {
  const attempt = ++loginAttempt;
  const previous = get().accountLogin;
  if (previous?.phase === "waiting") void backend().cancelAccountLogin(previous.flowId).catch(logFailure("Cancelling the sign-in"));
  set({ accountLogin: { phase: "starting", provider }, accountLoginError: null });
  try {
    const login = await backend().beginAccountLogin(provider, get().settings.language);
    if (attempt !== loginAttempt) {
      void backend().cancelAccountLogin(login.flowId).catch(logFailure("Cancelling the sign-in"));
      return;
    }
    set({ accountLogin: { phase: "waiting", provider, flowId: login.flowId, browser: login.browser } });
  } catch (error) {
    if (attempt === loginAttempt) set({ accountLogin: null, accountLoginError: { provider, text: errorText(error) } });
  }
}

/** Show the waiting login's sign-in page again, in the same browser. */
export async function reopenAccountLogin(): Promise<void> {
  const current = get().accountLogin;
  if (current?.phase !== "waiting") return;
  try {
    const browser = await backend().reopenAccountLogin(current.flowId);
    if (get().accountLogin === current) set({ accountLogin: { ...current, browser } });
  } catch (error) {
    if (get().accountLogin === current) set({ accountLogin: null, accountLoginError: { provider: current.provider, text: errorText(error) } });
  }
}

export function cancelAccountLogin(): void {
  const current = get().accountLogin;
  loginAttempt += 1;
  set({ accountLogin: null });
  if (current?.phase === "waiting") void backend().cancelAccountLogin(current.flowId).catch(logFailure("Cancelling the sign-in"));
}

/** A connected account is announced even when its login is no longer the one shown as waiting. */
function applyLoginResult(result: AccountLoginResult): void {
  const messages = messagesFor(get().settings.language).accounts;
  const brand = brandName(result.provider);
  if (result.status === "connected") {
    void reloadAccounts();
    showNotice(messages.connectedNotice(brand), "positive");
  }
  const current = get().accountLogin;
  if (current?.phase !== "waiting" || current.flowId !== result.flowId) return;
  set({ accountLogin: null });
  if (result.status === "failed" || result.status === "expired") {
    const text = translate(result.error ?? "", get().settings.language);
    set({ accountLoginError: { provider: result.provider, text } });
    if (result.status === "failed") showNotice(messages.notConnectedNotice(brand), "notice");
  }
}

export async function reloadChats(): Promise<void> {
  try {
    set({ chats: await backend().listChatSessions() });
  } catch (error) {
    logFailure("Listing chat sessions")(error);
  }
}

async function reloadEnabledProviders(): Promise<void> {
  try {
    set({ enabledProviders: enabledProvidersOf(await backend().loadDocument<unknown>("settings")) });
  } catch (error) {
    logFailure("Reading provider selection")(error);
  }
}

/** Open the saved in-app chat for this account label, creating one the first time. */
export async function openChatFor(provider: AccountProvider, label: string): Promise<void> {
  const api = backend();
  const existing = get().chats.find((chat) => chat.provider === provider && chat.label === label);
  if (existing) {
    await api.openChatSession(existing.id);
    return;
  }
  const created = await api.createChatSession(provider, label);
  set({ chats: [...get().chats, created] });
}

function applyCatalog(catalog: ProviderEntry[]): void {
  const previous = get().layout;
  const layout = reconcileLayout(previous, catalog);
  set({ catalog, layout });
  if (!sameLayout(previous, layout)) persistLayout();
}

function resetTransientState(): void {
  set({ screen: "dashboard", previousScreen: "dashboard", tabMotion: null, customizeProviderId: null });
  clearNotice();
  if (isTransientBanner(updateBannerOf(get().update))) dismissUpdate();
}

/** Load everything the popup shows and subscribe to the core's events; resolves to the teardown. */
async function boot(): Promise<Array<() => void>> {
  const api = backend();
  const [info, catalog, engine, settingsDoc, layoutDoc, update] = await Promise.all([
    api.appInfo(),
    api.catalog(),
    api.engineState(),
    api.loadDocument<unknown>("settings").catch(() => null),
    api.loadDocument<unknown>("layout").catch(() => null),
    api.updateStatus?.().catch(() => null) ?? Promise.resolve(null),
  ]);
  const stored = parseLayout(layoutDoc);
  const layout = reconcileLayout(stored, catalog);
  set({
    ready: true,
    info,
    catalog,
    engine,
    settings: parseSettings(settingsDoc),
    enabledProviders: enabledProvidersOf(settingsDoc),
    layout,
    update,
  });
  if (!stored || !sameLayout(stored, layout)) persistLayout();
  enableTokenSources();
  void reloadAccounts();
  void reloadChats();
  return [
    api.onUpdateStatus?.((next) => set({ update: next })) ?? (() => {}),
    api.onEngineState((state) => set({ engine: state })),
    api.onCatalogChanged((next) => {
      applyCatalog(next);
      void reloadAccounts();
      void reloadEnabledProviders().then(enableTokenSources);
    }),
    api.onPopupVisibility((shown) => {
      set({ popupVisible: shown });
      if (!shown) resetTransientState();
      else void reloadChats();
    }),
    api.onNavigate((screen) => navigate(screen)),
    api.onAccountLogin(applyLoginResult),
  ];
}

let session: Promise<Array<() => void>> | null = null;
let holders = 0;

/**
 * Start the popup's data session and return its release. Holders are counted and the teardown is
 * deferred a tick, so React StrictMode's mount → unmount → mount keeps one live subscription set
 * instead of tearing it down; a real unmount releases it.
 */
export function startApp(): () => void {
  holders += 1;
  session ??= boot().catch((error: unknown) => {
    console.error("Starting the popup failed", error);
    return [];
  });
  let released = false;
  return () => {
    if (released) return;
    released = true;
    holders -= 1;
    setTimeout(() => {
      if (holders > 0 || !session) return;
      const ending = session;
      session = null;
      void ending.then((unsubscribers) => {
        for (const unsubscribe of unsubscribers) unsubscribe();
      });
    }, 0);
  };
}
