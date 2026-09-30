import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { App } from "@/App";
import { insightsFor } from "@/i18n/insights";
import { setBackend } from "@/lib/backend";
import type { AppInfo } from "@/lib/types";
import type { PublicFeedName, PublicFeedSnapshot } from "@/lib/insightsTypes";
import { MockBackend } from "@/lib/mockBackend";
import { FEED_FIXTURES } from "@/lib/insightsFeedFixtures";
import type { GlanceDocument } from "@/model/glance";
import { parseResetFeeds } from "@/model/glanceResets";
import { buildClaudePresentation, dailyReliability } from "@/model/insights/claudePresentation";
import { parseClaudeResets } from "@/model/insights/claudeResets";
import { parseResets } from "@/model/insights/resets";
import { setProviderOpen } from "@/model/layout";
import { resetInsights } from "@/state/insights";
import { updateLayout, updateSettings, useApp } from "@/state/store";

vi.mock("@/strip/useTaskbarStrip", () => ({ useTaskbarStrip: () => {} }));

/** The store before any popup booted, so each test starts from nothing another test left behind. */
const FRESH = useApp.getState();

/** The mock backend on a Mac: it hosts the glance, and records every document and feed asked for. */
class MacBackend extends MockBackend {
  readonly glances: GlanceDocument[] = [];
  readonly feedsAsked: PublicFeedName[] = [];
  /** The Claude feed's last refresh failed, so the saved copy is served with its error. */
  claudeFeedFails = false;
  /** What the core reports about a feed on top of its saved copy: a failure, and whether it lasted. */
  readonly feedStates: Partial<Record<PublicFeedName, Partial<PublicFeedSnapshot>>> = {};

  override async appInfo(): Promise<AppInfo> {
    return { ...(await super.appInfo()), platform: "macos" };
  }

  async setGlance(document: GlanceDocument): Promise<void> {
    this.glances.push(structuredClone(document));
  }

  override async publicFeed(name: PublicFeedName): Promise<PublicFeedSnapshot> {
    this.feedsAsked.push(name);
    const snapshot = await super.publicFeed(name);
    const failed = name === "claudeResets" && this.claudeFeedFails ? { ...snapshot, error: "offline", stale: true } : snapshot;
    return { ...failed, ...this.feedStates[name] };
  }

  get latest(): GlanceDocument | undefined {
    return this.glances.at(-1);
  }
}

async function start(settings: Record<string, unknown>, prepare?: (api: MacBackend) => void): Promise<MacBackend> {
  const api = new MacBackend();
  await api.saveDocument("settings", settings);
  prepare?.(api);
  setBackend(api);
  render(<App />);
  await screen.findByText("Claude · Công ty");
  await waitFor(() => expect(api.latest).toBeDefined());
  return api;
}

/** Let the feeds load and the document follow them. */
async function settle(): Promise<void> {
  await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
}

beforeEach(() => {
  useApp.setState(FRESH, true);
  resetInsights();
});

afterEach(async () => {
  cleanup();
  await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
  act(() => useApp.setState({ screen: "dashboard", previousScreen: "dashboard", tabMotion: null }));
});

