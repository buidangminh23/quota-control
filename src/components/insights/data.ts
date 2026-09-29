/**
 * Parsed feeds, shared by the Benchmark views. Each parse is remembered against the body it came
 * from, so switching views or tabs never re-reads the same CSV or JSON.
 */
import { useEffect } from "react";
import type { PublicFeedName, PublicFeedSnapshot } from "@/lib/insightsTypes";
import { parseArena, parseArena3d, type Arena3dEntry, type ArenaSnapshot } from "@/model/insights/arena";
import { benchmarkBoards, parseEpochBenchmarks, parseEpochScores, type BenchmarkBoard, type EpochModel } from "@/model/insights/epoch";
import { ensureFeed, useInsights } from "@/state/insights";

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
