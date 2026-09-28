/**
 * The Codex free-reset tracker as the macOS Dynamic Island and desktop widgets draw it: the Reset
 * tab's announced reset, latest reset, chances, wait, calendar and rhythm, worked out and worded
 * here from the same feeds and the same helpers the tab uses. Everything that moves with the clock
 * travels as a moment plus words with `{d}` (a `GlanceCountdown`) and every chance as a whole
 * percent, so the document only changes when the tracker's numbers do.
 */
import { PROVIDER_MARKS } from "@/assets/providerMarks";
import { messagesFor, type Language } from "@/i18n";
import { insightsFor, type InsightsMessages } from "@/i18n/insights";
import { shortTime, timeOnDayLabel, type TimeFormat } from "./format";
import type { GlanceCountdown, GlanceResetCalendar, GlanceResetRhythm, GlanceResets, GlanceUpcomingReset } from "./glance";
import {
  announcementPattern,
  currentWait,
  FORECAST_HORIZONS,
  forecastResets,
  HOUR_BLOCKS,
  parseResets,
  parseResetStatus,
  resetCalendar,
  type CodexReset,
  type ResetStatus,
} from "./insights/resets";
import { numberText, percentText, shortDate } from "./insights/text";
import { AWAITING_MS, timingMoment, UNTIMED_LIFETIME_MS, upcomingReset, type UpcomingReset } from "./insights/upcomingReset";
import { SOURCE_COLORS } from "./palette";
import { deviceTimeZone, offsetLabel, zonedParts } from "./timeZone";

/** The placeholder Swift fills with the moving span of time. */
export const COUNTDOWN_SPAN = "{d}";
const BRAND = "codex";

/** The two tracker feeds read once, the way the Reset tab reads them. */
export interface ResetFeeds {
  status: ResetStatus | null;
  /** Every reset on record, newest first, the status's latest folded in. */
  resets: CodexReset[];
}

export function parseResetFeeds(statusBody: string | null | undefined, historyBody: string | null | undefined): ResetFeeds {
  const status = parseResetStatus(statusBody);
  return { status, resets: parseResets(historyBody, [status?.latest ?? null]) };
}

export interface GlanceResetsInput {
  feeds: ResetFeeds;
  /** A feed could not be refreshed and its last good copy is shown. */
  stale: boolean;
  now: Date;
  language: Language;
  timeFormat: TimeFormat;
}

/** The tracker for the glance document; `null` while there is neither a status nor any history. */
export function buildGlanceResets(input: GlanceResetsInput): GlanceResets | null {
  const { feeds, now, language, timeFormat } = input;
  if (!feeds.status && feeds.resets.length === 0) return null;
  const text = insightsFor(language);
  const resets: GlanceResets = {
    title: text.glanceTitle,
    source: text.glanceSource,
    brand: BRAND,
    color: SOURCE_COLORS.codex,
    forecastTitle: text.glanceChanceTitle,
    forecast: [],
    forecastNote: text.forecastUnavailable,
  };
  const mark = PROVIDER_MARKS[BRAND];
  if (mark) resets.mark = mark;
  if (input.stale) resets.stale = text.staleNote;

  const next = upcomingReset(feeds.status, now);
  if (next) resets.upcoming = upcomingOf(next, now, timeFormat, language, text);

  const latest = feeds.resets[0];
  if (latest) {
    const at = latest.announcedAt.toISOString();
    resets.latest = {
      at,
      kind: latest.kind,
      label: text.latestTitle,
      kindLabel: text.kind(latest.kind),
      since: { at, text: text.glanceSinceLast(COUNTDOWN_SPAN), since: true },
      when: timeOnDayLabel(latest.announcedAt, now, timeFormat, language, false),
    };
  }

  const forecast = forecastResets(feeds.resets, now);
  if (forecast) {
    resets.forecast = FORECAST_HORIZONS.map((days) => ({ days, percent: Math.round(forecast.chance[days] * 100), label: text.horizon(days) }));
    resets.forecastNote = text.glanceForecastNote;
  }

  const wait = currentWait(feeds.resets, now);
  if (wait) {
    resets.wait = text.waitLine(text.days(numberText(language, wait.waitedDays, 1)), percentText(language, wait.shorterShare, 0));
    const markTime = `${shortTime(wait.medianMark, timeFormat, language)} ${shortDate(wait.medianMark, now, language)}`;
    resets.median = (wait.medianMark.getTime() > now.getTime() ? text.medianMark : text.medianMarkPassed)(text.days(numberText(language, wait.medianGapDays, 1)), markTime);
  }

  if (feeds.resets.length > 0) {
    resets.calendar = calendarOf(feeds.resets, now, text, language);
    resets.rhythm = rhythmOf(feeds.resets, text);
  }
  return resets;
}

