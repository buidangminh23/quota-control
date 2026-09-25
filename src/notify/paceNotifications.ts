/**
 * Pace notifications (upstream `PaceMilestone`): "almost out" when a limit falls under 10% left,
 * "cutting it close" when the projection lands inside the last 10%, and "will run out" when the
 * projection passes the limit before the reset. Each fires once per reset window, on the transition
 * into the state; a metric's first reading with data only records its baseline, so launching the app
 * never replays alerts for states that already held.
 */
import { messagesFor, type Language } from "@/i18n";
import type { ProviderEntry, ProviderRuntimeState, WidgetDescriptor } from "@/lib/types";
import { boundedTrailingText, meterState } from "@/model/meterState";
import type { NotificationSettings } from "@/model/settings";
import { remainingFraction, widgetDataFor, type DisplayOptions, type WidgetData } from "@/model/widgetData";

export type Milestone = "almostOut" | "cuttingItClose" | "willRunOut";

const ALMOST_OUT_SHARE = 0.1;

export interface PaceAlert {
  key: string;
  milestone: Milestone;
  title: string;
  body: string;
}

export function milestonesFor(data: WidgetData, now: Date): Set<Milestone> {
  const reached = new Set<Milestone>();
  if (!data.hasData || data.limit === null || !(data.limit > 0)) return reached;
  if (remainingFraction(data) < ALMOST_OUT_SHARE) reached.add("almostOut");
  const state = meterState(data, now);
  if (state.kind === "closeToLimit") reached.add("cuttingItClose");
  if (state.kind === "runningOut") reached.add("willRunOut");
  return reached;
}

function windowKey(descriptor: WidgetDescriptor, data: WidgetData, milestone: Milestone): string {
  return `${descriptor.id}|${milestone}|${data.resetsAt?.toISOString() ?? "none"}`;
}

function body(milestone: Milestone, data: WidgetData, now: Date, language: Language): string {
  const text = messagesFor(language).notify;
  const state = meterState(data, now);
  switch (milestone) {
    case "almostOut": {
      const left = Math.max(0, Math.round(remainingFraction(data) * 100));
      return text.almostOut(left, boundedTrailingText(data, now));
    }
    case "cuttingItClose":
      return text.cuttingItClose(state.kind === "closeToLimit" ? Math.round(state.projectedFraction * 100) : 100);
    case "willRunOut":
      return text.willRunOut(state.kind === "runningOut" ? state.eta : null);
  }
}

export class PaceNotifier {
  private readonly seen = new Map<string, Set<Milestone>>();
  private readonly fired = new Set<string>();

  /** Alerts for milestones newly reached since the previous call, honoring the enabled toggles. */
  evaluate(
    entries: readonly ProviderEntry[],
    runtimes: Readonly<Record<string, ProviderRuntimeState | undefined>>,
    placed: ReadonlySet<string>,
    isEnabled: (providerId: string) => boolean,
    toggles: NotificationSettings,
    display: DisplayOptions,
    title: (entry: ProviderEntry, metric: string) => string,
    now: Date,
  ): PaceAlert[] {
    const alerts: PaceAlert[] = [];
    for (const entry of entries) {
      if (!isEnabled(entry.provider.id)) continue;
      const snapshot = runtimes[entry.provider.id]?.snapshot;
      if (!snapshot) continue;
      for (const descriptor of entry.descriptors) {
        if (!placed.has(descriptor.id)) continue;
        const data = widgetDataFor(descriptor, snapshot, display);
        if (!data.hasData) continue;
        const reached = milestonesFor(data, now);
        const previous = this.seen.get(descriptor.id);
        this.seen.set(descriptor.id, reached);
        if (!previous) continue;
        for (const milestone of reached) {
          if (previous.has(milestone) || !toggles[milestone]) continue;
          const key = windowKey(descriptor, data, milestone);
          if (this.fired.has(key)) continue;
          this.fired.add(key);
          alerts.push({ key, milestone, title: title(entry, data.title), body: body(milestone, data, now, display.language) });
        }
      }
    }
    return alerts;
  }
}
