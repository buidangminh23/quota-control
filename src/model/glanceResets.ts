/**
 * The Codex free-reset tracker as the macOS Dynamic Island and desktop widgets draw it: the Reset
 * tab's announced reset, latest reset, chances, wait, calendar and rhythm, worked out and worded
 * here from the same feeds and the same helpers the tab uses. Everything that moves with the clock
 * travels as a moment plus words with `{d}` (a `GlanceCountdown`) and every chance as a whole
 * percent, so the document only changes when the tracker's numbers do.
 */
import { PROVIDER_MARKS } from "@/assets/providerMarks";
import resetAvatar from "@/assets/thsottiaux.webp?inline";
import type { Language } from "@/i18n";
import { insightsFor, type InsightsMessages } from "@/i18n/insights";
import { compactDuration, shortTime, timeOnDayLabel, type TimeFormat } from "./format";
import type { GlanceCountdown, GlanceResetAuthor, GlanceResetCalendar, GlanceResetPresentation, GlanceResetRhythm, GlanceResets, GlanceResetStatusCard, GlanceUpcomingReset } from "./glance";
import {
  announcementPattern,
  activeWatch,
  currentWait,
  excerpt,
  FORECAST_HORIZONS,
  forecastResets,
  HOUR_BLOCKS,
  parseResets,
  parseResetStatus,
  resetCalendar,
  resetStats,
  type CodexReset,
  type ResetStatus,
} from "./insights/resets";
import { dateText, numberText, percentText, shortDate } from "./insights/text";
import { AWAITING_MS, timingMoment, UNTIMED_LIFETIME_MS, upcomingReset, type UpcomingReset } from "./insights/upcomingReset";
import { SOURCE_COLORS } from "./palette";
import { deviceTimeZone, offsetLabel, zonedParts } from "./timeZone";

/** The placeholder Swift fills with the moving span of time. */
export const COUNTDOWN_SPAN = "{d}";
const BRAND = "codex";
export const RESET_PRESENTATION_AUTHOR: GlanceResetAuthor = { handle: "@thsottiaux" };

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
  theme?: "system" | "light" | "dark";
  feeds: ResetFeeds;
  /** A feed could not be refreshed and its last good copy is shown. */
  stale: boolean;
  now: Date;
  language: Language;
  timeFormat: TimeFormat;
}

export function resetAgoText(date: Date, now: Date, language: Language): string {
  const minutes = Math.max(1, Math.floor((now.getTime() - date.getTime()) / 60_000));
  const hours = Math.floor(minutes / 60);
  const formatter = new Intl.RelativeTimeFormat(language === "vi" ? "vi-VN" : "en-US", { numeric: "always" });
  if (hours < 1) return formatter.format(-minutes, "minute");
  if (hours < 24) return formatter.format(-hours, "hour");
  return formatter.format(-Math.floor(hours / 24), "day");
}

