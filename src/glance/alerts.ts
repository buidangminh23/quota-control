/**
 * Notices the macOS Dynamic Island opens up for a few seconds: a limit running low, or a Codex or
 * Claude reset announced. The notification hooks announce them here, next to the system
 * notification when that is on; `useGlance` carries the newest one to the island, which shows each
 * id once.
 */
import { useSyncExternalStore } from "react";
import type { GlanceAlert, GlanceSeverity } from "@/model/glance";

let latest: GlanceAlert | null = null;
let sequence = 0;
const listeners = new Set<() => void>();

export function announceOnIsland(alert: { title: string; body: string; brand?: string; severity: GlanceSeverity }, now = Date.now()): void {
  sequence += 1;
  latest = { ...alert, id: `${now}-${sequence}` };
  for (const listener of listeners) listener();
}

export function useIslandAlert(): GlanceAlert | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => latest,
    () => null,
  );
}
