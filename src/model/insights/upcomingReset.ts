/**
 * The next free Codex reset, when codex-resets.com knows one is coming: an announced reset
 * (`scheduled_reset`) or, failing that, the site's watch (signs of one, until a deadline).
 *
 * An announcement with a time (`scheduled_for`) counts down to that time. Without one, the post's
 * own words set it, read the way the poster meant them: in their time zone (@thsottiaux posts from
 * San Francisco). "In 2 hours" and "at 10am PT" are exact. "Tomorrow" or a weekday is an estimate:
 * the post's clock time that many days later, so "tomorrow" counts down 24 hours, kept inside that
 * day there. "Today" and "tonight" run to the end of that day. A longer window ("next week", "this
 * weekend") or a post without timing words gets no countdown. The screen shows every time in the
 * device's own zone.
 */
import { dayNumber, instantOnDay, startOfDayIn, zonedParts } from "@/model/timeZone";
import { activeWatch, type ResetKind, type ResetSource, type ResetStatus } from "./resets";

/** Where @thsottiaux posts from, so the zone their "today" and "tomorrow" mean. */
export const ANNOUNCER_TIME_ZONE = "America/Los_Angeles";

const MINUTE_MS = 60_000;
const HOUR_MS = 3_600_000;
const DAY_MS = 86_400_000;
/** How long a countdown that reached its time keeps showing "waiting for confirmation". */
export const AWAITING_MS = 24 * HOUR_MS;
/** How long an announcement without timing words stays on screen. */
export const UNTIMED_LIFETIME_MS = 7 * DAY_MS;

export type NamedDay = { kind: "today" } | { kind: "tonight" } | { kind: "tomorrow" } | { kind: "weekday"; weekday: number };
export type ResetWindow = "thisWeek" | "weekend" | "nextWeek";

export type ResetTiming =
  /** A stated time: the site's `scheduled_for`, or a clock time or "in N hours" in the post. */
  | { kind: "exact"; at: Date; from: "site" | "post" }
  /** A named day without a time: an estimate inside that day. */
  | { kind: "day"; at: Date; day: NamedDay }
  /** The watch's deadline: the site expects a reset by then. */
  | { kind: "by"; at: Date }
  /** A span longer than a day; it counts until `ends`. */
  | { kind: "window"; window: ResetWindow; ends: Date }
  | { kind: "unknown" };

export interface UpcomingReset {
  origin: "scheduled" | "watch";
  /** The announced kind; `null` for a watch. */
  kind: ResetKind | null;
  timing: ResetTiming;
  /** The watch's own chance, 0..100, when it gives one. */
  chancePercent: number | null;
  /** When the post was made or the watch began. */
  announcedAt: Date;
  text: string;
  source: ResetSource;
}

const TIME_ZONES: Record<string, string> = {
  pt: ANNOUNCER_TIME_ZONE,
  pst: ANNOUNCER_TIME_ZONE,
  pdt: ANNOUNCER_TIME_ZONE,
  pacific: ANNOUNCER_TIME_ZONE,
  et: "America/New_York",
  est: "America/New_York",
  edt: "America/New_York",
  eastern: "America/New_York",
  ct: "America/Chicago",
  cst: "America/Chicago",
  cdt: "America/Chicago",
  central: "America/Chicago",
  mt: "America/Denver",
  mst: "America/Denver",
  mdt: "America/Denver",
  utc: "UTC",
  gmt: "UTC",
};

const NUMBER_WORDS: Record<string, number> = {
  a: 1,
  an: 1,
  one: 1,
  two: 2,
  three: 3,
  four: 4,
  five: 5,
  six: 6,
  seven: 7,
  eight: 8,
  nine: 9,
  ten: 10,
  eleven: 11,
  twelve: 12,
};

const WEEKDAY_NAMES = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"] as const;
const RELATIVE = /\b(?:in|within)\s+(?:the\s+next\s+)?(?:about\s+|around\s+|roughly\s+|~\s*)?(half\s+an|a\s+couple\s+(?:of\s+)?|an?|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|\d+(?:\.\d+)?)\s*(minutes?|mins?|m|hours?|hrs?|h)\b/;
const CLOCK = /(^|[^\w:.$%-])(\d{1,2})(?::([0-5]\d))?\s*(a\.?m\.?|p\.?m\.?)?\s*\b(pt|pst|pdt|pacific|et|est|edt|eastern|ct|cst|cdt|central|mt|mst|mdt|utc|gmt)?\b/g;

