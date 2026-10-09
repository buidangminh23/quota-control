import type { EngineState, ProgressLine, ProviderRuntimeState } from "@/lib/types";
import { hasDashboardCard } from "./layout";
import type { WidgetData } from "./widgetData";

function resetTime(line: ProgressLine): number | null {
  if (!line.resetsAt) return null;
  const time = Date.parse(line.resetsAt);
  return Number.isFinite(time) ? time : null;
}

function readingLifetime(state: EngineState): number {
  return Number.isFinite(state.refreshIntervalMs) && state.refreshIntervalMs > 0 ? state.refreshIntervalMs * 2 : 60_000;
}

export function rolledOverReading(data: WidgetData): WidgetData {
  return data.hasData ? { ...data, hasData: false } : data;
}

function validateRuntime(runtime: ProviderRuntimeState, now: number, lifetime: number): ProviderRuntimeState {
  const snapshot = runtime.snapshot;
  if (!snapshot) return runtime;
  const reading = Date.parse(snapshot.refreshedAt);
  const unavailable = snapshot.usageUnavailable === true
    || runtime.error !== undefined
    || snapshot.errorCategory !== undefined
    || !Number.isFinite(reading)
    || reading > now
    || now - reading >= lifetime
    || snapshot.lines.some((line) => line.type === "progress" && (resetTime(line) ?? Number.POSITIVE_INFINITY) <= now);
  return unavailable && !snapshot.usageUnavailable ? { ...runtime, snapshot: { ...snapshot, usageUnavailable: true } } : runtime;
}

export function rollOverPassedWindows(state: EngineState, now: Date): EngineState {
  const time = now.getTime();
  const lifetime = readingLifetime(state);
  let changed = false;
  const providers: EngineState["providers"] = {};
  for (const [id, runtime] of Object.entries(state.providers)) {
    const next = hasDashboardCard(id) ? validateRuntime(runtime, time, lifetime) : runtime;
    if (next !== runtime) changed = true;
    providers[id] = next;
  }
  return changed ? { ...state, providers } : state;
}

export function nextWindowReset(state: EngineState, now: Date): Date | null {
  const time = now.getTime();
  let next = Number.POSITIVE_INFINITY;
  for (const [id, runtime] of Object.entries(state.providers)) {
    if (!hasDashboardCard(id)) continue;
    for (const line of runtime.snapshot?.lines ?? []) {
      if (line.type !== "progress") continue;
      const reset = resetTime(line);
      if (reset !== null && reset > time && reset < next) next = reset;
    }
  }
  return Number.isFinite(next) ? new Date(next) : null;
}

export function nextUsageDeadline(state: EngineState, now: Date): Date | null {
  const time = now.getTime();
  let next = nextWindowReset(state, now)?.getTime() ?? Number.POSITIVE_INFINITY;
  for (const [id, runtime] of Object.entries(state.providers)) {
    if (!hasDashboardCard(id) || !runtime.snapshot) continue;
    const refreshed = Date.parse(runtime.snapshot.refreshedAt);
    const deadline = refreshed + readingLifetime(state);
    if (Number.isFinite(deadline) && deadline > time && deadline < next) next = deadline;
    if (Number.isFinite(refreshed) && refreshed > time && refreshed < next) next = refreshed;
  }
  return Number.isFinite(next) ? new Date(next) : null;
}
