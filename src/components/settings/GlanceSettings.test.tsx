import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { messagesFor } from "@/i18n";
import { setBackend } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import type { Platform } from "@/lib/types";
import { updateSettings, useApp } from "@/state/store";
import { App } from "../../App";
import { closeDialog } from "../ui/dialog";
import { closeMenu } from "../ui/menu";

vi.mock("@/strip/useTaskbarStrip", () => ({ useTaskbarStrip: () => {} }));

const CODEX_OFF = "Chưa có số liệu: bật tab Reset hoặc thông báo khi Codex reset.";
const CLAUDE_OFF = "Chưa có số liệu: bật tab Reset hoặc thông báo khi Claude reset.";
const RESETS_SCOPE = "Áp dụng cho widget Reset, Lịch reset và Tổng quan.";
const CLAUDE_WINGS = [
  "Reset Claude · Hạn dùng lượt để dành",
  "Reset Claude · Khả năng 24 giờ tới",
  "Reset Claude · Khả năng 3 ngày tới",
  "Reset Claude · Khả năng 7 ngày tới",
  "Reset Claude · Thời gian chưa reset",
];
const FOLLOWING_WINGS = [
  "Như tab Reset · Reset free sắp tới hoặc hạn dùng lượt để dành",
  "Như tab Reset · Khả năng 24 giờ tới",
  "Như tab Reset · Khả năng 3 ngày tới",
  "Như tab Reset · Khả năng 7 ngày tới",
  "Như tab Reset · Thời gian chưa reset",
];

async function openSettings(platform: Platform = "macos") {
  const api = new MockBackend();
  setBackend(api);
  render(<App />);
  await screen.findByText("Claude · Công ty");
  act(() => useApp.setState({ screen: "settings", info: { name: "Quota Control", version: "0.3.16", platform } }));
  await screen.findByRole("heading", { name: "Cài đặt" });
  return api;
}

/** A Settings card by its title. */
function section(title: string): HTMLElement {
  const element = screen.getByRole("heading", { name: title }).closest("section");
  if (!element) throw new Error(`No section titled ${title}`);
  return element;
}

afterEach(async () => {
  closeMenu();
  closeDialog();
  cleanup();
  await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
  act(() => useApp.setState({ screen: "dashboard", previousScreen: "dashboard", customizeProviderId: null, notice: null }));
});

