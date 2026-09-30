/**
 * The document the macOS Dynamic Island and desktop widgets draw (`src-tauri/macos/Shared/Glance.swift`
 * decodes it). Each surface lists the account cards and metrics its Settings choose (the Hạn mức
 * cards, the starred metrics, or a hand-picked set), exactly as the popup reads them: every headline,
 * meter fill, pace color and reset time is worked out and localized here, following Used/Left, so the
 * Swift side only lays it out. What moves with the clock travels as moments with the popup's words
 * and the inputs of its pace verdict, which Swift fills in at the moment it draws, as the popup does
 * at every tick: the countdowns in the Reset Times setting's form, a limit's pace note and even-pace
 * tick, and the reading a window rolls over to at its reset.
 */
import { PROVIDER_MARKS, type ProviderMark } from "@/assets/providerMarks";
import { knownBrandColor } from "./totalSpend";
import { messagesFor, type Language } from "@/i18n";
import type { When } from "@/i18n/messages";
import type { PlanTerm, Provider, WidgetDescriptor } from "@/lib/types";
import { insightsFor } from "@/i18n/insights";
import { usesTwentyFourHour, type TimeFormat } from "./format";
import { COUNTDOWN_SPAN } from "./glanceResets";
import { MOMENT_PLACEHOLDER, resetRowAvatars } from "./glanceResetRows";
import { brandOf, cardsAsShown, isLocalHistoryCard, layoutFamily, type ProviderMetrics } from "./layout";
import { periodLabel } from "./menuBar";
import { boundedTrailingText, isFreshSessionWindow, meterSeverity, meterState, type MeterState } from "./meterState";
import { SOURCE_COLORS } from "./palette";
import { PLAN_TERM_SOON_DAYS, planTermEnd } from "./planTerm";
import { isOutdated } from "./providerText";
import { readsClaudeResets, type DensitySetting, type GlanceContent, type GlanceSurfaceSettings, type IslandSettings, type IslandStyle, type IslandView, type ResetParts, type ResetProvider, type SurfaceResets, type ThemeSetting } from "./settings";
import { availableResets, boundedHeadline, fraction, isBounded, menuBarValue, soonestExpiry, unboundedDetail, type DisplayOptions, type WidgetData } from "./widgetData";
import { rolledOverReading } from "./windowReset";

export const GLANCE_VERSION = 1;
/** The open island lists at most this many accounts (in two columns); the widget decides what fits
 * its size. */
const ISLAND_PROVIDER_LIMIT = 12;

export type GlanceSeverity = "normal" | "warning" | "critical" | "none";

export interface GlanceMetric {
  id: string;
  label: string;
  /** The strip reading: `42%` for a meter, the compact value otherwise. */
  value: string;
  headline: string;
  /** Meter fill 0...1 following Used/Left; `null` without a limit. */
  fraction: number | null;
  severity: GlanceSeverity;
  resetsAt?: string;
  /** What shows where no countdown applies: `Not started`, a plan badge, `No data`. */
  detail?: string;
  /** A value that moves with the clock, which Swift ticks in place of `value` (island wings only). */
  countdown?: GlanceCountdown;
  /** When the soonest of the row's reset credits expires (Codex's `Lượt đặt lại`), for the dot before
   * its value: red within 48 hours, yellow within 7 days, blue otherwise, as the popup colors it. */
  expiresAt?: string;
  /** The limit window's short name (`5h`, `week`) the menu bar strip labels a reading with; only on
   * the readings beside the notch. */
  period?: string;
  /** What the row's pace note and even-pace tick are worked out from; absent on a row that can show
   * neither. */
  pace?: GlancePace;
  /** The reading once `resetsAt` has passed; only on a limit counting down. */
  after?: GlanceAfterReset;
  /** The popup's "Dùng 1 lượt" under this row: only on the reset credits of a connected Codex
   * account that can spend one now, where the popup shows the button. */
  redeem?: GlanceRedeem;
}

/**
 * What the popup's "Dùng 1 lượt" (`RedeemResetButton`) needs on the island and the widgets: the
 * account it spends a reset of, and its words and its confirmation's words exactly as the popup
 * builds them. The confirmation names the soonest credit's expiry as `whenLabel` words it at the
 * moment it is asked; that moves with the clock, so it travels as `messageAt` with `{at}` for the
 * expiry, the moment itself and the words that fill it in (`expiry`), and the document only changes
 * when the credits do.
 */
export interface GlanceRedeem {
  /** The account whose reset is spent (`redeemLimitReset`). */
  providerId: string;
  /** The button: `Dùng 1 lượt`. */
  redeem: string;
  /** The button while the reset is being spent: `Đang dùng…`. */
  redeeming: string;
  /** The confirmation's title: `Dùng 1 lượt đặt lại?`. */
  title: string;
  /** The confirmation's words without an expiry, once `expiresAt` has passed or where there is none. */
  message: string;
  /** The confirmation's words with `{at}` where the soonest credit's expiry goes. */
  messageAt?: string;
  /** When the soonest credit still ahead expires. */
  expiresAt?: string;
  /** How `{at}` words `expiresAt` at the moment the confirmation is asked (`whenLabel` in Exact Time):
   * `{t} hôm nay`, `{t} ngày mai`, `{t} ngày {d}`. */
  expiry?: GlanceDayWords;
  /** `Xác nhận`, in the destructive color. */
  confirm: string;
  /** `Hủy`. */
  cancel: string;
}

/**
 * What a limit's pace note and even-pace tick (`meterState`, `paceTick`) are worked out from at the
 * moment they are drawn, as the popup works them out each time it renders: `spent` for a limit used
 * up, which reads `Đã hết hạn mức` beside a flame whatever the time; for a limit counting down, the
 * share of it used and its window's length, the window starting that long before `resetsAt`. The
 * verdict, its figure and the tick move with the clock, so they travel as these inputs and the
 * document only changes when a reading does.
 */
export interface GlancePace {
  spent?: true;
  /** The share of the limit used, unrounded: `used / limit`. */
  used?: number;
  /** The window's length in milliseconds. */
  period?: number;
}

/**
 * A limit's reading once its reset has passed, as the popup shows the window the moment it rolls
 * over (`rollOverPassedWindows`): nothing used and no countdown, so `Còn 100%` with the next
 * period's words. The widgets draw it from the reset on, until the app writes the next reading.
 */
export interface GlanceAfterReset {
  value: string;
  headline: string;
  fraction: number;
  /** `Đặt lại sau 5 giờ`, `Chưa bắt đầu`. */
  detail?: string;
  /** The meter's color when it is not the normal one. */
  severity?: GlanceSeverity;
}

/** The words of the pace note on a limit's title line (`PaceWarning`), `{n}` for its figure. */
export interface GlancePaceWords {
  /** A limit used up, after a flame: `Đã hết hạn mức`. */
  limitReached: string;
  /** A limit close to running out, the share it will have left at reset: `Dư ~{n}%`. */
  spare: string;
  /** A limit on a healthy pace while Always Show Pacing is on: `Còn ~{n}% khi đặt lại`. */
  leftAtReset: string;
}

/**
 * Words around a moving span of time, filled in by Swift so the document itself stays unchanged
 * between real changes: `text` with `{d}` replaced by the time left until `at` (or, `since`, the
 * time gone by since `at`), worded with `labels.units`. Once `at` has passed, a countdown reads
 * `after` instead (`text` with `{d}` = 0 when it has none).
 */
export interface GlanceCountdown {
  at: string;
  text: string;
  since?: boolean;
  after?: string;
  /** What a `since` countdown reads while less than a minute has gone by (`Vừa tải`). */
  recent?: string;
}

/**
 * A provider's logo: the single-color path data the popup draws, tinted with `color`, and for a
 * brand whose official logo is several colors, that logo as a base64 PNG (`art`) drawn instead.
 */