describe("the glance document the popup sends", () => {
  it("carries no Claude tracker for someone who never chose it", async () => {
    const api = await start({});
    await settle();
    await waitFor(() => expect(api.latest?.resets?.brand).toBe("codex"));
    for (const document of api.glances) {
      expect("claudeResets" in document).toBe(false);
      expect("claudeResetsTab" in document.labels).toBe(false);
      expect("resetsProvider" in document.island).toBe(false);
      expect("resetsProvider" in document.widget).toBe(false);
    }
  });

  it("carries the Claude tracker for the surface that chose it while the Reset tab is on", async () => {
    const api = await start({ island: { resetsProvider: "claude" }, notifyClaudeResets: false });
    await waitFor(() => expect(api.latest?.claudeResets?.brand).toBe("claude"));
    const document = api.latest!;
    expect(document.island.resetsProvider).toBe("claude");
    expect("resetsProvider" in document.widget).toBe(false);
    expect(document.labels.claudeResetsTab).toBe("Reset Claude");
    expect(document.claudeResets).toMatchObject({ title: "Reset Claude", site: "https://claude-resets.com" });
    expect(document.resets?.brand).toBe("codex");
    expect(api.feedsAsked).toContain("claudeResets");
  });

  it("shows on the island and the widgets whichever tracker the Reset tab shows, until a surface picks its own", async () => {
    const api = await start({ resetsProvider: "claude", notifyClaudeResets: false });
    await waitFor(() => expect(api.latest?.claudeResets?.brand).toBe("claude"));
    expect(api.latest!.island.resetsProvider).toBe("claude");
    expect(api.latest!.widget.resetsProvider).toBe("claude");
    act(() => updateSettings({ island: { ...useApp.getState().settings.island, resetsProvider: "codex" } }));
    await waitFor(() => expect("resetsProvider" in api.latest!.island).toBe(false));
    expect(api.latest!.widget.resetsProvider).toBe("claude");
    act(() => updateSettings({ resetsProvider: "codex" }));
    await waitFor(() => expect("claudeResets" in api.latest!).toBe(false));
    expect("resetsProvider" in api.latest!.widget).toBe(false);
  });

  it("keeps a surface that follows the Reset tab on Codex while that tab is hidden, as before the choice", async () => {
    const api = await start({ resetsProvider: "claude", showResetsTab: false, notifyClaudeResets: false, notifyCodexResets: true });
    await settle();
    await waitFor(() => expect(api.latest?.resets?.brand).toBe("codex"));
    const document = api.latest!;
    expect("resetsProvider" in document.island).toBe(false);
    expect("resetsProvider" in document.widget).toBe(false);
    expect("claudeResets" in document).toBe(false);
    expect("claudeResetsTab" in document.labels).toBe(false);
    expect(api.feedsAsked).not.toContain("claudeResets");
    act(() => updateSettings({ showResetsTab: true }));
    await waitFor(() => expect(api.latest?.island.resetsProvider).toBe("claude"));
    expect(api.latest!.widget.resetsProvider).toBe("claude");
  });

  it("keeps the Codex tracker to what its status says while the Reset tab is hidden and reset notifications are on", async () => {
    const api = await start({ showResetsTab: false, notifyCodexResets: true });
    await settle();
    await waitFor(() => expect(api.latest?.resets?.brand).toBe("codex"));
    const resets = api.latest!.resets!;
    expect(resets.latest?.at).toBe("2026-09-22T18:23:37.000Z");
    expect(resets.presentation?.latest?.at).toBe("2026-09-22T18:23:37.000Z");
    expect(resets.presentation?.statuses.map((card) => card.kind)).toEqual(["scheduled"]);
    expect(resets.presentation?.forecast.chances).toEqual([]);
    expect(resets.presentation?.forecast.unavailable).toBeUndefined();
    expect(resets.presentation?.stats).toEqual([]);
    expect(resets.presentation?.history).toEqual([]);
    expect(resets.calendar).toBeUndefined();
    expect(resets.rhythm).toBeUndefined();
    expect(api.feedsAsked).not.toContain("codexResets");
  });

  it("says the Codex feeds could not be loaded, in the Reset tab's words, while neither has a copy", async () => {
    const api = await start({}, (backend) => {
      backend.feedStates.codexResetStatus = { body: null, error: "HTTP 500" };
      backend.feedStates.codexResets = { body: null, error: "HTTP 502" };
    });
    await waitFor(() => expect(api.latest?.resetsPending).toEqual({ text: "Chưa tải được: HTTP 500", failed: true }));
    expect("resets" in api.latest!).toBe(false);
    expect(api.latest!.labels.resetsOff).toBe("Bật tab Reset hoặc thông báo reset trong Quota Control để xem dự báo.");
  });

  it("says the Claude feed could not be loaded in place of its tracker, for the surface showing it", async () => {
    const api = await start({ island: { resetsProvider: "claude" }, notifyClaudeResets: false }, (backend) => {
      backend.feedStates.claudeResets = { body: null, error: "offline" };
    });
    await waitFor(() => expect(api.latest?.claudeResetsPending).toEqual({ text: "Chưa tải được: offline", failed: true }));
    expect("claudeResets" in api.latest!).toBe(false);
    expect(api.latest!.resets?.brand).toBe("codex");
    expect("resetsPending" in api.latest!).toBe(false);
  });

  it("dates each tracker's copy the way the Reset tab's line above its source does", async () => {
    const checked = "2026-09-30T01:02:03.000Z";
    const verified = "2026-09-29T22:00:00.000Z";
    const api = await start({ island: { resetsProvider: "claude" }, notifyClaudeResets: false }, (backend) => {
      backend.feedStates.codexResetStatus = { checkedAt: checked };
      backend.feedStates.claudeResets = { error: "offline", verifiedAt: verified };
    });
    await waitFor(() => expect(api.latest?.claudeResets?.presentation?.fetched?.at).toBe(verified));
    expect(api.latest!.resets!.presentation!.fetched).toEqual({ at: checked, text: "Tải {d} trước", since: true, recent: "Vừa tải" });
  });

  it("carries the Reset tab's self-check under the Codex chances once the history is long enough to try", async () => {
    const ago = (days: number) => new Date(Date.now() - days * 86_400_000).toISOString();
    const post = (id: string, days: number) => ({ id, reset_type: "regular", announced_at: ago(days), text: `Codex reset ${id}.`, source: { type: "x_post", author: "thsottiaux", url: `https://x.com/thsottiaux/status/${id}` } });
    const history = [...Array.from({ length: 8 }, (_, index) => post(`1${index}`, 300 - index * 25)), ...Array.from({ length: 33 }, (_, index) => post(`2${index + 10}`, 100 - index * 3))];
    const body = JSON.stringify({ data: history.reverse(), pagination: { has_more: false, next_cursor: null }, meta: { api_version: "v1" } });
    const api = await start({}, (backend) => {
      backend.feedStates.codexResets = { body };
    });
    await waitFor(() => expect(api.latest?.resets?.presentation?.forecast.reliability).toMatch(/^Thử lại trên \d+ ngày đã qua/));
    const feeds = parseResetFeeds(FEED_FIXTURES.codexResetStatus, body);
    expect(api.latest!.resets!.presentation!.forecast.reliability).toBe(dailyReliability(feeds.resets, new Date(), "vi"));
  });

  it("puts the saved-copy note on the Codex tracker while its status has never been read", async () => {
    const api = await start({}, (backend) => {
      backend.feedStates.codexResetStatus = { body: null, error: "HTTP 500", stale: false };
    });
    await waitFor(() => expect(api.latest?.resets?.stale).toBe(insightsFor("vi").staleNote));
  });

  it("follows Claude reset notifications alone when the Reset tab is off", async () => {
    const api = await start({ widget: { resetsProvider: "claude" }, showResetsTab: false, notifyClaudeResets: true });
    await waitFor(() => expect(api.latest?.claudeResets?.brand).toBe("claude"));
    expect(api.latest!.widget.resetsProvider).toBe("claude");
  });

  it("leaves the Claude tracker out, and its feed unasked, while the Reset tab and Claude notifications are both off", async () => {
    const api = await start({ island: { resetsProvider: "claude" }, widget: { resetsProvider: "claude" }, showResetsTab: false, notifyClaudeResets: false });
    await settle();
    const document = api.latest!;
    expect(document.labels.claudeResetsTab).toBe("Reset Claude");
    expect("claudeResets" in document).toBe(false);
    expect(api.feedsAsked).not.toContain("claudeResets");
  });

  it("builds the Claude tracker in the popup's theme and language, and follows a language change", async () => {
    const api = await start({ island: { resetsProvider: "claude" }, notifyClaudeResets: false, theme: "dark" });
    await waitFor(() => expect(api.latest?.claudeResets?.title).toBe("Reset Claude"));
    expect(api.latest!.claudeResets!.theme).toBe("dark");
    act(() => updateSettings({ language: "en" }));
    await waitFor(() => expect(api.latest?.claudeResets?.title).toBe("Claude Resets"));
    expect(api.latest!.labels.claudeResetsTab).toBe("Claude Resets");
  });

  it("marks the Claude tracker stale while its feed cannot be refreshed", async () => {
    const api = await start({ widget: { resetsProvider: "claude" }, notifyClaudeResets: false }, (backend) => {
      backend.claudeFeedFails = true;
    });
    await waitFor(() => expect(api.latest?.claudeResets?.brand).toBe("claude"));
    expect(api.latest!.claudeResets!.stale).toBe(insightsFor("vi").staleNote);
  });

  it("keeps the saved-copy note off the Codex tracker while the status still names the list's newest reset", async () => {
    const api = await start({}, (backend) => {
      backend.feedStates.codexResets = { error: "error sending request", stale: true };
    });
    await settle();
    await waitFor(() => expect(api.latest?.resets?.brand).toBe("codex"));
    expect(api.latest!.resets!.stale).toBeUndefined();
  });

  it("puts the saved-copy note on the Codex tracker once the status names a reset the saved list lacks", async () => {
    const api = await start({}, (backend) => {
      backend.feedStates.codexResets = { error: "error sending request", stale: true };
      backend.feedStates.codexResetStatus = { body: FEED_FIXTURES.codexResetStatus.replaceAll("2102463847714247142", "2103911959544610829") };
    });
    await waitFor(() => expect(api.latest?.resets?.stale).toBe(insightsFor("vi").staleNote));
  });

  it("keeps the saved-copy note off the Codex tracker after a single failed check of its status", async () => {
    const api = await start({}, (backend) => {
      backend.feedStates.codexResetStatus = { error: "HTTP 503", stale: false };
    });
    await settle();
    await waitFor(() => expect(api.latest?.resets?.brand).toBe("codex"));
    expect(api.latest!.resets!.stale).toBeUndefined();
  });

  it("puts the saved-copy note on the Codex tracker once its status has been failing for a while", async () => {
    const api = await start({}, (backend) => {
      backend.feedStates.codexResetStatus = { error: "HTTP 503", stale: true };
    });
    await waitFor(() => expect(api.latest?.resets?.stale).toBe(insightsFor("vi").staleNote));
  });

  it("clears the saved-copy note once the core reports the feed recovered", async () => {
    const api = await start({}, (backend) => {
      backend.feedStates.codexResetStatus = { error: "HTTP 503", stale: true };
    });
    await waitFor(() => expect(api.latest?.resets?.stale).toBe(insightsFor("vi").staleNote));
    delete api.feedStates.codexResetStatus;
    act(() => api.announceFeed("codexResetStatus"));
    await waitFor(() => expect(api.latest?.resets?.stale).toBeUndefined());
  });

  it("keeps the Claude tracker's saved-copy note off after a single failed check", async () => {
    const api = await start({ widget: { resetsProvider: "claude" }, notifyClaudeResets: false }, (backend) => {
      backend.feedStates.claudeResets = { error: "offline", stale: false };
    });
    await waitFor(() => expect(api.latest?.claudeResets?.brand).toBe("claude"));
    expect(api.latest!.claudeResets!.stale).toBeUndefined();
  });

  it("drops the Claude tracker once the Reset tab and Claude notifications are both turned off", async () => {
    const api = await start({ island: { resetsProvider: "claude" }, notifyClaudeResets: false });
    await waitFor(() => expect(api.latest?.claudeResets?.brand).toBe("claude"));
    act(() => updateSettings({ showResetsTab: false }));
    await waitFor(() => expect("claudeResets" in api.latest!).toBe(false));
    expect(api.latest!.labels.claudeResetsTab).toBe("Reset Claude");
  });

  const redeems = (document: GlanceDocument) =>
    [...document.providers, ...document.widget.providers].flatMap((entry) => entry.metrics.flatMap((metric) => (metric.redeem ? [`${entry.id}|${metric.id}|${metric.redeem.providerId}`] : [])));

  it("puts the popup's Dùng 1 lượt under a Codex account's reset credits while the app can spend one", async () => {
    const api = await start({});
    await waitFor(() => expect(redeems(api.latest!).length).toBeGreaterThan(0));
    for (const entry of redeems(api.latest!)) {
      const [provider, metric, spends] = entry.split("|");
      expect(provider).toMatch(/^codex@/);
      expect(metric).toBe(`${provider}.rateLimitResets`);
      expect(spends).toBe(provider);
    }
  });

  it("leaves Dùng 1 lượt out where the app cannot spend a reset, as the popup does", async () => {
    const api = await start({}, (backend) => {
      Object.defineProperty(backend, "redeemLimitReset", { value: undefined });
    });
    await settle();
    expect(api.glances.flatMap(redeems)).toEqual([]);
  });

  it("draws every widget in the app's theme, and leaves the theme out while it follows the Mac", async () => {
    const api = await start({ theme: "dark" });
    await waitFor(() => expect(api.latest?.theme).toBe("dark"));
    act(() => updateSettings({ theme: "light" }));
    await waitFor(() => expect(api.latest?.theme).toBe("light"));
    act(() => updateSettings({ theme: "system" }));
    await waitFor(() => expect("theme" in api.latest!).toBe(false));
  });

  it("shows the plan on the island for settings saved while it was hidden by default", async () => {
    const api = await start({ island: { content: "dashboard", showPlan: false, showAccount: true } });
    expect(api.latest!.island.shows.plan).toBe(true);
    expect(api.latest!.providers.find((provider) => provider.id === "codex@52d0")?.term).toMatchObject({ on: expect.any(String) });
    expect(api.latest!.labels.planTerm?.until).toBe("tới {d}");
  });

  it("sets the Claude tracker against the Codex history alone, as the Reset tab's comparison does", async () => {
    const newer = FEED_FIXTURES.codexResetStatus.replaceAll("2102463847714247142", "2103911959544610829");
    const api = await start({ island: { resetsProvider: "claude" }, notifyClaudeResets: false }, (backend) => {
      backend.feedStates.codexResetStatus = { body: newer };
    });
    await waitFor(() => expect(api.latest?.claudeResets?.presentation?.compare).toBeDefined());
    const compare = api.latest!.claudeResets!.presentation!.compare!;
    const popup = buildClaudePresentation({ feed: parseClaudeResets(FEED_FIXTURES.claudeResets)!, codex: parseResets(FEED_FIXTURES.codexResets), plans: [], used: [], now: new Date(), language: "vi", timeFormat: "auto" }).compare!;
    expect(compare.rows[0]).toEqual(popup.rows[0]);
    expect(compare.months.map((month) => month.codex)).toEqual(popup.months.map((month) => month.codex));
    expect(compare.columns).toMatchObject({ claude: { name: "Claude", color: "#DE7356" }, codex: { name: "Codex", color: "#10A37F" } });
  });

  it("leaves the comparison out while the Reset tab has not loaded the Codex history", async () => {
    const api = await start({ island: { resetsProvider: "claude" }, showResetsTab: false, notifyClaudeResets: true });
    await waitFor(() => expect(api.latest?.claudeResets?.brand).toBe("claude"));
    expect(api.latest!.claudeResets!.presentation!.changes?.length).toBeGreaterThan(0);
    expect("compare" in api.latest!.claudeResets!.presentation!).toBe(false);
    expect(api.feedsAsked).not.toContain("codexResets");
  });

  it("moves a wing that follows the Reset tab with the tab's Codex | Claude switch", async () => {
    const api = await start({ island: { wings: ["resets:chance-1", ""] }, notifyClaudeResets: false });
    await waitFor(() => expect(api.latest?.island.wings[0]?.id).toBe("codex-resets"));
    expect(api.latest!.island.wings[0]).toMatchObject({ brand: "codex", metrics: [{ id: "codex-resets:chance-1" }] });
    act(() => updateSettings({ resetsProvider: "claude" }));
    await waitFor(() => expect(api.latest?.island.wings[0]?.id).toBe("claude-resets"));
    expect(api.latest!.island.wings[0]).toMatchObject({ brand: "claude", metrics: [{ id: "claude-resets:chance-1" }] });
    const settings = useApp.getState().settings;
    act(() => updateSettings({ island: { ...settings.island, resetsProvider: "codex" }, widget: { ...settings.widget, resetsProvider: "codex" } }));
    await waitFor(() => expect("claudeResets" in api.latest!).toBe(false));
    expect(api.latest!.island.wings[0]?.id).toBe("claude-resets");
  });

  it("loads the Claude tracker for a Claude wing without copying it into the document", async () => {
    const api = await start({ island: { wings: ["claude-resets:since", ""] }, notifyClaudeResets: false });
    await waitFor(() => expect(api.latest?.island.wings[0]?.id).toBe("claude-resets"));
    const document = api.latest!;
    expect(document.island.wings[0]).toMatchObject({ brand: "claude", metrics: [{ id: "claude-resets:since" }] });
    expect("claudeResets" in document).toBe(false);
    expect("claudeResetsTab" in document.labels).toBe(false);
  });
});

