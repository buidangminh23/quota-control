/**
 * The document the macOS Dynamic Island and desktop widgets draw (`src-tauri/macos/Shared/Glance.swift`
 * decodes it). Each surface lists the account cards and metrics its Settings choose (the Hạn mức
 * cards, the starred metrics, or a hand-picked set), exactly as the popup reads them: every headline,
 * meter fill, pace color and reset time is worked out and localized here, following Used/Left, so the
 * Swift side only lays it out and ticks the countdowns.
 */
import { PROVIDER_MARKS, type ProviderMark } from "@/assets/providerMarks";
import { knownBrandColor } from "./totalSpend";
import { messagesFor, type Language } from "@/i18n";
import type { Provider, WidgetDescriptor } from "@/lib/types";
import { COUNTDOWN_SPAN } from "./glanceResets";
import { brandOf, type ProviderMetrics } from "./layout";
import { boundedTrailingText, isFreshSessionWindow, meterSeverity, meterState } from "./meterState";
import { SOURCE_COLORS } from "./palette";
import type { GlanceSurfaceSettings, IslandSettings, IslandStyle } from "./settings";
import { boundedHeadline, fraction, isBounded, menuBarValue, unboundedDetail, type WidgetData } from "./widgetData";

export const GLANCE_VERSION = 1;
/** The open island lists at most this many accounts; the widget decides what fits its size. */
const ISLAND_PROVIDER_LIMIT = 6;

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
  /** The account's email, which the card shows under its name. */
  account?: string;
  /** The plan badge (`Pro`, `Plus`). */
  plan?: string;
  /** Why an account without readings shows none (signed out, session expired); only without metrics. */
  notice?: string;
  brand: string;
  /** The mark's color on the island's black, `#FFFFFF` for providers without a brand color. */
  color: string;
  mark?: GlanceMark;
  metrics: GlanceMetric[];
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
  /** What the open island lists, top to bottom; a section without data is left out. */
  sections: GlanceIslandSections;
}

/** The parts of the open island, each switched in Settings. */
export interface GlanceIslandSections {
  /** The accounts and their limits. */
  quota: boolean;
  /** The Codex free-reset tracker (`GlanceDocument.resets`). */
  resets: boolean;
  /** The next limits to come back, soonest first, across the island's accounts. */
  upcoming: boolean;
}

/**
 * Wing choices that are not one metric of one account. `quota:next` counts down to the soonest
 * limit reset among the island's accounts; the `codex-resets:` ones read the Codex free-reset
 * tracker: the announced reset's countdown (or, without one, the 24-hour chance), a chance over 1, 3
 * or 7 days, or the time since the last reset.
 */
export const SPECIAL_WINGS = [
  "quota:next",
  "codex-resets:next",
  "codex-resets:chance-1",
  "codex-resets:chance-3",
  "codex-resets:chance-7",
  "codex-resets:since",
] as const;
export type SpecialWing = (typeof SPECIAL_WINGS)[number];

export function isSpecialWing(id: string): id is SpecialWing {
  return (SPECIAL_WINGS as readonly string[]).includes(id);
}

/** What one wing slot asks for: a picked metric, a special reading, or `null` for automatic. */
export type GlanceWingChoice = WidgetDescriptor | SpecialWing | null;

export interface GlanceWidget {
  providers: GlanceProvider[];
  shows: GlanceShows;
  /** What an empty widget says, worded for the chosen content. */
  empty: string;
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
  labels: {
    title: string;
    empty: string;
    updated: string;
    resetsIn: string;
    resetting: string;
    open: string;
    notRunning: string;
    noData: string;
    more: string;
    units: { day: string; hour: string; minute: string };
    /** What a reset widget or section says while the tracker is off (`GlanceDocument.resets` absent). */
    resetsOff: string;
    /** `Sắp đặt lại`: the heading of the next limits to come back. */
    upcoming: string;
    /** What that list says when no limit has a reset time. */
    upcomingEmpty: string;
  };
  /** The open island's accounts. */
  providers: GlanceProvider[];
  island: GlanceIsland;
  widget: GlanceWidget;
  /** The Codex free-reset tracker, for the island's reset section and wings and the reset widgets;
   * absent while the Reset tab and reset notifications are both off. */
  resets?: GlanceResets;
  alert?: GlanceAlert;
}

