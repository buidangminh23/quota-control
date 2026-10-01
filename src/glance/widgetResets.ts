/**
 * The reset trackers placed desktop widgets ask for. Each Reset, Reset Calendar and Overview widget
 * picks its tracker in its own Edit Widget (Codex, Claude, both, or as Settings say); one set to a
 * tracker the document does not carry (Claude's, while no Settings choice reads it) asks for it with
 * a `showResets` request, and the document carries it from then on. The asks last while the popup
 * runs: a widget asks again whenever the document it reads lacks its tracker, so they come back
 * after a restart while such a widget is still placed.
 */
import { create } from "zustand";
import { RESET_PROVIDERS, type ResetProvider } from "@/model/settings";

export const useWidgetResetAsks = create<{ asked: readonly ResetProvider[] }>(() => ({ asked: [] }));

/** Carry `provider`'s tracker for the widgets from now on. */
export function askWidgetResets(provider: ResetProvider): void {
  const { asked } = useWidgetResetAsks.getState();
  if (asked.includes(provider)) return;
  useWidgetResetAsks.setState({ asked: RESET_PROVIDERS.filter((candidate) => candidate === provider || asked.includes(candidate)) });
}
