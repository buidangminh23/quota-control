/**
 * The official price lists (`src/data/official-prices.json`, transcribed from the Claude and OpenAI
 * pricing pages) shaped into tables that fit the popup. A page table whose price columns come in groups
 * (Claude's base tokens and prompt caching, OpenAI's short and long context) becomes one table per
 * group, each keeping the leading label columns.
 *
 * The data keeps the pages' cells verbatim. A row shorter than its table continues the previous row's
 * leading cells (the pages' merged cells), drawn here as row spans; `{heading}` rows are sub-headings.
 */
import raw from "@/data/official-prices.json";
import type { PriceProvider, PriceTier } from "./settings";

interface RawGroup {
  label: string;
  span: number;
}

interface RawTable {
  section: string;
  tier: string;
  columns: string[];
  groups?: RawGroup[];
  rows: Array<string[] | { heading: string }>;
}

interface RawProvider {
  id: string;
  name: string;
  source: string;
  tables: RawTable[];
}

interface RawPrices {
  retrievedAt: string;
  providers: RawProvider[];
}

const DATA = raw as RawPrices;

export const PRICES_RETRIEVED_AT = DATA.retrievedAt;

/** The kind of price a column holds, for its color: input, output, cache reads, cache writes or other. */
export type PriceTone = "input" | "output" | "cacheRead" | "cacheWrite" | "other";

export interface PriceColumn {
  label: string;
  /** Label columns name the row (model, modality, category); price columns hold prices. */
  kind: "label" | "price";
  tone: PriceTone;
}

export interface PriceCell {
  text: string;
  column: number;
  /** How many rows the cell covers (the page's merged cells). */
  rowSpan: number;
}

export interface PriceNameCell extends PriceCell {
  /** The page's one-line pitch after a model name (Claude's current models). */
  description: string | null;
  /** A badge glued to the name on the page, e.g. `Legacy`. */
  tag: string | null;
}

export type PriceRow =
  | { kind: "heading"; label: string }
  | {
      kind: "row";
      /** Label cells this row draws (merged ones are drawn by the row above), then its price cells. */
      labels: PriceNameCell[];
      prices: PriceCell[];
      /** Every other model gets a shaded band, continued rows included. */
      shaded: boolean;
    };

export interface PriceTable {
  /** A column group (`Short context`, `Prompt caching`), or `null` for an ungrouped table. */
  caption: string | null;
  columns: PriceColumn[];
  rows: PriceRow[];
}

export interface PriceSection {
  id: string;
  tables: PriceTable[];
}

const NAME_DESCRIPTION_SEPARATOR = " — ";
const NAME_TAGS = ["Legacy", "with data sharing"] as const;
export const NOT_OFFERED = "-";

function providerData(provider: PriceProvider): RawProvider {
  const found = DATA.providers.find((entry) => entry.id === provider);
  if (!found) throw new Error(`No price list for ${provider}`);
  return found;
}

export function priceSource(provider: PriceProvider): string {
  return providerData(provider).source;
}

/** The tiers a provider's page lists, in the app's tier order. */
export function priceTiers(provider: PriceProvider): PriceTier[] {
  const tiers = new Set(providerData(provider).tables.map((table) => table.tier));
  return (["standard", "batch", "flex", "fast"] as const).filter((tier) => tiers.has(tier));
}

export function priceTone(column: string): PriceTone {
  if (column === "Input") return "input";
  if (column === "Output" || column === "Output / cost") return "output";
  if (column === "Cached input" || column === "Hits and refreshes") return "cacheRead";
  if (column.includes("writes")) return "cacheWrite";
  return "other";
}

function isPriceCell(cell: string): boolean {
  return cell.includes("$") || cell === NOT_OFFERED || cell === "Free";
}

/** How many leading columns name the row rather than price it. */
function labelColumnCount(table: RawTable): number {
  const full = table.rows.filter((row): row is string[] => Array.isArray(row) && row.length === table.columns.length);
  for (let column = 0; column < table.columns.length; column += 1) {
    if (full.some((row) => isPriceCell(row[column]!))) return Math.max(column, 1);
  }
  return 1;
}

export function splitName(cell: string): { name: string; description: string | null; tag: string | null } {
  const separator = cell.indexOf(NAME_DESCRIPTION_SEPARATOR);
  const head = separator >= 0 ? cell.slice(0, separator) : cell;
  const description = separator >= 0 ? cell.slice(separator + NAME_DESCRIPTION_SEPARATOR.length).trim() : "";
  const tag = NAME_TAGS.find((candidate) => head.endsWith(candidate) && head.length > candidate.length);
  return { name: (tag ? head.slice(0, -tag.length) : head).trim(), description: description || null, tag: tag ?? null };
}