export type GlanceMark = ProviderMark & { art?: string };

export interface GlanceProvider {
  id: string;
  /** The card heading: the brand for an account named by its email, the account title otherwise. */
  name: string;
  /** The account's email, which the card shows under its name: its label when that is an email,
   * else the address the provider reports. */
  account?: string;
  /** The plan (`Pro`, `Plus`), which the card shows after its name. */
  plan?: string;
  /** The plan's paid period, the card header's right corner. */
  term?: GlancePlanTerm;
  /** `Dữ liệu cũ`, while the reading is two refresh intervals old, as beside the card's name. */
  outdated?: string;
  /** The header notice's first line (a failed refresh, an error snapshot, a provider warning): the
   * reason for the warning triangle beside the name, whether or not the account has readings. */
  problem?: string;
  /** What an account without readings says in their place: its `problem`, else `Không có dữ liệu`
   * as the card's rows read; only without metrics. */
  notice?: string;
  brand: string;
  /** The mark's color on the island's black, `#FFFFFF` for providers without a brand color. */
  color: string;
  /** The mark's color on a light background where it differs from `color` (Cursor's is near black
   * there), for a widget in light mode, as the popup picks the brand color for its theme. */
  lightColor?: string;
  mark?: GlanceMark;
  metrics: GlanceMetric[];
  /** The row the popup's card starts with: Codex's free reset, or Claude's banked reset for this
   * card's plan. */
  resetRow?: GlanceResetRow;
}

/**
 * The row a Codex or Claude card starts with in the popup while its reset tracking is on: Codex's
 * coming free reset (`FreeResetRow`) or the banked reset a Claude card's plan can still apply
 * (`BankedResetRow`). What moves with the clock travels as moments and words, so the row only
 * changes when the reset does: the countdown as a `GlanceCountdown`, and `{at}` in a caption for
 * the clock time and day of `at` (`11:11 · ngày mai`), worded with `GlanceDocument.labels.days`
 * when drawn.
 */
export interface GlanceResetRow {
  /** Whose tracker the row stands for, and the Reset tab view pressing it opens. */
  tracker: ResetProvider;
  /** `Reset free`, `Có thể reset` (the site's watch), `Tặng lượt để dành`, `Lượt reset để dành`. */
  title: string;
  /** The value's color: `positive` for an announced Codex reset, `notice` for a watch, `accent` for a
   * Claude banked reset; a countdown that has passed reads in the secondary color instead. */
  tone: "positive" | "notice" | "accent";
  /** The account whose picture sits before the title (`@thsottiaux`, `@ClaudeDevs`); one without a
   * picture in `GlanceDocument.avatars` shows its initial. */
  author: string;
  /** The countdown (`sau {d}`, `còn {d}`), reading `after` once its moment has passed (`chờ xác nhận`). */
  countdown?: GlanceCountdown;
  /** The value when there is no countdown (`chưa rõ giờ`). */
  value?: string;
  /** Its time in the device's zone (`Lúc {at} · GMT+7`), or why there is none. */
  caption: string;
  /** The caption once the countdown has passed (`Hẹn {at} · GMT+7`). */
  captionAfter?: string;
  /** The moment `{at}` names. */
  at?: string;
  /** The poster's own day under an estimated time (`“Ngày mai” theo giờ Mỹ`). */
  note?: string;
  /** The post and how its time was read, the popup row's hover text. */
  details: string;
  /** When the row goes. */
  hideAt: string;
  /** Pressing the row opens the Reset tab at `tracker`, as the popup's row does while that tab is on. */
  opens?: true;
}

/**
 * A clock time with its day, as the popup words the moment a reset comes (`timeOnDayLabel`): `{t}`
 * stands for the clock time in the Time Format setting and `{d}` for a day neither today nor
 * tomorrow, which `date`, a Unicode date pattern in the document's locale, draws in the device's
 * zone. The day is picked at the moment drawn, so the words stay right between documents.
 */
export interface GlanceDayWords {
  /** `{t} · hôm nay`. */
  today: string;
  /** `{t} · ngày mai`. */
  tomorrow: string;
  /** `{t} · {d}`. */
  other: string;
  /** The clock time `{t}` for the Time Format setting: `H:mm` (`8:05`), `h:mm a` (`8:05 SA`), `HH:mm`. */
  time: string;
  /** `EEEEEE dd/MM` (`T2 05/10`), `EEE, MMM d` (`Mon, Oct 5`). */
  date: string;
}

/**
 * The plan's paid period, as the popup's card header shows it in its right corner (`planTermLines`):
 * the time left over the day it ends, in the warning color from three days out. It travels as moments
 * so the document only changes when the period does; the Swift side words the time left and names
 * the day (`today`, `tomorrow` or `on`) at the moment it draws, with `GlanceDocument.labels.planTerm`.
 */
export interface GlancePlanTerm {
  /** When the period ends: the date ChatGPT states, or Claude's next monthly renewal. */
  endsAt: string;
  /** Once past this moment the corner takes the warning color (three whole days or fewer left). */
  soonAt: string;
  /** The day it ends as the popup names a day neither today nor tomorrow: `T7 17/10`, `Sat, Oct 17`. */
  on: string;
  /** Claude's renewal, worked out from the subscription start: the count and the day read with `~`,
   * and past `endsAt` the corner waits for the next document, which carries the next renewal. */
  estimated?: true;
}

/** The words of the plan-period corner, filled in at the moment drawn. */
export interface GlancePlanTermWords {
  /** The time left with `{n}` for the count (`~` goes before it for an estimate), the form for 1 then
   * the form for any other count: `còn {n} ngày`, `{n} day left` / `{n} days left`. */
  days: [string, string];
  hours: [string, string];
  minutes: [string, string];
  /** A stated period whose end has passed: `đã tới hạn`. */
  due: string;
  /** The day it ends with `{d}` for the day (`~` before it for an estimate): `tới {d}`, `{d}`. Once
   * the period has ended the day stands alone. */
  until: string;
  today: string;
  tomorrow: string;
}

/** What each account shows, per surface. */
export interface GlanceShows {
  account: boolean;
  plan: boolean;
  resets: boolean;
}

export interface GlanceIsland {
  enabled: boolean;
  /** Whether a new alert opens the island; the alert travels either way, so the island can mark
   * it seen and not show it late once alerts are back on. */
  alerts: boolean;
  style: IslandStyle;
  /** The readings beside the notch, left then right: one provider with one metric each. */
  wings: GlanceProvider[];
  expandOnHover: boolean;
  shows: GlanceShows;
  empty: string;
  /** What the open island lists; a section without data is left out. Mirrors `tabs`. */
  sections: GlanceIslandSections;
  /** The chosen views in order: behind a tab bar when `arrangement` is `tabs`, else stacked. */
  tabs: IslandView[];
  arrangement: "tabs" | "stacked";
  /** The parts of the reset tracker the reset view shows. */
  resetParts: ResetParts;
  /** The most limits coming back listed; `0` for as many as fit. */
  upcomingLimit: number;
  /** `claude` when the reset view draws `GlanceDocument.claudeResets`, `both` when it draws the Codex
   * tracker then that one; absent for Codex. */
  resetsProvider?: Exclude<SurfaceResets, "codex">;
}

/** The parts of the open island, each switched in Settings. */
export interface GlanceIslandSections {
  /** The accounts and their limits. */
  quota: boolean;
  /** The reset tracker (`GlanceDocument.resets`, or `claudeResets` when `resetsProvider` says so). */
  resets: boolean;
  /** The next limits to come back, soonest first, across the island's accounts. */
  upcoming: boolean;
}

