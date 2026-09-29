/**
 * Claude usage resets as claude-resets.com records them from @ClaudeDevs and Claude Code team
 * members on X. The core keeps the Claude side of the site's live catalog (`claude_catalog` in
 * `public_feeds.rs`): resets, limit changes (a ceiling moved, nothing was flushed, so they never
 * count as resets), who each one covered, a banked reset's deadline and whether the site has
 * reviewed the entry yet.
 *
 * The site records announcements and does not predict the next one, so a reset never has a
 * schedule or a watch here. What this file adds on top of the history is the app's own: which
 * resets covered the plans of the accounts connected here, which banked resets can still be
 * applied, how Claude's history compares with Codex's over the time both were tracked, and how
 * well the chance estimate would have done on the history it is drawn from.
 */
import { dayNumber, deviceTimeZone, zonedParts } from "@/model/timeZone";
import { FORECAST_HORIZONS, forecastResets, xUrl, type CodexReset, type ForecastHorizon } from "./resets";

/** The account the tracker follows; a post by anyone else names its own account. */
export const CLAUDE_ACCOUNT = "ClaudeDevs";

const DAY_MS = 86_400_000;

export interface ClaudeReset extends CodexReset {
  /** The X account that posted it, without `@`. */
  account: string;
  /** Who it covered, in the site's words (`all`, `Pro + Max`…); `null` when it does not say. */
  scope: string | null;
  /** A banked reset's deadline, when the announcement set one. */
  usableUntil: Date | null;
  /** The site detected it and has not reviewed it yet; it may still be withdrawn. */
  provisional: boolean;
}

/** A limit change: the ceiling moved, usage was not flushed. */
export interface LimitChange {
  id: string;
  announcedAt: Date;
  text: string;
  account: string;
  scope: string | null;
  url: string | null;
  provisional: boolean;
}

