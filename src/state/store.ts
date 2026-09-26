/**
 * The popup's single store: backend data (catalog, engine state, accounts, chat sessions), the
 * user's settings and layout documents, navigation and transient notices. Components read it through
 * `useApp`; every mutation goes through the actions below so persistence and undo stay in one place.
 */
import { create } from "zustand";
import { backend } from "@/lib/backend";
import type {
  AccountProvider,
  AppInfo,
  ChatSession,
  ConnectedAccount,
  EngineState,
  PopoverScreen,
  ProviderEntry,
  UpdateStatus,
} from "@/lib/types";
import { reconcileLayout, parseLayout, resetAllLayout, sameLayout, type LayoutDocument } from "@/model/layout";
import { DEFAULT_SETTINGS, enabledProvidersOf, mergeSettingsDocument, parseSettings, type AppSettings } from "@/model/settings";
import { isTransientBanner, updateBannerKey, updateBannerOf } from "@/model/updateBanner";
import type { DisplayOptions } from "@/model/widgetData";

export type Screen = PopoverScreen | "accounts";

export type NoticeTone = "positive" | "notice";

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
  chats: ChatSession[];
  settings: AppSettings;
  /** Providers the core refreshes; `null` means all of them (the core's default). */
  enabledProviders: string[] | null;
  layout: LayoutDocument;
  undoStack: LayoutDocument[];
  screen: Screen;
  /** The screen being left, for the slide direction. */
  previousScreen: Screen;
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
  chats: [],
  settings: DEFAULT_SETTINGS,
  enabledProviders: null,
  layout: reconcileLayout(null, []),
  undoStack: [],
  screen: "dashboard",
  previousScreen: "dashboard",
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
  const state = get();
  const all = state.catalog.map((entry) => entry.provider.id);
  const current = new Set(state.enabledProviders ?? all);
  if (enabled) current.add(providerId);
  else current.delete(providerId);
  const ids = all.filter((id) => current.has(id));
  set({ enabledProviders: ids });
  void backend().setEnabledProviders(ids).catch(logFailure("Enabling providers"));
}

export function navigate(screen: Screen, customizeProviderId: string | null = null): void {
  const { screen: current } = get();
  set({ screen, previousScreen: current, customizeProviderId: screen === "customize" ? customizeProviderId : null });
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
  set({ screen: "dashboard", previousScreen: "dashboard", customizeProviderId: null });
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
  void reloadAccounts();
  void reloadChats();
  return [
    api.onUpdateStatus?.((next) => set({ update: next })) ?? (() => {}),
    api.onEngineState((state) => set({ engine: state })),
    api.onCatalogChanged((next) => {
      applyCatalog(next);
      void reloadAccounts();
      void reloadEnabledProviders();
    }),
    api.onPopupVisibility((shown) => {
      set({ popupVisible: shown });
      if (!shown) resetTransientState();
      else void reloadChats();
    }),
    api.onNavigate((screen) => navigate(screen)),
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