/**
 * Wing choices that are not one metric of one account. `quota:next` counts down to the soonest
 * limit reset among the island's accounts; the `codex-resets:` ones read the Codex free-reset
 * tracker: the Codex card's "Reset free" row (or, without one, the 24-hour chance), a chance over
 * 1, 3 or 7 days, or the time since the last reset. The `claude-resets:` ones read the Claude tracker the
 * same way, its `next` counting down to the deadline of a banked reset that can still be applied.
 * The `resets:` ones read whichever of the two the Reset tab shows (`followedWing`), so they move
 * with its Codex | Claude switch like the surfaces that follow it.
 */
export const SPECIAL_WINGS = [
  "quota:next",
  "resets:next",
  "resets:chance-1",
  "resets:chance-3",
  "resets:chance-7",
  "resets:since",
  "codex-resets:next",
  "codex-resets:chance-1",
  "codex-resets:chance-3",
  "codex-resets:chance-7",
  "codex-resets:since",
  "claude-resets:next",
  "claude-resets:chance-1",
  "claude-resets:chance-3",
  "claude-resets:chance-7",
  "claude-resets:since",
] as const;
export type SpecialWing = (typeof SPECIAL_WINGS)[number];

export function isSpecialWing(id: string): id is SpecialWing {
  return (SPECIAL_WINGS as readonly string[]).includes(id);
}

/** Whether a wing reads the Claude reset tracker; a `resets:` wing does once `followedWing` says so. */
export function isClaudeResetsWing(id: string): boolean {
  return isSpecialWing(id) && id.startsWith(`${CLAUDE_RESETS_PROVIDER_ID}:`);
}

const FOLLOWING_WING = "resets:";

/**
 * The wing a `resets:` wing stands for while the Reset tab shows `provider`'s tracker (Codex while
 * the tab is hidden, as for the surfaces set to follow it): the same reading of that tracker. Any
 * other wing, and a saved id this version does not know, is itself.
 */
export function followedWing(id: string, provider: ResetProvider): string {
  if (!id.startsWith(FOLLOWING_WING) || !isSpecialWing(id)) return id;
  const tracker = provider === "claude" ? CLAUDE_RESETS_PROVIDER_ID : CODEX_RESETS_PROVIDER_ID;
  return `${tracker}:${id.slice(FOLLOWING_WING.length)}`;
}

/** What one wing slot asks for: a picked metric, a special reading, or `null` for automatic. */
export type GlanceWingChoice = WidgetDescriptor | SpecialWing | null;

export interface GlanceWidget {
  providers: GlanceProvider[];
  shows: GlanceShows;
  /** What an empty widget says, worded for the chosen content. */
  empty: string;
  /** The parts of the Overview widget, in order. */
  tabs: IslandView[];
  /** The parts of the reset tracker the reset widgets and the Overview show. */
  resetParts: ResetParts;
  /** The most limits coming back listed; `0` for as many as fit. */
  upcomingLimit: number;
  /** `claude` when the reset widgets draw `GlanceDocument.claudeResets`, `both` when they page
   * through the Codex tracker then that one; absent for Codex. */
  resetsProvider?: Exclude<SurfaceResets, "codex">;
}

export interface GlanceDocument {
  version: typeof GLANCE_VERSION;
  /** When the newest reading was fetched, not when the document was built, so an unchanged
   * reading keeps an unchanged document. */
  generatedAt: string;
  /** A POSIX locale for the Swift formatters (`vi_VN`, `en_US`). */
  locale: string;
  /** Settings → Time Format: `true` for 12-hour, `false` for 24-hour, absent to follow the locale. */
  hour12?: boolean;
  /** Settings → Theme when it is not System: every widget draws in it, as the whole popup does (the
   * island keeps its black). */
  theme?: "light" | "dark";
  /** Settings → Reduce Animations while it is on: the island opens, closes and changes its numbers
   * without animating and the widgets swap entries at once, as the popup stops its transitions;
   * absent while it is off (the surfaces still follow the Mac's own Reduce Motion). */
  reduceMotion?: true;
  /** Settings → Density when it is Compact: the open island tightens its accounts, rows and reset
   * panel by the popup's compact sizes; absent for Default. */
  density?: "compact";
  /** Settings → Reset Times when it is Exact Time: a limit's row says when it comes back
   * (`labels.resetAbsolute`) instead of counting down; absent for Countdown. */
  resetDisplay?: "absolute";
  /** Settings → Always Show Pacing while it is on: a limit on a healthy pace also carries its tick and
   * `Còn ~40% khi đặt lại`, not only one close to its limit. */
  alwaysShowPacing?: true;
  /** Settings → Used/Left when it is Used: the even-pace tick then sits at the share of the window gone
   * by, as the meters fill with the share used; absent for Left. */
  displayMode?: "used";
  labels: {
    title: string;
    empty: string;
    updated: string;
    resetsIn: string;
    resetting: string;
    /** A limit's reset text in its last five minutes, as the popup's rows say it: `Sắp đặt lại`. */
    resetsSoon: string;
    /** Countdown: the line under a limit's countdown with the moment it comes back, `{at}` standing
     * for its clock time and day as `days` words them: `Hồi lại lúc {at}`. Absent in Exact Time. */
    restoresAt?: string;
    /** Exact Time: a limit's reset text, `{t}` standing for the clock time and `{d}` for a day neither
     * today nor tomorrow as `date` draws it: `Đặt lại lúc {t} hôm nay`, `Resets {d} at {t}`. Absent in
     * Countdown. */
    resetAbsolute?: GlanceDayWords;
    /** The pace notes' words, while a limit carries a pace (`GlanceMetric.pace`). */
    pace?: GlancePaceWords;
    open: string;
    notRunning: string;
    noData: string;
    more: string;
    units: { day: string; hour: string; minute: string };
    /** What a reset widget or section says while its tracker is off (`resets` or `claudeResets`
     * absent, and no `resetsPending` or `claudeResetsPending` saying it is on its way). */
    resetsOff: string;
    /** `Sắp đặt lại`: the heading of the next limits to come back. */
    upcoming: string;
    /** What that list says when no limit has a reset time. */
    upcomingEmpty: string;
    /** The open island's tab names, as the popup's tabs read. */
    tabs: Record<IslandView, string>;
    /** The reset view's name while a surface shows the Claude tracker; absent otherwise. */
    claudeResetsTab?: string;
    /** What that view says while the Claude tracker is off, naming Claude's notifications; absent
     * otherwise, like `claudeResetsTab`. */
    claudeResetsOff?: string;
    /** The reset view's name while a surface shows both trackers, the popup's name for the Reset
     * tab; absent otherwise. */
    resetsBothTab?: string;
    /** The plan-period corner's words, while an account the island or the widget lists has a term. */
    planTerm?: GlancePlanTermWords;
    /** A reset's clock time with its day, for the reset rows and the limits coming back. */
    days: GlanceDayWords;
  };
  /** The open island's accounts. */
  providers: GlanceProvider[];
  island: GlanceIsland;
  widget: GlanceWidget;
  /** The Codex free-reset tracker, for the island's reset section and wings and the reset widgets;
   * absent while the Reset tab and reset notifications are both off, and with the notifications
   * alone only what its status says, without the history. */
  resets?: GlanceResets;
  /** What a reset surface says in place of the Codex tracker while it is on but has nothing yet;
   * absent otherwise. */
  resetsPending?: GlanceResetsPending;
  /** The Claude reset tracker (claude-resets.com), for the island or the widget when its Settings
   * chose it or both trackers (a wing reading it needs no copy here); absent otherwise, and while
   * the Reset tab and Claude reset notifications are both off. */
  claudeResets?: GlanceResets;
  /** `resetsPending` for the Claude tracker, sent while a surface shows it. */
  claudeResetsPending?: GlanceResetsPending;
  /** The pictures before the reset rows' titles, as data URLs by lowercase handle, sent once each
   * while a row names an account that has one. */
  avatars?: Record<string, string>;
  alert?: GlanceAlert;
}

/**
 * The Reset tab's own line for a tracker that is on but has nothing to show yet: `Đang tải…` until
 * its feeds answer, or `Chưa tải được: …`, which the tab draws in the notice color (`failed`).
 */
