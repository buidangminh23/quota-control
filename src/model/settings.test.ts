import {
  anyNotificationEnabled,
  DEFAULT_SETTINGS,
  enabledProvidersOf,
  mergeSettingsDocument,
  parseSettings,
  SETTINGS_REVISION,
  surfaceResetProvider,
  taskbarDisplayOf,
  taskbarDisplayPatch,
} from "./settings";

describe("parseSettings", () => {
  it("defaults to Vietnamese and every documented default", () => {
    expect(parseSettings(null)).toEqual(DEFAULT_SETTINGS);
    expect(DEFAULT_SETTINGS.language).toBe("vi");
    expect(DEFAULT_SETTINGS.showTaskbarStrip).toBe(true);
    expect(DEFAULT_SETTINGS.displayMode).toBe("remaining");
    expect(DEFAULT_SETTINGS.automaticUpdateChecks).toBe(true);
    expect(DEFAULT_SETTINGS.automaticUpdateInstalls).toBe(true);
  });

  it("reads the update schedule the core also follows", () => {
    expect(parseSettings({ automaticUpdateChecks: false }).automaticUpdateChecks).toBe(false);
    expect(parseSettings({ automaticUpdateChecks: "off" }).automaticUpdateChecks).toBe(true);
    expect(parseSettings({ automaticUpdateInstalls: false }).automaticUpdateInstalls).toBe(false);
    expect(parseSettings({ automaticUpdateChecks: false }).automaticUpdateInstalls).toBe(true);
  });

  it("keeps valid keys and falls back per key for invalid ones", () => {
    const parsed = parseSettings({ language: "en", theme: "sepia", density: "compact", notifications: { almostOut: true, willRunOut: "yes" } });
    expect(parsed.language).toBe("en");
    expect(parsed.theme).toBe("system");
    expect(parsed.density).toBe("compact");
    expect(parsed.notifications).toEqual({ almostOut: true, cuttingItClose: false, willRunOut: false });
  });

  it("reads the Reset tab's provider and the banked resets marked as applied, dropping what is not an id", () => {
    expect(DEFAULT_SETTINGS).toMatchObject({ resetsProvider: "codex", usedBankedResets: [], notifyClaudeResets: true });
    expect(parseSettings({ resetsProvider: "claude" }).resetsProvider).toBe("claude");
    expect(parseSettings({ resetsProvider: "gemini" }).resetsProvider).toBe("codex");
    expect(parseSettings({ resetsProvider: ["claude"] }).resetsProvider).toBe("codex");
    expect(parseSettings({ notifyClaudeResets: false }).notifyClaudeResets).toBe(false);
    expect(parseSettings({ notifyClaudeResets: "no" }).notifyClaudeResets).toBe(true);
    const ids = parseSettings({ usedBankedResets: ["2102438800836489554", "observed-2097_a", "", "has space", "<script>", 77, null, "x".repeat(65)] }).usedBankedResets;
    expect(ids).toEqual(["2102438800836489554", "observed-2097_a"]);
    expect(parseSettings({ usedBankedResets: "2102438800836489554" }).usedBankedResets).toEqual([]);
    expect(parseSettings({ usedBankedResets: Array.from({ length: 80 }, (_, index) => `id-${index}`) }).usedBankedResets.length).toBeLessThanOrEqual(50);
  });
});

