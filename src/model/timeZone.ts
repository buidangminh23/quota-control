/**
 * The time zone wall-clock times are shown in, detected on its own: the operating system's current
 * zone as the core reads it, re-checked while the app runs, so crossing into another zone (or
 * changing it in the system settings) shows up without a restart. The webview's own zone is only
 * the fallback, because a running webview can keep the zone it started with (macOS caches it per
 * process). The helpers work on explicit zones rather than the host's local getters, so the same
 * code also reads other zones, such as the one an announcement was written in.
 */

const DAY_MS = 86_400_000;
const MINUTE_MS = 60_000;

let reportedZone: string | null = null;

/** Whether the runtime knows the IANA zone name. */
export function isKnownTimeZone(zone: string): boolean {
  if (!zone) return false;
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: zone });
    return true;
  } catch {
    return false;
  }
}

/** The zone the webview itself runs in, `UTC` when it does not say. */
export function webviewTimeZone(): string {
  try {
    return new Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/** Record the zone the core read from the operating system; `null` or an unknown name falls back to the webview's. */
export function setSystemTimeZone(zone: string | null): void {
  reportedZone = zone !== null && isKnownTimeZone(zone) ? zone : null;
}

/** The zone times are shown in right now, e.g. `Asia/Saigon`. */
export function deviceTimeZone(): string {
  return reportedZone ?? webviewTimeZone();
}

export interface WallTime {
  year: number;
  /** 1 … 12 */
  month: number;
  day: number;
  hour: number;
  minute: number;
}

export interface ZonedParts extends WallTime {
  /** 0 = Sunday … 6 = Saturday, like `Date.getDay()`. */
  weekday: number;
}

const WEEKDAYS: Record<string, number> = { Sun: 0, Mon: 1, Tue: 2, Wed: 3, Thu: 4, Fri: 5, Sat: 6 };
const partFormatters = new Map<string, Intl.DateTimeFormat>();

function partsFormatter(zone: string): Intl.DateTimeFormat {
  let formatter = partFormatters.get(zone);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat("en-US", {
      timeZone: zone,
      hourCycle: "h23",
      weekday: "short",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
    });
    partFormatters.set(zone, formatter);
  }
  return formatter;
}

/** The calendar date, clock time and weekday `date` reads as in `zone`. */
export function zonedParts(date: Date, zone: string = deviceTimeZone()): ZonedParts {
  const parts = partsFormatter(zone).formatToParts(date);
  const value = (type: Intl.DateTimeFormatPartTypes) => parts.find((part) => part.type === type)?.value ?? "";
  return {
    year: Number(value("year")),
    month: Number(value("month")),
    day: Number(value("day")),
    hour: Number(value("hour")) % 24,
    minute: Number(value("minute")),
    weekday: WEEKDAYS[value("weekday")] ?? 0,
  };
}

/** Minutes `zone` is ahead of UTC at `date`: GMT+7 is 420, US Pacific daylight time −420. */
export function offsetMinutes(date: Date, zone: string = deviceTimeZone()): number {
  const minute = Math.floor(date.getTime() / MINUTE_MS) * MINUTE_MS;
  const wall = zonedParts(new Date(minute), zone);
  return Math.round((Date.UTC(wall.year, wall.month - 1, wall.day, wall.hour, wall.minute) - minute) / MINUTE_MS);
}

/** The moment a wall-clock time in `zone` names; a time a clock change skips or repeats resolves to a neighbour. */
export function instantAt(wall: WallTime, zone: string): Date {
  const naive = Date.UTC(wall.year, wall.month - 1, wall.day, wall.hour, wall.minute);
  const first = naive - offsetMinutes(new Date(naive), zone) * MINUTE_MS;
  return new Date(naive - offsetMinutes(new Date(first), zone) * MINUTE_MS);
}

/** Days since 1970-01-01 of the calendar day `date` falls on in `zone`. */
export function dayNumber(date: Date, zone: string = deviceTimeZone()): number {
  const { year, month, day } = zonedParts(date, zone);
  return Math.round(Date.UTC(year, month - 1, day) / DAY_MS);
}

/** Calendar days from `from`'s day to `to`'s day, both read in `zone` (tomorrow is 1). */
export function calendarDaysBetween(from: Date, to: Date, zone: string = deviceTimeZone()): number {
  return dayNumber(to, zone) - dayNumber(from, zone);
}

/** The given wall-clock time on the calendar day `day` (a `dayNumber`) in `zone`. */
export function instantOnDay(day: number, hour: number, minute: number, zone: string): Date {
  const date = new Date(day * DAY_MS);
  return instantAt({ year: date.getUTCFullYear(), month: date.getUTCMonth() + 1, day: date.getUTCDate(), hour, minute }, zone);
}

/** Midnight starting the calendar day `offset` days after `date`'s, in `zone`. */
export function startOfDayIn(date: Date, zone: string, offset = 0): Date {
  return instantOnDay(dayNumber(date, zone) + offset, 0, 0, zone);
}

/** `GMT+7`, `GMT-7`, `GMT+5:30`, or `GMT` for the zone's offset at `date`. */
export function offsetLabel(date: Date, zone: string = deviceTimeZone()): string {
  const minutes = offsetMinutes(date, zone);
  if (minutes === 0) return "GMT";
  const hours = Math.floor(Math.abs(minutes) / 60);
  const rest = Math.abs(minutes) % 60;
  return `GMT${minutes > 0 ? "+" : "-"}${hours}${rest ? `:${String(rest).padStart(2, "0")}` : ""}`;
}

/** The zone's everyday name in `locale` at `date`, e.g. `Giờ Đông Dương`; the IANA name when the runtime has none. */
export function zoneName(date: Date, zone: string, locale: string): string {
  try {
    const parts = new Intl.DateTimeFormat(locale, { timeZone: zone, timeZoneName: "long" }).formatToParts(date);
    return parts.find((part) => part.type === "timeZoneName")?.value ?? zone;
  } catch {
    return zone;
  }
}