export interface GlanceResetsPending {
  text: string;
  failed?: boolean;
}

/**
 * A reset tracker: the Codex free-reset tracker (the Reset tab, from codex-resets.com) or the Claude
 * one (claude-resets.com, where `upcoming` is a banked reset's deadline), worked out and worded here
 * like the rest of the document. Everything that moves with the clock travels as a moment plus
 * words (`GlanceCountdown`), and chances as whole percents, so these fields only change when the
 * tracker's numbers do. `presentation`, the Reset tab's cards, keeps the same promise in the
 * document (`stillPresentation`): its words that move with the clock are left to Swift, which
 * words them from their moments, and its meters are kept to the whole percent they show.
 */
export interface GlanceResets {
  /** `Reset Codex`, `Reset Claude`. */
  title: string;
  /** The attribution the site asks for: `Theo codex-resets.com`. */
  source: string;
  /** The tracker's mark and color, so a reset surface can draw them without an account of it. */
  brand: string;
  color: string;
  mark?: GlanceMark;
  /** The feed could not be refreshed; the numbers are from its last good copy. */
  stale?: string;
  /** An announced reset, or the site's watch: the Codex card's "Reset free" row. For Claude, the
   * banked reset still to apply, counting down to its deadline. */
  upcoming?: GlanceUpcomingReset;
  /** The newest reset on record. */
  latest?: GlanceLatestReset;
  /** `Chance of a reset`, the three horizons (empty when there is too little history), and the
   * words under them: why there is no forecast, or that it is an estimate. */
  forecastTitle: string;
  forecast: GlanceResetChance[];
  forecastNote: string;
  /** How the current wait compares with earlier ones (`waitLine`) and the median mark. */
  wait?: string;
  median?: string;
  calendar?: GlanceResetCalendar;
  rhythm?: GlanceResetRhythm;
  presentation?: GlanceResetPresentation;
  theme?: "system" | "light" | "dark";
  /** The site the source line links to; absent for Codex, whose site the Swift side knows. */
  site?: string;
}

export interface GlanceResetAuthor {
  handle: string;
}

export interface GlanceResetStatusCard {
  id: string;
  /** `banked` (Claude only): a banked reset that can still be applied. */
  kind: "scheduled" | "watch" | "quiet" | "banked";
  level?: "elevated" | "strong";
  title: string;
  excerpt?: string;
  meta: string[];
  due?: string;
  author?: GlanceResetAuthor;
  url?: string;
  hideAt?: string;
  announced?: GlanceCountdown;
  scheduledMeta?: string;
  dueCountdown?: GlanceCountdown;
  overdueCountdown?: GlanceCountdown;
  /** The card is about the latest reset (a Claude banked reset still to apply). While the latest
   * card above it is drawn with the announcement, the card leaves out the author and the excerpt. */
  sameAsLatest?: boolean;
  /** The banked reset the card is about (Claude), which its buttons mark as used or take the mark
   * off; a string, since X's ids run past the numbers JSON keeps exact. Absent where no button
   * could act on it. */
  resetId?: string;
  /** The user marked the banked reset as used (Claude): the card folds to the line saying so, with
   * `Hoàn tác`. */
  used?: boolean;
  /** Where and how to apply the banked reset (Claude), the line after the card's meta lines. */
  how?: string;
}

/**
 * The words around the Reset tab's banked cards' buttons (`BankedCards` in `ClaudeResets.tsx`), for
 * the island and the widgets to say the same: `Tôi đã dùng rồi` and the confirmation it asks for,
 * then what a card marked as used folds to and the button that takes the mark off.
 */
export interface GlanceBankedActions {
  /** `Tôi đã dùng rồi`. */
  markUsed: string;
  /** The confirmation's title: `Đánh dấu đã dùng lượt reset này?`. */
  title: string;
  /** The confirmation's words. */
  message: string;
  /** `Xác nhận`, filled with the accent color. */
  confirm: string;
  /** `Hủy`. */
  cancel: string;
  /** What a card marked as used folds to: `Bạn đã đánh dấu lượt này là đã dùng.`. */
  used: string;
  /** `Hoàn tác`, which takes the mark off. */
  undo: string;
}

/**
 * The latest reset's card: how long ago, its moment and kind, and the words it was announced with,
 * the message the reset notification quoted, with its author and a link to the post, or the line
 * saying the site recorded it without one.
 */
export interface GlanceResetLatestPresentation {
  title: string;
  /** How long ago, in the Reset tab's words (`12 phút trước`); the glance document leaves it out,
   * since the island and the widgets word it from `at` as time passes. */
  ago?: string;
  at: string;
  meta: string;
  author?: GlanceResetAuthor;
  /** Lines under the announcement, e.g. whether the reset covers this account's plan (Claude). */
  notes?: string[];
  excerpt?: string;
  /** The whole announcement, when `excerpt` had to cut it (the popup's `Đọc tiếp`). */
  fullText?: string;
  url?: string;
  /** `Codex Resets tự ghi nhận khi reset xảy ra`, when no post announced it. */
  observed?: string;
}

export interface GlanceResetForecastPresentation {
  title: string;
  chances: { days: 1 | 3 | 7; label: string; percent: string; fraction: number }[];
  wait?: string;
  waitFraction?: number;
  median?: string;
  sampleNote?: string;
  /** How the estimate would have done on this history (the popup's Reset tab). */
  reliability?: string;
  disclaimer?: string;
  unavailable?: string;
}

export interface GlanceResetHistoryItem {
  id: string;
  kind: "regular" | "banked";
  kindLabel: string;
  when: string;
  excerpt: string;
  author?: GlanceResetAuthor;
  url?: string;
  observed?: string;
  /** Who it covered, worded (the Claude view). */
  scope?: string;
  /** `Chưa kiểm chứng`, while the site has not reviewed the entry. */
  provisional?: string;
}

/** A limit change (Claude): who posted it, when, the words, whom it covered, and whether the site reviewed it. */
export interface GlanceResetChangeItem {
  id: string;
  when: string;
  excerpt: string;
  author: GlanceResetAuthor;
  url?: string;
  scope?: string;
  /** `Chưa kiểm chứng`, while the site has not reviewed the entry. */
  provisional?: string;
}

/** One side of the comparison as its column heading shows it: the tracker's name, mark and color. */
export interface GlanceResetCompareColumn {
  name: string;
  color: string;
  mark?: GlanceMark;
}

/**
 * Claude against Codex over the time both were tracked, the last card of the Reset tab's Claude
 * view: a row per measure with each side's value, then the resets of each month side by side.
 */
export interface GlanceResetCompare {
  title: string;
  /** `Tính các lần reset sau …`, under the months. */
  since: string;
  /** The surfaces' column headings; the popup names and marks the two sides itself. */
  columns?: { claude: GlanceResetCompareColumn; codex: GlanceResetCompareColumn };
  rows: { label: string; claude: string; codex: string }[];
  monthsTitle: string;
  /** Each month's count on either side, oldest first, and what VoiceOver reads for it. */
  months: { label: string; claude: number; codex: number; summary: string }[];
}

