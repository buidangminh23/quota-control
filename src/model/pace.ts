/**
 * Burn-rate pacing for a bounded metric. Port of upstream `Support/Pace.swift`: given how much of a
 * quota is spent and how far through its reset window we are, project usage to the end of the window.
 */

export type PaceStatus = "ahead" | "onTrack" | "behind";

export interface PaceResult {
  status: PaceStatus;
  /** Projected end-of-period usage, in the same unit as `used` and `limit`. */
  projectedUsage: number;
}

/** Minimum time into the window (seconds) before a projection is meaningful. */
export function minimumElapsed(periodSeconds: number): number {
  return Math.max(60, periodSeconds * 0.01);
}

/**
 * Full pace evaluation, or `null` when there is no signal: window not started, already reset, too
 * early for a stable projection, or nothing spent yet.
 */
export function evaluatePace(
  used: number,
  limit: number,
  resetsAt: Date,
  periodSeconds: number,
  now: Date,
): PaceResult | null {
  if (!(limit > 0) || !(periodSeconds > 0) || !(used > 0)) return null;
  const elapsed = (now.getTime() - (resetsAt.getTime() - periodSeconds * 1000)) / 1000;
  if (elapsed < minimumElapsed(periodSeconds) || now.getTime() >= resetsAt.getTime()) return null;
  const projected = (used / elapsed) * periodSeconds;
  if (used >= limit) return { status: "behind", projectedUsage: projected };
  const status: PaceStatus = projected <= limit * 0.9 ? "ahead" : projected <= limit ? "onTrack" : "behind";
  return { status, projectedUsage: projected };
}

/** Projected seconds until the quota runs out, only when `behind` and the run-out lands before the reset. */
export function secondsToRunOut(
  used: number,
  limit: number,
  resetsAt: Date,
  periodSeconds: number,
  now: Date,
): number | null {
  const result = evaluatePace(used, limit, resetsAt, periodSeconds, now);
  if (!result || result.status !== "behind") return null;
  const rate = result.projectedUsage / periodSeconds;
  if (!(rate > 0)) return null;
  const eta = (limit - used) / rate;
  const remaining = (resetsAt.getTime() - now.getTime()) / 1000;
  if (!(eta > 0) || !(eta < remaining)) return null;
  return eta;
}
