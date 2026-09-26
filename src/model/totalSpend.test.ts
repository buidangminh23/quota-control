import { ringSectorPath } from "./ringPath";
import { knownBrandColor } from "./totalSpend";

describe("brand colors", () => {
  it("gives a mark its brand color only when the palette knows the brand", () => {
    expect(knownBrandColor("codex", true)).toBe("#10A37F");
    expect(knownBrandColor("cursor", false)).toBe("#13120A");
    expect(knownBrandColor("somebody", false)).toBeNull();
  });
});

describe("ringSectorPath", () => {
  it("draws a closed rounded wedge and skips slivers too thin to draw", () => {
    const path = ringSectorPath(0, 0.4, { size: 104 });
    expect(path.startsWith("M ")).toBe(true);
    expect(path.endsWith("Z")).toBe(true);
    expect(path.match(/A /g)?.length).toBe(6);
    expect(ringSectorPath(0.2, 0.2002, { size: 104 })).toBe("");
  });
});
