/**
 * The plans of the Claude accounts shown on the dashboard, read from their latest readings. The
 * Reset tab and the Claude reset notifications compare them with who a reset covered.
 */
import { useMemo } from "react";
import { planFamily, type ClaudePlan } from "@/model/insights/claudeResets";
import { brandOf, isLocalHistoryCard } from "@/model/layout";
import { isProviderEnabled, useApp, type AppState } from "./store";

const CLAUDE_BRAND = "claude";
const NOT_LOADED = "-";
const UNNAMED = "?";

/** One entry per Claude account, `null` for a plan this app cannot name; `null` until the core has answered. */
export function claudeAccountPlans(state: AppState): (ClaudePlan | null)[] | null {
  if (!state.engine) return null;
  return Object.entries(state.engine.providers)
    .filter(([id]) => brandOf(id) === CLAUDE_BRAND && !isLocalHistoryCard(id) && isProviderEnabled(state, id))
    .map(([, runtime]) => planFamily(runtime.snapshot?.plan));
}

/** `claudeAccountPlans`, stable while the plans stay the same. */
export function useClaudeAccountPlans(): readonly (ClaudePlan | null)[] | null {
  const key = useApp((state) => {
    const plans = claudeAccountPlans(state);
    return plans === null ? NOT_LOADED : plans.map((plan) => plan ?? UNNAMED).sort().join(",");
  });
  return useMemo(() => {
    if (key === NOT_LOADED) return null;
    return key ? key.split(",").map((plan) => (plan === UNNAMED ? null : (plan as ClaudePlan))) : [];
  }, [key]);
}

/** The plan families (`max`, `pro`…) of those accounts, each once. */
export function useClaudePlans(): ClaudePlan[] {
  const accounts = useClaudeAccountPlans();
  return useMemo(() => [...new Set((accounts ?? []).filter((plan): plan is ClaudePlan => plan !== null))], [accounts]);
}
