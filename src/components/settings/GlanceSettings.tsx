/**
 * What the menu bar strip, the Dynamic Island and the desktop widgets show. Each surface picks its
 * views (the island's tabs, the Overview widget's parts), then tunes every view on its own: the
 * limits list by account and metric with quick picks that can keep following the Hạn mức tab or the
 * stars, whose reset tracker it shows (Codex or Claude) and that tracker's parts, and how many limits
 * coming back are listed. The island also has its closed look (style and the readings beside the
 * notch) and how it opens.
 */
import { useMemo, useState, type ReactNode } from "react";
import { messagesFor, type Language } from "@/i18n";
import { insightsFor } from "@/i18n/insights";
import type { SettingsMessages } from "@/i18n/messages";
import type { WidgetDescriptor } from "@/lib/types";
import { SPECIAL_WINGS, type SpecialWing } from "@/model/glance";
import { cardsAsShown, glanceCandidates, glanceGroups, type ProviderMetrics } from "@/model/layout";
import { cardIdentity, providerBrand } from "@/model/providerText";
import {
  ISLAND_LAYOUTS,
  ISLAND_STYLES,
  ISLAND_VIEWS,
  RESET_PARTS,
  SURFACE_RESET_PROVIDERS,
  surfaceResetProvider,
  UPCOMING_LIMITS,
  type GlanceContent,
  type GlanceSurfaceSettings,
  type IslandSettings,
  type IslandView,
  type ResetPart,
  type ResetProvider,
  type StripSettings,
  type SurfaceResets,
  type TaskbarDisplay,
} from "@/model/settings";
import { descriptorTitle } from "@/model/widgetData";
import { useIsEnabled, useSettings } from "@/state/hooks";
import { updateSettings, useApp } from "@/state/store";
import { Chip, Picker, Switch } from "../ui/controls";
import { ChevronRight } from "../ui/icons";
import { ProviderMark } from "../ui/ProviderMark";
import { Row, Section } from "./parts";

const AUTO_WING = "auto";
const STRIP_VALUES = ["two", "one"] as const;
const SHOW_PARTS = ["account", "plan", "resets", "problems"] as const;
type ShowPart = (typeof SHOW_PARTS)[number];
const SHOW_KEYS: Readonly<Record<ShowPart, "showAccount" | "showPlan" | "showResets" | "showProblems">> = {
  account: "showAccount",
  plan: "showPlan",
  resets: "showResets",
  problems: "showProblems",
};
/** Besides the Reset tab, the switch that keeps each reset tracker's feed loading. */
const TRACKER_NOTIFICATIONS: Readonly<Record<ResetProvider, "notifyCodexResets" | "notifyClaudeResets">> = {
  codex: "notifyCodexResets",
  claude: "notifyClaudeResets",
};

function patchStrip(patch: Partial<StripSettings>): void {
  updateSettings({ strip: { ...useApp.getState().settings.strip, ...patch } });
}

function patchIsland(patch: Partial<IslandSettings>): void {
  updateSettings({ island: { ...useApp.getState().settings.island, ...patch } });
}

function patchWidget(patch: Partial<GlanceSurfaceSettings>): void {
  updateSettings({ widget: { ...useApp.getState().settings.widget, ...patch } });
}

/** Every enabled account card with all its metrics: the choices of the limits list. */
function useCandidates(): ProviderMetrics[] {
  const layout = useApp((state) => state.layout);
  const catalog = useApp((state) => state.catalog);
  const isEnabled = useIsEnabled();
  return useMemo(() => glanceCandidates(layout, catalog, isEnabled), [layout, catalog, isEnabled]);
}

/**
 * The metric ids a content choice shows now: what the tab or the stars hold, or the picked set. The
 * island and the widgets (`followsCards`) show the tab's cards as the tab shows them, a card's rows
 * behind its show-more button only while it is open.
 */
function useShown(content: GlanceContent, metrics: readonly string[], followsCards = false): Set<string> {
  const layout = useApp((state) => state.layout);
  const catalog = useApp((state) => state.catalog);
  const isEnabled = useIsEnabled();
  return useMemo(() => {
    const groups = glanceGroups(content, metrics, layout, catalog, isEnabled);
    const shown = followsCards && content === "dashboard" ? cardsAsShown(groups, layout.openProviders) : groups;
    return new Set(shown.flatMap((group) => [...group.always, ...group.onDemand].map((descriptor) => descriptor.id)));
  }, [content, metrics, followsCards, layout, catalog, isEnabled]);
}

/** A small heading inside a settings card, splitting it into steps. */
function SubHeading({ children }: { children: ReactNode }) {
  return <h3 className="uc-settings-subhead">{children}</h3>;
}