describe("glance surfaces", () => {
  it("default to the Hạn mức cards with every part shown", () => {
    expect(DEFAULT_SETTINGS.strip).toEqual({ content: "dashboard", metrics: [], values: 2 });
    expect(DEFAULT_SETTINGS.widget.content).toBe("dashboard");
    expect(DEFAULT_SETTINGS.island).toMatchObject({ content: "dashboard", style: "percent", wings: ["", ""], expandOnHover: true, alerts: true });
  });

  it("keeps valid choices, drops invalid ones and de-duplicates picked metrics", () => {
    const parsed = parseSettings({
      strip: { content: "custom", metrics: ["a", "a", 3, "b"], values: 1 },
      island: { content: "sideways", style: "ring", wings: ["x", 7], expandOnHover: false, showPlan: true },
      widget: { content: "starred", showAccount: false, metrics: "a" },
    });
    expect(parsed.strip).toEqual({ content: "custom", metrics: ["a", "b"], values: 1 });
    expect(parsed.island).toMatchObject({ content: "dashboard", style: "ring", wings: ["x", ""], expandOnHover: false, showPlan: true });
    expect(parsed.widget).toMatchObject({ content: "starred", showAccount: false, metrics: [] });
    expect(parseSettings({ strip: { values: 3 } }).strip.values).toBe(2);
  });

  it("saves copies, so later edits to the in-memory settings do not leak into the document", () => {
    const settings = parseSettings({ widget: { content: "custom", metrics: ["a"] } });
    const merged = mergeSettingsDocument({}, settings, new Set()) as {
      widget: { metrics: string[]; tabs: string[] };
      island: { wings: string[]; tabs: string[]; resetParts: { rhythm: boolean } };
    };
    settings.widget.metrics.push("b");
    settings.widget.tabs.pop();
    settings.island.wings[0] = "x";
    settings.island.tabs.pop();
    settings.island.resetParts.rhythm = false;
    expect(merged.widget.metrics).toEqual(["a"]);
    expect(merged.widget.tabs).toEqual(["quota", "resets", "upcoming"]);
    expect(merged.island.wings[0]).toBe("");
    expect(merged.island.tabs).toEqual(["quota", "resets", "upcoming"]);
    expect(merged.island.resetParts.rhythm).toBe(true);
  });

  it("gives the island every tab behind a tab bar by default and reads the chosen tabs in the popup's order", () => {
    expect(DEFAULT_SETTINGS.island.layout).toBe("separate");
    expect(parseSettings({ island: { layout: "combined" } }).island.layout).toBe("combined");
    expect(parseSettings({ island: { layout: "both" } }).island.layout).toBe("separate");
    expect(parseSettings({ island: {} }).island.tabs).toEqual(["quota", "resets", "upcoming"]);
    expect(parseSettings({ island: { tabs: ["upcoming", "quota", "upcoming", "x"] } }).island.tabs).toEqual(["upcoming", "quota"]);
    expect(parseSettings({ island: { tabs: [] } }).island.tabs).toEqual(["quota", "resets", "upcoming"]);
  });

  it("turns an island stored before tabs into tabs: combined keeps its sections, separate opens on its view first", () => {
    expect(parseSettings({ island: { layout: "combined", sections: { quota: true, resets: false, upcoming: true } } }).island.tabs).toEqual(["quota", "upcoming"]);
    expect(parseSettings({ island: { layout: "combined", sections: { quota: false, resets: false, upcoming: false } } }).island.tabs).toEqual(["quota"]);
    expect(parseSettings({ island: { layout: "separate", view: "resets", sections: { quota: true, resets: true, upcoming: false } } }).island.tabs).toEqual(["resets", "quota"]);
    expect(parseSettings({ island: { view: "upcoming" } }).island.tabs).toEqual(["upcoming"]);
  });

  it("reads each reset part and the coming-back limit on their own, never leaving no part on", () => {
    expect(parseSettings({ widget: { resetParts: { calendar: false, rhythm: "no" } } }).widget.resetParts).toEqual({
      next: true,
      latest: true,
      chances: true,
      wait: true,
      calendar: false,
      rhythm: true,
    });
    const none = { next: false, latest: false, chances: false, wait: false, calendar: false, rhythm: false };
    expect(parseSettings({ island: { resetParts: none } }).island.resetParts).toEqual(DEFAULT_SETTINGS.island.resetParts);
    expect(parseSettings({ island: { upcomingLimit: 8 } }).island.upcomingLimit).toBe(8);
    expect(parseSettings({ island: { upcomingLimit: 7 } }).island.upcomingLimit).toBe(5);
    expect(parseSettings({ widget: {} }).widget.upcomingLimit).toBe(0);
  });

  it("shows the plan on the island by default, as on the widgets and the popup's cards", () => {
    expect(DEFAULT_SETTINGS.island.showPlan).toBe(true);
    expect(DEFAULT_SETTINGS.widget.showPlan).toBe(true);
    expect(parseSettings({ island: {} }).island.showPlan).toBe(true);
  });

  it("turns the plan on once for an island saved while it was hidden by default, then keeps the choice", () => {
    const before = { language: "vi", island: { content: "starred", showPlan: false, showAccount: false }, widget: { showPlan: false } };
    const migrated = parseSettings(before);
    expect(migrated.island).toMatchObject({ content: "starred", showPlan: true, showAccount: false });
    expect(migrated.widget.showPlan).toBe(false);
    const saved = mergeSettingsDocument(before, migrated, new Set());
    expect(saved.settingsRevision).toBe(SETTINGS_REVISION);
    expect((saved.island as { showPlan: boolean }).showPlan).toBe(true);
    const hidden = mergeSettingsDocument(saved, { ...migrated, island: { ...migrated.island, showPlan: false } }, new Set());
    expect(parseSettings(hidden).island.showPlan).toBe(false);
    expect(parseSettings({ ...hidden, settingsRevision: 7 }).island.showPlan).toBe(false);
    expect(mergeSettingsDocument({ settingsRevision: 7 }, DEFAULT_SETTINGS, new Set()).settingsRevision).toBe(7);
    expect(parseSettings({ settingsRevision: "1", island: { showPlan: false } }).island.showPlan).toBe(true);
  });

  it("keeps any wing id up to 512 characters, special ones included", () => {
    expect(parseSettings({ island: { wings: ["codex-resets:chance-7", "quota:next"] } }).island.wings).toEqual(["codex-resets:chance-7", "quota:next"]);
    expect(parseSettings({ island: { wings: ["x".repeat(513), "codex-resets:since"] } }).island.wings).toEqual(["", "codex-resets:since"]);
    expect(parseSettings({ island: { wings: ["claude-resets:next", "claude-resets:chance-3"] } }).island.wings).toEqual(["claude-resets:next", "claude-resets:chance-3"]);
  });

  it("reads each surface's reset tracker on its own: the Reset tab's unless it picks Codex or Claude", () => {
    expect(DEFAULT_SETTINGS.island.resetsProvider).toBe("app");
    expect(DEFAULT_SETTINGS.widget.resetsProvider).toBe("app");
    const parsed = parseSettings({ island: { resetsProvider: "claude" }, widget: { resetsProvider: "gemini" } });
    expect(parsed.island.resetsProvider).toBe("claude");
    expect(parsed.widget.resetsProvider).toBe("app");
    expect(parseSettings({ widget: { resetsProvider: "claude" } }).widget.resetsProvider).toBe("claude");
    expect(parseSettings({ widget: { resetsProvider: "codex" } }).widget.resetsProvider).toBe("codex");
    expect(parseSettings({ widget: { resetsProvider: "claude" } }).island.resetsProvider).toBe("app");
    for (const value of ["Claude", "", ["claude"], 1, null, { claude: true }]) {
      expect(parseSettings({ island: { resetsProvider: value }, widget: { resetsProvider: value } }).island.resetsProvider).toBe("app");
      expect(parseSettings({ island: { resetsProvider: value }, widget: { resetsProvider: value } }).widget.resetsProvider).toBe("app");
    }
    expect(parseSettings({ resetsProvider: "claude" }).island.resetsProvider).toBe("app");
    const tab = (resetsProvider: "codex" | "claude", showResetsTab = true) => ({ resetsProvider, showResetsTab });
    expect(surfaceResetProvider("app", tab("claude"))).toBe("claude");
    expect(surfaceResetProvider("app", tab("codex"))).toBe("codex");
    expect(surfaceResetProvider("app", tab("claude", false))).toBe("codex");
    expect(surfaceResetProvider("codex", tab("claude"))).toBe("codex");
    expect(surfaceResetProvider("claude", tab("codex"))).toBe("claude");
    expect(surfaceResetProvider("claude", tab("codex", false))).toBe("claude");
  });
});

