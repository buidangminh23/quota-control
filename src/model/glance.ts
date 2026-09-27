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
}

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
  mark?: ProviderMark;
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
}

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
  };
  /** The open island's accounts. */
  providers: GlanceProvider[];
  island: GlanceIsland;
  widget: GlanceWidget;
  alert?: GlanceAlert;
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
    /** The wing metrics picked in Settings, left then right (`null` where the slot is automatic). */
    wings: readonly [WidgetDescriptor | null, WidgetDescriptor | null];
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
    if (mark) entry.mark = mark;
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
    },
    widget: {
      providers: widgetProviders,
      shows: shows(input.widget.settings),
      empty: text.empty[input.widget.settings.content],
    },
  };
  if (input.hour12 !== null) document.hour12 = input.hour12;
  if (input.alert) document.alert = input.alert;
  return document;
}

/**
 * The two readings beside the notch. A slot picked in Settings shows that metric while it has a
 * reading; an automatic slot takes the next reading of the island's accounts: each account's first
 * metric, then a lone account's second.
 */
function wings(
  input: GlanceInput,
  providers: readonly GlanceProvider[],
  make: (source: Provider, metrics: GlanceMetric[]) => GlanceProvider,
): GlanceProvider[] {
  const picked = input.island.wings.map((descriptor) => {
    if (!descriptor) return null;
    const source = input.providerOf(descriptor.providerId);
    const data = input.dataFor(descriptor);
    if (!source || !data.hasData) return null;
    return make(source, [glanceMetric(descriptor.id, data, input.now)]);
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
