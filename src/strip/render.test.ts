import { stripMarkColor } from "./render";

describe("strip mark colors", () => {
  it("tints each mark in its brand color for the taskbar's theme", () => {
    expect(stripMarkColor("claude", "#ffffff", "taskbar")).toBe("#DE7356");
    expect(stripMarkColor("codex", "#000000", "taskbar")).toBe("#10A37F");
    expect(stripMarkColor("cursor", "#ffffff", "taskbar")).toBe("#F5F5F7");
    expect(stripMarkColor("cursor", "#000000", "taskbar")).toBe("#13120A");
  });

  it("tints the marks on the Linux panel like the Windows taskbar", () => {
    expect(stripMarkColor("claude", "#ffffff", "panel")).toBe("#DE7356");
    expect(stripMarkColor("codex", "#ffffff", "panel")).toBe(stripMarkColor("codex", "#ffffff", "taskbar"));
  });

  it("keeps an unknown brand and the macOS template in the text color", () => {
    expect(stripMarkColor("no-such-brand", "#ffffff", "taskbar")).toBe("#ffffff");
    expect(stripMarkColor("claude", "#000000", "menuBar")).toBe("#000000");
  });
});