describe("whose reset tracker the island and the widgets show", () => {
  it("follows the Reset tab, in Codex's wording, for someone who never picks a tracker", async () => {
    await openSettings();
    const island = section("Dynamic Island");
    const widget = section("Widget màn hình");
    for (const surface of [island, widget]) {
      expect(within(surface).getByRole("checkbox", { name: "Reset Codex" })).toBeChecked();
      fireEvent.click(within(surface).getByRole("button", { name: /^Reset Codex/ }));
      expect(within(surface).getByText("Reset của")).toBeInTheDocument();
      expect(within(surface).getByRole("radio", { name: "Như tab Reset" })).toBeChecked();
      expect(within(surface).getByRole("radio", { name: "Codex" })).not.toBeChecked();
      expect(within(surface).getByRole("radio", { name: "Claude" })).not.toBeChecked();
      expect(within(surface).queryByText(/Reset Claude|của Claude/)).not.toBeInTheDocument();
      expect(within(surface).queryByText(CODEX_OFF)).not.toBeInTheDocument();
    }
    expect(within(widget).getByText(RESETS_SCOPE)).toBeInTheDocument();
    expect(useApp.getState().settings.island.resetsProvider).toBe("app");
    expect(useApp.getState().settings.widget.resetsProvider).toBe("app");
  });

  it("turns the island and the widgets to Claude when the Reset tab turns to Claude, until a surface picks its own", async () => {
    await openSettings();
    const island = section("Dynamic Island");
    const widget = section("Widget màn hình");
    act(() => updateSettings({ resetsProvider: "claude" }));
    for (const surface of [island, widget]) {
      expect(within(surface).getByRole("checkbox", { name: "Reset Claude" })).toBeChecked();
      fireEvent.click(within(surface).getByRole("button", { name: /^Reset Claude/ }));
      expect(within(surface).getByRole("radio", { name: "Như tab Reset" })).toBeChecked();
    }
    expect(within(widget).getByText(RESETS_SCOPE)).toBeInTheDocument();

    fireEvent.click(within(island).getByRole("radio", { name: "Codex" }));
    expect(useApp.getState().settings.island.resetsProvider).toBe("codex");
    expect(within(island).getByRole("checkbox", { name: "Reset Codex" })).toBeChecked();
    expect(within(widget).getByRole("checkbox", { name: "Reset Claude" })).toBeChecked();
    act(() => updateSettings({ resetsProvider: "codex" }));
    expect(within(widget).getByRole("checkbox", { name: "Reset Codex" })).toBeChecked();

    fireEvent.click(within(island).getByRole("radio", { name: "Như tab Reset" }));
    expect(useApp.getState().settings.island.resetsProvider).toBe("app");
  });

  it("names the followed tracker Codex while the Reset tab is hidden, and Claude again once it is shown", async () => {
    await openSettings();
    act(() => updateSettings({ resetsProvider: "claude", showResetsTab: false }));
    const island = section("Dynamic Island");
    expect(within(island).getByRole("checkbox", { name: "Reset Codex" })).toBeChecked();
    act(() => updateSettings({ showResetsTab: true }));
    expect(within(island).getByRole("checkbox", { name: "Reset Claude" })).toBeChecked();
  });

  it("lets the island show Claude's resets and renames its reset view, leaving the widgets on Codex", async () => {
    const api = await openSettings();
    const island = section("Dynamic Island");
    const widget = section("Widget màn hình");
    fireEvent.click(within(island).getByRole("button", { name: /^Reset Codex/ }));
    fireEvent.click(within(island).getByRole("radio", { name: "Claude" }));

    expect(useApp.getState().settings.island.resetsProvider).toBe("claude");
    expect(useApp.getState().settings.widget.resetsProvider).toBe("app");
    expect(within(island).getByRole("radio", { name: "Claude" })).toBeChecked();
    expect(within(island).getByRole("radio", { name: "Codex" })).not.toBeChecked();
    expect(within(island).getByRole("checkbox", { name: "Reset Claude" })).toBeChecked();
    expect(within(island).getByRole("button", { name: /^Reset Claude/ })).toHaveAttribute("aria-expanded", "true");
    expect(within(island).queryByText("Reset Codex")).not.toBeInTheDocument();
    expect(within(widget).getByRole("checkbox", { name: "Reset Codex" })).toBeChecked();
    expect(within(widget).getByRole("button", { name: /^Reset Codex/ })).toBeInTheDocument();
    expect(within(widget).queryByText("Reset Claude")).not.toBeInTheDocument();

    await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
    const saved = await api.loadDocument<{ island: { resetsProvider?: string }; widget: { resetsProvider?: string } }>("settings");
    expect(saved?.island.resetsProvider).toBe("claude");
    expect(saved?.widget.resetsProvider ?? "app").toBe("app");

    fireEvent.click(within(island).getByRole("radio", { name: "Codex" }));
    expect(useApp.getState().settings.island.resetsProvider).toBe("codex");
    expect(within(island).getByRole("checkbox", { name: "Reset Codex" })).toBeChecked();
    expect(within(island).getByRole("button", { name: /^Reset Codex/ })).toBeInTheDocument();
  });

  it("lets the widgets show Claude's resets, naming the reset widgets for either tracker", async () => {
    await openSettings();
    const island = section("Dynamic Island");
    const widget = section("Widget màn hình");
    fireEvent.click(within(widget).getByRole("button", { name: /^Reset Codex/ }));
    fireEvent.click(within(widget).getByRole("radio", { name: "Claude" }));

    expect(useApp.getState().settings.widget.resetsProvider).toBe("claude");
    expect(useApp.getState().settings.island.resetsProvider).toBe("app");
    expect(within(widget).getByRole("checkbox", { name: "Reset Claude" })).toBeChecked();
    expect(within(widget).getByRole("button", { name: /^Reset Claude/ })).toHaveAttribute("aria-expanded", "true");
    expect(within(widget).getByText(RESETS_SCOPE)).toBeInTheDocument();
    expect(within(widget).queryByText(/Reset Codex|Lịch reset Codex|cả ba/)).not.toBeInTheDocument();
    expect(within(island).getByRole("checkbox", { name: "Reset Codex" })).toBeChecked();
    expect(within(island).queryByText("Reset Claude")).not.toBeInTheDocument();
  });

  it("says a tracker has no data only while the Reset tab and that tracker's notifications are both off", async () => {
    await openSettings();
    const island = section("Dynamic Island");
    fireEvent.click(within(island).getByRole("button", { name: /^Reset Codex/ }));

    act(() => updateSettings({ showResetsTab: false, notifyCodexResets: true, notifyClaudeResets: false }));
    expect(within(island).queryByText(CODEX_OFF)).not.toBeInTheDocument();
    fireEvent.click(within(island).getByRole("radio", { name: "Claude" }));
    expect(within(island).getByText(CLAUDE_OFF)).toBeInTheDocument();

    act(() => updateSettings({ notifyCodexResets: false, notifyClaudeResets: true }));
    expect(within(island).queryByText(CLAUDE_OFF)).not.toBeInTheDocument();
    fireEvent.click(within(island).getByRole("radio", { name: "Codex" }));
    expect(within(island).getByText(CODEX_OFF)).toBeInTheDocument();

    act(() => updateSettings({ showResetsTab: true }));
    expect(within(island).queryByText(CODEX_OFF)).not.toBeInTheDocument();
  });

  it("offers the Claude reset readings on both sides of the notch", async () => {
    await openSettings();
    const island = section("Dynamic Island");

    fireEvent.click(within(island).getByRole("button", { name: "Bên trái tai thỏ: Tự động" }));
    expect(screen.getByRole("menuitemcheckbox", { name: "Reset Codex · Reset free sắp tới" })).toBeInTheDocument();
    for (const name of CLAUDE_WINGS) expect(screen.getByRole("menuitemcheckbox", { name })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Reset Claude · Hạn dùng lượt để dành" }));
    expect(useApp.getState().settings.island.wings).toEqual(["claude-resets:next", ""]);
    expect(within(island).getByRole("button", { name: "Bên trái tai thỏ: Reset Claude · Hạn dùng lượt để dành" })).toBeInTheDocument();

    fireEvent.click(within(island).getByRole("button", { name: "Bên phải tai thỏ: Tự động" }));
    for (const name of CLAUDE_WINGS) expect(screen.getByRole("menuitemcheckbox", { name })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Reset Claude · Thời gian chưa reset" }));
    expect(useApp.getState().settings.island.wings).toEqual(["claude-resets:next", "claude-resets:since"]);
    expect(within(island).getByRole("button", { name: "Bên phải tai thỏ: Reset Claude · Thời gian chưa reset" })).toBeInTheDocument();
  });

  it("offers reset readings beside the notch that follow the Reset tab's tracker", async () => {
    await openSettings();
    const island = section("Dynamic Island");

    fireEvent.click(within(island).getByRole("button", { name: "Bên phải tai thỏ: Tự động" }));
    for (const name of FOLLOWING_WINGS) expect(screen.getByRole("menuitemcheckbox", { name })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Như tab Reset · Khả năng 24 giờ tới" }));
    expect(useApp.getState().settings.island.wings).toEqual(["", "resets:chance-1"]);
    expect(within(island).getByRole("button", { name: "Bên phải tai thỏ: Như tab Reset · Khả năng 24 giờ tới" })).toBeInTheDocument();
  });

  it("words the choice in English as well", async () => {
    await openSettings();
    act(() => updateSettings({ language: "en" }));
    const island = section("Dynamic Island");
    const widget = section("Desktop Widget");

    expect(within(island).getByRole("checkbox", { name: "Codex Resets" })).toBeChecked();
    fireEvent.click(within(island).getByRole("button", { name: /^Codex Resets/ }));
    expect(within(island).getByText("Resets of")).toBeInTheDocument();
    expect(within(island).getByRole("radio", { name: "As in the Reset tab" })).toBeChecked();
    fireEvent.click(within(island).getByRole("radio", { name: "Claude" }));
    expect(within(island).getByRole("checkbox", { name: "Claude Resets" })).toBeChecked();
    expect(within(island).getByRole("button", { name: /^Claude Resets/ })).toBeInTheDocument();
    act(() => updateSettings({ showResetsTab: false, notifyClaudeResets: false }));
    expect(within(island).getByText("No data yet: turn on the Resets tab or Claude reset notifications.")).toBeInTheDocument();

    fireEvent.click(within(widget).getByRole("button", { name: /^Codex Resets/ }));
    expect(within(widget).getByText("Applies to the Resets, Reset Calendar and Overview widgets.")).toBeInTheDocument();
    fireEvent.click(within(widget).getByRole("radio", { name: "Claude" }));
    expect(within(widget).getByText("Applies to the Resets, Reset Calendar and Overview widgets.")).toBeInTheDocument();

    fireEvent.click(within(island).getByRole("button", { name: "Left of the Notch: Automatic" }));
    expect(screen.getByRole("menuitemcheckbox", { name: "Codex Resets · Next Free Reset" })).toBeInTheDocument();
    expect(screen.getByRole("menuitemcheckbox", { name: "Claude Resets · Banked Reset Deadline" })).toBeInTheDocument();
    expect(screen.getByRole("menuitemcheckbox", { name: "Claude Resets · Time Since the Last Reset" })).toBeInTheDocument();
    expect(screen.getByRole("menuitemcheckbox", { name: "As in the Reset Tab · Chance in 24 Hours" })).toBeInTheDocument();
    expect(screen.getByRole("menuitemcheckbox", { name: "As in the Reset Tab · Next Free Reset or Banked Reset Deadline" })).toBeInTheDocument();
  });

  it.each<Platform>(["windows", "linux"])("leaves the %s Settings without the island and the widgets", async (platform) => {
    await openSettings(platform);
    expect(screen.queryByRole("heading", { name: "Dynamic Island" })).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Widget màn hình" })).not.toBeInTheDocument();
    expect(screen.queryByText("Reset của")).not.toBeInTheDocument();
  });
});

describe("the widgets' names in the widget gallery", () => {
  const swift = readFileSync(resolve(process.cwd(), "src-tauri/macos/Widget/QuotaWidget.swift"), "utf8");
  const name = (style: string, language: "vi" | "en") => {
    const match = new RegExp(`case \\.${style}: return vietnamese \\? "([^"]+)" : "([^"]+)"`).exec(swift);
    if (!match) throw new Error(`No gallery name for ${style}`);
    return language === "vi" ? match[1]! : match[2]!;
  };

  it("name the reset widgets for either tracker, as the Settings notes do", () => {
    for (const language of ["vi", "en"] as const) {
      const settings = messagesFor(language).settings;
      const resets = name("codexResets", language);
      const calendar = name("resetCalendar", language);
      expect(`${resets} ${calendar}`, language).not.toMatch(/Codex|Claude/);
      for (const widget of [resets, calendar, name("upcoming", language)]) expect(settings.desktopWidgetKindsNote, language).toContain(widget);
      for (const widget of [resets, calendar, name("overview", language)]) expect(settings.glanceWidgetScope("resets"), language).toContain(widget);
      for (const widget of ["details", "rings", "compact", "overview"]) expect(settings.glanceWidgetScope("quota"), language).toContain(name(widget, language));
    }
  });
});
