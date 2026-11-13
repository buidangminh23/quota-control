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

describe("strip sizes per system", () => {
  it("keeps the Windows taskbar and Linux panel styles as they were (Rules.md §0.56)", () => {
    expect(STRIP_METRICS.taskbar).toEqual({
      font: '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif',
      singleSize: 14,
      stackedSize: 12,
      stackedLineHeight: 14,
      markSide: 18,
      markGap: 5,
      groupGap: 14,
      sidePadding: 6,
      labelSize: 10,
      labelGap: 3,
    });
    expect(STRIP_METRICS.panel).toEqual({
      font: 'Ubuntu, Cantarell, "Noto Sans", system-ui, sans-serif',
      singleSize: 12,
      stackedSize: 11,
      stackedLineHeight: 12,
      markSide: 18,
      markGap: 4,
      groupGap: 10,
      sidePadding: 1,
      labelSize: 9,
      labelGap: 2,
    });
  });

  it("places the macOS menu bar readings on measured baselines with a stable value column", () => {
    expect(STRIP_METRICS.menuBar.baselines?.minValue).toBe("00%");
    expect(STRIP_METRICS.menuBar.markSide).toBe(16);
    expect(STRIP_METRICS.taskbar.baselines).toBeUndefined();
    expect(STRIP_METRICS.panel.baselines).toBeUndefined();
  });
});
