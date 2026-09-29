/**
 * The words of the banked-reset row on a Claude card: how long is left to apply it on the right,
 * the deadline in the device's zone with its GMT offset underneath, and a hover note with the post
 * and where to apply it. The row stands for an announcement, not for the account's own balance,
 * which neither claude-resets.com nor this app can see; the hover note says so.
 */
import type { Language } from "@/i18n";
import { insightsFor } from "@/i18n/insights";
import { compactDuration, timeOnDayLabel, type TimeFormat } from "@/model/format";
import { calendarDaysBetween, deviceTimeZone, offsetLabel } from "@/model/timeZone";
import { covers, openBanked, planFamily, type ClaudeReset } from "./claudeResets";
import { excerpt } from "./resets";

const POST_LENGTH = 200;

export interface BankedResetLines {
  title: string;
  /** How long is left to apply it. */
  value: string;
  /** The deadline, in the device's zone. */
  caption: string;
  /** The post and where to apply it, one paragraph per line. */
  details: string;
}

/**
 * The banked reset a card reminds of: the one expiring first that the user has not marked as
 * applied and that the account's plan is not left out of. A plan this app cannot name is not ruled
 * out.
 */
export function bankedResetFor(resets: readonly ClaudeReset[], plan: string | null | undefined, used: readonly string[], now: Date): ClaudeReset | null {
  const family = planFamily(plan);
  return openBanked(resets, now).find((reset) => !used.includes(reset.id) && (family === null || covers(reset.scope, family) !== false)) ?? null;
}

export function bankedResetLines(reset: ClaudeReset, now: Date, timeFormat: TimeFormat, language: Language): BankedResetLines | null {
  const until = reset.usableUntil;
  if (!until || until.getTime() <= now.getTime()) return null;
  const text = insightsFor(language).claude;
  const clock = (date: Date) => timeOnDayLabel(date, now, timeFormat, language);
  const left = compactDuration((until.getTime() - now.getTime()) / 1000, language) ?? "";
  const postedAt = timeOnDayLabel(reset.announcedAt, now, timeFormat, language, calendarDaysBetween(reset.announcedAt, now) === 0);
  const posted = text.cardPosted(`@${reset.account}`, postedAt, excerpt(reset.text, POST_LENGTH));
  return {
    title: text.cardTitle,
    value: text.cardLeft(left),
    caption: text.cardCaption(clock(until), offsetLabel(until, deviceTimeZone())),
    details: `${posted}\n${text.cardHow}`,
  };
}
