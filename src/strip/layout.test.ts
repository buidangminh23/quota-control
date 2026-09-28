import { groupTextWidth, stackBaselines, type RowFontMetrics } from "./layout";

const SF_9PT_AT_2X: RowFontMetrics = { capAscent: 13, ascent: 13.3, descent: 4.06 };

describe("stacked strip rows", () => {
  it("keeps both rows, descenders included, inside an 18pt menu bar image", () => {
    const [top, bottom] = stackBaselines(2, 36, SF_9PT_AT_2X, { rowGap: 3, edge: 0 });
    expect(top! - SF_9PT_AT_2X.ascent).toBeGreaterThanOrEqual(0);
    expect(bottom! + SF_9PT_AT_2X.descent).toBeLessThanOrEqual(36 + 1e-9);
    expect(bottom! - top!).toBeCloseTo(SF_9PT_AT_2X.ascent + 3);
  });

  it("centers the digits on the mark when there is room", () => {
    const font: RowFontMetrics = { capAscent: 10, ascent: 10, descent: 2 };
    const [top, bottom] = stackBaselines(2, 60, font, { rowGap: 4, edge: 0 });
    expect((top! - font.capAscent + bottom!) / 2).toBeCloseTo(30);
  });

  it("centers a single reading's digits on the mark", () => {
    const font: RowFontMetrics = { capAscent: 17, ascent: 17.5, descent: 5 };
    const [baseline] = stackBaselines(1, 36, font, { rowGap: 3, edge: 0 });
    expect(baseline! - font.capAscent / 2).toBeCloseTo(18);
  });

  it("moves the rows closer rather than cut a glyph when the band is short", () => {
    const [top, bottom] = stackBaselines(2, 24, SF_9PT_AT_2X, { rowGap: 3, edge: 1 });
    expect(top! - SF_9PT_AT_2X.ascent).toBeGreaterThanOrEqual(1 - 1e-9);
    expect(bottom! + SF_9PT_AT_2X.descent).toBeLessThanOrEqual(23 + 1e-9);
  });
});

describe("strip reading columns", () => {
  it("puts the same gap between the name and value columns on every row", () => {
    const width = groupTextWidth(
      [
        { labelWidth: 10, valueWidth: 20 },
        { labelWidth: 20, valueWidth: 14 },
      ],
      7,
      18,
    );
    expect(width).toBe(20 + 7 + 20);
  });

  it("keeps the value column at least as wide as the reserved reading", () => {
    const narrow = groupTextWidth([{ labelWidth: 10, valueWidth: 6 }], 7, 18);
    const wide = groupTextWidth([{ labelWidth: 10, valueWidth: 16 }], 7, 18);
    expect(narrow).toBe(wide);
  });

  it("lets a reading without a window name reach into the name column", () => {
    const rows = [
      { labelWidth: 20, valueWidth: 18 },
      { labelWidth: null, valueWidth: 30 },
    ];
    expect(groupTextWidth(rows, 7, 18)).toBe(45);
    expect(groupTextWidth([{ labelWidth: null, valueWidth: 60 }, { labelWidth: 20, valueWidth: 18 }], 7, 18)).toBe(60);
  });

  it("sizes a group without window names by its values alone", () => {
    expect(groupTextWidth([{ labelWidth: null, valueWidth: 12 }], 7, 18)).toBe(18);
  });
});
