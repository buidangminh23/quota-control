/**
 * A limit window whose reset time has passed is shown as reset: nothing used and no countdown, the way
 * a provider reports a window that has not started, until the next reading replaces it (the core asks
 * again a few seconds after every reset). So a reading taken before a reset never passes for the
 * current one, not even offline or while its provider is backed off.
 */
import type { EngineState, ProgressLine, ProviderRuntimeState } from "@/lib/types";
import type { WidgetData } from "./widgetData";

function resetTime(line: ProgressLine): number | null {
  if (!line.resetsAt) return null;
  const time = Date.parse(line.resetsAt);
  return Number.isNaN(time) ? null : time;
}

function rolledOver(line: ProgressLine): ProgressLine {
  const fresh: ProgressLine = { ...line, used: 0 };
  delete fresh.resetsAt;
  return fresh;
}

/**
 * A row's reading once its window has rolled over, as `rollOverPassedWindows` leaves its line:
 * nothing used and no countdown. The island and the widgets draw it from the reset time on, before
 * the next reading reaches them.
 */
export function rolledOverReading(data: WidgetData): WidgetData {
  return { ...data, used: 0, resetsAt: null };
}

function rollOverRuntime(runtime: ProviderRuntimeState, now: number): ProviderRuntimeState {
  const snapshot = runtime.snapshot;
  if (!snapshot) return runtime;
  let changed = false;
  const lines = snapshot.lines.map((line) => {
    if (line.type !== "progress") return line;
    const reset = resetTime(line);
    if (reset === null || reset > now) return line;
    changed = true;
    return rolledOver(line);
  });
  return changed ? { ...runtime, snapshot: { ...snapshot, lines } } : runtime;
}

/** `state` with every window whose reset is at or before `now` rolled over; `state` itself when none is. */
export function rollOverPassedWindows(state: EngineState, now: Date): EngineState {
  const time = now.getTime();
  let changed = false;
  const providers: EngineState["providers"] = {};
  for (const [id, runtime] of Object.entries(state.providers)) {
    const next = rollOverRuntime(runtime, time);
    if (next !== runtime) changed = true;
    providers[id] = next;
  }
  return changed ? { ...state, providers } : state;
}

/** The first window reset in `state` after `now`, or `null` when none lies ahead. */
export function nextWindowReset(state: EngineState, now: Date): Date | null {
  const time = now.getTime();
  let next = Number.POSITIVE_INFINITY;
  for (const runtime of Object.values(state.providers)) {
    for (const line of runtime.snapshot?.lines ?? []) {
      if (line.type !== "progress") continue;
      const reset = resetTime(line);
      if (reset !== null && reset > time && reset < next) next = reset;
    }
  }
  return Number.isFinite(next) ? new Date(next) : null;
}
