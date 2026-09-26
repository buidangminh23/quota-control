/**
 * What the menu bar strip, the Dynamic Island and the desktop widgets show: the Hạn mức cards, the
 * starred metrics or a hand-picked set, plus how each account reads there (email, plan, reset time,
 * accounts that need attention) and, for the island, its closed style and the readings beside the
 * notch.
 */
import { useMemo } from "react";
import { messagesFor, translate, type Language } from "@/i18n";
import type { SettingsMessages } from "@/i18n/messages";
import type { WidgetDescriptor } from "@/lib/types";
import { displayGroups, glanceCandidates, type ProviderMetrics } from "@/model/layout";
import { cardIdentity, providerBrand } from "@/model/providerText";
import {
  GLANCE_CONTENTS,
  ISLAND_STYLES,
  type GlanceContent,
  type GlanceSurfaceSettings,
  type IslandSettings,
  type StripSettings,
  type TaskbarDisplay,
} from "@/model/settings";
import { useIsEnabled, useSettings } from "@/state/hooks";
import { updateSettings, useApp } from "@/state/store";
import { Picker, Switch } from "../ui/controls";
import { ProviderMark } from "../ui/ProviderMark";
import { Row, Section } from "./parts";

const AUTO_WING = "auto";
const STRIP_VALUES = ["two", "one"] as const;

function patchStrip(patch: Partial<StripSettings>): void {
  updateSettings({ strip: { ...useApp.getState().settings.strip, ...patch } });
}

function patchIsland(patch: Partial<IslandSettings>): void {
  updateSettings({ island: { ...useApp.getState().settings.island, ...patch } });
}

function patchWidget(patch: Partial<GlanceSurfaceSettings>): void {
  updateSettings({ widget: { ...useApp.getState().settings.widget, ...patch } });
}

/** Every enabled account card with all its metrics, the choices of a hand-picked set. */
function useCandidates(): ProviderMetrics[] {
  const layout = useApp((state) => state.layout);
  const catalog = useApp((state) => state.catalog);
  const isEnabled = useIsEnabled();
  return useMemo(() => glanceCandidates(layout, catalog, isEnabled), [layout, catalog, isEnabled]);
}

interface ContentProps {
  content: GlanceContent;
  metrics: readonly string[];
  onChange: (patch: { content?: GlanceContent; metrics?: string[] }) => void;
  text: SettingsMessages;
  language: Language;
}

/** The content picker and, for a hand-picked set, one switch per metric under each account. */
function ContentRows({ content, metrics, onChange, text, language }: ContentProps) {
  const layout = useApp((state) => state.layout);
  const catalog = useApp((state) => state.catalog);
  const isEnabled = useIsEnabled();
  const candidates = useCandidates();
  const picked = new Set(metrics);

  const choose = (value: GlanceContent) => {
    if (value === "custom" && metrics.length === 0) {
      const seeded = displayGroups(layout, catalog, isEnabled).flatMap((group) => [...group.always, ...group.onDemand].map((descriptor) => descriptor.id));
      onChange({ content: value, metrics: seeded });
      return;
    }
    onChange({ content: value });
  };

  const toggle = (id: string, on: boolean) => {
    const offered = candidates.flatMap((group) => group.always.map((descriptor) => descriptor.id));
    const known = new Set(offered);
    const kept = metrics.filter((candidate) => !known.has(candidate));
    const chosen = offered.filter((candidate) => (candidate === id ? on : picked.has(candidate)));
    onChange({ metrics: [...chosen, ...kept] });
  };

  return (
    <>
      <Row label={text.glanceContent} note={text.glanceContentNote(content)}>
        <Picker value={content} options={GLANCE_CONTENTS} label={text.glanceContentOption} onChange={choose} ariaLabel={text.glanceContent} />
      </Row>
      {content === "custom" ? (
        candidates.length === 0 ? (
          <p className="uc-settings-note">{text.glanceMetricsNone}</p>
        ) : (
          <div className="uc-settings-row-group" role="group" aria-label={text.glanceMetrics}>
            <p className="uc-settings-note is-lead">{text.glanceMetricsNote}</p>
            {candidates.map((group) => (
              <AccountChecklist key={group.provider.id} group={group} picked={picked} onToggle={toggle} language={language} />
            ))}
          </div>
        )
      ) : null}
    </>
  );
}

