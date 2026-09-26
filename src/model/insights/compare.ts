/**
 * Side-by-side comparison of up to three models across every source the tab reads: quality on the
 * user's own projects, Epoch AI (index and each benchmark) and the Arena boards. Sources join only on
 * the exact model name (see `modelKey`). An Arena entry that is the same model at a stated reasoning
 * effort is shown under its own effort label, never passed off as the plain model.
 *
 * A row's winner is marked `decided` only when its interval clears every other model's interval;
 * rows without intervals (single Epoch benchmark results) mark the highest value and say nothing more.
 */
import type { Arena3dEntry, ArenaBoardSlug, ArenaSnapshot } from "./arena";
import { BENCHMARK_CATEGORIES, type BenchmarkBoard, type BenchmarkCategory, type EpochModel } from "./epoch";
import { effortVariant, modelKey, type EffortLevel } from "./modelNames";
import { QUALITY_PARTS, type ModelQuality, type QualityPartKey } from "./quality";

export const MAX_COMPARED = 3;

export type CandidateSource = "mine" | "epoch" | "arena" | "arena3d";

export interface CompareCandidate {
  key: string;
  name: string;
  sources: CandidateSource[];
  /** How much the user has used it here (turns), for ordering. */
  turns: number;
  eci: number | null;
}

export interface CompareInput {
  quality: readonly ModelQuality[];
  epoch: readonly EpochModel[];
  boards: readonly BenchmarkBoard[];
  arena: ArenaSnapshot | null;
  arena3d: readonly Arena3dEntry[];
}

/** Every model any source names, the user's own first, then by Epoch index. */
export function compareCandidates(input: CompareInput): CompareCandidate[] {
  const byKey = new Map<string, CompareCandidate>();
  const add = (name: string, source: CandidateSource, patch: Partial<Pick<CompareCandidate, "turns" | "eci">> = {}, preferName = false) => {
    const key = modelKey(name);
    if (!key) return;
    const existing = byKey.get(key);
    if (!existing) {
      byKey.set(key, { key, name, sources: [source], turns: patch.turns ?? 0, eci: patch.eci ?? null });
      return;
    }
    if (!existing.sources.includes(source)) existing.sources.push(source);
    if (preferName) existing.name = name;
    existing.turns += patch.turns ?? 0;
    if (patch.eci !== undefined) existing.eci = patch.eci;
  };
  for (const model of input.quality) add(model.name, "mine", { turns: model.counts.turns });
  for (const model of input.epoch) add(model.name, "epoch", { eci: model.eci }, true);
  for (const board of input.boards) for (const result of board.results) add(result.model, "epoch");
  for (const board of input.arena?.boards ?? []) {
    for (const entry of board.entries) add(entry.model, "arena");
    for (const entry of board.agentEntries) add(entry.model, "arena");
  }
  for (const entry of input.arena3d) add(entry.model, "arena3d");
  return [...byKey.values()].sort((a, b) => {
    const mine = Number(b.sources.includes("mine")) - Number(a.sources.includes("mine"));
    return mine || b.turns - a.turns || (b.eci ?? -Infinity) - (a.eci ?? -Infinity) || a.name.localeCompare(b.name);
  });
}

