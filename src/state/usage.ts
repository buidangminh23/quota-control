/**
 * The Token tab's data hooks: summaries from the core's usage ledger (cached per query until the
 * ledger changes), today's date, the colors every view shares, and recent sessions' context windows
 * (polled while the popup is on screen).
 */
import { useEffect, useMemo, useState } from "react";
import { localDay } from "@/lib/days";
import { backend } from "@/lib/backend";
import type { ContextWindowSession, UsageGroupRow, UsageQuery } from "@/lib/types";
import { assignColors } from "@/model/palette";
import { totalsByKey } from "@/model/usage";
import { useNow } from "./hooks";
import { useApp } from "./store";

const CACHE_LIMIT = 80;
const summaries = new Map<string, { version: number; rows: UsageGroupRow[] }>();

function remember(key: string, version: number, rows: UsageGroupRow[]): void {
  summaries.delete(key);
  summaries.set(key, { version, rows });
  while (summaries.size > CACHE_LIMIT) summaries.delete(summaries.keys().next().value!);
}

function cachedRows(key: string | null): UsageGroupRow[] | null {
  return key === null ? null : (summaries.get(key)?.rows ?? null);
}

export interface UsageRows {
  /** The rows, or `null` while the first read is in flight. A newer ledger keeps the old rows on screen until it answers. */
  rows: UsageGroupRow[] | null;
  failed: boolean;
}

/** The ledger's answer to `query`; `null` skips the read. */
export function useUsageRows(query: UsageQuery | null): UsageRows {
  const version = useApp((state) => state.ledgerVersion);
  const key = query === null ? null : JSON.stringify(query);
  const [result, setResult] = useState(() => ({ key, rows: cachedRows(key), failed: false }));

  useEffect(() => {
    if (key === null) return;
    const hit = summaries.get(key);
    if (hit && hit.version === version) {
      setResult({ key, rows: hit.rows, failed: false });
      return;
    }
    let live = true;
    backend()
      .usageSummary(JSON.parse(key) as UsageQuery)
      .then(
        (rows) => {
          remember(key, version, rows);
          if (live) setResult({ key, rows, failed: false });
        },
        () => {
          if (live) setResult({ key, rows: cachedRows(key), failed: true });
        },
      );
    return () => {
      live = false;
    };
  }, [key, version]);

  return result.key === key ? { rows: result.rows, failed: result.failed } : { rows: cachedRows(key), failed: false };
}

/** Today's local day (`YYYY-MM-DD`), turning over at midnight while the popup is open. */
export function useToday(): string {
  const now = useNow(60_000);
  return localDay(now);
}

export interface UsageColors {
  models: ReadonlyMap<string, string>;
  projects: ReadonlyMap<string, string>;
}

/** Model and project colors handed out along the all-time ranking, so every view agrees. */
export function useUsageColors(): UsageColors {
  const today = useToday();
  const models = useUsageRows({ to: today, groupBy: "model" }).rows;
  const projects = useUsageRows({ to: today, groupBy: "project" }).rows;
  return useMemo(() => {
    const ranked = (rows: UsageGroupRow[] | null, withSource: boolean) =>
      totalsByKey(rows ?? [])
        .sort((a, b) => b.total.totalTokens - a.total.totalTokens)
        .map((item) => (withSource ? { key: item.key, source: item.source } : { key: item.key }));
    return { models: assignColors(ranked(models, true)), projects: assignColors(ranked(projects, false)) };
  }, [models, projects]);
}

const CONTEXT_POLL_MS = 10_000;
let lastSessions: ContextWindowSession[] | null = null;

/**
 * Recent sessions' context windows, newest first: `undefined` where the core cannot read them, `null`
 * until the first answer. Read every ten seconds while the popup is on screen.
 */
export function useContextWindows(): ContextWindowSession[] | null | undefined {
  const supported = typeof backend().contextWindows === "function";
  const visible = useApp((state) => state.popupVisible);
  const [sessions, setSessions] = useState(lastSessions);

  useEffect(() => {
    if (!supported || !visible) return;
    let live = true;
    const load = () =>
      backend()
        .contextWindows?.()
        .then(
          (next) => {
            lastSessions = next;
            if (live) setSessions(next);
          },
          () => undefined,
        );
    void load();
    const timer = setInterval(() => void load(), CONTEXT_POLL_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [supported, visible]);

  return supported ? sessions : undefined;
}