function AccountChecklist({
  group,
  picked,
  onToggle,
  language,
}: {
  group: ProviderMetrics;
  picked: ReadonlySet<string>;
  onToggle: (id: string, on: boolean) => void;
  language: Language;
}) {
  const identity = cardIdentity(group.provider, undefined, language);
  return (
    <>
      <div className="uc-list-row is-glance-account">
        <span className="uc-list-mark">
          <ProviderMark brand={providerBrand(group.provider)} size={14} />
        </span>
        <span className="uc-list-text">
          <span className="uc-list-title uc-truncate">{identity.name}</span>
          {identity.account ? <span className="uc-list-subtitle uc-truncate">{identity.account}</span> : null}
        </span>
      </div>
      {group.always.map((descriptor) => {
        const title = translate(descriptor.template.title, language);
        return (
          <Row key={descriptor.id} label={title} nested>
            <Switch checked={picked.has(descriptor.id)} label={title} onChange={(on) => onToggle(descriptor.id, on)} />
          </Row>
        );
      })}
    </>
  );
}

/** Which parts of an account the surface shows. */
function ShowRows({ value, onChange, text }: { value: GlanceSurfaceSettings; onChange: (patch: Partial<GlanceSurfaceSettings>) => void; text: SettingsMessages }) {
  return (
    <>
      <Row label={text.glanceShowAccount}>
        <Switch checked={value.showAccount} label={text.glanceShowAccount} onChange={(on) => onChange({ showAccount: on })} />
      </Row>
      <Row label={text.glanceShowPlan}>
        <Switch checked={value.showPlan} label={text.glanceShowPlan} onChange={(on) => onChange({ showPlan: on })} />
      </Row>
      <Row label={text.glanceShowResets}>
        <Switch checked={value.showResets} label={text.glanceShowResets} onChange={(on) => onChange({ showResets: on })} />
      </Row>
      <Row label={text.glanceShowProblems} note={text.glanceShowProblemsNote}>
        <Switch checked={value.showProblems} label={text.glanceShowProblems} onChange={(on) => onChange({ showProblems: on })} />
      </Row>
    </>
  );
}

/** The strip's content and how many readings each account shows, under the menu bar display. */
export function StripRows({ display }: { display: TaskbarDisplay }) {
  const settings = useSettings();
  const text = messagesFor(settings.language).settings;
  if (display === "icon") return null;
  const strip = settings.strip;
  return (
    <>
      <ContentRows content={strip.content} metrics={strip.metrics} onChange={patchStrip} text={text} language={settings.language} />
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
  const metricsById = useMemo(() => {
    const byId = new Map<string, { group: ProviderMetrics; descriptor: WidgetDescriptor }>();
    for (const group of candidates) for (const descriptor of group.always) byId.set(descriptor.id, { group, descriptor });
    return byId;
  }, [candidates]);
  const wingOptions = [AUTO_WING, ...metricsById.keys()];
  const wingLabel = (id: string) => {
    const entry = metricsById.get(id);
    if (!entry) return text.islandWingAuto;
    return `${cardIdentity(entry.group.provider, undefined, language).name} · ${translate(entry.descriptor.template.title, language)}`;
  };
  const wingValue = (id: string) => (metricsById.has(id) ? id : AUTO_WING);
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
          <Row label={text.islandStyle}>
            <Picker value={island.style} options={ISLAND_STYLES} label={text.islandStyleOption} onChange={(style) => patchIsland({ style })} ariaLabel={text.islandStyle} />
          </Row>
          <Row label={text.islandWing("left")}>
            <Picker value={wingValue(island.wings[0])} options={wingOptions} label={wingLabel} onChange={(id) => setWing(0, id)} ariaLabel={text.islandWing("left")} />
          </Row>
          <Row label={text.islandWing("right")}>
            <Picker value={wingValue(island.wings[1])} options={wingOptions} label={wingLabel} onChange={(id) => setWing(1, id)} ariaLabel={text.islandWing("right")} />
          </Row>
          <ContentRows content={island.content} metrics={island.metrics} onChange={patchIsland} text={text} language={language} />
          <ShowRows value={island} onChange={patchIsland} text={text} />
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
      <ContentRows content={widget.content} metrics={widget.metrics} onChange={patchWidget} text={text} language={settings.language} />
      <ShowRows value={widget} onChange={patchWidget} text={text} />
    </Section>
  );
}
