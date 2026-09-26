/**
 * Codex usage resets as codex-resets.com records them from @thsottiaux's posts on X (API v1:
 * `/api/v1/status` and `/api/v1/resets`), and this app's own estimate of how likely the next one is.
 *
 * The estimate is a recency-weighted Poisson rate: each past reset counts with weight 2^(−age/21 days),
 * divided by the same weight integrated over the history, so recent weeks dominate and a quiet spell
 * pulls the rate down. P(at least one reset within t days) = 1 − e^(−rate·t). It only reads the past;
 * an announced reset (`scheduled`) or the site's own watch is shown beside it, never folded into it.
 */

export type ResetKind = "regular" | "banked";

export interface ResetSource {
  /** An X post by @thsottiaux, or a reset the site saw happen without an announcement. */
  kind: "x_post" | "observed";
  /** A link to the post on X, when there is one. */
  url: string | null;
}

export interface CodexReset {
  id: string;
  kind: ResetKind;
  announcedAt: Date;
  text: string;
  source: ResetSource;
}

export interface ScheduledReset extends CodexReset {
  /** When it is due, if the announcement said. A passed time does not mean it happened. */
  scheduledFor: Date | null;
}

export interface ResetWatch {
  level: "elevated" | "strong";
  /** The site's own estimate, 0..100, when it gives one. */
  chancePercent: number | null;
  forecastWindow: string;
  observedAt: Date;
  expiresAt: Date;
  text: string;
  source: ResetSource;
}

