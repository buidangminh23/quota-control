/**
 * What the macOS Dynamic Island and the desktop widgets ask the popup to do with their buttons:
 * spend one of a Codex account's banked limit resets, mark a Claude banked reset as used or take
 * the mark back, or open the Reset tab on a tracker. The surface has already asked the user to
 * confirm, in the popup's own words; the core only relays the request (`glance-action`), and the
 * popup does it with the code its own buttons run, so the result is the same wherever it was pressed.
 * A widget also asks, unprompted, for a reset tracker its own Edit Widget chose that the document it
 * reads does not carry (`showResets`).
 */
import { messagesFor } from "@/i18n";
import { backend } from "@/lib/backend";
import { GLANCE_ACTION_PROVIDER_ID, GLANCE_ACTION_RESET_ID } from "@/model/glance";
import { layoutFamily } from "@/model/layout";
import { RESET_PROVIDERS, type ResetProvider } from "@/model/settings";
import { setBankedUsed } from "@/state/bankedResets";
import { selectDashboardTab, showNotice, updateSettings, useApp } from "@/state/store";
import { announceOnIsland } from "./alerts";
import { askWidgetResets } from "./widgetResets";

export type GlanceAction =
  | { kind: "redeemLimitReset"; providerId: string }
  | { kind: "markBankedReset"; resetId: string; used: boolean }
  | { kind: "openResets"; provider: ResetProvider }
  | { kind: "showResets"; provider: ResetProvider };

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

/** The request as sent, or `null` for anything else: only these four shapes are ever acted on. */
export function parseGlanceAction(value: unknown): GlanceAction | null {
  const request = record(value);
  if (!request) return null;
  switch (request.kind) {
    case "redeemLimitReset":
      return typeof request.providerId === "string" && GLANCE_ACTION_PROVIDER_ID.test(request.providerId) ? { kind: "redeemLimitReset", providerId: request.providerId } : null;
    case "markBankedReset":
      return typeof request.resetId === "string" && GLANCE_ACTION_RESET_ID.test(request.resetId) && typeof request.used === "boolean"
        ? { kind: "markBankedReset", resetId: request.resetId, used: request.used }
        : null;
    case "openResets":
    case "showResets":
      return (RESET_PROVIDERS as readonly unknown[]).includes(request.provider) ? { kind: request.kind, provider: request.provider as ResetProvider } : null;
    default:
      return null;
  }
}

/**
 * Spend one reset of a connected Codex account, then say how it went in the popup and on the
 * island, as the popup's own button does. An account that is not a connected Codex account is left
 * alone.
 */
async function redeem(providerId: string): Promise<void> {
  const state = useApp.getState();
  const api = backend();
  if (layoutFamily(providerId) !== "codex" || !state.engine?.providers[providerId] || !api.redeemLimitReset) return;
  const messages = messagesFor(state.settings.language);
  const text = messages.limitReset;
  let result: string;
  let reset = false;
  try {
    const answer = await api.redeemLimitReset(providerId);
    reset = answer.status === "reset";
    result = text.result(answer, messages.meter.errors);
  } catch (error) {
    console.error("Using a limit reset failed", error);
    result = text.failed;
  }
  showNotice(result, reset ? "positive" : "notice");
  announceOnIsland({ title: text.redeem, body: result, brand: "codex", severity: reset ? "normal" : "warning" });
}

/** Do what the island or a widget asked. */
export async function performGlanceAction(action: GlanceAction): Promise<void> {
  switch (action.kind) {
    case "redeemLimitReset":
      await redeem(action.providerId);
      return;
    case "markBankedReset":
      setBankedUsed(action.resetId, action.used);
      return;
    case "openResets":
      updateSettings({ resetsProvider: action.provider });
      selectDashboardTab("resets");
      return;
    case "showResets":
      askWidgetResets(action.provider);
      return;
  }
}

/** Follow the island's and the widgets' requests for as long as the popup lives. */
export function startGlanceActions(): () => void {
  const api = backend();
  if (!api.onGlanceAction) return () => {};
  return api.onGlanceAction((value) => {
    const action = parseGlanceAction(value);
    if (action) void performGlanceAction(action);
  });
}