export interface GlanceResetPresentation {
  locale: string;
  authorAvatar: string;
  /** The one account `authorAvatar` pictures (Claude: `@ClaudeDevs`); absent, it pictures every author. */
  avatarHandle?: string;
  /** Lines above the cards (Claude): the site is behind, or its published copy is shown. */
  notices?: string[];
  latest?: GlanceResetLatestPresentation;
  statuses: GlanceResetStatusCard[];
  /** The words of the banked cards' buttons (Claude), while one of `statuses` is a banked card. */
  bankedActions?: GlanceBankedActions;
  quietTitle?: string;
  forecast: GlanceResetForecastPresentation;
  statsTitle: string;
  stats: { label: string; value: string }[];
  historyTitle: string;
  history: GlanceResetHistoryItem[];
  /** The limit changes (Claude), listed apart from the history since they reset nothing: the
   * heading, the word on each row's badge, the changes newest first and the note under them. */
  changesTitle?: string;
  changeBadge?: string;
  changes?: GlanceResetChangeItem[];
  changesNote?: string;
  /** Claude against Codex (Claude), once the Codex history is at hand. */
  compare?: GlanceResetCompare;
  patternNote: string;
  /** When the copy shown was read (`Tải 5 phút trước`, `Vừa tải`), the line above the source. */
  fetched?: GlanceCountdown;
  source: string;
  methodTitle: string;
  method: string[];
}

export interface GlanceUpcomingReset {
  /** `Reset free`, `Có thể reset` (a watch), `Tặng lượt để dành` (banked), Claude's `Lượt reset để dành`. */
  title: string;
  /** Scheduled resets and Claude's banked reset read positive, a watch reads as a notice. */
  tone: "positive" | "notice";
  /** The countdown to its time (`sau ~{d}`), with the "waiting for confirmation" word after it. */
  countdown?: GlanceCountdown;
  /** The value when there is no countdown (`chưa rõ giờ`). */
  value?: string;
  /** Its time in the device's zone (`Lúc 11:11 · ngày mai · GMT+7`) or why there is none. */
  caption: string;
  /** The caption once the countdown has passed (`Hẹn 11:11 · GMT+7`). */
  captionAfter?: string;
  /** The poster's own day under an estimated time (`“Ngày mai” theo giờ Mỹ`). */
  note?: string;
  /** When it stops showing. */
  hideAt: string;
  /** The watch's own chance, 0...100. */
  chancePercent?: number;
}

export interface GlanceLatestReset {
  at: string;
  kind: "regular" | "banked";
  /** `Lần gần nhất`. */
  label: string;
  /** `Reset thường` / `Lượt để dành`. */
  kindLabel: string;
  /** The time since it, e.g. `{d} chưa có reset` (a `since` countdown from `at`). */
  since: GlanceCountdown;
  /** Its moment in the device's zone and clock (`T6 26/09 · 01:17`). */
  when: string;
}

export interface GlanceResetChance {
  days: 1 | 3 | 7;
  /** Whole percent, 0...100. */
  percent: number;
  /** `24 giờ tới`, `3 ngày tới`, `7 ngày tới`. */
  label: string;
}

/** The Reset tab's calendar: one cell per day, oldest week first, Monday to Sunday. */
export interface GlanceResetCalendar {
  title: string;
  weeks: number;
  /** `weeks × 7` characters: `.` no reset, `r` a regular reset, `b` a banked one, `-` a day not
   * come yet. */
  cells: string;
  /** The cell of today. */
  today: number;
  /** Monday to Sunday (`T2`…`CN`). */
  weekdays: string[];
  /** A month's short name at the first week it starts in. */
  months: { week: number; label: string }[];
  legend: { regular: string; banked: string; today: string };
}

/** When resets were announced, by weekday and by four-hour block of the day, in the device's zone. */
export interface GlanceResetRhythm {
  title: string;
  total: number;
  weekdayTitle: string;
  weekdays: { label: string; count: number }[];
  hourTitle: string;
  hours: { label: string; count: number }[];
}

export interface GlanceAlert {
  id: string;
  title: string;
  body: string;
  brand?: string;
  severity: GlanceSeverity;
}

/** How an account card introduces itself, as the popup's card header does. */
export interface GlanceProviderText {
  name: string;
  account?: string | null;
  plan?: string | null;
  /** The header notice's first line, behind the header's warning triangle. */
  notice?: string | null;
  planTerm?: PlanTerm | null;
}

export interface GlanceInput {
  island: {
    groups: readonly ProviderMetrics[];
    settings: IslandSettings;
    enabled: boolean;
    /** The wing readings picked in Settings, left then right (`null` where the slot is automatic). */
    wings: readonly [GlanceWingChoice, GlanceWingChoice];
  };
  widget: {
    groups: readonly ProviderMetrics[];
    settings: GlanceSurfaceSettings;
  };
  dataFor: (descriptor: WidgetDescriptor) => WidgetData;
  describe: (provider: Provider) => GlanceProviderText;
  /** The provider a picked wing metric belongs to. */
  providerOf: (providerId: string) => Provider | undefined;
  /** When a provider's snapshot was fetched (ISO), if it has one. */
  refreshedAt: (providerId: string) => string | undefined;
  /** How often the core refreshes, which says when a reading is outdated. */
  refreshIntervalMs: number;
  /** The cards open on the Hạn mức tab (`layout.openProviders`): a surface following that tab lists
   * a card's rows behind its show-more button only while the card is open, as the tab shows them. */
  openProviders: readonly string[];
  /** The row a card starts with in the popup (`GlanceResetRow`), `null` for none; absent, no card has one. */
  resetRowFor?: (provider: Provider) => GlanceResetRow | null;
  /** The popup's settings its rows follow beyond the language: Used/Left, Reset Times and Always
   * Show Pacing. */
  display: Pick<DisplayOptions, "displayMode" | "resetDisplayMode" | "alwaysShowPacing">;
  language: Language;
  /** Settings → Time Format as a 12-hour flag, `null` for the locale's own clock. */
  hour12: boolean | null;
  /** Settings → Theme. */
  theme: ThemeSetting;
  /** Settings → Reduce Animations; off when left out. */
  reduceAnimations?: boolean;
  /** Settings → Density; Default when left out. */
  density?: DensitySetting;
  appName: string;
  alert: GlanceAlert | null;
  /** The Codex free-reset tracker (`buildGlanceResets`), `null` while it is off or has no data. */
  resets: GlanceResets | null;
  /** Why a Codex tracker that is on has no data yet (`trackerPending`); taken only without one. */
  resetsPending?: GlanceResetsPending | null;
  /** The Claude reset tracker (`buildClaudeGlanceResets`), `null` or absent while neither a surface
   * nor a wing uses it, it is off or it has no data. */
  claudeResets?: GlanceResets | null;
  /** Why a Claude tracker that is on has no data yet; taken only without one, for a surface showing it. */
  claudeResetsPending?: GlanceResetsPending | null;
  /** The tracker the Reset tab shows (Codex while the tab is hidden), which the `resets:` wings
   * read; Codex when left out. */
  resetsTab?: ResetProvider;
  /** The official color logos drawn so far (`useMarkArt`), base64 PNG by brand; optional. */
  markArt?: Readonly<Record<string, string>>;
  /** Whether the app can spend a Codex limit reset (`backend().redeemLimitReset`), which puts the
   * popup's "Dùng 1 lượt" under a connected Codex account's reset credits; absent, no row has it. */
  redeemsResets?: boolean;
  now: Date;
}

const BRAND_COLORS: Readonly<Record<string, string>> = { claude: SOURCE_COLORS.claude, codex: SOURCE_COLORS.codex };
const PLAIN_MARK_COLOR = "#FFFFFF";
const LOCALES: Readonly<Record<Language, string>> = { vi: "vi_VN", en: "en_US" };
const DAY_MS = 86_400_000;
const COUNT_PLACEHOLDER = "{n}";
const DAY_PLACEHOLDER = "{d}";
const TIME_PLACEHOLDER = "{t}";
/** A figure no pace note writes on its own, put in its words to find where `{n}` goes. */
const FIGURE_SENTINEL = 7919;

