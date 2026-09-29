import { PROVIDER_MARKS } from "@/assets/providerMarks";
import type { StripContent, StripMetric } from "@/model/menuBar";
import { groupTextWidth, stackBaselines } from "./layout";
import { nativeStrip } from "./native";
import { MARK_INSET, SINGLE_WEIGHT, STACKED_WEIGHT, STRIP_METRICS, stripMarkColor } from "./render";

function metric(id: string, period: string | null, value: string): StripMetric {
  return { id, label: id, period, value, fraction: 0.5, bounded: true };
}

const CONTENT: StripContent = {
  groups: [
    { providerId: "claude", displayName: "Claude", brand: "claude", metrics: [metric("Phiên 5h", "5h", "12%"), metric("Tuần", "week", "58%"), metric("Fable", "week", "3%")] },
    { providerId: "codex", displayName: "Codex", brand: "codex", metrics: [metric("Tín dụng", null, "$4.20")] },
    { providerId: "mystery", displayName: "Mystery", brand: "no-such-brand", metrics: [metric("Tháng", "month", "7%")] },
  ],
  bars: [],
};

describe("the strip described for macOS", () => {
  const strip = nativeStrip(CONTENT, 36, 2)!;

  it("carries the menu bar style's own sizes, so both drawings are one design (Rules.md §0.58)", () => {
    const style = STRIP_METRICS.menuBar;
    expect(strip.metrics).toEqual({
      singleSize: style.singleSize,
      singleWeight: SINGLE_WEIGHT,
      stackedSize: style.stackedSize,
      stackedWeight: STACKED_WEIGHT,
      markSide: style.markSide,
      markGap: style.markGap,
      markInset: MARK_INSET,
      groupGap: style.groupGap,
      sidePadding: style.sidePadding,
      labelSize: style.labelSize,
      labelGap: style.labelGap,
      labelWeight: style.labelWeight,
      labelAlpha: style.labelAlpha,
      rowGap: style.baselines!.rowGap,
      edge: style.baselines!.edge,
      minValue: style.baselines!.minValue,
    });
    expect([strip.version, strip.height, strip.scale]).toEqual([1, 36, 2]);
  });

  it("shows the readings the picture shows: two a provider at most, each with its window's name", () => {
    expect(strip.groups.map((group) => group.rows)).toEqual([
      [
        { label: "5h", value: "12%" },
        { label: "week", value: "58%" },
      ],
      [{ label: null, value: "$4.20" }],
      [{ label: "month", value: "7%" }],
    ]);
  });

  it("tints each mark as the picture does on a light and on a dark bar", () => {
    for (const group of strip.groups) {
      expect(group.light ?? "#000000").toBe(stripMarkColor(group.brand, "#000000", "menuBar"));
      expect(group.dark ?? "#ffffff").toBe(stripMarkColor(group.brand, "#ffffff", "menuBar"));
    }
    expect(strip.groups[2]).toMatchObject({ light: null, dark: null });
  });

  it("sends each brand's mark, and its color logo when one was drawn", () => {
    expect(strip.groups[0]!.mark).toEqual(PROVIDER_MARKS.claude);
    expect(strip.groups[2]!.mark).toBeUndefined();
    const withArt = nativeStrip(CONTENT, 36, 2, { codex: "UE5H" })!;
    expect(withArt.groups[1]!.mark).toEqual({ ...PROVIDER_MARKS.codex, art: "UE5H" });
    expect(withArt.groups[0]!.mark).toEqual(PROVIDER_MARKS.claude);
  });

  it("says the readings in words on one line", () => {
    expect(strip.text).toBe("Claude: Phiên 5h 12%, Tuần 58%, Fable 3%; Codex: Tín dụng $4.20; Mystery: Tháng 7%");
  });

  it("describes nothing for an empty strip or a bar that places its rows another way", () => {
    expect(nativeStrip({ groups: [], bars: [] }, 36, 2)).toBeNull();
    expect(nativeStrip(CONTENT, 48, 1, {}, "taskbar")).toBeNull();
    expect(nativeStrip(CONTENT, 24, 1, {}, "panel")).toBeNull();
  });
});

/**
 * `MenuBarStrip.swift` lays the strip out with its own port of `layout.ts`. These are the glyph
 * measurements CoreText gave it for the two-provider strip at 2x, and where it then placed
 * everything: `layout.ts` must place the same from the same measurements, or the port is behind.
 */
describe("the Swift layout against layout.ts", () => {
  const font = { capAscent: 12.9990234375, ascent: 13.2978515625, descent: 4.0517578125 };
  const rows = [
    { labelWidth: 19.580162048339844, valueWidth: 36.63884286704706 },
    { labelWidth: 39.69925117492676, valueWidth: 40.18630351543834 },
  ];
  const style = STRIP_METRICS.menuBar;
  const scale = 2;

  it("stacks the two readings on the same baselines", () => {
    const baselines = stackBaselines(2, 36, font, { rowGap: style.baselines!.rowGap * scale, edge: style.baselines!.edge * scale });
    expect(baselines[0]).toBeCloseTo(15.650390625, 9);
    expect(baselines[1]).toBeCloseTo(31.9482421875, 9);
  });

  it("gives a provider's readings the same width", () => {
    const text = groupTextWidth(rows, style.labelGap * scale, 40.362919417675585);
    const left = (style.sidePadding + style.markSide + style.markGap) * scale;
    expect([left, left + Math.ceil(text)]).toEqual([44, 132]);
  });
});