/** The price columns cut into the page's column groups; one unnamed group when the page has none. */
function priceGroups(table: RawTable, labelColumns: number): Array<{ label: string | null; columns: number[] }> {
  const all = table.columns.map((_, index) => index).filter((index) => index >= labelColumns);
  const named: Array<{ label: string; columns: number[] }> = [];
  let cursor = 0;
  for (const group of table.groups ?? []) {
    const columns = Array.from({ length: group.span }, (_, offset) => cursor + offset).filter((index) => index >= labelColumns);
    cursor += group.span;
    if (columns.length > 0) named.push({ label: group.label, columns });
  }
  if (named.length === 0) return [{ label: null, columns: all }];
  return named;
}

interface ExpandedRow {
  cells: string[];
  /** Leading cells taken over from the row above. */
  carried: number;
}

function expandRows(table: RawTable): Array<ExpandedRow | { heading: string }> {
  let previous: string[] = [];
  return table.rows.map((row) => {
    if (!Array.isArray(row)) return row;
    const carried = table.columns.length - row.length;
    const cells = [...previous.slice(0, carried), ...row];
    previous = cells;
    return { cells, carried };
  });
}

function buildTable(table: RawTable, labelColumns: number, group: { label: string | null; columns: number[] }, split: boolean): PriceTable {
  const columnIndexes = [...Array.from({ length: labelColumns }, (_, index) => index), ...group.columns];
  const columns: PriceColumn[] = columnIndexes.map((index) => ({
    label: table.columns[index]!,
    kind: index < labelColumns ? "label" : "price",
    tone: index < labelColumns ? "other" : priceTone(table.columns[index]!),
  }));
  const rows: PriceRow[] = [];
  const spanning: Array<PriceNameCell | null> = Array.from({ length: labelColumns }, () => null);
  let shaded = true;
  for (const entry of expandRows(table)) {
    if ("heading" in entry) {
      rows.push({ kind: "heading", label: entry.heading });
      spanning.fill(null);
      continue;
    }
    const prices = group.columns.map((index) => ({ text: entry.cells[index] ?? NOT_OFFERED, column: index, rowSpan: 1 }));
    if (split && entry.carried === 0 && prices.every((cell) => cell.text === NOT_OFFERED)) {
      spanning.fill(null);
      continue;
    }
    if (entry.carried === 0) shaded = !shaded;
    const labels: PriceNameCell[] = [];
    for (let index = 0; index < labelColumns; index += 1) {
      const open = spanning[index];
      if (index < entry.carried && open) {
        open.rowSpan += 1;
        continue;
      }
      const parts = index === 0 ? splitName(entry.cells[index]!) : { name: entry.cells[index]!, description: null, tag: null };
      const cell: PriceNameCell = { text: parts.name, column: index, rowSpan: 1, description: parts.description, tag: parts.tag };
      spanning[index] = cell;
      labels.push(cell);
    }
    rows.push({ kind: "row", labels, prices, shaded });
  }
  return { caption: group.label, columns, rows };
}

/** A provider's price tables for one tier, grouped by page section in page order. */
export function priceSections(provider: PriceProvider, tier: PriceTier): PriceSection[] {
  const sections: PriceSection[] = [];
  for (const table of providerData(provider).tables) {
    if (table.tier !== tier) continue;
    const labelColumns = labelColumnCount(table);
    const groups = priceGroups(table, labelColumns);
    const tables = groups.map((group) => buildTable(table, labelColumns, group, groups.length > 1));
    const last = sections[sections.length - 1];
    if (last && last.id === table.section) last.tables.push(...tables);
    else sections.push({ id: table.section, tables });
  }
  return sections;
}

const PURE_PRICE = /^\$(\d[\d,]*(?:\.\d+)?)(?: \/ MTok)?$/;
const DOLLAR_AMOUNT = /\$(\d[\d,]*(?:\.\d+)?)/g;
const PER_MTOK = / \/ MTok/g;

/** The dollar amount of a cell that is nothing but a price (per million tokens or per the column's unit). */
export function purePrice(cell: string): { usd: number; text: string } | null {
  const match = PURE_PRICE.exec(cell);
  return match ? { usd: Number(match[1]!.replace(/,/g, "")), text: match[1]! } : null;
}

/** A cell with words around its prices: the per-million-token unit dropped, each dollar amount rewritten. */
export function priceText(cell: string, convert: ((usd: number, text: string) => string) | null): string {
  const text = cell.replace(PER_MTOK, "");
  if (!convert) return text;
  return text.replace(DOLLAR_AMOUNT, (_, amount: string) => convert(Number(amount.replace(/,/g, "")), amount));
}