/**
 * The Codex free-reset tracker (the Reset tab, from codex-resets.com), worked out and worded here
 * like the rest of the document. Everything that moves with the clock travels as a moment plus
 * words (`GlanceCountdown`), and chances as whole percents, so the document only changes when the
 * tracker's numbers do.
 */
export interface GlanceResets {
  /** `Reset Codex`. */
  title: string;
  /** The attribution the site asks for: `Theo codex-resets.com`. */
  source: string;
  /** The Codex mark and color, so a reset surface can draw them without a Codex account. */
  brand: string;
  color: string;
  mark?: GlanceMark;
  /** The feed could not be refreshed; the numbers are from its last good copy. */
  stale?: string;
  /** An announced reset, or the site's watch: the Codex card's "Reset free" row. */
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
}

export interface GlanceUpcomingReset {
  /** `Reset free`, `Có thể reset` (a watch), `Tặng lượt để dành` (banked). */
  title: string;
  /** Scheduled resets read positive, a watch reads as a notice. */
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
  notice?: string | null;
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
  language: Language;
  /** Settings → Time Format as a 12-hour flag, `null` for the locale's own clock. */
  hour12: boolean | null;
  appName: string;
  alert: GlanceAlert | null;
  /** The Codex free-reset tracker (`buildGlanceResets`), `null` while it is off or has no data. */
  resets: GlanceResets | null;
  /** The official color logos drawn so far (`useMarkArt`), base64 PNG by brand; optional. */
  markArt?: Readonly<Record<string, string>>;
  now: Date;
}

const BRAND_COLORS: Readonly<Record<string, string>> = { claude: SOURCE_COLORS.claude, codex: SOURCE_COLORS.codex };
const PLAIN_MARK_COLOR = "#FFFFFF";
const LOCALES: Readonly<Record<Language, string>> = { vi: "vi_VN", en: "en_US" };

export function glanceMetric(id: string, data: WidgetData, now: Date): GlanceMetric {
  const bounded = isBounded(data);
  const counting = bounded && data.resetsAt !== null && data.subtitleOverride === undefined && !isFreshSessionWindow(data, now);
  const metric: GlanceMetric = {
    id,
    label: data.title,
    value: menuBarValue(data),
    headline: bounded ? boundedHeadline(data) : unboundedDetail(data),
    fraction: bounded ? fraction(data) : null,
    severity: bounded ? (meterSeverity(meterState(data, now)) ?? "none") : "none",
  };
  if (counting && data.resetsAt) metric.resetsAt = data.resetsAt.toISOString();
  else if (bounded) {
    const detail = boundedTrailingText(data, now);
    if (detail) metric.detail = detail;
  }
  return metric;
}

function shows(settings: GlanceSurfaceSettings): GlanceShows {
  return { account: settings.showAccount, plan: settings.showPlan, resets: settings.showResets };
}

export function buildGlance(input: GlanceInput): GlanceDocument {
  const text = messagesFor(input.language).glance;
  const times: number[] = [];

  const provider = (source: Provider, metrics: GlanceMetric[]): GlanceProvider => {
    const brand = brandOf(source.icon || source.id);
    const mark = PROVIDER_MARKS[brand];
    const about = input.describe(source);
    const entry: GlanceProvider = { id: source.id, name: about.name, brand, color: BRAND_COLORS[brand] ?? knownBrandColor(brand, true) ?? PLAIN_MARK_COLOR, metrics };
    const art = input.markArt?.[brand];
    if (mark) entry.mark = art ? { ...mark, art } : mark;
    if (about.account) entry.account = about.account;
    if (about.plan) entry.plan = about.plan;
    if (metrics.length === 0) entry.notice = about.notice ?? text.noData;
    return entry;
  };

  const readings = (descriptors: readonly WidgetDescriptor[]): GlanceMetric[] =>
    descriptors.flatMap((descriptor) => {
      const data = input.dataFor(descriptor);
      return data.hasData ? [glanceMetric(descriptor.id, data, input.now)] : [];
    });

  const list = (groups: readonly ProviderMetrics[], problems: boolean): GlanceProvider[] =>
    groups.flatMap((group) => {
      const metrics = readings([...group.always, ...group.onDemand]);
      if (metrics.length === 0 && !problems) return [];
      if (metrics.length > 0) {
        const refreshed = Date.parse(input.refreshedAt(group.provider.id) ?? "");
        if (Number.isFinite(refreshed)) times.push(refreshed);
      }
      return [provider(group.provider, metrics)];
    });

  const islandProviders = list(input.island.groups, input.island.settings.showProblems).slice(0, ISLAND_PROVIDER_LIMIT);
  const widgetProviders = list(input.widget.groups, input.widget.settings.showProblems);

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
      open: text.open,
      notRunning: text.notRunning,
      noData: text.noData,
      more: text.more,
      units: text.units,
      resetsOff: text.resetsOff,
      upcoming: text.upcoming,
      upcomingEmpty: text.upcomingEmpty,
    },
    providers: islandProviders,
    island: {
      enabled: input.island.enabled,
      alerts: input.island.settings.alerts,
      style: input.island.settings.style,
      wings: wings(input, islandProviders, provider),
      expandOnHover: input.island.settings.expandOnHover,
      shows: shows(input.island.settings),
      empty: text.empty[input.island.settings.content],
      sections: { ...input.island.settings.sections },
    },
    widget: {
      providers: widgetProviders,
      shows: shows(input.widget.settings),
      empty: text.empty[input.widget.settings.content],
    },
  };
  if (input.hour12 !== null) document.hour12 = input.hour12;
  if (input.resets) document.resets = input.resets;
  if (input.alert) document.alert = input.alert;
  return document;
}