/**
 * The Codex card's "Reset free" row (`freeResetLines`) as moments and words: an exact or named-day
 * time counts down and then waits for confirmation for a day, a watch counts down to its deadline,
 * and a window or an untimed post shows a fixed value until the tab would drop it. Times are named
 * with their weekday and date, never "today" or "tomorrow", so a caption stays true however long a
 * widget keeps it.
 */
function upcomingOf(next: UpcomingReset, now: Date, timeFormat: TimeFormat, language: Language, text: InsightsMessages): GlanceUpcomingReset {
  const timing = next.timing;
  const at = timingMoment(timing);
  const offset = offsetLabel(at ?? now, deviceTimeZone());
  const clock = (date: Date) => timeOnDayLabel(date, now, timeFormat, language, false);
  const base = { title: text.freeResetTitle(next.origin, next.kind), tone: next.origin === "scheduled" ? ("positive" as const) : ("notice" as const) };
  const chance = next.chancePercent === null ? {} : { chancePercent: Math.round(next.chancePercent) };
  const countdown = (words: string, after?: string): GlanceCountdown => ({ at: at!.toISOString(), text: words, ...(after === undefined ? {} : { after }) });
  switch (timing.kind) {
    case "exact":
      return {
        ...base,
        countdown: countdown(text.freeResetIn(COUNTDOWN_SPAN, false), text.freeResetAwaiting),
        caption: text.freeResetAt(clock(timing.at), offset),
        captionAfter: text.freeResetDue(clock(timing.at), offset),
        hideAt: new Date(timing.at.getTime() + AWAITING_MS).toISOString(),
        ...chance,
      };
    case "day":
      return {
        ...base,
        countdown: countdown(text.freeResetIn(COUNTDOWN_SPAN, true), text.freeResetAwaiting),
        caption: text.freeResetAround(clock(timing.at), offset),
        captionAfter: text.freeResetDue(clock(timing.at), offset),
        note: text.freeResetDayNote(text.namedDay(timing.day)),
        hideAt: new Date(timing.at.getTime() + AWAITING_MS).toISOString(),
        ...chance,
      };
    case "by": {
      const percent = next.chancePercent === null ? null : `${next.chancePercent}%`;
      return {
        ...base,
        countdown: countdown(text.freeResetWithin(COUNTDOWN_SPAN)),
        caption: text.freeResetBy(clock(timing.at), offset, percent),
        hideAt: timing.at.toISOString(),
        ...chance,
      };
    }
    case "window":
      return { ...base, value: text.freeResetNoTime, caption: text.freeResetWindow(timing.window), hideAt: timing.ends.toISOString(), ...chance };
    case "unknown":
      return {
        ...base,
        value: text.freeResetNoTime,
        caption: text.freeResetUntimed,
        hideAt: new Date(next.announcedAt.getTime() + UNTIMED_LIFETIME_MS).toISOString(),
        ...chance,
      };
  }
}

const CELL = { regular: "r", banked: "b" } as const;

/** The Reset tab's calendar as one character per day, oldest week first, Monday to Sunday. */
function calendarOf(resets: readonly CodexReset[], now: Date, text: InsightsMessages, language: Language): GlanceResetCalendar {
  const weeks = resetCalendar(resets, now);
  let today = -1;
  const cells = weeks
    .flatMap((week, weekIndex) =>
      week.map((day, weekday) => {
        if (day.isToday) today = weekIndex * 7 + weekday;
        if (day.future) return "-";
        const kind = day.kinds[day.kinds.length - 1];
        return kind ? CELL[kind] : ".";
      }),
    )
    .join("");
  const months = weeks.flatMap((week, index) => {
    const month = zonedParts(week[0]!.date).month - 1;
    const starts = index === 0 || month !== zonedParts(weeks[index - 1]![0]!.date).month - 1;
    return starts ? [{ week: index, label: messagesFor(language).glance.calendarMonth(month) }] : [];
  });
  return {
    title: text.calendarTitle(weeks.length),
    weeks: weeks.length,
    cells,
    today,
    weekdays: [...text.weekdayShort],
    months,
    legend: { regular: text.kind("regular"), banked: text.kind("banked"), today: text.calendarToday },
  };
}

/** When announcements land, by weekday and by four-hour block, in the device's zone. */
function rhythmOf(resets: readonly CodexReset[], text: InsightsMessages): GlanceResetRhythm {
  const pattern = announcementPattern(resets);
  return {
    title: text.patternTitle,
    total: pattern.total,
    weekdayTitle: text.patternWeekdays,
    weekdays: pattern.weekdays.map((count, index) => ({ label: text.weekdayShort[index] ?? "", count })),
    hourTitle: text.patternHours,
    hours: pattern.hours.map((count, index) => ({ label: text.hourBlock(index * (24 / HOUR_BLOCKS)), count })),
  };
}
