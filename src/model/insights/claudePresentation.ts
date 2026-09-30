/**
 * The Claude view of the Reset tab, worded: the same cards the Codex view draws (latest reset,
 * chances, wait, statistics, history) built by the same helper, then what only Claude's catalog
 * has: who each reset covered and whether that includes the plans connected here, banked resets
 * that can still be applied, limit changes, entries the site has not reviewed yet, and Claude
 * against Codex over the time both were tracked.
 */
import type { Language } from "@/i18n";
import { insightsFor, type ClaudeResetMessages, type CompareRow, type InsightsMessages } from "@/i18n/insights";
import { compactDuration, shortTime, timeOnDayLabel, type TimeFormat } from "../format";
import type { GlanceResetAuthor, GlanceResetHistoryItem, GlanceResetPresentation, GlanceResetStatusCard } from "../glance";
import { buildResetPresentation, POST_EXCERPT_LENGTH } from "../glanceResets";
import {
  compareTrackers,
  concerns,
  covers,
  detectorBehind,
  forecastSkill,
  latestFor,
  latestForEveryone,
  openBanked,
  readScope,
  type ClaudePlan,
  type ClaudeReset,
  type ClaudeResetFeed,
  type ForecastSkill,
  type TrackerSide,
} from "./claudeResets";
import { excerpt, type CodexReset } from "./resets";
import { dateText, numberText, percentText, shortDate } from "./text";

const PAID_PLAN_COUNT = 4;
const DAY_MS = 86_400_000;
const COMPARE_ROWS: readonly CompareRow[] = ["resets", "average", "median", "longest", "sinceLast", "last30"];
/**
 * The chart shows this many months, the newest. Under twelve, so no month name comes twice, and
 * few enough that a column of the 320px popup holds two two-digit counts side by side (24px).
 */
export const COMPARE_MONTHS = 8;

export interface ClaudeBankedCard extends GlanceResetStatusCard {
  /** The announcement the card is about, for marking it as applied. */
  resetId: string;
  /** The user marked it as applied; the card folds to one line. */
  used: boolean;
  /** Where and how to apply it. */
  how: string;
}

export interface ClaudeChangeItem {
  id: string;
  when: string;
  excerpt: string;
  author: GlanceResetAuthor;
  url?: string;
  scope?: string;
  provisional?: string;
}

export interface ComparePresentation {
  title: string;
  since: string;
  rows: { label: string; claude: string; codex: string }[];
  monthsTitle: string;
  months: { label: string; claude: number; codex: number; summary: string }[];
}

export interface ClaudePresentation extends GlanceResetPresentation {
  /** Lines above the cards: the site is behind, or its published copy is shown. */
  notices: string[];
  banked: ClaudeBankedCard[];
  changesTitle: string;
  changesNote: string;
  changes: ClaudeChangeItem[];
  compare?: ComparePresentation;
}

export interface ClaudePresentationInput {
  feed: ClaudeResetFeed;
  /** The Codex history, for the comparison; empty while it is not loaded. */
  codex: readonly CodexReset[];
  /** The plan families of the Claude accounts connected here. */
  plans: readonly ClaudePlan[];
  /** One entry per Claude account, `null` for a plan this app cannot name; `plans` when left out. */
  accounts?: readonly (ClaudePlan | null)[];
  /** Banked resets the user marked as applied. */
  used: readonly string[];
  now: Date;
  language: Language;
  timeFormat: TimeFormat;
}

export function authorOf(account: string): GlanceResetAuthor {
  return { handle: `@${account}` };
}

/** Who an announcement covered, worded; the site's own words when this app does not know them. */
export function scopeText(scope: string | null, text: ClaudeResetMessages): string | null {
  const reading = readScope(scope);
  switch (reading.reach) {
    case "everyone":
      return text.scopeEveryone;
    case "affected":
      return text.scopeAffected;
    case "plans":
      return reading.plans.length === PAID_PLAN_COUNT ? text.scopePaid : text.scopePlans(reading.plans.map((plan) => text.plan(plan)).join(", "));
    case "unknown":
      return scope;
  }
}

/** The connected plans the scope settles, covered ones first: `Gói Max của bạn: có áp dụng`. */
export function planLines(scope: string | null, plans: readonly ClaudePlan[], text: ClaudeResetMessages): string[] {
  if (readScope(scope).reach !== "plans") return [];
  return [true, false].flatMap((covered) => {
    const names = plans.filter((plan) => covers(scope, plan) === covered).map((plan) => text.plan(plan));
    return names.length > 0 ? [text.yourPlan(names.join(", "), covered)] : [];
  });
}

/** The forecast's self-check as one sentence, or nothing while the history is too short to try. */
export function reliabilityText(skill: ForecastSkill | null, language: Language, text: InsightsMessages): string | undefined {
  if (!skill) return undefined;
  return text.forecastReliability(skill.verdict, percentText(language, Math.abs(skill.skill), 0), numberText(language, skill.days), numberText(language, skill.resets));
}

