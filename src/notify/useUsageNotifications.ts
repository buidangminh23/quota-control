/**
 * Runs the pace notifier on every engine update. Each alert turned on under Notifications sends a
 * system notification and opens the macOS Dynamic Island for a moment; a limit running low also
 * opens the island whenever its Open for Alerts switch is on, since that switch promises it without
 * asking for notifications as well.
 */
import { useEffect, useRef } from "react";
import { announceOnIsland } from "@/glance/alerts";
import { messagesFor } from "@/i18n";
import { brandOf, descriptorProviderId } from "@/model/layout";
import { providerTitle } from "@/model/providerText";
import type { NotificationSettings } from "@/model/settings";
import { notify } from "@/platform/system";
import { useDisplay, useIsEnabled } from "@/state/hooks";
import { useApp } from "@/state/store";
import { PaceNotifier, type Milestone } from "./paceNotifications";

const ISLAND_SEVERITY: Readonly<Record<Milestone, "warning" | "critical">> = {
  almostOut: "critical",
  cuttingItClose: "warning",
  willRunOut: "critical",
};

/** The milestones to watch: those turned on under Notifications, and a limit running low for an
 * island that opens for alerts. */
export function watchedMilestones(toggles: NotificationSettings, islandAlerts: boolean): NotificationSettings {
  return islandAlerts ? { ...toggles, almostOut: true } : toggles;
}

export function useUsageNotifications(): void {
  const engine = useApp((state) => state.engine);
  const catalog = useApp((state) => state.catalog);
  const layout = useApp((state) => state.layout);
  const toggles = useApp((state) => state.settings.notifications);
  const islandAlerts = useApp((state) => state.info?.platform === "macos" && state.settings.dynamicIsland && state.settings.island.alerts);
  const display = useDisplay();
  const isEnabled = useIsEnabled();
  const notifier = useRef(new PaceNotifier());

  useEffect(() => {
    if (!engine) return;
    const alerts = notifier.current.evaluate(
      catalog,
      engine.providers,
      new Set(layout.placed),
      isEnabled,
      watchedMilestones(toggles, islandAlerts),
      display,
      (entry, metric) => messagesFor(display.language).notify.title(providerTitle(entry.provider, display.language), metric),
      new Date(),
    );
    for (const alert of alerts) {
      const descriptorId = alert.key.split("|")[0] ?? "";
      if (toggles[alert.milestone]) {
        const topic = { id: `usage.${descriptorId}`, group: `usage.${descriptorProviderId(descriptorId)}` };
        notify(alert.title, alert.body, topic).catch((error: unknown) => console.error("Sending notification failed", error));
      }
      announceOnIsland({ title: alert.title, body: alert.body, brand: brandOf(descriptorProviderId(descriptorId)), severity: ISLAND_SEVERITY[alert.milestone] });
    }
  }, [engine, catalog, layout.placed, toggles, islandAlerts, display, isEnabled]);
}