export function glanceMetric(id: string, data: WidgetData, now: Date): GlanceMetric {
  const bounded = isBounded(data);
  const counting = bounded && data.resetsAt !== null && data.subtitleOverride === undefined && !isFreshSessionWindow(data, now);
  const state = bounded ? meterState(data, now) : null;
  const metric: GlanceMetric = {
    id,
    label: data.title,
    value: menuBarValue(data),
    headline: bounded ? boundedHeadline(data) : unboundedDetail(data),
    fraction: bounded ? fraction(data) : null,
    severity: state ? (meterSeverity(state) ?? "none") : "none",
  };
  if (counting && data.resetsAt) metric.resetsAt = data.resetsAt.toISOString();
  else if (bounded) {
    const detail = boundedTrailingText(data, now);
    if (detail) metric.detail = detail;
  }
  if (!bounded && data.hasData && data.expiriesAt.length > 0) metric.expiresAt = new Date(Math.min(...data.expiriesAt.map((date) => date.getTime()))).toISOString();
  const pace = state ? glancePace(data, state, counting) : null;
  if (pace) metric.pace = pace;
  if (counting) metric.after = glanceAfterReset(data, now);
  return metric;
}

/**
 * `GlanceMetric.pace`: `spent` for a limit used up; for a limit counting down whose window the popup
 * can pace (something used, the window's length known), the share used and that length.
 */
function glancePace(data: WidgetData, state: MeterState, counting: boolean): GlancePace | null {
  if (state.kind === "spent") return { spent: true };
  if (!counting || data.limit === null || !(data.limit > 0) || !(data.used > 0)) return null;
  if (data.periodDurationMs === undefined || !(data.periodDurationMs > 0)) return null;
  return { used: data.used / data.limit, period: data.periodDurationMs };
}

/** `GlanceMetric.after`: the row as the popup reads it once its window has rolled over. */
function glanceAfterReset(data: WidgetData, now: Date): GlanceAfterReset {
  const rolled = rolledOverReading(data);
  const after: GlanceAfterReset = { value: menuBarValue(rolled), headline: boundedHeadline(rolled), fraction: fraction(rolled) };
  const detail = boundedTrailingText(rolled, now);
  if (detail) after.detail = detail;
  const severity = meterSeverity(meterState(rolled, now)) ?? "none";
  if (severity !== "normal") after.severity = severity;
  return after;
}

function shows(settings: GlanceSurfaceSettings): GlanceShows {
  return { account: settings.showAccount, plan: settings.showPlan, resets: settings.showResets };
}

/**
 * The plan's paid period as the header corner reads it at `now`: when it ends (Claude's next monthly
 * renewal, else the stated date), when it turns to the warning color, and its day as a date.
 */
export function glancePlanTerm(term: PlanTerm, now: Date, language: Language): GlancePlanTerm | null {
  const end = planTermEnd(term, now);
  if (!end) return null;
  const entry: GlancePlanTerm = {
    endsAt: end.endsAt.toISOString(),
    soonAt: new Date(end.endsAt.getTime() - (PLAN_TERM_SOON_DAYS + 1) * DAY_MS).toISOString(),
    on: messagesFor(language).dashboard.planTermDay({ kind: "on", date: end.endsAt }, false, true),
  };
  if (end.estimated) entry.estimated = true;
  return entry;
}

/**
 * The popup's plan-period words (`planTermLeft`, `planTermDay`) with `{n}` where the count goes and
 * `{d}` where the day goes, read off the words the popup itself renders so the two never drift.
 */
export function glancePlanTermWords(language: Language): GlancePlanTermWords {
  const text = messagesFor(language).dashboard;
  const left = (kind: "days" | "hours" | "minutes"): [string, string] => [
    text.planTermLeft({ kind, count: 1 }, false).replace("1", COUNT_PLACEHOLDER),
    text.planTermLeft({ kind, count: 2 }, false).replace("2", COUNT_PLACEHOLDER),
  ];
  const today = text.planTermDay({ kind: "today" }, false, true);
  return {
    days: left("days"),
    hours: left("hours"),
    minutes: left("minutes"),
    due: text.planTermLeft({ kind: "due" }, false),
    until: text.planTermDay({ kind: "today" }, false, false).replace(today, DAY_PLACEHOLDER),
    today,
    tomorrow: text.planTermDay({ kind: "tomorrow" }, false, true),
  };
}

/**
 * The popup's words for a clock time and its day (`format.timeOnDay`) with `{t}` where the time
 * goes and `{d}` where another day goes, read off the words the popup renders so the two never
 * drift, with the patterns that draw the time as `shortTime` does for `timeFormat` and another day
 * as `format.day` does.
 */
export function glanceDayWords(language: Language, timeFormat: TimeFormat): GlanceDayWords {
  const messages = messagesFor(language);
  const format = messages.format;
  const today = format.day({ kind: "today" });
  return {
    today: format.timeOnDay(TIME_PLACEHOLDER, { kind: "today" }),
    tomorrow: format.timeOnDay(TIME_PLACEHOLDER, { kind: "tomorrow" }),
    other: format.timeOnDay(TIME_PLACEHOLDER, { kind: "today" }).replace(today, DAY_PLACEHOLDER),
    time: messages.glance.clockPattern(usesTwentyFourHour(timeFormat, language)),
    date: messages.glance.dayPattern,
  };
}

/**
 * The popup's exact reset time (`resetAbsoluteLabel`, what a limit's row says in Exact Time) with
 * `{t}` where the clock time goes and `{d}` where a day neither today nor tomorrow goes, read off
 * the words the popup renders, with the patterns that draw the time as `shortTime` does for
 * `timeFormat` and that day as `format.monthDay` does.
 */
export function glanceResetAbsoluteWords(language: Language, timeFormat: TimeFormat): GlanceDayWords {
  const messages = messagesFor(language);
  const resets = (when: When) => messages.format.deadline("resets", when);
  return {
    today: resets({ kind: "today", time: TIME_PLACEHOLDER }),
    tomorrow: resets({ kind: "tomorrow", time: TIME_PLACEHOLDER }),
    other: resets({ kind: "on", date: DAY_PLACEHOLDER, time: TIME_PLACEHOLDER }),
    time: messages.glance.clockPattern(usesTwentyFourHour(timeFormat, language)),
    date: messages.glance.monthDayPattern,
  };
}

/**
 * The popup's line under a reset countdown (`restoreLabel`) with `{at}` where the moment goes, as
 * `glanceDayWords` words a clock time and its day: `Hồi lại lúc {at}`.
 */
export function glanceRestoreWords(language: Language): string {
  const format = messagesFor(language).format;
  const today = { kind: "today" } as const;
  return format.restoresAt(TIME_PLACEHOLDER, today).replace(format.timeOnDay(TIME_PLACEHOLDER, today), MOMENT_PLACEHOLDER);
}

/**
 * The words `{at}` of the redemption's confirmation is filled with: a credit's expiry as `whenLabel`
 * words it in Exact Time (`13:05 hôm nay`, `1:05 PM tomorrow`, `13:05 ngày 5/10`), read off the
 * words the popup renders, with the patterns that draw the time as `shortTime` does for
 * `timeFormat` and another day as `format.monthDay` does.
 */
export function glanceExpiryWords(language: Language, timeFormat: TimeFormat): GlanceDayWords {
  const messages = messagesFor(language);
  const when = messages.format.when;
  return {
    today: when({ kind: "today", time: TIME_PLACEHOLDER }),
    tomorrow: when({ kind: "tomorrow", time: TIME_PLACEHOLDER }),
    other: when({ kind: "on", date: DAY_PLACEHOLDER, time: TIME_PLACEHOLDER }),
    time: messages.glance.clockPattern(usesTwentyFourHour(timeFormat, language)),
    date: messages.glance.monthDayPattern,
  };
}

/** An account the island and the widgets may ask to spend a reset of, as `glanceActions` accepts it. */
export const GLANCE_ACTION_PROVIDER_ID = /^[a-z0-9-]{1,64}@[A-Za-z0-9_-]{1,128}$/;

/** A banked reset the island and the widgets may ask to mark as used, as `glanceActions` accepts it. */
export const GLANCE_ACTION_RESET_ID = /^[A-Za-z0-9_-]{1,64}$/;

