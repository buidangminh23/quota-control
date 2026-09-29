/**
 * The Claude reset tracker (claude-resets.com) as the macOS Dynamic Island and desktop widgets draw
 * it, in the shape of the Codex one (`buildGlanceResets`) so every reset surface can draw either.
 * The latest reset, chances, wait, calendar and rhythm come from the same helper as Codex's; limit
 * changes never reach them, since they did not reset anything. Claude announces no reset ahead, so
 * the tracker's `upcoming` is instead the banked reset that can still be applied: the one with the
 * soonest deadline that the user has not marked as applied and that can concern the Claude accounts
 * connected here, counting down to its deadline.
 */
import { PROVIDER_MARKS } from "@/assets/providerMarks";
import claudeDevsAvatar from "@/assets/claudedevs.webp?inline";
import type { Language } from "@/i18n";
import { insightsFor } from "@/i18n/insights";
import { timeOnDayLabel, type TimeFormat } from "./format";
import type { GlanceResetPresentation, GlanceResets, GlanceResetStatusCard, GlanceUpcomingReset } from "./glance";
import { addHistory, COUNTDOWN_SPAN } from "./glanceResets";
import { CLAUDE_ACCOUNT, concerns, openBanked, type ClaudePlan, type ClaudeReset, type ClaudeResetFeed } from "./insights/claudeResets";
import { authorOf, buildClaudePresentation, type ClaudeBankedCard, type ClaudePresentation } from "./insights/claudePresentation";
import { SOURCE_COLORS } from "./palette";
import { deviceTimeZone, offsetLabel } from "./timeZone";

const BRAND = "claude";
/** Where the tracker's source link leads; the Codex tracker's is the island's own default. */
export const CLAUDE_RESETS_SITE = "https://claude-resets.com";

export interface ClaudeGlanceResetsInput {
  theme?: "system" | "light" | "dark";
  feed: ClaudeResetFeed;
  /** One entry per Claude account connected here, `null` for a plan this app cannot name. */
  accounts: readonly (ClaudePlan | null)[];
  /** Banked resets the user marked as applied (`settings.usedBankedResets`). */
  used: readonly string[];
  /** The feed could not be refreshed and its last good copy is shown. */
  stale: boolean;
  now: Date;
  language: Language;
  timeFormat: TimeFormat;
}

/** Banked resets still to apply, soonest deadline first: open, not marked as applied, and not ruling out every account here. */
export function pendingBanked(resets: readonly ClaudeReset[], accounts: readonly (ClaudePlan | null)[], used: readonly string[], now: Date): ClaudeReset[] {
  const applied = new Set(used);
  return openBanked(resets, now).filter((reset) => !applied.has(reset.id) && concerns(reset, accounts));
}

/** The tracker for the glance document; `null` while the feed records no reset (limit changes alone are none). */
export function buildClaudeGlanceResets(input: ClaudeGlanceResetsInput): GlanceResets | null {
  const { feed, accounts, used, now, language, timeFormat } = input;
  if (feed.resets.length === 0) return null;
  const text = insightsFor(language);
  const plans = [...new Set(accounts.filter((plan): plan is ClaudePlan => plan !== null))];
  const presentation = buildClaudePresentation({ feed, codex: [], plans, accounts, used, now, language, timeFormat });
  const pending = pendingBanked(feed.resets, accounts, used, now);
  const resets: GlanceResets = {
    title: text.claude.glanceTitle,
    source: text.claude.glanceSource,
    brand: BRAND,
    color: SOURCE_COLORS.claude,
    forecastTitle: text.glanceChanceTitle,
    forecast: [],
    forecastNote: text.forecastUnavailable,
    presentation: glancePresentation(presentation, pending, language),
    theme: input.theme,
    site: CLAUDE_RESETS_SITE,
  };
  const mark = PROVIDER_MARKS[BRAND];
  if (mark) resets.mark = mark;
  if (input.stale) resets.stale = text.staleNote;
  const next = pending[0];
  if (next) resets.upcoming = upcomingOf(next, now, timeFormat, language);
  addHistory(resets, feed.resets, now, language, timeFormat);
  return resets;
}

/**
 * The banked reset to apply, as a Claude card's row reads it: the time left to its deadline, and the
 * deadline named with its weekday and date (never "today"), so a caption a widget keeps stays true.
 */
function upcomingOf(reset: ClaudeReset, now: Date, timeFormat: TimeFormat, language: Language): GlanceUpcomingReset {
  const until = reset.usableUntil!;
  const text = insightsFor(language).claude;
  return {
    title: text.cardTitle,
    tone: "positive",
    countdown: { at: until.toISOString(), text: text.cardLeft(COUNTDOWN_SPAN) },
    caption: text.cardCaption(timeOnDayLabel(until, now, timeFormat, language, false), offsetLabel(until, deviceTimeZone())),
    hideAt: until.toISOString(),
  };
}

/**
 * The Reset tab's Claude view cut down to what the island and the widgets draw: its cards in the
 * Codex shape, the banked resets still to apply as status cards counting down to their deadlines,
 * the @ClaudeDevs picture for that account's posts, and none of what only the popup has (limit
 * changes, the comparison with Codex, marking a banked reset as applied, the forecast's self-check).
 * The chances' meters are kept to the whole percent they show, so the document does not change
 * each minute as the estimate drifts.
 */
function glancePresentation(presentation: ClaudePresentation, pending: readonly ClaudeReset[], language: Language): GlanceResetPresentation {
  const text = insightsFor(language).claude;
  const waiting = new Set(pending.map((reset) => reset.id));
  const left = text.bankedLeft(COUNTDOWN_SPAN);
  const banked = presentation.banked.filter((card) => waiting.has(card.resetId)).map((card) => bankedStatus(card, text.glanceBankedHow, left));
  const { reliability: _reliability, ...forecast } = presentation.forecast;
  const chances = forecast.chances.map((chance) => ({ ...chance, fraction: Math.round(chance.fraction * 100) / 100 }));
  const reduced: GlanceResetPresentation = {
    locale: presentation.locale,
    authorAvatar: claudeDevsAvatar,
    avatarHandle: authorOf(CLAUDE_ACCOUNT).handle,
    statuses: banked.length > 0 ? banked : presentation.statuses,
    forecast: { ...forecast, chances },
    statsTitle: presentation.statsTitle,
    stats: presentation.stats,
    historyTitle: presentation.historyTitle,
    history: presentation.history,
    patternNote: presentation.patternNote,
    source: presentation.source,
    methodTitle: presentation.methodTitle,
    method: [...text.glanceMethod],
  };
  if (presentation.latest) reduced.latest = presentation.latest;
  if (presentation.quietTitle !== undefined) reduced.quietTitle = presentation.quietTitle;
  return reduced;
}

/**
 * A banked card as a status card: where to apply it after its lines, and its time left as a moving
 * countdown in place of the popup's fixed words, so the document does not change as time passes.
 */
function bankedStatus(card: ClaudeBankedCard, how: string, left: string): GlanceResetStatusCard {
  const { resetId: _resetId, used: _used, how: _how, due: _due, ...status } = card;
  return { ...status, meta: [...card.meta, how], dueCountdown: { at: card.hideAt!, text: left } };
}