/** Chips on their own lines under a label, with an optional note. */
function ChipRow({ label, note, children }: { label?: string; note?: string; children: ReactNode }) {
  return (
    <div className="uc-chip-row">
      {label ? <span className="uc-chip-row-label">{label}</span> : null}
      <div className="uc-chips">{children}</div>
      {note ? <p className="uc-settings-note is-flush">{note}</p> : null}
    </div>
  );
}

/** A view's options behind its name and a one-line summary; one opens at a time per surface. */
function Disclosure({ title, summary, open, onToggle, children }: { title: string; summary: string; open: boolean; onToggle: () => void; children: ReactNode }) {
  return (
    <div className={`uc-disclosure${open ? " is-open" : ""}`}>
      <button type="button" className="uc-disclosure-head" aria-expanded={open} onClick={onToggle}>
        <span className="uc-disclosure-chevron">
          <ChevronRight size={10} />
        </span>
        <span className="uc-disclosure-text">
          <span className="uc-disclosure-title">{title}</span>
          <span className="uc-disclosure-summary">{summary}</span>
        </span>
      </button>
      {open ? <div className="uc-disclosure-body">{children}</div> : null}
    </div>
  );
}

/** The views as chips, kept in the popup's tab order, the reset view named after `provider`'s tracker; the last one on cannot be switched off. */
function TabChips({ tabs, provider, onChange, text }: { tabs: readonly IslandView[]; provider: SurfaceResets; onChange: (tabs: IslandView[]) => void; text: SettingsMessages }) {
  const toggle = (view: IslandView, on: boolean) => {
    const next = ISLAND_VIEWS.filter((candidate) => (candidate === view ? on : tabs.includes(candidate)));
    if (next.length > 0) onChange(next);
  };
  return (
    <>
      {ISLAND_VIEWS.map((view) => {
        const on = tabs.includes(view);
        return (
          <Chip key={view} checked={on} disabled={on && tabs.length === 1} onChange={(value) => toggle(view, value)}>
            {text.glanceTabName(view, provider)}
          </Chip>
        );
      })}
    </>
  );
}

interface QuotaValue {
  content: GlanceContent;
  metrics: readonly string[];
}

/** One line saying what the limits list shows, for the closed editor. */
function useQuotaSummary(value: QuotaValue, text: SettingsMessages, followsCards = false): string {
  const shown = useShown(value.content, value.metrics, followsCards);
  const candidates = useCandidates();
  const accounts = candidates.filter((group) => group.always.some((descriptor) => shown.has(descriptor.id))).length;
  return text.glanceQuotaSummary(value.content, shown.size, accounts);
}

/**
 * The limits list: quick picks (following the Hạn mức tab or the stars, every metric, none), then
 * each account with a switch for all its metrics and a chip per metric. Picking a metric by hand
 * turns the list into a custom one seeded with what showed, so nothing else moves.
 */
function QuotaEditor({
  value,
  onChange,
  shows,
  followsCards = false,
  text,
  language,
}: {
  value: QuotaValue;
  onChange: (patch: { content?: GlanceContent; metrics?: string[] }) => void;
  shows?: { value: GlanceSurfaceSettings; onChange: (patch: Partial<GlanceSurfaceSettings>) => void };
  /** The island and the widgets, which show the Hạn mức tab's cards as the tab shows them. */
  followsCards?: boolean;
  text: SettingsMessages;
  language: Language;
}) {
  const candidates = useCandidates();
  const shown = useShown(value.content, value.metrics, followsCards);
  const offered = useMemo(() => candidates.flatMap((group) => group.always.map((descriptor) => descriptor.id)), [candidates]);

  const pick = (chosen: ReadonlySet<string>) => {
    const known = new Set(offered);
    const kept = value.content === "custom" ? value.metrics.filter((id) => !known.has(id)) : [];
    onChange({ content: "custom", metrics: [...offered.filter((id) => chosen.has(id)), ...kept] });
  };
  const toggle = (ids: readonly string[], on: boolean) => {
    const next = new Set(shown);
    for (const id of ids) {
      if (on) next.add(id);
      else next.delete(id);
    }
    pick(next);
  };
  const allOn = offered.length > 0 && offered.every((id) => shown.has(id));

  return (
    <div className="uc-editor">
      <ChipRow label={text.glancePresets}>
        <Chip kind="radio" checked={value.content === "dashboard"} onChange={() => onChange({ content: "dashboard" })}>
          {text.glancePreset("dashboard")}
        </Chip>
        <Chip kind="radio" checked={value.content === "starred"} onChange={() => onChange({ content: "starred" })}>
          {text.glancePreset("starred")}
        </Chip>
        <Chip kind="radio" checked={value.content === "custom" && allOn} disabled={offered.length === 0} onChange={() => pick(new Set(offered))}>
          {text.glancePreset("all")}
        </Chip>
        <Chip kind="radio" checked={value.content === "custom" && shown.size === 0} onChange={() => pick(new Set())}>
          {text.glancePreset("none")}
        </Chip>
      </ChipRow>
      <p className="uc-settings-note is-flush">{followsCards && value.content === "dashboard" ? text.glanceFollowCardsNote : text.glanceFollowNote(value.content)}</p>
      {candidates.length === 0 ? (
        <p className="uc-settings-note is-flush">{text.glanceMetricsNone}</p>
      ) : (
        <div className="uc-editor-accounts">
          {candidates.map((group) => (
            <AccountPicker key={group.provider.id} group={group} shown={shown} onToggle={toggle} text={text} language={language} />
          ))}
        </div>
      )}
      {shows ? (
        <ChipRow label={text.glanceShows} note={shows.value.showProblems ? text.glanceShowProblemsNote : undefined}>
          {SHOW_PARTS.map((part) => {
            const key = SHOW_KEYS[part];
            return (
              <Chip key={part} checked={shows.value[key]} onChange={(on) => shows.onChange({ [key]: on })}>
                {text.glanceShow(part)}
              </Chip>
            );
          })}
        </ChipRow>
      ) : null}
    </div>
  );
}