describe("the banked reset the Claude tracker counts down to", () => {
  const BANKED = "2102438800836489554";
  const DEADLINE = "2026-10-22T23:59:59.000Z";
  const CLAUDE = { island: { resetsProvider: "claude" }, notifyClaudeResets: false };

  /** The fixture catalog's banked reset is open until 22/10/2026; pin the clock inside that window. */
  beforeEach(() => vi.useFakeTimers({ toFake: ["Date"], now: new Date("2026-09-29T13:00:00Z") }));
  afterEach(() => vi.useRealTimers());

  it("is the one open for the plans of the Claude accounts connected here", async () => {
    const api = await start(CLAUDE);
    await waitFor(() => expect(api.latest?.claudeResets?.upcoming?.hideAt).toBe(DEADLINE));
  });

  it("goes once the user marks it as used", async () => {
    const api = await start({ ...CLAUDE, usedBankedResets: [BANKED] });
    await waitFor(() => expect(api.latest?.claudeResets?.brand).toBe("claude"));
    expect(api.latest!.claudeResets!.upcoming).toBeUndefined();
  });

  it("does not show for accounts whose plan it leaves out", async () => {
    const api = await start(CLAUDE, (backend) =>
      backend.editEngineState((state) => {
        for (const [id, runtime] of Object.entries(state.providers)) {
          if (id.startsWith("claude@") && runtime.snapshot) runtime.snapshot.plan = "Free";
        }
      }),
    );
    await waitFor(() => expect(api.latest?.claudeResets?.brand).toBe("claude"));
    expect(api.latest!.claudeResets!.upcoming).toBeUndefined();
  });
});

