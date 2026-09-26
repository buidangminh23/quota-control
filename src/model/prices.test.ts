import { formatDongNumber, formatDongPrice, formatPageDollars } from "./currency";
import { priceSections, priceText, priceTiers, purePrice, splitName, type PriceRow, type PriceTable } from "./prices";

function dataRows(table: PriceTable): Array<Extract<PriceRow, { kind: "row" }>> {
  return table.rows.filter((row): row is Extract<PriceRow, { kind: "row" }> => row.kind === "row");
}

function rowNamed(table: PriceTable, name: string) {
  return dataRows(table).find((row) => row.labels[0]?.text === name);
}

describe("official price tables", () => {
  it("lists each provider's tiers in the app's order", () => {
    expect(priceTiers("claude")).toEqual(["standard", "batch", "fast"]);
    expect(priceTiers("openai")).toEqual(["standard", "batch", "flex", "fast"]);
  });

  it("cuts Claude's standard table into base tokens and prompt caching", () => {
    const [models] = priceSections("claude", "standard");
    expect(models!.id).toBe("models");
    expect(models!.tables.map((table) => table.caption)).toEqual(["Base tokens", "Prompt caching"]);
    const [base, caching] = models!.tables;
    expect(base!.columns.map((column) => column.label)).toEqual(["Name", "Input", "Output"]);
    expect(caching!.columns.map((column) => [column.label, column.tone])).toEqual([
      ["Name", "other"],
      ["5m writes", "cacheWrite"],
      ["1h writes", "cacheWrite"],
      ["Hits and refreshes", "cacheRead"],
    ]);
    const opus = rowNamed(base!, "Claude Opus 5.5")!;
    expect(opus.prices.map((cell) => cell.text)).toEqual(["$4 / MTok", "$20 / MTok"]);
    expect(opus.labels[0]!.description).toBe("For long-running agentic coding and knowledge work");
    expect(base!.rows.some((row) => row.kind === "heading" && row.label === "Additional models")).toBe(true);
    expect(rowNamed(caching!, "Claude Sonnet 5")!.prices.map((cell) => cell.text)).toEqual(["$2.50 / MTok", "$4 / MTok", "$0.20 / MTok"]);
  });

  it("splits OpenAI's flagship table by context length and drops rows with no long-context price", () => {
    const flagship = priceSections("openai", "standard").find((section) => section.id === "flagship")!;
    expect(flagship.tables.map((table) => table.caption)).toEqual(["Short context", "Long context", null]);
    const [short, long] = flagship.tables;
    expect(rowNamed(short!, "gpt-5.6-sol")!.prices.map((cell) => cell.text)).toEqual(["$4.00", "$0.40", "$5.00", "$20.00"]);
    expect(rowNamed(long!, "gpt-5.6-sol")!.prices.map((cell) => cell.text)).toEqual(["$8.00", "$0.80", "$10.00", "$30.00"]);
    const cyber = priceSections("openai", "standard").find((section) => section.id === "cyber")!;
    expect(rowNamed(cyber.tables[1]!, "gpt-5.6-cyber")).toBeUndefined();
    expect(rowNamed(cyber.tables[0]!, "gpt-5.6-cyber")).toBeDefined();
  });

  it("draws a model's continued rows under one merged name cell", () => {
    const realtime = priceSections("openai", "standard").find((section) => section.id === "realtime")!.tables[0]!;
    const rows = dataRows(realtime);
    const first = rows[0]!;
    expect(first.labels.map((cell) => [cell.text, cell.rowSpan])).toEqual([
      ["gpt-realtime-2.1", 3],
      ["Audio", 1],
    ]);
    expect(rows[1]!.labels.map((cell) => cell.text)).toEqual(["Text"]);
    expect(rows[1]!.shaded).toBe(first.shaded);
    expect(rows[3]!.shaded).not.toBe(first.shaded);
    const specialized = priceSections("openai", "standard").find((section) => section.id === "specialized")!.tables[0]!;
    const embedding = dataRows(specialized).find((row) => row.labels[0]?.text === "Embedding")!;
    expect(embedding.labels[0]!.rowSpan).toBe(3);
  });

  it("separates the badges glued to names", () => {
    expect(splitName("gpt-3.5-turboLegacy")).toEqual({ name: "gpt-3.5-turbo", description: null, tag: "Legacy" });
    expect(splitName("o4-mini-2025-04-16with data sharing").tag).toBe("with data sharing");
    expect(splitName("Claude Mythos 5.1 — ")).toEqual({ name: "Claude Mythos 5.1", description: null, tag: null });
  });
});

describe("price cells", () => {
  it("reads a plain price and rewrites amounts inside longer cells", () => {
    expect(purePrice("$10 / MTok")).toEqual({ usd: 10, text: "10" });
    expect(purePrice("$0.075")).toEqual({ usd: 0.075, text: "0.075" });
    expect(purePrice("$100.00 / hour")).toBeNull();
    expect(priceText("$0.034 / minute", (usd) => `${usd * 1000}d`)).toBe("34d / minute");
    expect(priceText("1 GB $0.03, 4 GB $0.12", null)).toBe("1 GB $0.03, 4 GB $0.12");
  });

  it("writes đồng with the precision a price needs and keeps the page's dollar decimals", () => {
    expect(formatDongNumber(10 * 26_170, "vi")).toBe("261.700");
    expect(formatDongNumber(0.01 * 26_170, "vi")).toBe("262");
    expect(formatDongNumber(0.0002 * 26_170, "vi")).toBe("5,23");
    expect(formatDongPrice(0.034 * 26_170, "vi")).toBe("890 ₫");
    expect(formatDongPrice(0.0004 * 26_170, "vi")).toBe("10,5 ₫");
    expect(formatPageDollars("0.20", "vi")).toBe("0,20");
    expect(formatPageDollars("1.25", "en")).toBe("1.25");
  });
});