function AccountPicker({
  group,
  shown,
  onToggle,
  text,
  language,
}: {
  group: ProviderMetrics;
  shown: ReadonlySet<string>;
  onToggle: (ids: readonly string[], on: boolean) => void;
  text: SettingsMessages;
  language: Language;
}) {
  const identity = cardIdentity(group.provider, undefined, language);
  const snapshot = useApp((state) => state.engine?.providers[group.provider.id]?.snapshot);
  const ids = group.always.map((descriptor) => descriptor.id);
  const count = ids.filter((id) => shown.has(id)).length;
  return (
    <div className={`uc-account-picker${count === 0 ? " is-off" : ""}`}>
      <div className="uc-account-picker-head">
        <span className="uc-list-mark">
          <ProviderMark brand={providerBrand(group.provider)} size={14} />
        </span>
        <span className="uc-list-text">
          <span className="uc-list-title uc-truncate">{identity.name}</span>
          {identity.account ? <span className="uc-list-subtitle uc-truncate">{identity.account}</span> : null}
        </span>
        <span className="uc-account-picker-count">{text.glanceAccountCount(count, ids.length)}</span>
        <Switch checked={count > 0} label={identity.name} onChange={(on) => onToggle(ids, on)} />
      </div>
      <div className="uc-chips">
        {group.always.map((descriptor) => (
          <Chip key={descriptor.id} checked={shown.has(descriptor.id)} onChange={(on) => onToggle([descriptor.id], on)}>
            {descriptorTitle(descriptor, snapshot, language)}
          </Chip>
        ))}
      </div>
    </div>
  );
}

/**
 * Whether a surface's reset view has no data: the Reset tab is off, and so are the notifications of
 * the tracker it shows (of both trackers, for a view showing both).
 */
function useTrackerOff(provider: SurfaceResets): boolean {
  const settings = useSettings();
  const trackers: readonly ResetProvider[] = provider === "both" ? ["codex", "claude"] : [provider];
  return !settings.showResetsTab && trackers.every((tracker) => !settings[TRACKER_NOTIFICATIONS[tracker]]);
}

/**
 * Whose reset tracker a surface shows, worded like the Reset tab's own choice, then that tracker's
 * parts; the last part on stays on.
 */
function ResetEditor({
  value,
  onChange,
  text,
  language,
}: {
  value: GlanceSurfaceSettings;
  onChange: (patch: Partial<GlanceSurfaceSettings>) => void;
  text: SettingsMessages;
  language: Language;
}) {
  const provider = surfaceResetProvider(value.resetsProvider, useSettings());
  const parts = value.resetParts;
  const trackerOff = useTrackerOff(provider);
  const insights = insightsFor(language);
  const on = RESET_PARTS.filter((part) => parts[part]).length;
  const toggle = (part: ResetPart, checked: boolean) => {
    const next = { ...parts, [part]: checked };
    if (RESET_PARTS.some((candidate) => next[candidate])) onChange({ resetParts: next });
  };
  return (
    <div className="uc-editor">
      <ChipRow label={insights.resetProviderLabel}>
        {SURFACE_RESET_PROVIDERS.map((option) => (
          <Chip key={option} kind="radio" checked={value.resetsProvider === option} onChange={() => onChange({ resetsProvider: option })}>
            {option === "app" ? insights.resetProviderFollow : option === "both" ? insights.resetProviderBoth : insights.resetProvider(option)}
          </Chip>
        ))}
      </ChipRow>
      <ChipRow label={text.resetParts} note={trackerOff ? text.resetPartsOff(provider) : undefined}>
        {RESET_PARTS.map((part) => (
          <Chip key={part} checked={parts[part]} disabled={parts[part] && on === 1} onChange={(checked) => toggle(part, checked)}>
            {text.resetPart(part)}
          </Chip>
        ))}
      </ChipRow>
    </div>
  );
}

