/**
 * The rows a Codex or Claude card starts with in the popup (`FreeResetRow`, `BankedResetRow`), as
 * the macOS Dynamic Island and desktop widgets draw them at the top of the same account: worked out
 * and worded here with the helpers the popup's rows use, independent of which reset tracker a
 * surface shows. The countdown and the caption's day move with the clock, so they travel as moments
 * with words (`{d}` for the time left, `{at}` for the moment's clock time and day) that the Swift
 * side fills in when it draws, the way the popup's rows word them at every tick.
 */
import claudeDevsAvatar from "@/assets/claudedevs.webp?inline";
import resetAvatar from "@/assets/thsottiaux.webp?inline";
import type { Language } from "@/i18n";
import { insightsFor } from "@/i18n/insights";
import { timeOnDayLabel, type TimeFormat } from "./format";
import type { GlanceCountdown, GlanceProvider, GlanceResetRow } from "./glance";
import { COUNTDOWN_SPAN, RESET_PRESENTATION_AUTHOR } from "./glanceResets";
import { bankedResetFor, bankedResetLines } from "./insights/bankedResetLines";
import { CLAUDE_ACCOUNT, type ClaudeReset } from "./insights/claudeResets";
import { freeResetLines } from "./insights/freeResetLines";
import type { ResetStatus } from "./insights/resets";
import { AWAITING_MS, timingMoment, UNTIMED_LIFETIME_MS, upcomingReset, type UpcomingReset } from "./insights/upcomingReset";
import { deviceTimeZone, offsetLabel } from "./timeZone";

/** Where a caption names its moment, filled with the clock time and its day when drawn. */
export const MOMENT_PLACEHOLDER = "{at}";

const CLAUDE_AUTHOR = `@${CLAUDE_ACCOUNT}`;

/** The pictures the popup's rows show, by lowercase handle; any other poster gets an initial. */
const AVATARS: Readonly<Record<string, string>> = {
  [RESET_PRESENTATION_AUTHOR.handle.toLowerCase()]: resetAvatar,
  [CLAUDE_AUTHOR.toLowerCase()]: claudeDevsAvatar,
};

/**
 * A Codex card's first row while codex-resets.com knows a free reset is coming (`FreeResetRow`):
 * the announced reset, else the site's watch. `opens` while the Reset tab is on, which the row opens.
 */
export function codexResetRow(status: ResetStatus | null, now: Date, timeFormat: TimeFormat, language: Language, opens: boolean): GlanceResetRow | null {
  const next = upcomingReset(status, now);
  return next ? freeResetRow(next, now, timeFormat, language, opens) : null;
}

/**
 * The free-reset row for `next` (`freeResetLines`): the countdown, its time in the device's zone and
 * the poster's own day under an estimate; a named day keeps its weekday and date, as the popup's
 * caption does.
 */
export function freeResetRow(next: UpcomingReset, now: Date, timeFormat: TimeFormat, language: Language, opens: boolean): GlanceResetRow {
  const text = insightsFor(language);
  const lines = freeResetLines(next, now, timeFormat, language);
  const timing = next.timing;
  const at = timingMoment(timing);
  const offset = offsetLabel(at ?? now, deviceTimeZone());
  const countdown = (words: string): GlanceCountdown => ({ at: at!.toISOString(), text: words, after: text.freeResetAwaiting });
  const base = {
    tracker: "codex" as const,
    title: lines.title,
    tone: next.origin === "watch" ? ("notice" as const) : ("positive" as const),
    author: RESET_PRESENTATION_AUTHOR.handle,
  };
  const row = ((): Omit<GlanceResetRow, "tracker" | "title" | "tone" | "author" | "details"> => {
    switch (timing.kind) {
      case "exact":
        return {
          countdown: countdown(text.freeResetIn(COUNTDOWN_SPAN, false)),
          caption: text.freeResetAt(MOMENT_PLACEHOLDER, offset),
          captionAfter: text.freeResetDue(MOMENT_PLACEHOLDER, offset),
          at: timing.at.toISOString(),
          hideAt: new Date(timing.at.getTime() + AWAITING_MS).toISOString(),
        };
      case "day":
        return {
          countdown: countdown(text.freeResetIn(COUNTDOWN_SPAN, true)),
          caption: text.freeResetAround(timeOnDayLabel(timing.at, now, timeFormat, language, false), offset),
          captionAfter: text.freeResetDue(MOMENT_PLACEHOLDER, offset),
          at: timing.at.toISOString(),
          note: text.freeResetDayNote(text.namedDay(timing.day)),
          hideAt: new Date(timing.at.getTime() + AWAITING_MS).toISOString(),
        };
      case "by":
        return {
          countdown: countdown(text.freeResetWithin(COUNTDOWN_SPAN)),
          caption: text.freeResetBy(MOMENT_PLACEHOLDER, offset, next.chancePercent === null ? null : `${next.chancePercent}%`),
          at: timing.at.toISOString(),
          hideAt: timing.at.toISOString(),
        };
      case "window":
        return { value: text.freeResetNoTime, caption: text.freeResetWindow(timing.window), hideAt: timing.ends.toISOString() };
      case "unknown":
        return { value: text.freeResetNoTime, caption: text.freeResetUntimed, hideAt: new Date(next.announcedAt.getTime() + UNTIMED_LIFETIME_MS).toISOString() };
    }
  })();
  return { ...base, ...row, details: opens ? `${lines.details}\n${text.freeResetOpenTab}` : lines.details, ...(opens ? { opens: true as const } : {}) };
}

/**
 * A Claude card's first row while a banked reset its plan can still apply is open
 * (`BankedResetRow`): the one expiring first that the user has not marked as used, counting down
 * to its deadline. `plan` is the card's plan as its reading names it.
 */
export function claudeResetRow(
  resets: readonly ClaudeReset[],
  plan: string | null | undefined,
  used: readonly string[],
  now: Date,
  timeFormat: TimeFormat,
  language: Language,
  opens: boolean,
): GlanceResetRow | null {
  const reset = bankedResetFor(resets, plan, used, now);
  const lines = reset ? bankedResetLines(reset, now, timeFormat, language) : null;
  if (!reset || !lines) return null;
  const text = insightsFor(language);
  const until = reset.usableUntil!.toISOString();
  return {
    tracker: "claude",
    title: lines.title,
    tone: "accent",
    author: reset.account ? `@${reset.account}` : CLAUDE_AUTHOR,
    countdown: { at: until, text: text.claude.cardLeft(COUNTDOWN_SPAN) },
    caption: text.claude.cardCaption(MOMENT_PLACEHOLDER, offsetLabel(reset.usableUntil!, deviceTimeZone())),
    at: until,
    details: opens ? `${lines.details}\n${text.freeResetOpenTab}` : lines.details,
    hideAt: until,
    ...(opens ? { opens: true as const } : {}),
  };
}

/** The pictures `providers`' rows name, by lowercase handle, or `null` when none names one. */
export function resetRowAvatars(providers: readonly GlanceProvider[]): Record<string, string> | null {
  const avatars: Record<string, string> = {};
  for (const entry of providers) {
    const handle = entry.resetRow?.author.toLowerCase();
    const avatar = handle ? AVATARS[handle] : undefined;
    if (handle && avatar) avatars[handle] = avatar;
  }
  return Object.keys(avatars).length > 0 ? avatars : null;
}