describe("mergeSettingsDocument", () => {
  it("preserves keys the popup does not own and narrows the core's provider list", () => {
    const stored = { enabledProviders: ["claude@1", "gone@2"], futureKey: 7, language: "vi" };
    const merged = mergeSettingsDocument(stored, { ...DEFAULT_SETTINGS, language: "en" }, new Set(["claude@1"]));
    expect(merged.enabledProviders).toEqual(["claude@1"]);
    expect(merged.futureKey).toBe(7);
    expect(merged.language).toBe("en");
  });

  it("does not invent an enabledProviders key", () => {
    expect("enabledProviders" in mergeSettingsDocument({}, DEFAULT_SETTINGS, new Set())).toBe(false);
  });

  it("saves the surfaces of someone who never picked Claude exactly as before, with no reset tracker key", () => {
    const stored = { island: { content: "starred", tabs: ["quota", "resets"], wings: ["codex-resets:next", ""] }, widget: { content: "custom", metrics: ["a"] } };
    const merged = mergeSettingsDocument(stored, parseSettings(stored), new Set()) as { island: Record<string, unknown>; widget: Record<string, unknown> };
    expect(Object.keys(merged.island)).toEqual(["content", "metrics", "showAccount", "showPlan", "showResets", "showProblems", "tabs", "resetParts", "upcomingLimit", "style", "wings", "expandOnHover", "alerts", "layout"]);
    expect(Object.keys(merged.widget)).toEqual(["content", "metrics", "showAccount", "showPlan", "showResets", "showProblems", "tabs", "resetParts", "upcomingLimit"]);
    const fresh = mergeSettingsDocument({}, DEFAULT_SETTINGS, new Set()) as { island: object; widget: object };
    expect("resetsProvider" in fresh.island).toBe(false);
    expect("resetsProvider" in fresh.widget).toBe(false);
  });

  it("saves a surface's own tracker on that surface alone, and keeps the key once a choice was saved", () => {
    const settings = parseSettings({});
    const picked = { ...settings, island: { ...settings.island, resetsProvider: "claude" as const } };
    const merged = mergeSettingsDocument({}, picked, new Set()) as { island: Record<string, unknown>; widget: Record<string, unknown> };
    expect(merged.island.resetsProvider).toBe("claude");
    expect("resetsProvider" in merged.widget).toBe(false);
    const reread = parseSettings(merged);
    expect(reread.island.resetsProvider).toBe("claude");
    expect(reread.widget.resetsProvider).toBe("app");
    const again = mergeSettingsDocument(merged, reread, new Set()) as { island: Record<string, unknown>; widget: Record<string, unknown> };
    expect(again.island.resetsProvider).toBe("claude");
    expect("resetsProvider" in again.widget).toBe(false);
    const back = mergeSettingsDocument(merged, { ...reread, island: { ...reread.island, resetsProvider: "codex" } }, new Set()) as { island: Record<string, unknown>; widget: Record<string, unknown> };
    expect(back.island.resetsProvider).toBe("codex");
    expect("resetsProvider" in back.widget).toBe(false);
    expect(parseSettings(back).island.resetsProvider).toBe("codex");
    const follows = mergeSettingsDocument(back, { ...parseSettings(back), island: { ...parseSettings(back).island, resetsProvider: "app" } }, new Set()) as { island: Record<string, unknown> };
    expect(follows.island.resetsProvider).toBe("app");
    expect(parseSettings(follows).island.resetsProvider).toBe("app");
  });
});

