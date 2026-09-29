/**
 * Parsed feeds, shared by the Benchmark views. Each parse is remembered against the body it came
 * from, so switching views or tabs never re-reads the same CSV or JSON. The Reset tab's Claude view
 * also reads which Claude plans are connected here.
 */
import { useEffect, useMemo } from "react";
import type { PublicFeedName, PublicFeedSnapshot } from "@/lib/insightsTypes";
import { parseArena, parseArena3d, type Arena3dEntry, type ArenaSnapshot } from "@/model/insights/arena";
import { benchmarkBoards, parseEpochBenchmarks, parseEpochScores, type BenchmarkBoard, type EpochModel } from "@/model/insights/epoch";
import { planFamily, type ClaudePlan } from "@/model/insights/claudeResets";
import { brandOf, isLocalHistoryCard } from "@/model/layout";
import { ensureFeed, useInsights } from "@/state/insights";
import { isProviderEnabled, useApp } from "@/state/store";

function remember<T>(parse: (body: string | null, snapshot: PublicFeedSnapshot | undefined) => T): (snapshot: PublicFeedSnapshot | undefined) => T {
  let last: { body: string | null; value: T } | null = null;
  return (snapshot) => {
    const body = snapshot?.body ?? null;
    if (last && last.body === body) return last.value;
    const value = parse(body, snapshot);
    last = { body, value };
    return value;
  };
}

export const epochScoresOf = remember<EpochModel[]>((body) => parseEpochScores(body));

/** Benchmarks age against the day the data was fetched, so a board's "dated" mark is stable. */
export const epochBoardsOf = remember<BenchmarkBoard[]>((body, snapshot) =>
  benchmarkBoards(parseEpochBenchmarks(body), snapshot?.fetchedAt ? new Date(snapshot.fetchedAt) : new Date()),
);

export const arenaOf = remember<ArenaSnapshot | null>((body) => parseArena(body));

export const arena3dOf = remember<Arena3dEntry[]>((body) => parseArena3d(body));

/** Load the named feeds once and return their snapshots. */
export function useFeeds(names: readonly PublicFeedName[]): Partial<Record<PublicFeedName, PublicFeedSnapshot>> {
  const key = names.join(",");
  useEffect(() => {
    for (const name of key.split(",") as PublicFeedName[]) ensureFeed(name);
  }, [key]);
  return useInsights((state) => state.feeds);
}

const CLAUDE_BRAND = "claude";

/** The plan families (`max`, `pro`…) of the Claude accounts shown on the dashboard, each once. */
export function useClaudePlans(): ClaudePlan[] {
  const key = useApp((state) => {
    const families = Object.entries(state.engine?.providers ?? {})
      .filter(([id]) => brandOf(id) === CLAUDE_BRAND && !isLocalHistoryCard(id) && isProviderEnabled(state, id))
      .map(([, runtime]) => planFamily(runtime.snapshot?.plan))
      .filter((family): family is ClaudePlan => family !== null);
    return [...new Set(families)].sort().join(",");
  });
  return useMemo(() => (key ? (key.split(",") as ClaudePlan[]) : []), [key]);
}