function compareOf(feed: ClaudeResetFeed, codex: readonly CodexReset[], now: Date, language: Language, text: InsightsMessages): ComparePresentation | undefined {
  const comparison = compareTrackers(feed.resets, codex, now);
  if (!comparison) return undefined;
  const days = (value: number | null) => (value === null ? "—" : text.days(numberText(language, value, 1)));
  const count = (value: number) => numberText(language, value);
  const cell = (row: CompareRow, side: TrackerSide): string => {
    switch (row) {
      case "resets":
        return count(side.resets);
      case "average":
        return days(side.averageGapDays);
      case "median":
        return days(side.medianGapDays);
      case "longest":
        return days(side.longestGapDays);
      case "sinceLast":
        return days(side.daysSinceLast);
      case "last30":
        return count(side.last30Days);
    }
  };
  return {
    title: text.claude.compareTitle,
    since: text.claude.compareSince(dateText(comparison.from, language)),
    rows: COMPARE_ROWS.map((row) => ({ label: text.claude.compareRow(row), claude: cell(row, comparison.claude), codex: cell(row, comparison.codex) })),
    monthsTitle: comparison.months.length > COMPARE_MONTHS ? text.claude.compareMonthsRecent(numberText(language, COMPARE_MONTHS)) : text.claude.compareMonths,
    months: comparison.months.slice(-COMPARE_MONTHS).map((month) => {
      const label = text.monthShort(month.month);
      return { label, claude: month.claude, codex: month.codex, summary: text.claude.compareMonth(label, count(month.claude), count(month.codex)) };
    }),
  };
}

export function buildClaudePresentation(input: ClaudePresentationInput): ClaudePresentation {
  const { feed, plans, now, language, timeFormat } = input;
  const text = insightsFor(language);
  const claude = text.claude;
  const base = buildResetPresentation({ feeds: { status: null, resets: feed.resets }, now, language, timeFormat });
  const when = (date: Date) => `${shortTime(date, timeFormat, language)} ${dateText(date, language)}`;
  const span = (from: Date, to: Date) => compactDuration(Math.max(60, (to.getTime() - from.getTime()) / 1000), language) ?? "";
  const ago = (date: Date) => claude.ago(text.days(numberText(language, (now.getTime() - date.getTime()) / DAY_MS, 1)), shortDate(date, now, language));
  const flag = (item: { provisional: boolean }) => (item.provisional ? claude.provisional : undefined);

  const used = new Set(input.used);
  const latest = feed.resets[0];
  const accounts = input.accounts ?? plans;
  const open = openBanked(feed.resets, now).filter((reset) => concerns(reset, accounts));
  const latestOpen = latest !== undefined && !used.has(latest.id) && open.some((reset) => reset.id === latest.id);
  const latestNotes = latest ? [...(latestOpen ? [] : planLines(latest.scope, plans, claude)), ...(latest.provisional ? [claude.provisionalNote] : [])] : [];
  const latestScope = latest ? scopeText(latest.scope, claude) : null;

  const banked = open.map((reset): ClaudeBankedCard => {
    const until = reset.usableUntil!;
    const scope = scopeText(reset.scope, claude);
    return {
      id: `banked:${reset.id}`,
      resetId: reset.id,
      kind: "banked",
      title: claude.bankedTitle,
      excerpt: excerpt(reset.text, POST_EXCERPT_LENGTH),
      author: authorOf(reset.account),
      url: reset.source.url ?? undefined,
      meta: [...(scope ? [scope] : []), ...planLines(reset.scope, plans, claude), claude.bankedUntil(when(until))],
      due: claude.bankedLeft(span(now, until)),
      hideAt: until.toISOString(),
      used: used.has(reset.id),
      how: claude.bankedHow,
      ...(reset.id === latest?.id ? { sameAsLatest: true } : {}),
    };
  });

  const everyone = latestForEveryone(feed.resets);
  const extraStats: { label: string; value: string }[] = [];
  if (everyone && everyone.id !== latest?.id) extraStats.push({ label: claude.statEveryone, value: ago(everyone.announcedAt) });
  for (const plan of plans) {
    const mine = latestFor(feed.resets, plan);
    if (mine && mine.id !== latest?.id && mine.id !== everyone?.id) extraStats.push({ label: claude.statYourPlan(claude.plan(plan)), value: ago(mine.announcedAt) });
  }
  if (feed.resets.length > 0) extraStats.push({ label: claude.statChanges, value: numberText(language, feed.changes.length) });

  const history = feed.resets.map((reset: ClaudeReset): GlanceResetHistoryItem => ({
    id: reset.id,
    kind: reset.kind,
    kindLabel: text.kind(reset.kind),
    when: when(reset.announcedAt),
    excerpt: excerpt(reset.text, 220),
    author: authorOf(reset.account),
    url: reset.source.url ?? undefined,
    scope: scopeText(reset.scope, claude) ?? undefined,
    provisional: flag(reset),
  }));

  const moment = latest ? timeOnDayLabel(latest.announcedAt, now, timeFormat, language, false) : "";
  const latestMeta = [moment, ...(latest?.kind === "banked" ? [text.kind("banked")] : []), ...(latestScope ? [latestScope] : []), ...(latest?.provisional ? [claude.provisional] : [])];
  return {
    ...base,
    authorAvatar: "",
    latest: base.latest && latest ? { ...base.latest, meta: latestMeta.join(" · "), author: authorOf(latest.account), notes: latestNotes } : undefined,
    statuses: banked.some((card) => !card.used) ? [] : base.statuses,
    banked,
    forecast: {
      ...base.forecast,
      disclaimer: base.forecast.disclaimer ? claude.forecastDisclaimer : undefined,
      reliability: base.forecast.chances.length > 0 ? reliabilityText(forecastSkill(feed.resets, now), language, text) : undefined,
    },
    stats: [...base.stats, ...extraStats],
    history,
    source: claude.source,
    method: [...claude.method],
    notices: [...(detectorBehind(feed) ? [claude.detectorBehind] : []), ...(feed.live ? [] : [claude.datasetNote])],
    changesTitle: claude.changesTitle,
    changesNote: claude.changesNote,
    changes: feed.changes.map((change) => ({
      id: change.id,
      when: when(change.announcedAt),
      excerpt: excerpt(change.text, 220),
      author: authorOf(change.account),
      url: change.url ?? undefined,
      scope: scopeText(change.scope, claude) ?? undefined,
      provisional: flag(change),
    })),
    compare: compareOf(feed, input.codex, now, language, text),
  };
}