function UpcomingEditor({ limit, onChange, text }: { limit: number; onChange: (limit: number) => void; text: SettingsMessages }) {
  return (
    <div className="uc-editor">
      <ChipRow label={text.upcomingLimit} note={text.upcomingNote}>
        {UPCOMING_LIMITS.map((option) => (
          <Chip key={option} kind="radio" checked={limit === option} onChange={() => onChange(option)}>
            {text.upcomingLimitOption(option)}
          </Chip>
        ))}
      </ChipRow>
    </div>
  );
}

/** Every view's options for one surface, one open at a time; the reset view is named after its tracker. */
function ViewEditors({
  views,
  value,
  onChange,
  scope,
  text,
  language,
}: {
  views: readonly IslandView[];
  value: GlanceSurfaceSettings;
  onChange: (patch: Partial<GlanceSurfaceSettings>) => void;
  scope?: (view: IslandView) => string;
  text: SettingsMessages;
  language: Language;
}) {
  const [open, setOpen] = useState<IslandView | null>(views[0] ?? null);
  const quotaSummary = useQuotaSummary(value, text, true);
  const provider = surfaceResetProvider(value.resetsProvider, useSettings());
  const resetsOn = RESET_PARTS.filter((part) => value.resetParts[part]).length;
  const summary = (view: IslandView): string => {
    if (view === "quota") return quotaSummary;
    if (view === "resets") return text.resetPartsSummary(resetsOn, RESET_PARTS.length);
    return text.upcomingLimitOption(value.upcomingLimit);
  };
  return (
    <div className="uc-disclosures">
      {views.map((view) => (
        <Disclosure key={view} title={text.glanceTabName(view, provider)} summary={summary(view)} open={open === view} onToggle={() => setOpen(open === view ? null : view)}>
          {scope ? <p className="uc-settings-note is-flush">{scope(view)}</p> : null}
          {view === "quota" ? <QuotaEditor value={value} onChange={onChange} shows={{ value, onChange }} followsCards text={text} language={language} /> : null}
          {view === "resets" ? <ResetEditor value={value} onChange={onChange} text={text} language={language} /> : null}
          {view === "upcoming" ? <UpcomingEditor limit={value.upcomingLimit} onChange={(upcomingLimit) => onChange({ upcomingLimit })} text={text} /> : null}
        </Disclosure>
      ))}
    </div>
  );
}

/** The strip's content and how many readings each account shows, under the menu bar display. */
export function StripRows({ display }: { display: TaskbarDisplay }) {
  const settings = useSettings();
  const text = messagesFor(settings.language).settings;
  const strip = settings.strip;
  const summary = useQuotaSummary(strip, text);
  const [open, setOpen] = useState(false);
  if (display === "icon") return null;
  return (
    <>
      <div className="uc-disclosures is-inset">
        <Disclosure title={text.glanceGroup("content")} summary={summary} open={open} onToggle={() => setOpen(!open)}>
          <QuotaEditor value={strip} onChange={patchStrip} text={text} language={settings.language} />
        </Disclosure>
      </div>
      {display === "text" ? (
        <Row label={text.stripValues}>
          <Picker
            value={strip.values === 1 ? "one" : "two"}
            options={STRIP_VALUES}
            label={(option) => text.stripValuesOption(option === "one" ? 1 : 2)}
            onChange={(option) => patchStrip({ values: option === "one" ? 1 : 2 })}
            ariaLabel={text.stripValues}
          />
        </Row>
      ) : null}
    </>
  );
}