describe("helpers", () => {
  it("reads the enabled provider list or null for the default (all)", () => {
    expect(enabledProvidersOf({ enabledProviders: ["a", 3, "b"] })).toEqual(["a", "b"]);
    expect(enabledProvidersOf({})).toBeNull();
  });

  it("detects whether any pace notification is on", () => {
    expect(anyNotificationEnabled(DEFAULT_SETTINGS)).toBe(false);
    expect(anyNotificationEnabled({ notifications: { almostOut: false, cuttingItClose: true, willRunOut: false } })).toBe(true);
  });
});

describe("taskbar display", () => {
  it("reads the picker value from the stored strip switch and style", () => {
    expect(taskbarDisplayOf({ showTaskbarStrip: true, iconStyle: "text" }, true)).toBe("text");
    expect(taskbarDisplayOf({ showTaskbarStrip: true, iconStyle: "bars" }, true)).toBe("bars");
    expect(taskbarDisplayOf({ showTaskbarStrip: false, iconStyle: "text" }, true)).toBe("icon");
    expect(taskbarDisplayOf({ showTaskbarStrip: true, iconStyle: "text" }, false)).toBe("bars");
  });

  it("stores a choice without losing the style behind App Icon Only", () => {
    expect(taskbarDisplayPatch("icon")).toEqual({ showTaskbarStrip: false });
    expect(taskbarDisplayPatch("text")).toEqual({ showTaskbarStrip: true, iconStyle: "text" });
    expect(taskbarDisplayPatch("bars")).toEqual({ showTaskbarStrip: true, iconStyle: "bars" });
  });
});

describe("token and price views", () => {
  it("drops the retired Yesterday period and reads the new view keys", () => {
    expect(parseSettings({ totalSpendPeriod: "yesterday" }).totalSpendPeriod).toBe("today");
    expect(parseSettings({ totalSpendPeriod: "last365" }).totalSpendPeriod).toBe("last365");
    const parsed = parseSettings({ dashboardTab: "prices", tokenView: "history", tokenChart: "project", priceProvider: "openai", priceTier: "flex", priceCurrency: "usd" });
    expect(parsed).toMatchObject({ dashboardTab: "prices", tokenView: "history", tokenChart: "project", priceProvider: "openai", priceTier: "flex", priceCurrency: "usd" });
    expect(parseSettings({ tokenView: "tables", tokenRingBy: "account" })).toMatchObject({ tokenView: "overview", tokenRingBy: "model" });
  });
});
