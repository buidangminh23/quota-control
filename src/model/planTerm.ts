/**
 * The plan's paid period for the card header. ChatGPT states when the period ends. For Claude only
 * the subscription start is known, so the next renewal is estimated on that day of each month (the
 * month's last day when the month is shorter), counted in UTC like a billing cycle anchor.
 */
import type { PlanTerm } from "@/lib/types";

const MINUTE_MS = 60_000;
const HOUR_MS = 3_600_000;
const DAY_MS = 86_400_000;
/** The header turns to the warning color once this many whole days or fewer are left. */
export const PLAN_TERM_SOON_DAYS = 3;

export interface PlanTermEnd {
  endsAt: Date;
  /** Worked out from the subscription start rather than stated by the provider. */
  estimated: boolean;
  /** When the provider last confirmed a stated date. */
  checkedAt: Date | null;
  /** The subscription start an estimate counts from. */
  startedAt: Date | null;
}

/** Time left in its largest whole unit; `due` once the stated end has passed. */
export type PlanTermLeft = { kind: "days" | "hours" | "minutes"; count: number } | { kind: "due" };

function dateOf(value: string | undefined): Date | null {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

/** `start` moved on by `months` months, on the same day of the month or the month's last day, in UTC. */
export function addMonthsClamped(start: Date, months: number): Date {
  const year = start.getUTCFullYear();
  const month = start.getUTCMonth() + months;
  const lastDay = new Date(Date.UTC(year, month + 1, 0)).getUTCDate();
  return new Date(
    Date.UTC(year, month, Math.min(start.getUTCDate(), lastDay), start.getUTCHours(), start.getUTCMinutes(), start.getUTCSeconds(), start.getUTCMilliseconds()),
  );
}

/** The first monthly renewal after `now` for a subscription that began at `start`. */
export function nextMonthlyRenewal(start: Date, now: Date): Date {
  let months = Math.max(1, (now.getUTCFullYear() - start.getUTCFullYear()) * 12 + now.getUTCMonth() - start.getUTCMonth());
  let renewal = addMonthsClamped(start, months);
  while (renewal.getTime() <= now.getTime()) renewal = addMonthsClamped(start, ++months);
  return renewal;
}

/** When the plan's current paid period ends, or `null` when the term cannot be read. */
export function planTermEnd(term: PlanTerm, now: Date): PlanTermEnd | null {
  if (term.basis === "stated") {
    const endsAt = dateOf(term.endsAt);
    return endsAt ? { endsAt, estimated: false, checkedAt: dateOf(term.checkedAt), startedAt: null } : null;
  }
  const startedAt = dateOf(term.startedAt);
  return startedAt ? { endsAt: nextMonthlyRenewal(startedAt, now), estimated: true, checkedAt: null, startedAt } : null;
}

export function planTermLeft(endsAt: Date, now: Date): PlanTermLeft {
  const ms = endsAt.getTime() - now.getTime();
  if (ms <= 0) return { kind: "due" };
  if (ms < HOUR_MS) return { kind: "minutes", count: Math.ceil(ms / MINUTE_MS) };
  if (ms < DAY_MS) return { kind: "hours", count: Math.floor(ms / HOUR_MS) };
  return { kind: "days", count: Math.floor(ms / DAY_MS) };
}

/** True once the whole days left, as the header shows them, reach `PLAN_TERM_SOON_DAYS`. */
export function isPlanTermSoon(left: PlanTermLeft): boolean {
  return left.kind !== "days" || left.count <= PLAN_TERM_SOON_DAYS;
}
