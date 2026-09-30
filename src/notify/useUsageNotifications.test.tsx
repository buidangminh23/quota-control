import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { App } from "@/App";
import { setBackend } from "@/lib/backend";
import type { AppInfo } from "@/lib/types";
import { MockBackend } from "@/lib/mockBackend";
import type { GlanceDocument } from "@/model/glance";
import { resetInsights } from "@/state/insights";
import { useApp } from "@/state/store";
import { watchedMilestones } from "./useUsageNotifications";

vi.mock("@/strip/useTaskbarStrip", () => ({ useTaskbarStrip: () => {} }));

const FRESH = useApp.getState();
const OFF = { almostOut: false, cuttingItClose: false, willRunOut: false };

/** The mock backend on a Mac, recording every glance document the popup sends. */
class MacBackend extends MockBackend {
  readonly glances: GlanceDocument[] = [];

  override async appInfo(): Promise<AppInfo> {
    return { ...(await super.appInfo()), platform: "macos" };
  }

  async setGlance(document: GlanceDocument): Promise<void> {
    this.glances.push(structuredClone(document));
  }

  get latest(): GlanceDocument | undefined {
    return this.glances.at(-1);
  }
}

const shown: string[] = [];

/** Stands in for the browser's notifications, which `notify` uses outside the app. */
class FakeNotification {
  static permission = "granted";

  constructor(title: string) {
    shown.push(title);
  }
}

async function start(settings: Record<string, unknown>): Promise<MacBackend> {
  const api = new MacBackend();
  await api.saveDocument("settings", settings);
  setBackend(api);
  render(<App />);
  await screen.findByText("Claude · Công ty");
  await waitFor(() => expect(api.latest).toBeDefined());
  return api;
}

/** Set how much of the Codex account's 5-hour limit is used, as the core would report it. */
function useCodexSession(api: MacBackend, used: number): void {
  act(() =>
    api.editEngineState((state) => {
      const line = state.providers["codex@52d0"]?.snapshot?.lines.find((entry) => entry.type === "progress" && entry.label === "Session");
      if (line?.type === "progress") line.used = used;
    }),
  );
}

async function settle(): Promise<void> {
  await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
}

beforeEach(() => {
  useApp.setState(FRESH, true);
  resetInsights();
  shown.length = 0;
  vi.stubGlobal("Notification", FakeNotification);
});

afterEach(async () => {
  cleanup();
  vi.unstubAllGlobals();
  await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
  act(() => useApp.setState({ screen: "dashboard", previousScreen: "dashboard", tabMotion: null }));
});

describe("watchedMilestones", () => {
  it("adds a limit running low for an island that opens for alerts, and nothing else", () => {
    expect(watchedMilestones(OFF, true)).toEqual({ ...OFF, almostOut: true });
    expect(watchedMilestones(OFF, false)).toEqual(OFF);
    const on = { almostOut: false, cuttingItClose: true, willRunOut: true };
    expect(watchedMilestones(on, true)).toEqual({ almostOut: true, cuttingItClose: true, willRunOut: true });
  });
});

describe("usage alerts on the Dynamic Island", () => {
  it("open the island for a limit running low while every notification switch is off, as its default does", async () => {
    const api = await start({});
    const before = api.latest?.alert?.id;
    useCodexSession(api, 95);
    await waitFor(() => expect(api.latest?.alert?.id).not.toBe(before));
    expect(api.latest?.alert?.body).toMatch(/^Chỉ còn 5% hạn mức\./);
    expect(api.latest?.alert?.severity).toBe("critical");
    expect(shown).toEqual([]);
  });

  it("also send the system notification once its switch is on", async () => {
    const api = await start({ notifications: { ...OFF, almostOut: true } });
    const before = api.latest?.alert?.id;
    useCodexSession(api, 96);
    await waitFor(() => expect(api.latest?.alert?.id).not.toBe(before));
    expect(api.latest?.alert?.body).toMatch(/^Chỉ còn 4% hạn mức\./);
    expect(shown).toHaveLength(1);
  });

  it.each([
    ["Open for Alerts is off", { island: { alerts: false } }],
    ["the island is off", { dynamicIsland: false }],
  ])("stay closed while %s and the notifications are off", async (_case, settings) => {
    const api = await start(settings);
    const before = api.latest?.alert?.id;
    useCodexSession(api, 97);
    await settle();
    expect(api.latest?.alert?.id).toBe(before);
    expect(shown).toEqual([]);
  });
});
