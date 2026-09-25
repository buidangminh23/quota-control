import { fixtureCatalog, fixtureSnapshots } from "@/lib/fixtures";
import type { Provider } from "@/lib/types";
import { ringSectorPath } from "./ringPath";
import { brandColor, MINIMUM_SLICE_SHARE, projectTotalSpend, ringArcs, totalSpendSlices } from "./totalSpend";

const providers: Provider[] = fixtureCatalog()
  .filter((entry) => entry.descriptors.some((descriptor) => descriptor.isSpendTile))
  .map((entry) => entry.provider);
const snapshots = fixtureSnapshots(Date.UTC(2026, 8, 26, 3));
const compare = (a: string, b: string) => a.localeCompare(b);

describe("total spend", () => {
  it("sums each local card's period line into one slice", () => {
    const slices = totalSpendSlices("today", providers, snapshots);
    expect(slices.map((slice) => slice.provider.id)).toEqual(["claude-local", "codex-local"]);
    expect(slices[0]!.amountUSD).toBeCloseTo(49.85, 2);
    expect(slices[0]!.tokenCount).toBe(35_800_000);
    expect(slices[0]!.estimated).toBe(true);
  });

  it("ranks slices by the chosen metric and totals the center", () => {
    const slices = totalSpendSlices("today", providers, snapshots);
    const cost = projectTotalSpend(slices, "cost", compare);
    expect(cost.slices[0]!.provider.id).toBe("claude-local");
    expect(cost.center).toBeCloseTo(49.85 + 10.01, 2);
    const blended = projectTotalSpend(slices, "costPerMtok", compare);
    expect(blended.center).toBeCloseTo(((49.85 + 10.01) / (35_800_000 + 9_100_000)) * 1_000_000, 4);
    expect(projectTotalSpend(slices, "tokens", compare).estimated).toBe(false);
  });

  it("gives tiny slices a visible sliver and always closes the ring", () => {
    const projection = { metric: "cost" as const, center: 100.1, estimated: false, slices: [
      { provider: providers[0]!, amount: 100, estimated: false },
      { provider: providers[1]!, amount: 0.1, estimated: false },
    ] };
    const arcs = ringArcs(projection);
    expect(arcs.at(-1)!.end).toBeCloseTo(1, 10);
    expect(arcs[1]!.end - arcs[1]!.start).toBeGreaterThanOrEqual(MINIMUM_SLICE_SHARE / (1 + MINIMUM_SLICE_SHARE) - 1e-9);
  });

  it("colors by brand, never by rank", () => {
    expect(brandColor("claude", false)).toBe("#DE7356");
    expect(brandColor("cursor", true)).toBe("#F5F5F7");
    expect(brandColor("somebody", false)).toBe(brandColor("somebody", true));
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