export function buildResetPresentation(input: Omit<GlanceResetsInput, "stale">): GlanceResetPresentation {
  const { feeds, now, language, timeFormat } = input;
  const text = insightsFor(language);
  const when = (date: Date) => `${shortTime(date, timeFormat, language)} ${dateText(date, language)}`;
  const ago = (date: Date) => compactDuration(Math.max(60, (now.getTime() - date.getTime()) / 1000), language);
  const author = (source: CodexReset["source"]) => source.kind === "x_post" ? RESET_PRESENTATION_AUTHOR : undefined;
  const statuses: GlanceResetStatusCard[] = [];
  const scheduled = feeds.status?.scheduled;
  if (scheduled) {
    const due = scheduled.scheduledFor;
    const duration = due && (due > now ? compactDuration((due.getTime() - now.getTime()) / 1000, language) : ago(due));
    statuses.push({
      id: `scheduled:${scheduled.id}`, kind: "scheduled", title: text.scheduledTitle,
      excerpt: excerpt(scheduled.text, 280), author: author(scheduled.source), url: scheduled.source.url ?? undefined,
      meta: [[text.announcedAgo(ago(scheduled.announcedAt) ?? "—"), due ? text.scheduledFor(when(due)) : text.scheduledNoTime].join(" · ")],
      due: due && duration ? (due > now ? text.scheduledIn(duration) : text.scheduledOverdue(duration)) : undefined,
      announced: { at: scheduled.announcedAt.toISOString(), text: text.announcedAgo(COUNTDOWN_SPAN), since: true },
      scheduledMeta: due ? text.scheduledFor(when(due)) : text.scheduledNoTime,
      dueCountdown: due ? { at: due.toISOString(), text: text.scheduledIn(COUNTDOWN_SPAN) } : undefined,
      overdueCountdown: due ? { at: due.toISOString(), text: text.scheduledOverdue(COUNTDOWN_SPAN), since: true } : undefined,
    });
  }
  const watch = activeWatch(feeds.status, now);
  if (watch) {
    statuses.push({
      id: `watch:${watch.observedAt.toISOString()}`, kind: "watch", level: watch.level, title: text.watchTitle(watch.level),
      excerpt: excerpt(watch.text, 280), author: author(watch.source), url: watch.source.url ?? undefined,
      meta: [...(watch.chancePercent === null ? [] : [text.watchChance(`${watch.chancePercent}%`, watch.forecastWindow)]), text.watchUntil(when(watch.expiresAt))],
      hideAt: watch.expiresAt.toISOString(),
    });
  }
  if (!scheduled && !watch && (feeds.status || feeds.resets.length)) {
    statuses.push({ id: "quiet", kind: "quiet", title: text.quietTitle, meta: [] });
  }
  const latest = feeds.resets[0];
  const forecast = forecastResets(feeds.resets, now);
  const wait = forecast ? currentWait(feeds.resets, now) : null;
  const stats = resetStats(feeds.resets, now);
  const days = (value: number | null) => value === null ? "—" : text.days(numberText(language, value, 1));
  const statsRows: [string, string][] = [
    [text.statTotal, `${numberText(language, stats.total)} (${text.statKinds(numberText(language, stats.regular), numberText(language, stats.banked))})`],
    [text.statSinceLast, days(stats.daysSinceLast)],
    [text.statLast30, numberText(language, stats.last30Days)],
    [text.statAverage, days(stats.averageGapDays)],
    [text.statMedian, days(stats.medianGapDays)],
    [text.statLongest, stats.longestGap ? `${days(stats.longestGap.days)} · ${text.gapRange(shortDate(stats.longestGap.from, now, language), shortDate(stats.longestGap.to, now, language))}` : "—"],
  ];
  const moment = latest ? timeOnDayLabel(latest.announcedAt, now, timeFormat, language, false) : "";
  return {
    locale: language === "vi" ? "vi-VN" : "en-US",
    authorAvatar: resetAvatar,
    latest: latest ? { title: text.latestTitle, ago: resetAgoText(latest.announcedAt, now, language), at: latest.announcedAt.toISOString(), meta: latest.kind === "banked" ? `${moment} · ${text.kind("banked")}` : moment, author: author(latest.source) } : undefined,
    statuses,
    quietTitle: feeds.status || feeds.resets.length ? text.quietTitle : undefined,
    forecast: {
      title: text.forecastTitle,
      chances: forecast ? FORECAST_HORIZONS.map((days) => ({ days, label: text.horizon(days), percent: percentText(language, forecast.chance[days], 0), fraction: forecast.chance[days] })) : [],
      wait: wait ? text.waitLine(text.days(numberText(language, wait.waitedDays, 1)), percentText(language, wait.shorterShare, 0)) : undefined,
      waitFraction: wait?.shorterShare,
      median: wait ? (wait.medianMark > now ? text.medianMark : text.medianMarkPassed)(text.days(numberText(language, wait.medianGapDays, 1)), `${shortTime(wait.medianMark, timeFormat, language)} ${shortDate(wait.medianMark, now, language)}`) : undefined,
      sampleNote: forecast ? text.forecastNote(numberText(language, forecast.resets)) : undefined,
      disclaimer: forecast ? text.forecastDisclaimer : undefined,
      unavailable: forecast || feeds.resets.length === 0 ? undefined : text.forecastUnavailable,
    },
    statsTitle: text.statsTitle,
    stats: feeds.resets.length ? statsRows.map(([label, value]) => ({ label, value })) : [],
    historyTitle: text.historyTitle,
    history: feeds.resets.map((reset) => ({ id: reset.id, kind: reset.kind, kindLabel: text.kind(reset.kind), when: when(reset.announcedAt), excerpt: excerpt(reset.text, 220), author: author(reset.source), url: reset.source.url ?? undefined, observed: reset.source.kind === "observed" ? text.observed : undefined })),
    patternNote: text.patternNote(numberText(language, announcementPattern(feeds.resets).total)),
    source: text.resetsSource,
    methodTitle: text.methodTitle,
    method: [...text.resetsMethod],
  };
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
    presentation: buildResetPresentation(input),
    theme: input.theme,
  };
  const mark = PROVIDER_MARKS[BRAND];
  if (mark) resets.mark = mark;
  if (input.stale) resets.stale = text.staleNote;

  const next = upcomingReset(feeds.status, now);
  if (next) resets.upcoming = upcomingOf(next, now, timeFormat, language, text);
  addHistory(resets, feeds.resets, now, language, timeFormat);
  return resets;
}

/**
 * What a tracker's history gives it, the same way for Codex and Claude: the newest reset and the
 * time since it, the chances with the words under them, the wait against earlier ones, the calendar
 * and the rhythm. `history` holds resets only, newest first.
 */
export function addHistory(resets: GlanceResets, history: readonly CodexReset[], now: Date, language: Language, timeFormat: TimeFormat): void {
  const text = insightsFor(language);
  const latest = history[0];
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

  const forecast = forecastResets(history, now);
  if (forecast) {
    resets.forecast = FORECAST_HORIZONS.map((days) => ({ days, percent: Math.round(forecast.chance[days] * 100), label: text.horizon(days) }));
    resets.forecastNote = text.glanceForecastNote;
  }

  const wait = currentWait(history, now);
  if (wait) {
    resets.wait = text.waitLine(text.days(numberText(language, wait.waitedDays, 1)), percentText(language, wait.shorterShare, 0));
    const markTime = `${shortTime(wait.medianMark, timeFormat, language)} ${shortDate(wait.medianMark, now, language)}`;
    resets.median = (wait.medianMark.getTime() > now.getTime() ? text.medianMark : text.medianMarkPassed)(text.days(numberText(language, wait.medianGapDays, 1)), markTime);
  }

  if (history.length > 0) {
    resets.calendar = calendarOf(history, now, text);
    resets.rhythm = rhythmOf(history, text);
  }
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
function calendarOf(resets: readonly CodexReset[], now: Date, text: InsightsMessages): GlanceResetCalendar {
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
    return starts ? [{ week: index, label: text.monthShort(month) }] : [];
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
