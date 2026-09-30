/**
 * The Claude banked resets the user marked as applied. The app cannot apply one itself (the user
 * does it on claude.ai), so the mark is how the popup, the island and the widgets stop counting
 * down to a reset already used.
 */
import { updateSettings, useApp } from "./store";

/** Mark a banked reset as applied, or take the mark back. */
export function setBankedUsed(resetId: string, used: boolean): void {
  const current = useApp.getState().settings.usedBankedResets.filter((id) => id !== resetId);
  updateSettings({ usedBankedResets: used ? [...current, resetId] : current });
}
