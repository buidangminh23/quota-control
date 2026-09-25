/** Runs the pace notifier on every engine update while any notification toggle is on. */
import { useEffect, useRef } from "react";
import { messagesFor } from "@/i18n";
import { providerTitle } from "@/model/providerText";
import { anyNotificationEnabled } from "@/model/settings";
import { notify } from "@/platform/system";
import { useDisplay, useIsEnabled } from "@/state/hooks";
import { useApp } from "@/state/store";
import { PaceNotifier } from "./paceNotifications";

export function useUsageNotifications(): void {
  const engine = useApp((state) => state.engine);
  const catalog = useApp((state) => state.catalog);
  const layout = useApp((state) => state.layout);
  const toggles = useApp((state) => state.settings.notifications);
  const display = useDisplay();
  const isEnabled = useIsEnabled();
  const notifier = useRef(new PaceNotifier());

  useEffect(() => {
    if (!engine) return;
    const settingsOn = anyNotificationEnabled({ notifications: toggles });
    const alerts = notifier.current.evaluate(
      catalog,
      engine.providers,
      new Set(layout.placed),
      isEnabled,
      toggles,
      display,
      (entry, metric) => messagesFor(display.language).notify.title(providerTitle(entry.provider, display.language), metric),
      new Date(),
    );
    if (!settingsOn) return;
    for (const alert of alerts) {
      notify(alert.title, alert.body).catch((error: unknown) => console.error("Sending notification failed", error));
    }
  }, [engine, catalog, layout.placed, toggles, display, isEnabled]);
}