/**
 * `GlanceMetric.redeem` for the row `data` of `providerId`, where the popup's card shows "Dùng 1
 * lượt" under it (`canRedeemReset`, not on a local-history card) and a surface's press would be
 * acted on (a connected Codex account); `null` elsewhere.
 */
export function glanceRedeem(providerId: string, data: WidgetData, now: Date): GlanceRedeem | null {
  if (availableResets(data) < 1 || isLocalHistoryCard(providerId) || layoutFamily(providerId) !== "codex" || !GLANCE_ACTION_PROVIDER_ID.test(providerId)) return null;
  const messages = messagesFor(data.language);
  const text = messages.limitReset;
  const redeem: GlanceRedeem = {
    providerId,
    redeem: text.redeem,
    redeeming: text.redeeming,
    title: text.confirmTitle,
    message: text.confirmMessage(null),
    confirm: text.confirm,
    cancel: messages.chrome.cancel,
  };
  const expiry = soonestExpiry(data.expiriesAt, now);
  if (expiry) {
    redeem.messageAt = text.confirmMessage(MOMENT_PLACEHOLDER);
    redeem.expiresAt = expiry.toISOString();
    redeem.expiry = glanceExpiryWords(data.language, data.timeFormat);
  }
  return redeem;
}

/** The popup's pace notes (`PaceWarning`) with `{n}` where the figure goes, read off its words. */
export function glancePaceWords(language: Language): GlancePaceWords {
  const meter = messagesFor(language).meter;
  const figure = (words: string) => words.replace(String(FIGURE_SENTINEL), COUNT_PLACEHOLDER);
  return { limitReached: meter.limitReached, spare: figure(meter.spare(FIGURE_SENTINEL)), leftAtReset: figure(meter.leftAtReset(FIGURE_SENTINEL)) };
}

export function buildGlance(input: GlanceInput): GlanceDocument {
  const messages = messagesFor(input.language);
  const text = messages.glance;
  const timeFormat: TimeFormat = input.hour12 === null ? "auto" : input.hour12 ? "12h" : "24h";
  const times: number[] = [];

  const provider = (source: Provider, metrics: GlanceMetric[]): GlanceProvider => {
    const brand = brandOf(source.icon || source.id);
    const mark = PROVIDER_MARKS[brand];
    const about = input.describe(source);
    const color = BRAND_COLORS[brand] ?? knownBrandColor(brand, true) ?? PLAIN_MARK_COLOR;
    const entry: GlanceProvider = { id: source.id, name: about.name, brand, color, metrics };
    const light = BRAND_COLORS[brand] ?? knownBrandColor(brand, false);
    if (light && light !== color) entry.lightColor = light;
    const art = input.markArt?.[brand];
    if (mark) entry.mark = art ? { ...mark, art } : mark;
    if (about.account) entry.account = about.account;
    if (about.plan) entry.plan = about.plan;
    const term = about.planTerm ? glancePlanTerm(about.planTerm, input.now, input.language) : null;
    if (term) entry.term = term;
    if (isOutdated(input.refreshedAt(source.id), input.refreshIntervalMs, input.now)) entry.outdated = messages.meter.outdated;
    if (about.notice) entry.problem = about.notice;
    if (metrics.length === 0) entry.notice = about.notice ?? messages.meter.noData;
    return entry;
  };

  const periods = new Map<string, string>();
  const readings = (descriptors: readonly WidgetDescriptor[], providerId?: string): GlanceMetric[] =>
    descriptors.flatMap((descriptor) => {
      const data = input.dataFor(descriptor);
      if (!data.hasData) return [];
      const period = periodLabel(data.periodDurationMs);
      if (period) periods.set(descriptor.id, period);
      const metric = glanceMetric(descriptor.id, data, input.now);
      const redeem = providerId && input.redeemsResets ? glanceRedeem(providerId, data, input.now) : null;
      if (redeem) metric.redeem = redeem;
      return [metric];
    });

  const list = (groups: readonly ProviderMetrics[], content: GlanceContent, problems: boolean): GlanceProvider[] =>
    (content === "dashboard" ? cardsAsShown(groups, input.openProviders) : groups).flatMap((group) => {
      const metrics = readings([...group.always, ...group.onDemand], group.provider.id);
      if (metrics.length === 0 && !problems) return [];
      if (metrics.length > 0) {
        const refreshed = Date.parse(input.refreshedAt(group.provider.id) ?? "");
        if (Number.isFinite(refreshed)) times.push(refreshed);
      }
      const entry = provider(group.provider, metrics);
      const row = input.resetRowFor?.(group.provider);
      if (row) entry.resetRow = row;
      return [entry];
    });

  const islandProviders = list(input.island.groups, input.island.settings.content, input.island.settings.showProblems).slice(0, ISLAND_PROVIDER_LIMIT);
  const widgetProviders = list(input.widget.groups, input.widget.settings.content, input.widget.settings.showProblems);

  const document: GlanceDocument = {
    version: GLANCE_VERSION,
    generatedAt: new Date(times.length > 0 ? Math.max(...times) : input.now.getTime()).toISOString(),
    locale: LOCALES[input.language],
    labels: {
      title: input.appName,
      empty: text.empty.starred,
      updated: text.updated,
      resetsIn: text.resetsIn,
      resetting: text.resetting,
      resetsSoon: messages.format.deadline("resets", { kind: "soon" }),
      open: text.open,
      notRunning: text.notRunning,
      noData: messages.meter.noData,
      more: text.more,
      units: text.units,
      resetsOff: text.resetsOff,
      upcoming: text.upcoming,
      upcomingEmpty: text.upcomingEmpty,
      tabs: { ...text.tabs },
      days: glanceDayWords(input.language, timeFormat),
    },
    providers: islandProviders,
    island: {
      enabled: input.island.enabled,
      alerts: input.island.settings.alerts,
      style: input.island.settings.style,
      wings: wings(input, islandProviders, provider, periods),
      expandOnHover: input.island.settings.expandOnHover,
      shows: shows(input.island.settings),
      empty: text.empty[input.island.settings.content],
      sections: islandSections(input.island.settings.tabs),
      tabs: [...input.island.settings.tabs],
      arrangement: input.island.settings.layout === "combined" ? "stacked" : "tabs",
      resetParts: { ...input.island.settings.resetParts },
      upcomingLimit: input.island.settings.upcomingLimit,
    },
    widget: {
      providers: widgetProviders,
      shows: shows(input.widget.settings),
      empty: text.empty[input.widget.settings.content],
      tabs: [...input.widget.settings.tabs],
      resetParts: { ...input.widget.settings.resetParts },
      upcomingLimit: input.widget.settings.upcomingLimit,
    },
  };
  const islandResets = input.island.settings.resetsProvider;
  const widgetResets = input.widget.settings.resetsProvider;
  const claudeIsland = readsClaudeResets(islandResets);
  const claudeWidget = readsClaudeResets(widgetResets);
  if (islandResets === "claude" || islandResets === "both") document.island.resetsProvider = islandResets;
  if (widgetResets === "claude" || widgetResets === "both") document.widget.resetsProvider = widgetResets;
  if (claudeIsland || claudeWidget) {
    document.labels.claudeResetsTab = insightsFor(input.language).claude.glanceTitle;
    document.labels.claudeResetsOff = text.claudeResetsOff;
  }
  if (islandResets === "both" || widgetResets === "both") document.labels.resetsBothTab = messages.dashboard.tab("resets");
  if ([...islandProviders, ...widgetProviders].some((entry) => entry.term)) document.labels.planTerm = glancePlanTermWords(input.language);
  if (input.display.resetDisplayMode === "absolute") {
    document.resetDisplay = "absolute";
    document.labels.resetAbsolute = glanceResetAbsoluteWords(input.language, timeFormat);
  } else {
    document.labels.restoresAt = glanceRestoreWords(input.language);
  }
  if ([...islandProviders, ...widgetProviders, ...document.island.wings].some((entry) => entry.metrics.some((metric) => metric.pace))) {
    document.labels.pace = glancePaceWords(input.language);
  }
  if (input.display.alwaysShowPacing) document.alwaysShowPacing = true;
  if (input.display.displayMode === "used") document.displayMode = "used";
  if (input.hour12 !== null) document.hour12 = input.hour12;
  if (input.theme !== "system") document.theme = input.theme;
  if (input.reduceAnimations) document.reduceMotion = true;
  if (input.density === "compact") document.density = "compact";
  if (input.resets) document.resets = input.resets;
  else if (input.resetsPending) document.resetsPending = input.resetsPending;
  if (claudeIsland || claudeWidget) {
    if (input.claudeResets) document.claudeResets = input.claudeResets;
    else if (input.claudeResetsPending) document.claudeResetsPending = input.claudeResetsPending;
  }
  const avatars = resetRowAvatars([...islandProviders, ...widgetProviders]);
  if (avatars) document.avatars = avatars;
  if (input.alert) document.alert = input.alert;
  return document;
}

