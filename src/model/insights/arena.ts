/**
 * Human-preference leaderboards: Arena (arena.ai; the core reads a community snapshot of its boards
 * published on GitHub) and 3D Arena (Hugging Face). Scores are Elo-style ratings from people voting
 * between two anonymous outputs; `ci` is the ± margin Arena publishes next to each rating. The agent
 * board is different: several measured dimensions per model instead of one rating.
 */

export type ArenaBoardSlug =
  | "text"
  | "code"
  | "vision"
  | "document"
  | "search"
  | "agent"
  | "text-to-image"
  | "image-edit"
  | "text-to-video"
  | "image-to-video"
  | "video-edit";

/** Display order: language work first, then images, video and 3D. */
export const ARENA_BOARDS: readonly ArenaBoardSlug[] = [
  "text",
  "code",
  "agent",
  "search",
  "document",
  "vision",
  "text-to-image",
  "image-edit",
  "text-to-video",
  "image-to-video",
  "video-edit",
];

export interface ArenaEntry {
  model: string;
  rank: number;
  score: number;
  /** ± margin as published, when present. */
  ci: number | null;
  votes: number | null;
  vendor: string;
  openWeights: boolean;
}

export interface ArenaDimension {
  name: string;
  score: number;
  ci: number | null;
}

export interface ArenaAgentEntry {
  model: string;
  rank: number;
  sessions: number | null;
  vendor: string;
  dimensions: ArenaDimension[];
}

export interface ArenaBoard {
  slug: ArenaBoardSlug;
  /** As Arena writes it, e.g. `Sep 25, 2026`. */
  updated: string;
  sourceUrl: string | null;
  entries: ArenaEntry[];
  /** Only on the agent board. */
  agentEntries: ArenaAgentEntry[];
  dimensions: string[];
}

export interface ArenaSnapshot {
  /** The snapshot's day, `YYYY-MM-DD`. */
  date: string;
  boards: ArenaBoard[];
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function finite(value: unknown): number | null {
  const number = typeof value === "string" && value.trim() !== "" ? Number(value) : value;
  return typeof number === "number" && Number.isFinite(number) ? number : null;
}

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function arenaUrl(value: unknown): string | null {
  const raw = text(value);
  try {
    const url = new URL(raw);
    return url.protocol === "https:" && (url.hostname === "arena.ai" || url.hostname.endsWith(".arena.ai") || url.hostname === "lmarena.ai") ? url.toString() : null;
  } catch {
    return null;
  }
}

function entry(value: unknown): ArenaEntry | null {
  const raw = record(value);
  const model = text(raw?.model);
  const rank = finite(raw?.rank);
  const score = finite(raw?.score);
  if (!raw || !model || rank === null || score === null) return null;
  return { model, rank, score, ci: finite(raw.ci), votes: finite(raw.votes), vendor: text(raw.vendor), openWeights: raw.license === "open" };
}

function agentEntry(value: unknown): ArenaAgentEntry | null {
  const raw = record(value);
  const model = text(raw?.model);
  const rank = finite(raw?.rank);
  if (!raw || !model || rank === null || !Array.isArray(raw.scores)) return null;
  const dimensions = raw.scores.flatMap((item) => {
    const dimension = record(item);
    const name = text(dimension?.name);
    const score = finite(dimension?.score);
    return name && score !== null ? [{ name, score, ci: finite(dimension?.ci) }] : [];
  });
  return dimensions.length > 0 ? { model, rank, sessions: finite(raw.sessions), vendor: text(raw.vendor), dimensions } : null;
}

export function parseArena(body: string | null | undefined): ArenaSnapshot | null {
  if (!body) return null;
  let root: Record<string, unknown> | null;
  try {
    root = record(JSON.parse(body));
  } catch {
    return null;
  }
  const boards = record(root?.boards);
  if (!root || !boards) return null;
  const parsed: ArenaBoard[] = [];
  for (const slug of ARENA_BOARDS) {
    const board = record(boards[slug]);
    if (!board || !Array.isArray(board.models)) continue;
    const meta = record(board.meta);
    const agent = slug === "agent";
    const entries = agent ? [] : board.models.map(entry).filter((item): item is ArenaEntry => item !== null).sort((a, b) => a.rank - b.rank || b.score - a.score);
    const agentEntries = agent ? board.models.map(agentEntry).filter((item): item is ArenaAgentEntry => item !== null).sort((a, b) => a.rank - b.rank) : [];
    if (entries.length === 0 && agentEntries.length === 0) continue;
    const dimensions = Array.isArray(meta?.dimensions) ? meta.dimensions.filter((name): name is string => typeof name === "string") : [];
    parsed.push({ slug, updated: text(meta?.last_updated), sourceUrl: arenaUrl(meta?.source_url), entries, agentEntries, dimensions });
  }
  return { date: text(root.date), boards: parsed };
}

export interface Arena3dEntry {
  model: string;
  rank: number;
  score: number;
  votes: number | null;
  openSource: boolean;
}

export function parseArena3d(body: string | null | undefined): Arena3dEntry[] {
  if (!body) return [];
  let rows: unknown;
  try {
    rows = JSON.parse(body);
  } catch {
    return [];
  }
  if (!Array.isArray(rows)) return [];
  return rows
    .flatMap((value) => {
      const raw = record(value);
      const model = text(raw?.name);
      const score = finite(raw?.score);
      if (!raw || !model || score === null) return [];
      return [{ model, rank: finite(raw.rank) ?? 0, score, votes: finite(raw.votes), openSource: raw.open_source === true }];
    })
    .sort((a, b) => b.score - a.score)
    .map((item, index) => ({ ...item, rank: item.rank > 0 ? item.rank : index + 1 }));
}