function normalized(text: string): string {
  return text
    .replace(/https?:\/\/\S+/g, " ")
    .replace(/[’‘]/g, "'")
    .toLowerCase()
    .replace(/\s+/g, " ");
}

function relativeTime(text: string, announcedAt: Date): Date | null {
  if (/\bwithin the (?:next )?hour\b/.test(text)) return new Date(announcedAt.getTime() + HOUR_MS);
  const match = RELATIVE.exec(text);
  if (!match) return null;
  const amountWord = match[1]!.replace(/\s+/g, " ").trim();
  const amount = amountWord === "half an" ? 0.5 : amountWord.startsWith("a couple") ? 2 : (NUMBER_WORDS[amountWord] ?? Number(amountWord));
  if (!(amount > 0)) return null;
  const unit = match[2]!.startsWith("h") ? HOUR_MS : MINUTE_MS;
  return new Date(announcedAt.getTime() + amount * unit);
}

interface ClockTime {
  hour: number;
  minute: number;
  zone: string;
}

/** The first clock time that is clearly a time: it has am/pm, a zone or minutes, or is "noon". */
function clockTime(text: string): ClockTime | null {
  for (const match of text.matchAll(CLOCK)) {
    const [, , hourText, minuteText, meridiem, zoneWord] = match;
    if (!meridiem && !zoneWord && minuteText === undefined) continue;
    let hour = Number(hourText);
    const minute = minuteText === undefined ? 0 : Number(minuteText);
    if (meridiem) {
      if (hour < 1 || hour > 12) continue;
      hour = (hour % 12) + (meridiem.startsWith("p") ? 12 : 0);
    }
    if (hour > 23) continue;
    return { hour, minute, zone: zoneWord ? TIME_ZONES[zoneWord]! : ANNOUNCER_TIME_ZONE };
  }
  const noon = /\bnoon\b\s*\b(pt|pst|pdt|pacific|et|est|edt|eastern|ct|cst|cdt|central|mt|mst|mdt|utc|gmt)?\b/.exec(text);
  return noon ? { hour: 12, minute: 0, zone: noon[1] ? TIME_ZONES[noon[1]]! : ANNOUNCER_TIME_ZONE } : null;
}

interface DayMention {
  day: NamedDay;
  /** "next Monday": a week ahead when it is Monday already. */
  next: boolean;
}

/** The day a post names; "tomorrow" wins over a weekday, which wins over "tonight" and "today". */
function namedDay(text: string): DayMention | null {
  if (/\b(?:tomorrow|tmrw|tmr)\b/.test(text)) return { day: { kind: "tomorrow" }, next: false };
  const weekday = /\b(next\s+)?(sunday|monday|tuesday|wednesday|thursday|friday|saturday)\b/.exec(text);
  if (weekday) return { day: { kind: "weekday", weekday: WEEKDAY_NAMES.indexOf(weekday[2] as (typeof WEEKDAY_NAMES)[number]) }, next: Boolean(weekday[1]) };
  if (/\b(?:tonight|this evening)\b/.test(text)) return { day: { kind: "tonight" }, next: false };
  if (/\b(?:today|this afternoon|this morning)\b/.test(text)) return { day: { kind: "today" }, next: false };
  return null;
}

/** Days from the post's day to the named day, both in `zone`. */
function dayOffset(mention: DayMention, announcedAt: Date, zone: string): number {
  const day = mention.day;
  if (day.kind === "tomorrow") return 1;
  if (day.kind !== "weekday") return 0;
  const ahead = (day.weekday - zonedParts(announcedAt, zone).weekday + 7) % 7;
  return ahead === 0 && mention.next ? 7 : ahead;
}

function atClock(clock: ClockTime, mention: DayMention | null, announcedAt: Date): Date {
  const today = dayNumber(announcedAt, clock.zone);
  if (mention) return instantOnDay(today + dayOffset(mention, announcedAt, clock.zone), clock.hour, clock.minute, clock.zone);
  const sameDay = instantOnDay(today, clock.hour, clock.minute, clock.zone);
  return sameDay.getTime() > announcedAt.getTime() ? sameDay : instantOnDay(today + 1, clock.hour, clock.minute, clock.zone);
}

