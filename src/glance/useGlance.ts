/**
 * Keeps the macOS Dynamic Island and desktop widgets in step with the popup, the way
 * `useTaskbarStrip` keeps the menu bar: every engine update, layout or setting change rebuilds the
 * glance document, which goes to the core only when it changed. Each surface lists what its Settings
 * choose (the Hạn mức cards, the starred metrics or a hand-picked set). The Codex free-reset
 * tracker rides along while the Reset tab or reset notifications are on, loading the feeds they
 * need; the Claude tracker only while a surface or a wing reads it and the Reset tab or Claude reset
 * notifications are on. Each Codex and Claude account starts with the row its popup card starts
 * with, which follows the same settings whichever tracker a surface shows. The hidden popup keeps
 * running, so all of it stays live while it is closed. Elsewhere the core has no glance and this
 * does nothing.
 */
import { useEffect, useMemo, useRef } from "react";
import { messagesFor } from "@/i18n";
import { backend } from "@/lib/backend";
import type { Provider, WidgetDescriptor } from "@/lib/types";
import { buildGlance, isClaudeResetsWing, isSpecialWing, type GlanceResetRow, type GlanceWingChoice } from "@/model/glance";
import { buildClaudeGlanceResets } from "@/model/glanceClaudeResets";
import { buildGlanceResets, parseResetFeeds } from "@/model/glanceResets";
import { claudeResetRow, codexResetRow } from "@/model/glanceResetRows";
import { parseClaudeResets } from "@/model/insights/claudeResets";
import { feedOutdated, resetTrackerOutdated } from "@/model/insights/resets";
import { brandOf, glanceGroups, isLocalHistoryCard } from "@/model/layout";
import { surfaceResetProvider } from "@/model/settings";
import { cardIdentity, providerBrand } from "@/model/providerText";
import { widgetDataFor } from "@/model/widgetData";
import { useClaudeAccountPlans } from "@/state/claudePlans";
import { useDisplay, useIsEnabled, useWallClock } from "@/state/hooks";
import { ensureFeed, useInsights } from "@/state/insights";
import { useApp } from "@/state/store";
import { useIslandAlert } from "./alerts";
import { useMarkArt } from "./markArt";

/** Pace colors move with the clock; a minute is fine enough for the island and the widget. */
const CLOCK_MS = 60_000;
/** The core's refresh interval before its first state arrives, as the popup's cards assume. */
const DEFAULT_REFRESH_INTERVAL_MS = 300_000;

