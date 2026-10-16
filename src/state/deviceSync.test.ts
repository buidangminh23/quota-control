import { setBackend, type DocumentName } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import { reconcileLayout, type LayoutDocument } from "@/model/layout";
import { DEFAULT_SETTINGS } from "@/model/settings";
import { startApp, updateLayout, updateSettings, useApp } from "./store";

const initial = useApp.getState();
let stop: (() => void) | undefined;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

async function boot() {
  const api = new MockBackend();
  setBackend(api);
  const settings = { ...DEFAULT_SETTINGS, showTotalSpend: false };
  const layout = reconcileLayout(null, await api.catalog());
  await api.saveDocument("settings", settings);
  await api.saveDocument("layout", layout);
  const subscribed = vi.spyOn(api, "onDocumentsChanged");
  stop = startApp();
  await vi.waitFor(() => expect(subscribed).toHaveBeenCalledOnce());
  return { api, settings, layout };
}

beforeEach(() => useApp.setState(initial, true));

afterEach(async () => {
  stop?.();
  stop = undefined;
  await new Promise((resolve) => setTimeout(resolve, 5));
  vi.restoreAllMocks();
  useApp.setState(initial, true);
});

it("reloads synchronized preferences and layout through the document event", async () => {
  const { api, settings, layout } = await boot();
  const providerOrder = [...layout.providerOrder].reverse();
  await api.saveDocument("settings", { ...settings, theme: "dark", density: "compact", language: "en" });
  await api.saveDocument("layout", { ...layout, providerOrder });
  useApp.setState({ undoStack: [layout] });
  api.documentsChanged();
  await vi.waitFor(() => {
    expect(useApp.getState().settings).toMatchObject({ theme: "dark", density: "compact", language: "en" });
    expect(useApp.getState().layout.providerOrder).toEqual(providerOrder);
    expect(useApp.getState().undoStack).toEqual([]);
  });
});

it("preserves another device's fields when a local save follows a stale document read", async () => {
  const { api, settings } = await boot();
  const waiting = deferred<unknown>();
  const entered = deferred<void>();
  const load = api.loadDocument.bind(api);
  let intercept = true;
  vi.spyOn(api, "loadDocument").mockImplementation(<T>(name: DocumentName): Promise<T | null> => {
    if (name === "settings" && intercept) {
      intercept = false;
      entered.resolve();
      return waiting.promise as Promise<T | null>;
    }
    return load<T>(name);
  });
  updateSettings({ theme: "dark" });
  await entered.promise;
  await api.saveDocument("settings", { ...settings, density: "compact" });
  api.documentsChanged();
  waiting.resolve(settings);
  await vi.waitFor(async () => {
    expect(await load("settings")).toMatchObject({ theme: "dark", density: "compact" });
    expect(useApp.getState().settings).toMatchObject({ theme: "dark", density: "compact" });
  });
});

it("retries a delayed remote load after local settings and layout edits", async () => {
  const { api, settings, layout } = await boot();
  const providerOrder = [...layout.providerOrder].reverse();
  const remoteSettings = { ...settings, theme: "dark" };
  const remoteLayout = { ...layout, providerOrder };
  await api.saveDocument("settings", remoteSettings);
  await api.saveDocument("layout", remoteLayout);
  const waiting = deferred<unknown>();
  const entered = deferred<void>();
  const load = api.loadDocument.bind(api);
  let intercept = true;
  vi.spyOn(api, "loadDocument").mockImplementation(<T>(name: DocumentName): Promise<T | null> => {
    if (name === "settings" && intercept) {
      intercept = false;
      entered.resolve();
      return waiting.promise as Promise<T | null>;
    }
    return load<T>(name);
  });
  api.documentsChanged();
  await entered.promise;
  updateSettings({ density: "compact" });
  const openProviders = [layout.providerOrder[0]!];
  updateLayout((current) => ({ ...current, openProviders }));
  waiting.resolve(remoteSettings);
  await vi.waitFor(async () => {
    expect(useApp.getState().settings).toMatchObject({ theme: "dark", density: "compact" });
    expect(useApp.getState().layout).toMatchObject({ providerOrder, openProviders });
    expect(await load<LayoutDocument>("layout")).toMatchObject({ providerOrder, openProviders });
  });
});

it("keeps unrelated nested preferences changed remotely before a local toggle", async () => {
  const { api, settings } = await boot();
  await api.saveDocument("settings", { ...settings, notifications: { ...settings.notifications, willRunOut: true } });
  updateSettings({ notifications: { ...useApp.getState().settings.notifications, almostOut: true } });
  await vi.waitFor(async () => {
    const saved = await api.loadDocument<{ notifications: { almostOut: boolean; willRunOut: boolean } }>("settings");
    expect(saved?.notifications).toMatchObject({ almostOut: true, willRunOut: true });
  });
});