export interface ResetStatus {
  latest: CodexReset | null;
  scheduled: ScheduledReset | null;
  watch: ResetWatch | null;
  total: number;
  generatedAt: Date | null;
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function date(value: unknown): Date | null {
  if (typeof value !== "string") return null;
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

/** Only https links to X are opened from this data. */
export function xUrl(value: unknown): string | null {
  if (typeof value !== "string") return null;
  try {
    const url = new URL(value);
    const host = url.hostname.toLowerCase();
    const known = host === "x.com" || host === "twitter.com" || host.endsWith(".x.com") || host.endsWith(".twitter.com");
    return url.protocol === "https:" && known && !url.username && !url.password ? url.toString() : null;
  } catch {
    return null;
  }
}

function source(value: unknown): ResetSource | null {
  const raw = record(value);
  if (!raw || (raw.type !== "x_post" && raw.type !== "observed")) return null;
  return { kind: raw.type, url: xUrl(raw.url) };
}

function reset(value: unknown): CodexReset | null {
  const raw = record(value);
  if (!raw) return null;
  const announcedAt = date(raw.announced_at);
  const origin = source(raw.source);
  const kind = raw.reset_type;
  if (typeof raw.id !== "string" || !raw.id || !announcedAt || !origin || (kind !== "regular" && kind !== "banked")) return null;
  return { id: raw.id, kind, announcedAt, text: typeof raw.text === "string" ? raw.text : "", source: origin };
}

function scheduled(value: unknown): ScheduledReset | null {
  const raw = record(value);
  const base = reset(value);
  if (!raw || !base || raw.status !== "scheduled") return null;
  return { ...base, scheduledFor: date(raw.scheduled_for) };
}

function watch(value: unknown): ResetWatch | null {
  const raw = record(value);
  if (!raw || (raw.level !== "elevated" && raw.level !== "strong")) return null;
  const observedAt = date(raw.observed_at);
  const expiresAt = date(raw.expires_at);
  const origin = source(raw.source);
  if (!observedAt || !expiresAt || !origin) return null;
  const chance = typeof raw.reset_chance_percent === "number" && raw.reset_chance_percent >= 0 && raw.reset_chance_percent <= 100 ? raw.reset_chance_percent : null;
  return {
    level: raw.level,
    chancePercent: chance,
    forecastWindow: typeof raw.forecast_window === "string" ? raw.forecast_window : "",
    observedAt,
    expiresAt,
    text: typeof raw.text === "string" ? raw.text : "",
    source: origin,
  };
}

function json(body: string | null | undefined): unknown {
  if (!body) return null;
  try {
    return JSON.parse(body) as unknown;
  } catch {
    return null;
  }
}

export function parseResetStatus(body: string | null | undefined): ResetStatus | null {
  const root = record(json(body));
  const data = record(root?.data);
  if (!data) return null;
  const stats = record(data.stats);
  return {
    latest: reset(data.latest_reset),
    scheduled: scheduled(data.scheduled_reset),
    watch: watch(data.active_watch),
    total: typeof stats?.total === "number" ? stats.total : 0,
    generatedAt: date(record(root?.meta)?.generated_at),
  };
}

/** Every valid reset in the list, newest first, each id once. */
export function parseResets(body: string | null | undefined, extra: readonly (CodexReset | null)[] = []): CodexReset[] {
  const data = record(json(body))?.data;
  const rows = [...(Array.isArray(data) ? data.map(reset) : []), ...extra];
  const byId = new Map<string, CodexReset>();
  for (const row of rows) if (row && !byId.has(row.id)) byId.set(row.id, row);
  return [...byId.values()].sort((a, b) => b.announcedAt.getTime() - a.announcedAt.getTime());
}

/** A watch counts until it expires. */
export function activeWatch(status: ResetStatus | null, now: Date): ResetWatch | null {
  const current = status?.watch ?? null;
  return current && current.expiresAt.getTime() > now.getTime() ? current : null;
}

const DAY_MS = 86_400_000;

export interface ResetGap {
  days: number;
  from: Date;
  to: Date;
}

export interface ResetStats {
  total: number;
  regular: number;
  banked: number;
  first: Date | null;
  last: Date | null;
  daysSinceLast: number | null;
  /** Mean days between consecutive resets. */
  averageGapDays: number | null;
  medianGapDays: number | null;
  longestGap: ResetGap | null;
  last30Days: number;
  last90Days: number;
}

export function resetStats(resets: readonly CodexReset[], now: Date): ResetStats {
  const times = resets.map((item) => item.announcedAt.getTime()).sort((a, b) => a - b);
  const gaps = times.slice(1).map((time, index) => ({ days: (time - times[index]!) / DAY_MS, from: new Date(times[index]!), to: new Date(time) }));
  const sortedGaps = gaps.map((gap) => gap.days).sort((a, b) => a - b);
  const middle = Math.floor(sortedGaps.length / 2);
  const median = sortedGaps.length === 0 ? null : sortedGaps.length % 2 === 1 ? sortedGaps[middle]! : (sortedGaps[middle - 1]! + sortedGaps[middle]!) / 2;
  const last = times.length > 0 ? times[times.length - 1]! : null;
  const within = (days: number) => times.filter((time) => time <= now.getTime() && now.getTime() - time <= days * DAY_MS).length;
  return {
    total: resets.length,
    regular: resets.filter((item) => item.kind === "regular").length,
    banked: resets.filter((item) => item.kind === "banked").length,
    first: times.length > 0 ? new Date(times[0]!) : null,
    last: last === null ? null : new Date(last),
    daysSinceLast: last === null ? null : Math.max(0, (now.getTime() - last) / DAY_MS),
    averageGapDays: gaps.length > 0 ? (times[times.length - 1]! - times[0]!) / DAY_MS / gaps.length : null,
    medianGapDays: median,
    longestGap: gaps.reduce<ResetGap | null>((longest, gap) => (!longest || gap.days > longest.days ? gap : longest), null),
    last30Days: within(30),
    last90Days: within(90),
  };
}

export const HALF_LIFE_DAYS = 21;
export const FORECAST_HORIZONS = [1, 3, 7] as const;
export type ForecastHorizon = (typeof FORECAST_HORIZONS)[number];

export interface ResetForecast {
  /** Expected resets per day right now. */
  ratePerDay: number;
  /** Chance of at least one reset within 1, 3 and 7 days, 0..1. */
  chance: Record<ForecastHorizon, number>;
  halfLifeDays: number;
  /** Resets the estimate read, and the weight the recent ones carry. */
  resets: number;
  weightedResets: number;
}

/** Needs at least two resets spanning more than a day; `null` otherwise. */
export function forecastResets(resets: readonly CodexReset[], now: Date, halfLifeDays = HALF_LIFE_DAYS): ResetForecast | null {
  const ages = resets.map((item) => (now.getTime() - item.announcedAt.getTime()) / DAY_MS).filter((age) => age >= 0);
  if (ages.length < 2) return null;
  const span = Math.max(...ages);
  if (!(span > 1)) return null;
  const decay = Math.LN2 / halfLifeDays;
  const weightedResets = ages.reduce((sum, age) => sum + Math.exp(-decay * age), 0);
  const exposure = (1 - Math.exp(-decay * span)) / decay;
  const ratePerDay = weightedResets / exposure;
  const chance = Object.fromEntries(FORECAST_HORIZONS.map((days) => [days, 1 - Math.exp(-ratePerDay * days)])) as Record<ForecastHorizon, number>;
  return { ratePerDay, chance, halfLifeDays, resets: ages.length, weightedResets };
}
