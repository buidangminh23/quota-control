import { STRIP_METRICS, stripMarkColor } from "./render";

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

  it("keeps an unknown brand in the text color", () => {
    expect(stripMarkColor("no-such-brand", "#ffffff", "taskbar")).toBe("#ffffff");
  });

  it("gives the three systems the same mark colors", () => {
    for (const color of ["#000000", "#ffffff"] as const) {
      for (const brand of ["claude", "codex", "cursor"]) {
        const windows = stripMarkColor(brand, color, "taskbar");
        expect(stripMarkColor(brand, color, "menuBar")).toBe(windows);
        expect(stripMarkColor(brand, color, "panel")).toBe(windows);
      }
    }
  });
});

describe("strip parity across systems", () => {
  it("names the windows (5h, week) on every system's bar", () => {
    for (const metrics of Object.values(STRIP_METRICS)) expect(metrics.labelSize).not.toBeNull();
  });
});
