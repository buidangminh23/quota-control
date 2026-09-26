import { anyNotificationEnabled, DEFAULT_SETTINGS, enabledProvidersOf, mergeSettingsDocument, parseSettings, taskbarDisplayOf, taskbarDisplayPatch } from "./settings";

describe("parseSettings", () => {
  it("defaults to Vietnamese and every documented default", () => {
    expect(parseSettings(null)).toEqual(DEFAULT_SETTINGS);
    expect(DEFAULT_SETTINGS.language).toBe("vi");
    expect(DEFAULT_SETTINGS.showTaskbarStrip).toBe(true);
    expect(DEFAULT_SETTINGS.displayMode).toBe("remaining");
    expect(DEFAULT_SETTINGS.automaticUpdateChecks).toBe(true);
  });

  it("reads the update schedule the core also follows", () => {
    expect(parseSettings({ automaticUpdateChecks: false }).automaticUpdateChecks).toBe(false);
    expect(parseSettings({ automaticUpdateChecks: "off" }).automaticUpdateChecks).toBe(true);
  });

  it("keeps valid keys and falls back per key for invalid ones", () => {
    const parsed = parseSettings({ language: "en", theme: "sepia", density: "compact", notifications: { almostOut: true, willRunOut: "yes" } });
    expect(parsed.language).toBe("en");
    expect(parsed.theme).toBe("system");
    expect(parsed.density).toBe("compact");
    expect(parsed.notifications).toEqual({ almostOut: true, cuttingItClose: false, willRunOut: false });
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