export function useGlance(): void {
  const ready = useApp((state) => state.ready);
  const layout = useApp((state) => state.layout);
  const catalog = useApp((state) => state.catalog);
  const engine = useApp((state) => state.engine);
  const info = useApp((state) => state.info);
  const islandEnabled = useApp((state) => state.settings.dynamicIsland);
  const islandChoice = useApp((state) => state.settings.island);
  const widgetChoice = useApp((state) => state.settings.widget);
  const resetsProvider = useApp((state) => state.settings.resetsProvider);
  const showResetsTab = useApp((state) => state.settings.showResetsTab);
  const island = useMemo(
    () => ({ ...islandChoice, resetsProvider: surfaceResetProvider(islandChoice.resetsProvider, { resetsProvider, showResetsTab }) }),
    [islandChoice, resetsProvider, showResetsTab],
  );
  const widget = useMemo(
    () => ({ ...widgetChoice, resetsProvider: surfaceResetProvider(widgetChoice.resetsProvider, { resetsProvider, showResetsTab }) }),
    [widgetChoice, resetsProvider, showResetsTab],
  );
  const timeFormat = useApp((state) => state.settings.timeFormat);
  const theme = useApp((state) => state.settings.theme);
  const notifyCodexResets = useApp((state) => state.settings.notifyCodexResets);
  const notifyClaudeResets = useApp((state) => state.settings.notifyClaudeResets);
  const usedBankedResets = useApp((state) => state.settings.usedBankedResets);
  const tracking = showResetsTab || notifyCodexResets;
  const statusFeed = useInsights((state) => state.feeds.codexResetStatus);
  const historyFeed = useInsights((state) => state.feeds.codexResets);
  const statusError = useInsights((state) => state.feedErrors.codexResetStatus);
  const historyError = useInsights((state) => state.feedErrors.codexResets);
  const claudeFeed = useInsights((state) => state.feeds.claudeResets);
  const claudeError = useInsights((state) => state.feedErrors.claudeResets);
  const claudeAccounts = useClaudeAccountPlans();
  const display = useDisplay();
  const isEnabled = useIsEnabled();
  const alert = useIslandAlert();
  const now = useWallClock(CLOCK_MS);
  const lastKey = useRef<string | null>(null);
  const supported = info?.platform === "macos" && typeof backend().setGlance === "function";
  const brands = useMemo(() => (supported ? catalog.map((entry) => brandOf(entry.provider.icon || entry.provider.id)) : []), [supported, catalog]);
  const markArt = useMarkArt(brands);

  useEffect(() => {
    if (!supported) return;
    if (tracking) ensureFeed("codexResetStatus");
    if (showResetsTab) ensureFeed("codexResets");
  }, [supported, tracking, showResetsTab]);

  const statusBody = tracking ? (statusFeed?.body ?? null) : null;
  const historyBody = showResetsTab ? (historyFeed?.body ?? null) : null;
  const feeds = useMemo(() => parseResetFeeds(statusBody, historyBody), [statusBody, historyBody]);
  const stale =
    tracking &&
    resetTrackerOutdated({
      status: feeds.status,
      statusStale: feedOutdated(statusFeed, statusError),
      historyBody,
      historyStale: showResetsTab && feedOutdated(historyFeed, historyError),
    });
  const resets = useMemo(
    () => (tracking ? buildGlanceResets({ feeds, stale, now, language: display.language, timeFormat, theme }) : null),
    [tracking, feeds, stale, now, display.language, timeFormat, theme],
  );

  const claudeRead = island.resetsProvider === "claude" || widget.resetsProvider === "claude" || island.wings.some(isClaudeResetsWing);
  const claudeFollowed = showResetsTab || notifyClaudeResets;
  const claudeCards = (claudeAccounts?.length ?? 0) > 0;
  const claudeTracking = supported && claudeFollowed && (claudeRead || claudeCards);
  useEffect(() => {
    if (claudeTracking) ensureFeed("claudeResets");
  }, [claudeTracking]);
  const claudeBody = claudeTracking ? (claudeFeed?.body ?? null) : null;
  const claudeParsed = useMemo(() => parseClaudeResets(claudeBody), [claudeBody]);
  const claudeStale = Boolean(claudeFeed?.stale || claudeError);
  const claudeResets = useMemo(
    () =>
      claudeParsed && claudeRead
        ? buildClaudeGlanceResets({ feed: claudeParsed, accounts: claudeAccounts ?? [], used: usedBankedResets, stale: claudeStale, now, language: display.language, timeFormat, theme })
        : null,
    [claudeParsed, claudeRead, claudeAccounts, usedBankedResets, claudeStale, now, display.language, timeFormat, theme],
  );

  const codexRow = useMemo(
    () => (tracking ? codexResetRow(feeds.status, now, timeFormat, display.language, showResetsTab) : null),
    [tracking, feeds, now, timeFormat, display.language, showResetsTab],
  );
  const resetRowFor = useMemo(() => {
    const claudeResets = claudeFollowed ? (claudeParsed?.resets ?? null) : null;
    return (provider: Provider): GlanceResetRow | null => {
      if (isLocalHistoryCard(provider.id)) return null;
      switch (providerBrand(provider)) {
        case "codex":
          return codexRow;
        case "claude":
          return claudeResets
            ? claudeResetRow(claudeResets, engine?.providers[provider.id]?.snapshot?.plan, usedBankedResets, now, timeFormat, display.language, showResetsTab)
            : null;
        default:
          return null;
      }
    };
  }, [codexRow, claudeFollowed, claudeParsed, engine, usedBankedResets, now, timeFormat, display.language, showResetsTab]);

  const document = useMemo(() => {
    if (!supported) return null;
    const descriptors = new Map<string, WidgetDescriptor>(catalog.flatMap((entry) => entry.descriptors.map((descriptor) => [descriptor.id, descriptor] as const)));
    const providers = new Map<string, Provider>(catalog.map((entry) => [entry.provider.id, entry.provider] as const));
    const wing = (id: string): GlanceWingChoice => {
      if (isSpecialWing(id)) return id;
      const descriptor = id ? descriptors.get(id) : undefined;
      return descriptor && isEnabled(descriptor.providerId) ? descriptor : null;
    };
    return buildGlance({
      island: {
        groups: glanceGroups(island.content, island.metrics, layout, catalog, isEnabled),
        settings: island,
        enabled: islandEnabled,
        wings: [wing(island.wings[0]), wing(island.wings[1])],
      },
      widget: {
        groups: glanceGroups(widget.content, widget.metrics, layout, catalog, isEnabled),
        settings: widget,
      },
      dataFor: (descriptor) => widgetDataFor(descriptor, engine?.providers[descriptor.providerId]?.snapshot, display),
      describe: (provider) => cardIdentity(provider, engine?.providers[provider.id], display.language),
      providerOf: (providerId) => providers.get(providerId),
      refreshedAt: (providerId) => engine?.providers[providerId]?.snapshot?.refreshedAt,
      refreshIntervalMs: engine?.refreshIntervalMs ?? DEFAULT_REFRESH_INTERVAL_MS,
      openProviders: layout.openProviders,
      resetRowFor,
      language: display.language,
      hour12: timeFormat === "auto" ? null : timeFormat === "12h",
      theme,
      appName: info?.name ?? messagesFor(display.language).chrome.appName,
      alert,
      resets,
      claudeResets,
      markArt,
      now,
    });
  }, [supported, layout, catalog, isEnabled, engine, display, info, islandEnabled, island, widget, timeFormat, theme, alert, resets, claudeResets, resetRowFor, markArt, now]);

  useEffect(() => {
    if (!ready || !document) return;
    const key = JSON.stringify(document);
    if (key === lastKey.current) return;
    lastKey.current = key;
    backend()
      .setGlance?.(document)
      .catch((error: unknown) => {
        lastKey.current = null;
        console.error("Updating the island and widget failed", error);
      });
  }, [ready, document]);
}