/**
 * The two readings beside the notch. A slot picked in Settings shows that metric, or that special
 * reading (a `resets:` one from the tracker the Reset tab shows), while it has one; an automatic
 * slot, or a pick without data, takes the next reading of the island's accounts: each account's
 * first metric, then a lone account's second. A reading of a limit window carries the window's
 * short name (`periods`, by metric), as the menu bar strip labels it.
 */
function wings(
  input: GlanceInput,
  providers: readonly GlanceProvider[],
  make: (source: Provider, metrics: GlanceMetric[]) => GlanceProvider,
  periods: ReadonlyMap<string, string>,
): GlanceProvider[] {
  const picked = input.island.wings.map((choice) => {
    if (!choice) return null;
    if (typeof choice === "string") {
      const wing = followedWing(choice, input.resetsTab ?? "codex");
      return isSpecialWing(wing) ? specialWing(wing, providers, input) : null;
    }
    const source = input.providerOf(choice.providerId);
    const data = input.dataFor(choice);
    if (!source || !data.hasData) return null;
    return make(source, [withPeriod(glanceMetric(choice.id, data, input.now), periodLabel(data.periodDurationMs))]);
  });
  const used = new Set(picked.flatMap((wing) => (wing ? [`${wing.id}|${wing.metrics[0]!.id}`] : [])));
  const firsts = providers.flatMap((entry) => (entry.metrics[0] ? [{ entry, metric: entry.metrics[0] }] : []));
  const seconds = providers.length === 1 ? providers.flatMap((entry) => entry.metrics.slice(1).map((metric) => ({ entry, metric }))) : [];
  const automatic = [...firsts, ...seconds].filter(({ entry, metric }) => !used.has(`${entry.id}|${metric.id}`));
  const result: GlanceProvider[] = [];
  for (const wing of picked) {
    if (wing) {
      result.push(wing);
      continue;
    }
    const next = automatic.shift();
    if (next) result.push(wingOf(next.entry, withPeriod(next.metric, periods.get(next.metric.id))));
  }
  return result;
}

function withPeriod(metric: GlanceMetric, period: string | null | undefined): GlanceMetric {
  return period ? { ...metric, period } : metric;
}

/** An island account beside the notch: its header and one reading, without the row its card starts with. */
function wingOf(entry: GlanceProvider, metric: GlanceMetric): GlanceProvider {
  const { resetRow: _row, ...rest } = entry;
  return { ...rest, metrics: [metric] };
}

/** The provider standing for the Codex free-reset tracker in a wing. */
export const CODEX_RESETS_PROVIDER_ID = "codex-resets";
/** The provider standing for the Claude reset tracker in a wing. */
export const CLAUDE_RESETS_PROVIDER_ID = "claude-resets";

/**
 * A wing that is not one picked metric, or `null` when it has nothing to show (the slot then fills
 * automatically). `quota:next` is the island's account whose limit comes back first, counting down
 * to it; the reset wings read their tracker as a provider of their own. Codex's `next` reads the
 * Codex card's "Reset free" row word for word while the row shows (`sau ~2 giờ`, `trong 5 giờ
 * tới`, then `chờ xác nhận`, or `chưa rõ giờ` without a time); Claude's, which announces none
 * ahead, counts down to a banked reset's deadline. Without either, `next` reads the 24-hour chance.
 */
function specialWing(wing: SpecialWing, providers: readonly GlanceProvider[], input: GlanceInput): GlanceProvider | null {
  if (wing === "quota:next") return soonestReset(providers, input.now, input.language);
  const [trackerId, reading] = wing.split(":") as [string, "next" | "chance-1" | "chance-3" | "chance-7" | "since"];
  const resets = trackerId === CLAUDE_RESETS_PROVIDER_ID ? input.claudeResets : input.resets;
  if (!resets) return null;
  const text = messagesFor(input.language).glance;
  const tracker = (metric: GlanceMetric): GlanceProvider => {
    const entry: GlanceProvider = { id: trackerId, name: resets.title, brand: resets.brand, color: resets.color, metrics: [metric] };
    if (resets.mark) entry.mark = resets.mark;
    return entry;
  };
  const chance = (days: 1 | 3 | 7): GlanceProvider | null => {
    const found = resets.forecast.find((entry) => entry.days === days);
    if (!found) return null;
    const value = `${found.percent}%`;
    return tracker({ id: wing, label: found.label, value, headline: `${value} · ${found.label}`, fraction: found.percent / 100, severity: "normal" });
  };
  switch (reading) {
    case "next": {
      const upcoming = resets.upcoming;
      if (!upcoming || Date.parse(upcoming.hideAt) <= input.now.getTime()) return chance(1);
      const countdown = upcoming.countdown;
      const row = (value: string): GlanceMetric => ({ id: wing, label: upcoming.title, value, headline: upcoming.caption, fraction: null, severity: "normal" });
      if (!countdown) return tracker(row(upcoming.value ?? upcoming.title));
      const moving: GlanceCountdown = { at: countdown.at, text: countdown.text };
      if (countdown.after !== undefined) moving.after = countdown.after;
      return tracker({ ...row(upcoming.title), countdown: moving });
    }
    case "chance-1":
      return chance(1);
    case "chance-3":
      return chance(3);
    case "chance-7":
      return chance(7);
    case "since": {
      const latest = resets.latest;
      if (!latest) return null;
      return tracker({
        id: wing,
        label: text.sinceReset,
        value: latest.when,
        headline: `${latest.label} · ${latest.when}`,
        fraction: null,
        severity: "normal",
        countdown: { at: latest.at, text: text.wingSince(COUNTDOWN_SPAN), since: true },
      });
    }
  }
}

/** Which views the open island has, whichever way it arranges them. */
function islandSections(tabs: readonly IslandView[]): GlanceIslandSections {
  return { quota: tabs.includes("quota"), resets: tabs.includes("resets"), upcoming: tabs.includes("upcoming") };
}

/** The island's metric whose limit comes back first after `now`, counting down to it. */
function soonestReset(providers: readonly GlanceProvider[], now: Date, language: Language): GlanceProvider | null {
  let soonest: { entry: GlanceProvider; metric: GlanceMetric; at: number } | null = null;
  for (const entry of providers) {
    for (const metric of entry.metrics) {
      const at = metric.resetsAt ? Date.parse(metric.resetsAt) : Number.NaN;
      if (at > now.getTime() && (!soonest || at < soonest.at)) soonest = { entry, metric, at };
    }
  }
  if (!soonest) return null;
  return wingOf(soonest.entry, { ...soonest.metric, countdown: { at: soonest.metric.resetsAt!, text: messagesFor(language).glance.wingIn(COUNTDOWN_SPAN) } });
}