export interface ClaudeResetFeed {
  /** Newest first. */
  resets: ClaudeReset[];
  /** Newest first. */
  changes: LimitChange[];
  /** The live catalog, or the published dataset read while the live one was out of reach. */
  live: boolean;
  /** Whether the site's own detector is up to date (`fresh`), as it reports; `null` when unknown. */
  detector: string | null;
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function date(value: unknown): Date | null {
  if (typeof value !== "string") return null;
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

/** An X handle as the site writes it: letters, digits and `_`, at most 15. */
function handle(value: unknown): string | null {
  const name = text(value)?.replace(/^@/, "") ?? null;
  return name && /^[A-Za-z0-9_]{1,15}$/.test(name) ? name : null;
}

function ids(value: unknown): Set<string> {
  return new Set(Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : []);
}

export function parseClaudeResets(body: string | null | undefined): ClaudeResetFeed | null {
  if (!body) return null;
  let root: Record<string, unknown> | null;
  try {
    root = record(JSON.parse(body));
  } catch {
    return null;
  }
  if (!root || !Array.isArray(root.events)) return null;
  const main = handle(root.account) ?? CLAUDE_ACCOUNT;
  const awaiting = new Set([...ids(root.provisionalEventIds), ...ids(root.provisionalPolicyIds)]);
  const resets = new Map<string, ClaudeReset>();
  const changes = new Map<string, LimitChange>();
  for (const item of root.events) {
    const raw = record(item);
    const id = text(raw?.id);
    const announcedAt = date(raw?.date);
    if (!raw || !id || !announcedAt) continue;
    const shared = {
      id,
      announcedAt,
      text: text(raw.note) ?? "",
      account: handle(raw.account) ?? main,
      scope: text(raw.scope),
      provisional: raw.verification === "provisional" || awaiting.has(id),
    };
    const url = xUrl(raw.url);
    if (raw.kind === "reset" && !resets.has(id)) {
      const kind = raw.resetType === "banked" ? "banked" : "regular";
      resets.set(id, { ...shared, kind, source: { kind: "x_post", url }, usableUntil: kind === "banked" ? date(raw.usableUntil) : null });
    } else if (raw.kind === "policy" && !changes.has(id)) {
      changes.set(id, { ...shared, url });
    }
  }
  const newestFirst = <T extends { announcedAt: Date }>(rows: Iterable<T>) => [...rows].sort((a, b) => b.announcedAt.getTime() - a.announcedAt.getTime());
  return { resets: newestFirst(resets.values()), changes: newestFirst(changes.values()), live: root.live === true, detector: text(root.detector) };
}

/** The site's detector says it is behind; an unknown state is not reported as a problem. */
export function detectorBehind(feed: ClaudeResetFeed | null): boolean {
  return Boolean(feed?.live && feed.detector !== null && feed.detector !== "fresh");
}

export const CLAUDE_PLANS = ["free", "pro", "max", "team", "enterprise"] as const;
export type ClaudePlan = (typeof CLAUDE_PLANS)[number];
const PAID_PLANS: readonly ClaudePlan[] = ["pro", "max", "team", "enterprise"];

export type ScopeReading =
  /** Every user. */
  | { reach: "everyone" }
  /** The named plans. */
  | { reach: "plans"; plans: ClaudePlan[] }
  /** Only the users an incident touched; no plan decides it. */
  | { reach: "affected" }
  /** The site does not say, or says it in words this app does not know. */
  | { reach: "unknown" };

/** Who an announcement covered, read from the site's scope words. */
export function readScope(scope: string | null): ScopeReading {
  const words = scope?.toLowerCase() ?? "";
  if (!words) return { reach: "unknown" };
  if (/^(all|everyone|all users|all plans)$/.test(words)) return { reach: "everyone" };
  if (/\baffected\b/.test(words)) return { reach: "affected" };
  if (/\bpaid\b/.test(words)) return { reach: "plans", plans: [...PAID_PLANS] };
  const plans = CLAUDE_PLANS.filter((plan) => new RegExp(`\\b${plan}\\b`).test(words));
  return plans.length > 0 ? { reach: "plans", plans } : { reach: "unknown" };
}

/** The plan family of an account's plan name (`Max 20x` is `max`); `null` for a name not known. */
export function planFamily(plan: string | null | undefined): ClaudePlan | null {
  const words = plan?.toLowerCase() ?? "";
  return CLAUDE_PLANS.find((family) => new RegExp(`\\b${family}\\b`).test(words)) ?? null;
}

/** Whether the announcement covered the plan: `null` when the scope cannot settle it. */
export function covers(scope: string | null, plan: ClaudePlan): boolean | null {
  const reading = readScope(scope);
  if (reading.reach === "everyone") return true;
  if (reading.reach === "plans") return reading.plans.includes(plan);
  return null;
}

/** The newest reset that covered the plan for certain. */
export function latestFor(resets: readonly ClaudeReset[], plan: ClaudePlan): ClaudeReset | null {
  return resets.find((reset) => covers(reset.scope, plan) === true) ?? null;
}

/** The newest reset that covered every user. */
export function latestForEveryone(resets: readonly ClaudeReset[]): ClaudeReset | null {
  return resets.find((reset) => readScope(reset.scope).reach === "everyone") ?? null;
}

/**
 * Banked resets that can still be applied, soonest deadline first. This is what was announced:
 * whether an account has already applied its own is not something the site or this app can see.
 */
export function openBanked(resets: readonly ClaudeReset[], now: Date): ClaudeReset[] {
  return resets
    .filter((reset) => reset.kind === "banked" && reset.usableUntil !== null && reset.usableUntil.getTime() > now.getTime() && reset.announcedAt.getTime() <= now.getTime())
    .sort((a, b) => a.usableUntil!.getTime() - b.usableUntil!.getTime());
}

export interface TrackerSide {
  resets: number;
  banked: number;
  averageGapDays: number | null;
  medianGapDays: number | null;
  longestGapDays: number | null;
  daysSinceLast: number | null;
  last30Days: number;
}

export interface TrackerMonth {
  year: number;
  /** 0 is January. */
  month: number;
  claude: number;
  codex: number;
}

export interface TrackerComparison {
  /** Where the shared window starts: the later of the two first resets. */
  from: Date;
  claude: TrackerSide;
  codex: TrackerSide;
  /** Every month of the window, oldest first, in the device's zone. */
  months: TrackerMonth[];
}

function side(resets: readonly CodexReset[], from: Date, now: Date): TrackerSide {
  const rows = resets.filter((reset) => reset.announcedAt.getTime() >= from.getTime() && reset.announcedAt.getTime() <= now.getTime());
  const times = rows.map((reset) => reset.announcedAt.getTime()).sort((a, b) => a - b);
  const gaps = times
    .slice(1)
    .map((time, index) => (time - times[index]!) / DAY_MS)
    .sort((a, b) => a - b);
  const middle = Math.floor(gaps.length / 2);
  const last = times[times.length - 1];
  return {
    resets: rows.length,
    banked: rows.filter((reset) => reset.kind === "banked").length,
    averageGapDays: gaps.length > 0 ? (times[times.length - 1]! - times[0]!) / DAY_MS / gaps.length : null,
    medianGapDays: gaps.length === 0 ? null : gaps.length % 2 === 1 ? gaps[middle]! : (gaps[middle - 1]! + gaps[middle]!) / 2,
    longestGapDays: gaps.length > 0 ? gaps[gaps.length - 1]! : null,
    daysSinceLast: last === undefined ? null : (now.getTime() - last) / DAY_MS,
    last30Days: times.filter((time) => now.getTime() - time <= 30 * DAY_MS).length,
  };
}

/** Both histories over the time both were tracked; `null` until each has a reset. */
export function compareTrackers(claude: readonly CodexReset[], codex: readonly CodexReset[], now: Date, zone: string = deviceTimeZone()): TrackerComparison | null {
  const first = (resets: readonly CodexReset[]) => resets.reduce<number | null>((earliest, reset) => (earliest === null || reset.announcedAt.getTime() < earliest ? reset.announcedAt.getTime() : earliest), null);
  const claudeStart = first(claude);
  const codexStart = first(codex);
  if (claudeStart === null || codexStart === null) return null;
  const from = new Date(Math.max(claudeStart, codexStart));
  if (from.getTime() > now.getTime()) return null;
  const monthOf = (moment: Date) => {
    const parts = zonedParts(moment, zone);
    return parts.year * 12 + parts.month - 1;
  };
  const count = (resets: readonly CodexReset[]) => {
    const counts = new Map<number, number>();
    for (const reset of resets) {
      if (reset.announcedAt.getTime() < from.getTime() || reset.announcedAt.getTime() > now.getTime()) continue;
      const key = monthOf(reset.announcedAt);
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    return counts;
  };
  const claudeMonths = count(claude);
  const codexMonths = count(codex);
  const months: TrackerMonth[] = [];
  for (let key = monthOf(from); key <= monthOf(now); key += 1) {
    months.push({ year: Math.floor(key / 12), month: key % 12, claude: claudeMonths.get(key) ?? 0, codex: codexMonths.get(key) ?? 0 });
  }
  return { from, claude: side(claude, from, now), codex: side(codex, from, now), months };
}

/** The days a forecast is tried on need this much history behind them. */
const SKILL_WARM_UP_DAYS = 21;
const SKILL_MIN_RESETS = 3;
/** Fewer tried days than this say nothing either way. */
const SKILL_MIN_DAYS = 60;
/** How much better than the plain average the estimate has to be to be called better. */
export const SKILL_MARGIN = 0.03;

export interface ForecastSkill {
  /** Days in the past the estimate was tried on. */
  days: number;
  /**
   * 1 − (the estimate's Brier score ÷ the plain average's), averaged over the three horizons:
   * above zero the recency-weighted estimate did better than the history's plain average rate.
   */
  skill: number;
  verdict: "better" | "same" | "worse";
  byHorizon: Record<ForecastHorizon, number>;
}

/**
 * How the chance estimate would have done on this same history. Each past day, the estimate as it
 * stood then (`forecastResets`) and the plain average rate until then (resets ÷ days) each give a
 * chance of a reset within 1, 3 and 7 days, scored against what happened (Brier). `null` while the
 * history is too short to try.
 */
export function forecastSkill(resets: readonly CodexReset[], now: Date): ForecastSkill | null {
  const times = resets
    .map((reset) => reset.announcedAt.getTime())
    .filter((time) => time <= now.getTime())
    .sort((a, b) => a - b);
  if (times.length < SKILL_MIN_RESETS + 1) return null;
  const start = times[0]! + SKILL_WARM_UP_DAYS * DAY_MS;
  const sorted = [...resets].sort((a, b) => a.announcedAt.getTime() - b.announcedAt.getTime());
  const byHorizon = {} as Record<ForecastHorizon, number>;
  let days = 0;
  for (const horizon of FORECAST_HORIZONS) {
    let weighted = 0;
    let plain = 0;
    let tried = 0;
    for (let day = start; day + horizon * DAY_MS <= now.getTime(); day += DAY_MS) {
      const past = times.filter((time) => time <= day);
      if (past.length < SKILL_MIN_RESETS) continue;
      const at = new Date(day);
      const estimate = forecastResets(sorted, at);
      if (!estimate) continue;
      const flatRate = (past.length - 1) / ((day - past[0]!) / DAY_MS);
      const flat = 1 - Math.exp(-flatRate * horizon);
      const happened = times.some((time) => time > day && time <= day + horizon * DAY_MS) ? 1 : 0;
      weighted += (estimate.chance[horizon] - happened) ** 2;
      plain += (flat - happened) ** 2;
      tried += 1;
    }
    if (tried < SKILL_MIN_DAYS || plain === 0) return null;
    byHorizon[horizon] = 1 - weighted / plain;
    days = Math.max(days, tried);
  }
  const skill = FORECAST_HORIZONS.reduce((sum, horizon) => sum + byHorizon[horizon], 0) / FORECAST_HORIZONS.length;
  return { days, skill, verdict: skill > SKILL_MARGIN ? "better" : skill < -SKILL_MARGIN ? "worse" : "same", byHorizon };
}

/** Whole days left until `until`, counted on the device's calendar. */
export function calendarDaysLeft(until: Date, now: Date, zone: string = deviceTimeZone()): number {
  return dayNumber(until, zone) - dayNumber(now, zone);
}
