/**
 * The Benchmark and Reset tabs' data, kept apart from the main store: the model-quality summary for
 * the chosen period and the public feeds the core caches. Feeds load when a tab first needs them
 * and follow the core's change events afterwards, so a tab switch never waits on the network.
 */
import { create } from "zustand";
import { backend } from "@/lib/backend";
import type { PublicFeedName, PublicFeedSnapshot, QualityInfo, QualitySummary } from "@/lib/insightsTypes";
import { QUALITY_RANGES, rangeQuery, type ProjectFilter, type QualityRange } from "@/model/insights/quality";

export type BenchmarkView = "mine" | "public" | "compare";
export const BENCHMARK_VIEWS: readonly BenchmarkView[] = ["mine", "public", "compare"];

/** Which public board the Công khai view shows. */
export type PublicBoardChoice =
  | { kind: "eci" }
  | { kind: "epochCategory"; category: string }
  | { kind: "arena"; board: string }
  | { kind: "arena3d" };

export interface InsightsState {
  feeds: Partial<Record<PublicFeedName, PublicFeedSnapshot>>;
  refreshing: Partial<Record<PublicFeedName, boolean>>;
  feedErrors: Partial<Record<PublicFeedName, string>>;
  quality: QualitySummary | null;
  /** The query `quality` answers, as `from|to`. */
  qualityKey: string | null;
  qualityInfo: QualityInfo | null;
  qualityLoading: boolean;
  qualityError: string | null;
  view: BenchmarkView;
  range: QualityRange;
  project: ProjectFilter;
  board: PublicBoardChoice;
  compared: string[] | null;
}

export const useInsights = create<InsightsState>(() => ({
  feeds: {},
  refreshing: {},
  feedErrors: {},
  quality: null,
  qualityKey: null,
  qualityInfo: null,
  qualityLoading: false,
  qualityError: null,
  view: "mine",
  range: "30",
  project: null,
  board: { kind: "eci" },
  compared: null,
}));

const get = () => useInsights.getState();
const set = useInsights.setState;

function message(error: unknown): string {
  return error instanceof Error ? error.message : typeof error === "string" ? error : "";
}

const requested = new Set<PublicFeedName>();

/** Load a feed's cached copy once; later changes arrive through `startInsights`. */
export function ensureFeed(name: PublicFeedName): void {
  if (requested.has(name)) return;
  requested.add(name);
  void reloadFeed(name);
}

async function reloadFeed(name: PublicFeedName): Promise<void> {
  try {
    const snapshot = await backend().publicFeed(name);
    set({ feeds: { ...get().feeds, [name]: snapshot }, feedErrors: { ...get().feedErrors, [name]: undefined } });
  } catch (error) {
    requested.delete(name);
    set({ feedErrors: { ...get().feedErrors, [name]: message(error) } });
  }
}

/** Ask the sources now; the core throttles repeated requests. */
export async function refreshFeeds(names: readonly PublicFeedName[]): Promise<void> {
  set({ refreshing: { ...get().refreshing, ...Object.fromEntries(names.map((name) => [name, true])) } });
  await Promise.all(
    names.map(async (name) => {
      try {
        const snapshot = await backend().refreshPublicFeed(name);
        set({ feeds: { ...get().feeds, [name]: snapshot }, feedErrors: { ...get().feedErrors, [name]: undefined } });
      } catch (error) {
        set({ feedErrors: { ...get().feedErrors, [name]: message(error) } });
      } finally {
        set({ refreshing: { ...get().refreshing, [name]: false } });
      }
    }),
  );
}

let qualityRequest = 0;
/** The query of the request in flight, so a second caller does not ask again. */
let pendingKey: string | null = null;

/** Load the summary for the current period unless it is already on screen or on its way. */
export function loadQuality(force = false, now = new Date()): void {
  const query = rangeQuery(get().range, now);
  const key = `${query.from ?? ""}|${query.to ?? ""}`;
  if (!force && (pendingKey === key || (get().qualityKey === key && get().quality))) return;
  const request = ++qualityRequest;
  pendingKey = key;
  set({ qualityLoading: true, qualityError: null });
  backend()
    .modelQuality(query)
    .then((summary) => {
      if (request !== qualityRequest) return;
      set({ quality: summary, qualityKey: key, qualityInfo: summary.info, qualityLoading: false });
      if (summary.info.scannedAt === null && !summary.info.scanning) rescanQuality();
    })
    .catch((error: unknown) => {
      if (request !== qualityRequest) return;
      set({ qualityLoading: false, qualityError: message(error) });
    })
    .finally(() => {
      if (request === qualityRequest) pendingKey = null;
    });
}

export function rescanQuality(): void {
  set({ qualityInfo: get().qualityInfo ? { ...get().qualityInfo!, scanning: true } : null });
  backend()
    .rescanModelQuality()
    .catch((error: unknown) => set({ qualityError: message(error) }));
}

export function setBenchmarkView(view: BenchmarkView): void {
  set({ view });
}

export function setQualityRange(range: QualityRange): void {
  if (!QUALITY_RANGES.includes(range) || range === get().range) return;
  set({ range });
  loadQuality();
}

export function setQualityProject(project: ProjectFilter): void {
  set({ project });
}

export function setPublicBoard(board: PublicBoardChoice): void {
  set({ board });
}

export function setCompared(keys: string[]): void {
  set({ compared: keys });
}

const INITIAL: InsightsState = useInsights.getState();

/** Forget every loaded feed and choice (tests start each case from a fresh backend). */
export function resetInsights(): void {
  requested.clear();
  qualityRequest += 1;
  pendingKey = null;
  set(INITIAL, true);
}

/** Follow the core's change events for as long as the popup lives. */
export function startInsights(): () => void {
  const api = backend();
  const stopQuality = api.onModelQualityChanged((info) => {
    set({ qualityInfo: info });
    if (!info.scanning && get().qualityKey !== null) loadQuality(true);
  });
  const stopFeeds = api.onPublicFeedChanged((name) => {
    if (requested.has(name)) void reloadFeed(name);
  });
  return () => {
    stopQuality();
    stopFeeds();
  };
}
