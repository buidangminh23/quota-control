/** Provider sections in layout order: account cards on Hạn mức, token sources on the Token tab. */
import type { ProviderMetrics } from "@/model/layout";
import { useDisplay, useNow } from "@/state/hooks";
import { useApp } from "@/state/store";
import { ProviderSection } from "./ProviderSection";

export function ProviderSections({ groups }: { groups: ProviderMetrics[] }) {
  const display = useDisplay();
  const engine = useApp((state) => state.engine);
  const now = useNow();
  const interval = engine?.refreshIntervalMs ?? 300_000;
  return groups.map((group) => (
    <ProviderSection
      key={group.provider.id}
      group={group}
      runtime={engine?.providers[group.provider.id]}
      display={display}
      refreshIntervalMs={interval}
      now={now}
    />
  ));
}
