import { fixtureCatalog } from "@/lib/fixtures";
import type { ProviderEntry } from "@/lib/types";
import {
  applyMetricSections,
  brandOf,
  canPin,
  customizeRows,
  defaultProviderOrder,
  displayGroups,
  isLocalHistoryCard,
  layoutFamily,
  MAX_PINS_PER_PROVIDER,
  parseLayout,
  pinnedGroups,
  reconcileLayout,
  reorderProvider,
  resetProvider,
  setMetricEnabled,
  setPinned,
  setProviderOpen,
  spendCapableProviders,
} from "./layout";

const catalog = fixtureCatalog();
const all = () => true;
const WORK = "claude@7c1e";
const PERSONAL = "claude@a93f";
const CODEX = "codex@52d0";

function ids(entries: readonly { provider: { id: string } }[]): string[] {
  return entries.map((entry) => entry.provider.id);
}

describe("layout identity", () => {
  it("maps account and local cards to their family and brand", () => {
    expect(layoutFamily(WORK)).toBe("claude");
    expect(layoutFamily("claude-local")).toBe("claude-local");
    expect(brandOf("claude-local")).toBe("claude");
    expect(brandOf(CODEX)).toBe("codex");
    expect(isLocalHistoryCard("codex-local")).toBe(true);
    expect(isLocalHistoryCard(CODEX)).toBe(false);
  });

  it("orders brands claude, codex, then others, accounts before the brand's local card", () => {
    expect(defaultProviderOrder(catalog)).toEqual([WORK, PERSONAL, "claude-local", CODEX, "codex-local"]);
  });
});

describe("reconcileLayout", () => {
  const layout = reconcileLayout(null, catalog);

  it("applies the claude account defaults to every connected claude account", () => {
    for (const id of [WORK, PERSONAL]) {
      expect(layout.placed).toEqual(expect.arrayContaining([`${id}.session`, `${id}.weekly`, `${id}.fable`, `${id}.extra`]));
      expect(layout.placed).not.toContain(`${id}.sonnet`);
      expect(layout.onDemandWhenEnabled).toContain(`${id}.sonnet`);
      expect(layout.pinned).toEqual(expect.arrayContaining([`${id}.session`, `${id}.weekly`]));
    }
  });

  it("puts token rows of local cards behind the caret and pins nothing there", () => {
    expect(layout.onDemand).toEqual(expect.arrayContaining(["claude-local.inputTokens", "claude-local.outputTokens", "claude-local.cachedInputTokens"]));
    expect(layout.pinned.filter((id) => id.startsWith("claude-local"))).toEqual([]);
    expect(layout.placed).toContain("claude-local.today");
  });

  it("offers defaults once, so a metric the user turned off stays off", () => {
    const off = setMetricEnabled(layout, `${WORK}.fable`, false);
    expect(reconcileLayout(off, catalog).placed).not.toContain(`${WORK}.fable`);
  });

  it("inserts a newly connected account before its brand's local card", () => {
    const provider = { id: "claude@beef", displayName: "Claude · Mới", icon: "claude" };
    const next: ProviderEntry[] = [...catalog, { provider, descriptors: [{ ...catalog[0]!.descriptors[0]!, id: "claude@beef.session", providerId: "claude@beef" }] }];
    const order = reconcileLayout(layout, next).providerOrder;
    expect(order.indexOf("claude@beef")).toBe(order.indexOf("claude-local") - 1);
  });

  it("round-trips through parseLayout and rejects foreign documents", () => {
    expect(parseLayout(JSON.parse(JSON.stringify(layout)))).toEqual(layout);
    expect(parseLayout({ version: 99 })).toBeNull();
    expect(parseLayout("nope")).toBeNull();
  });
});

describe("layout views and edits", () => {
  const layout = reconcileLayout(null, catalog);

  it("lists every enabled provider with placed metrics and hides disabled ones", () => {
    expect(ids(displayGroups(layout, catalog, all))).toEqual([WORK, PERSONAL, "claude-local", CODEX, "codex-local"]);
    expect(ids(displayGroups(layout, catalog, (id) => id !== PERSONAL))).not.toContain(PERSONAL);
    expect(customizeRows(layout, catalog, (id) => id !== PERSONAL).find((row) => row.provider.id === PERSONAL)?.enabled).toBe(false);
  });

  it("promotes a card whose every metric is On Demand above the caret", () => {
    const onlyTokens = catalog.find((entry) => entry.provider.id === "codex-local")!;
    let next = layout;
    for (const descriptor of onlyTokens.descriptors) {
      if (!descriptor.id.endsWith("Tokens")) next = setMetricEnabled(next, descriptor.id, false);
    }
    const group = displayGroups(next, catalog, all).find((entry) => entry.provider.id === "codex-local")!;
    expect(group.onDemand).toEqual([]);
    expect(group.always.map((descriptor) => descriptor.id)).toEqual(["codex-local.inputTokens", "codex-local.outputTokens", "codex-local.cachedInputTokens"]);
  });

  it("caps stars at two per provider", () => {
    expect(MAX_PINS_PER_PROVIDER).toBe(2);
    expect(canPin(layout, catalog, `${WORK}.fable`)).toBe(false);
    expect(setPinned(layout, catalog, `${WORK}.fable`, true)).toBe(layout);
    const freed = setPinned(layout, catalog, `${WORK}.weekly`, false);
    expect(setPinned(freed, catalog, `${WORK}.fable`, true).pinned).toContain(`${WORK}.fable`);
  });

  it("never lets the unpinnable trend chart be starred", () => {
    expect(canPin(layout, catalog, "claude-local.trend")).toBe(false);
  });

  it("starts an optional metric On Demand the first time it is enabled", () => {
    const next = setMetricEnabled(layout, `${WORK}.sonnet`, true);
    expect(next.onDemand).toContain(`${WORK}.sonnet`);
    expect(next.onDemandWhenEnabled).not.toContain(`${WORK}.sonnet`);
  });

  it("moves a dragged metric between sections and records the new order", () => {
    const always = [`${WORK}.session`, `${WORK}.weekly`, `${WORK}.sonnet`, `${WORK}.extra`];
    const next = applyMetricSections(layout, WORK, always, [`${WORK}.fable`], `${WORK}.fable`);
    expect(next.onDemand).toContain(`${WORK}.fable`);
    expect(next.metricOrder[WORK]?.slice(0, 5)).toEqual([...always, `${WORK}.fable`]);
  });

  it("reorders enabled providers and keeps caret state out of the order", () => {
    const next = reorderProvider(layout, catalog, all, CODEX, WORK);
    expect(next.providerOrder[0]).toBe(CODEX);
    expect(setProviderOpen(layout, WORK, true).openProviders).toEqual([WORK]);
  });

  it("restores one provider's defaults without touching the others", () => {
    const edited = setMetricEnabled(setMetricEnabled(layout, `${WORK}.fable`, false), `${CODEX}.weekly`, false);
    const reset = resetProvider(edited, catalog, WORK);
    expect(reset.placed).toContain(`${WORK}.fable`);
    expect(reset.placed).not.toContain(`${CODEX}.weekly`);
  });

  it("feeds the strip from starred metrics and Total Spend from local cards", () => {
    expect(pinnedGroups(layout, catalog, all).map((group) => group.provider.id)).toEqual([WORK, PERSONAL, CODEX]);
    expect(spendCapableProviders(layout, catalog, all).map((provider) => provider.id)).toEqual(["claude-local", "codex-local"]);
  });
});