describe("the row each Codex and Claude account starts with", () => {
  const BANKED = "2102438800836489554";
  const DEADLINE = "2026-10-22T23:59:59.000Z";

  /** Inside the Claude catalog's banked reset (open until 22/10/2026), while the Codex status's untimed announcement of 26/09 is still on the card. */
  beforeEach(() => vi.useFakeTimers({ toFake: ["Date"], now: new Date("2026-09-29T13:00:00Z") }));
  afterEach(() => vi.useRealTimers());

  const rowOf = (document: GlanceDocument | undefined, id: string) => document?.providers.find((provider) => provider.id === id)?.resetRow;

  it("starts every Codex and Claude account with its card's row, on the island and in the widgets, whichever tracker they show", async () => {
    const api = await start({});
    await waitFor(() => expect(rowOf(api.latest, "claude@7c1e")).toBeDefined());
    const document = api.latest!;
    expect(rowOf(document, "codex@52d0")).toMatchObject({ tracker: "codex", title: "Reset free", author: "@thsottiaux", value: "chưa rõ giờ", opens: true });
    for (const id of ["claude@7c1e", "claude@a93f"]) {
      expect(rowOf(document, id)).toMatchObject({ tracker: "claude", title: "Lượt reset để dành", tone: "accent", author: "@ClaudeDevs", hideAt: DEADLINE, opens: true });
    }
    expect(document.widget.providers.map((provider) => provider.resetRow)).toEqual(document.providers.map((provider) => provider.resetRow));
    expect(Object.keys(document.avatars ?? {}).sort()).toEqual(["@claudedevs", "@thsottiaux"]);
    expect("claudeResets" in document).toBe(false);
    expect(api.feedsAsked).toContain("claudeResets");
  });

  it("keeps the rows but opens nothing while the Reset tab is off, as the popup's rows do", async () => {
    const api = await start({ showResetsTab: false });
    await waitFor(() => expect(rowOf(api.latest, "claude@7c1e")).toBeDefined());
    for (const id of ["codex@52d0", "claude@7c1e", "claude@a93f"]) {
      const row = rowOf(api.latest, id)!;
      expect("opens" in row, id).toBe(false);
      expect(row.details, id).not.toContain(insightsFor("vi").freeResetOpenTab);
    }
  });

  it("leaves the rows out, and the Claude feed unasked, while the Reset tab and the reset notifications are off", async () => {
    const api = await start({ showResetsTab: false, notifyCodexResets: false, notifyClaudeResets: false });
    await settle();
    expect(api.latest!.providers.some((provider) => "resetRow" in provider)).toBe(false);
    expect("avatars" in api.latest!).toBe(false);
    expect(api.feedsAsked).not.toContain("claudeResets");
  });

  it("takes the banked reset off the Claude accounts once it is marked as used", async () => {
    const api = await start({ usedBankedResets: [BANKED] });
    await waitFor(() => expect(rowOf(api.latest, "codex@52d0")).toBeDefined());
    await settle();
    expect(rowOf(api.latest, "claude@7c1e")).toBeUndefined();
    expect(rowOf(api.latest, "claude@a93f")).toBeUndefined();
  });

  it("lists a card's rows behind its show-more button only while the card is open in the popup", async () => {
    const api = await start({});
    const codex = () => api.latest?.providers.find((provider) => provider.id === "codex@52d0")?.metrics.map((metric) => metric.id);
    await waitFor(() => expect(codex()).toEqual(["codex@52d0.session", "codex@52d0.weekly", "codex@52d0.rateLimitResets"]));
    act(() => void updateLayout((layout) => setProviderOpen(layout, "codex@52d0", true), { undoable: false }));
    await waitFor(() => expect(codex()).toContain("codex@52d0.spark"));
    expect(api.latest!.widget.providers.find((provider) => provider.id === "codex@52d0")?.metrics.map((metric) => metric.id)).toEqual(codex());
  });
});