/** The post's clock time `offset` days later, kept inside that day in `zone`; the end of the day for today. */
function dayEstimate(mention: DayMention, announcedAt: Date, zone: string): Date {
  const offset = dayOffset(mention, announcedAt, zone);
  const start = startOfDayIn(announcedAt, zone, offset);
  const end = startOfDayIn(announcedAt, zone, offset + 1);
  if (offset === 0) return end;
  const estimate = announcedAt.getTime() + offset * DAY_MS;
  return new Date(Math.min(Math.max(estimate, start.getTime()), end.getTime() - MINUTE_MS));
}

/** Midnight starting the Monday `weeks` after the week of `date`, in `zone`. */
function mondayAfter(date: Date, zone: string, weeks: number): Date {
  const weekday = zonedParts(date, zone).weekday;
  const toMonday = (8 - weekday) % 7 || 7;
  return startOfDayIn(date, zone, toMonday + (weeks - 1) * 7);
}

function windowOf(text: string, announcedAt: Date, zone: string): ResetTiming | null {
  if (/\bnext week\b/.test(text)) return { kind: "window", window: "nextWeek", ends: mondayAfter(announcedAt, zone, 2) };
  if (/\b(?:this|the) weekend\b/.test(text)) return { kind: "window", window: "weekend", ends: mondayAfter(announcedAt, zone, 1) };
  if (/\bthis week\b/.test(text)) return { kind: "window", window: "thisWeek", ends: mondayAfter(announcedAt, zone, 1) };
  return null;
}

/** When a post without `scheduled_for` says the reset comes, read in the poster's zone. */
export function timingFromText(text: string, announcedAt: Date, zone: string = ANNOUNCER_TIME_ZONE): ResetTiming {
  const words = normalized(text);
  const relative = relativeTime(words, announcedAt);
  if (relative) return { kind: "exact", at: relative, from: "post" };
  const mention = namedDay(words);
  const clock = clockTime(words);
  if (clock) return { kind: "exact", at: atClock(clock, mention, announcedAt), from: "post" };
  if (mention) return { kind: "day", at: dayEstimate(mention, announcedAt, zone), day: mention.day };
  return windowOf(words, announcedAt, zone) ?? { kind: "unknown" };
}

/** The moment a countdown points at, if it has one. */
export function timingMoment(timing: ResetTiming): Date | null {
  return timing.kind === "exact" || timing.kind === "day" || timing.kind === "by" ? timing.at : null;
}

function stillShown(timing: ResetTiming, announcedAt: Date, now: Date): boolean {
  switch (timing.kind) {
    case "exact":
    case "day":
      return now.getTime() < timing.at.getTime() + AWAITING_MS;
    case "by":
      return now.getTime() < timing.at.getTime();
    case "window":
      return now.getTime() < timing.ends.getTime();
    case "unknown":
      return now.getTime() - announcedAt.getTime() < UNTIMED_LIFETIME_MS;
  }
}

/** The free reset to count down to now: the announced one first, else the site's watch; `null` when neither is current. */
export function upcomingReset(status: ResetStatus | null, now: Date): UpcomingReset | null {
  const scheduled = status?.scheduled ?? null;
  if (scheduled) {
    const timing: ResetTiming = scheduled.scheduledFor ? { kind: "exact", at: scheduled.scheduledFor, from: "site" } : timingFromText(scheduled.text, scheduled.announcedAt);
    if (stillShown(timing, scheduled.announcedAt, now)) {
      return { origin: "scheduled", kind: scheduled.kind, timing, chancePercent: null, announcedAt: scheduled.announcedAt, text: scheduled.text, source: scheduled.source };
    }
  }
  const watch = activeWatch(status, now);
  if (!watch) return null;
  return {
    origin: "watch",
    kind: null,
    timing: { kind: "by", at: watch.expiresAt },
    chancePercent: watch.chancePercent,
    announcedAt: watch.observedAt,
    text: watch.text,
    source: watch.source,
  };
}