function fold(text: string): string {
  return text.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

/** Candidates whose name contains every word of `query`, in their usual order. */
export function searchCandidates(candidates: readonly CompareCandidate[], query: string, exclude: readonly string[], limit: number): CompareCandidate[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  const matches = candidates.filter((candidate) => {
    if (exclude.includes(candidate.key)) return false;
    const haystack = `${fold(candidate.name)} ${candidate.key}`;
    return words.every((word) => haystack.includes(word));
  });
  return matches.slice(0, limit);
}

export interface CompareCell {
  value: number;
  low: number | null;
  high: number | null;
  /** Sample size behind a rate from the user's own projects. */
  samples?: number;
  /** The Arena entry was the model at this reasoning effort. */
  effort?: EffortLevel;
  /** The exact name the source used. */
  sourceName?: string;
}

export type CompareRowKind =
  | { kind: "mineScore" }
  | { kind: "minePart"; part: QualityPartKey }
  | { kind: "mineTurns" }
  | { kind: "eci" }
  | { kind: "benchmark"; benchmark: string; category: BenchmarkCategory }
  | { kind: "arena"; board: ArenaBoardSlug }
  | { kind: "arenaAgent" }
  | { kind: "arena3d" };

export type CompareRow = CompareRowKind & {
  cells: (CompareCell | null)[];
  /** Indices of the best cells (ties share). */
  best: number[];
  /** The best cell's interval clears every other cell's interval. */
  decided: boolean;
  /** For rank rows a lower value is better. */
  lowerIsBetter: boolean;
};

function withBest(kind: CompareRowKind, cells: (CompareCell | null)[], lowerIsBetter = false): CompareRow | null {
  const present = cells.flatMap((cell, index) => (cell ? [{ cell, index }] : []));
  if (present.length === 0) return null;
  const target = lowerIsBetter ? Math.min(...present.map(({ cell }) => cell.value)) : Math.max(...present.map(({ cell }) => cell.value));
  const best = present.filter(({ cell }) => cell.value === target).map(({ index }) => index);
  let decided = false;
  if (present.length > 1 && best.length === 1) {
    const winner = cells[best[0]!]!;
    const others = present.filter(({ index }) => index !== best[0]).map(({ cell }) => cell);
    decided =
      winner.low !== null &&
      winner.high !== null &&
      others.every((cell) => cell.low !== null && cell.high !== null && (lowerIsBetter ? winner.high! < cell.low! : winner.low! > cell.high!));
  }
  return { ...kind, cells, best, decided, lowerIsBetter };
}

function arenaCell<T extends { model: string }>(entries: readonly T[], key: string, toCell: (entry: T) => CompareCell): CompareCell | null {
  const exact = entries.find((entry) => modelKey(entry.model) === key);
  if (exact) return { ...toCell(exact), sourceName: exact.model };
  const variants = entries.flatMap((entry) => {
    const effort = effortVariant(key, modelKey(entry.model));
    return effort ? [{ entry, effort, cell: toCell(entry) }] : [];
  });
  if (variants.length === 0) return null;
  const top = variants.reduce((best, candidate) => (candidate.cell.value > best.cell.value ? candidate : best));
  return { ...top.cell, effort: top.effort, sourceName: top.entry.model };
}

export function compareRows(keys: readonly string[], input: CompareInput): CompareRow[] {
  const rows: CompareRow[] = [];
  const push = (row: CompareRow | null) => {
    if (row) rows.push(row);
  };
  const mine = keys.map((key) => input.quality.filter((model) => modelKey(model.name) === key));
  const single = mine.map((models) => (models.length === 1 ? models[0]! : null));

  push(withBest({ kind: "mineScore" }, single.map((model) => (model?.score ? { value: model.score.value, low: model.score.low, high: model.score.high } : null))));
  for (const part of QUALITY_PARTS) {
    push(
      withBest(
        { kind: "minePart", part },
        single.map((model) => {
          const interval = model?.parts[part].interval;
          return model && interval && model.parts[part].enough ? { value: 100 * interval.rate, low: 100 * interval.low, high: 100 * interval.high, samples: interval.n } : null;
        }),
      ),
    );
  }
  const turnsRow = withBest({ kind: "mineTurns" }, single.map((model) => (model ? { value: model.counts.turns, low: null, high: null } : null)));
  if (turnsRow) rows.push({ ...turnsRow, best: [], decided: false });

  push(
    withBest(
      { kind: "eci" },
      keys.map((key) => {
        const model = input.epoch.find((candidate) => modelKey(candidate.name) === key);
        return model ? { value: model.eci, low: model.low, high: model.high } : null;
      }),
    ),
  );

  const current = input.boards
    .filter((board) => !board.dated)
    .sort((a, b) => BENCHMARK_CATEGORIES.indexOf(a.info.category) - BENCHMARK_CATEGORIES.indexOf(b.info.category) || a.name.localeCompare(b.name));
  for (const board of current) {
    const cells = keys.map((key) => {
      const result = board.results.find((candidate) => modelKey(candidate.model) === key);
      return result ? { value: 100 * result.performance, low: null, high: null } : null;
    });
    if (cells.filter(Boolean).length < Math.min(2, keys.length)) continue;
    push(withBest({ kind: "benchmark", benchmark: board.name, category: board.info.category }, cells));
  }

  for (const board of input.arena?.boards ?? []) {
    if (board.slug === "agent") {
      push(
        withBest(
          { kind: "arenaAgent" },
          keys.map((key) => arenaCell(board.agentEntries, key, (entry) => ({ value: entry.rank, low: null, high: null }))),
          true,
        ),
      );
      continue;
    }
    push(
      withBest(
        { kind: "arena", board: board.slug },
        keys.map((key) => arenaCell(board.entries, key, (entry) => ({ value: entry.score, low: entry.ci === null ? null : entry.score - entry.ci, high: entry.ci === null ? null : entry.score + entry.ci }))),
      ),
    );
  }
  push(
    withBest(
      { kind: "arena3d" },
      keys.map((key) => {
        const entry = input.arena3d.find((candidate) => modelKey(candidate.model) === key);
        return entry ? { value: entry.score, low: null, high: null, sourceName: entry.model } : null;
      }),
    ),
  );
  return rows;
}