/**
 * The two readings beside the notch. A slot picked in Settings shows that metric, or that special
 * reading, while it has one; an automatic slot, or a pick without data, takes the next reading of the
 * island's accounts: each account's first metric, then a lone account's second.
 */
function wings(
  input: GlanceInput,
  providers: readonly GlanceProvider[],
  make: (source: Provider, metrics: GlanceMetric[]) => GlanceProvider,
): GlanceProvider[] {
  const picked = input.island.wings.map((choice) => {
    if (!choice) return null;
    if (typeof choice === "string") return specialWing(choice, providers, input);
    const source = input.providerOf(choice.providerId);
    const data = input.dataFor(choice);
    if (!source || !data.hasData) return null;
    return make(source, [glanceMetric(choice.id, data, input.now)]);
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
    if (next) result.push({ ...next.entry, metrics: [next.metric] });
  }
  return result;
}

/** The provider standing for the Codex free-reset tracker in a wing. */
export const CODEX_RESETS_PROVIDER_ID = "codex-resets";

/**
 * A wing that is not one picked metric, or `null` when it has nothing to show (the slot then fills
 * automatically). `quota:next` is the island's account whose limit comes back first, counting down
 * to it; the Codex reset wings read the tracker as a provider of their own.
 */
function specialWing(wing: SpecialWing, providers: readonly GlanceProvider[], input: GlanceInput): GlanceProvider | null {
  if (wing === "quota:next") return soonestReset(providers, input.now, input.language);
  const resets = input.resets;
  if (!resets) return null;
  const text = messagesFor(input.language).glance;
  const tracker = (metric: GlanceMetric): GlanceProvider => {
    const entry: GlanceProvider = { id: CODEX_RESETS_PROVIDER_ID, name: resets.title, brand: resets.brand, color: resets.color, metrics: [metric] };
    if (resets.mark) entry.mark = resets.mark;
    return entry;
  };
  const chance = (days: 1 | 3 | 7): GlanceProvider | null => {
    const found = resets.forecast.find((entry) => entry.days === days);
    if (!found) return null;
    const value = `${found.percent}%`;
    return tracker({ id: wing, label: found.label, value, headline: `${value} · ${found.label}`, fraction: found.percent / 100, severity: "normal" });
  };
  switch (wing) {
    case "codex-resets:next": {
      const upcoming = resets.upcoming;
      const countdown = upcoming?.countdown;
      if (upcoming && countdown && Date.parse(countdown.at) > input.now.getTime()) {
        const moving: GlanceCountdown = { at: countdown.at, text: text.wingIn(COUNTDOWN_SPAN) };
        if (countdown.after !== undefined) moving.after = countdown.after;
        return tracker({ id: wing, label: upcoming.title, value: upcoming.title, headline: upcoming.caption, fraction: null, severity: "normal", countdown: moving });
      }
      return chance(1);
    }
    case "codex-resets:chance-1":
      return chance(1);
    case "codex-resets:chance-3":
      return chance(3);
    case "codex-resets:chance-7":
      return chance(7);
    case "codex-resets:since": {
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
  return { ...soonest.entry, metrics: [{ ...soonest.metric, countdown: { at: soonest.metric.resetsAt!, text: messagesFor(language).glance.wingIn(COUNTDOWN_SPAN) } }] };
}