export function IslandSection() {
  const settings = useSettings();
  const language = settings.language;
  const text = messagesFor(language).settings;
  const island = settings.island;
  const candidates = useCandidates();
  const engine = useApp((state) => state.engine);
  const metricsById = useMemo(() => {
    const byId = new Map<string, { group: ProviderMetrics; descriptor: WidgetDescriptor }>();
    for (const group of candidates) for (const descriptor of group.always) byId.set(descriptor.id, { group, descriptor });
    return byId;
  }, [candidates]);
  const special = new Set<string>(SPECIAL_WINGS);
  const wingOptions = [AUTO_WING, ...SPECIAL_WINGS, ...metricsById.keys()];
  const wingLabel = (id: string) => {
    if (special.has(id)) return text.islandWingSpecial(id as SpecialWing);
    const entry = metricsById.get(id);
    if (!entry) return text.islandWingAuto;
    const snapshot = engine?.providers[entry.group.provider.id]?.snapshot;
    return `${cardIdentity(entry.group.provider, undefined, language).name} · ${descriptorTitle(entry.descriptor, snapshot, language)}`;
  };
  const wingValue = (id: string) => (special.has(id) || metricsById.has(id) ? id : AUTO_WING);
  const setWing = (index: 0 | 1, id: string) => {
    const wings: [string, string] = [...island.wings];
    wings[index] = id === AUTO_WING ? "" : id;
    patchIsland({ wings });
  };

  return (
    <Section title={text.section("island")}>
      <Row label={text.dynamicIsland} note={text.dynamicIslandNote}>
        <Switch checked={settings.dynamicIsland} label={text.dynamicIsland} onChange={(on) => updateSettings({ dynamicIsland: on })} />
      </Row>
      {settings.dynamicIsland ? (
        <>
          <SubHeading>{text.glanceGroup("closed")}</SubHeading>
          <Row label={text.islandStyle}>
            <Picker value={island.style} options={ISLAND_STYLES} label={text.islandStyleOption} onChange={(style) => patchIsland({ style })} ariaLabel={text.islandStyle} />
          </Row>
          <Row label={text.islandWing("left")}>
            <Picker value={wingValue(island.wings[0])} options={wingOptions} label={wingLabel} onChange={(id) => setWing(0, id)} ariaLabel={text.islandWing("left")} />
          </Row>
          <Row label={text.islandWing("right")}>
            <Picker value={wingValue(island.wings[1])} options={wingOptions} label={wingLabel} onChange={(id) => setWing(1, id)} ariaLabel={text.islandWing("right")} />
          </Row>

          <SubHeading>{text.glanceGroup("open")}</SubHeading>
          <ChipRow label={text.glanceTabs("island")} note={text.glanceTabsNote("island")}>
            <TabChips tabs={island.tabs} provider={surfaceResetProvider(island.resetsProvider, settings)} onChange={(tabs) => patchIsland({ tabs })} text={text} />
          </ChipRow>
          {island.tabs.length > 1 ? (
            <Row label={text.islandLayout} note={text.islandLayoutNote(island.layout)}>
              <Picker value={island.layout} options={ISLAND_LAYOUTS} label={text.islandLayoutOption} onChange={(layout) => patchIsland({ layout })} ariaLabel={text.islandLayout} />
            </Row>
          ) : null}
          <ViewEditors views={island.tabs} value={island} onChange={patchIsland} text={text} language={language} />

          <SubHeading>{text.glanceGroup("behavior")}</SubHeading>
          <Row label={text.islandExpandOnHover} note={text.islandExpandOnHoverNote}>
            <Switch checked={island.expandOnHover} label={text.islandExpandOnHover} onChange={(on) => patchIsland({ expandOnHover: on })} />
          </Row>
          <Row label={text.islandAlerts} note={text.islandAlertsNote}>
            <Switch checked={island.alerts} label={text.islandAlerts} onChange={(on) => patchIsland({ alerts: on })} />
          </Row>
        </>
      ) : null}
    </Section>
  );
}

export function WidgetSection() {
  const settings = useSettings();
  const text = messagesFor(settings.language).settings;
  const widget = settings.widget;
  return (
    <Section title={text.section("widget")}>
      <Row label={text.desktopWidget} note={text.desktopWidgetNote}>
        <span />
      </Row>
      <p className="uc-settings-note">{text.desktopWidgetKindsNote}</p>
      <ChipRow label={text.glanceTabs("widget")} note={text.glanceTabsNote("widget")}>
        <TabChips tabs={widget.tabs} provider={surfaceResetProvider(widget.resetsProvider, settings)} onChange={(tabs) => patchWidget({ tabs })} text={text} />
      </ChipRow>
      <SubHeading>{text.glanceGroup("content")}</SubHeading>
      <ViewEditors views={ISLAND_VIEWS} value={widget} onChange={patchWidget} scope={text.glanceWidgetScope} text={text} language={settings.language} />
    </Section>
  );
}
