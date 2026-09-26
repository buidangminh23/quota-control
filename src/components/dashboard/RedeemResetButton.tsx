/**
 * "Dùng 1 lượt" under Codex's free limit resets. A reset is spent only when the user presses the
 * button and confirms; nothing spends one on its own, and the regular limits keep resetting on the
 * provider's schedule.
 */
import { useState } from "react";
import { messagesFor } from "@/i18n";
import { backend } from "@/lib/backend";
import { whenLabel } from "@/model/format";
import type { WidgetData } from "@/model/widgetData";
import { showNotice } from "@/state/store";
import { Button } from "../ui/controls";
import { confirmAction } from "../ui/dialog";

/** How many banked resets the row reports, or 0 for any other row. */
export function availableResets(data: WidgetData): number {
  if (!data.showsResetExpiries || !data.hasData) return 0;
  return data.values.find((value) => value.kind === "count")?.number ?? 0;
}

export function canRedeemReset(data: WidgetData): boolean {
  return availableResets(data) >= 1 && typeof backend().redeemLimitReset === "function";
}

function soonest(dates: readonly Date[], now: Date): Date | null {
  return dates.reduce<Date | null>((best, date) => (date.getTime() > now.getTime() && (best === null || date < best) ? date : best), null);
}

export function RedeemResetButton({ providerId, data, now }: { providerId: string; data: WidgetData; now: Date }) {
  const [busy, setBusy] = useState(false);
  const messages = messagesFor(data.language);
  const text = messages.limitReset;

  const redeem = async () => {
    const expiry = soonest(data.expiriesAt, now);
    const confirmed = await confirmAction({
      title: text.confirmTitle,
      message: text.confirmMessage(expiry ? whenLabel(expiry, "absolute", now, data.timeFormat, data.language) : null),
      confirmLabel: text.confirm,
      cancelLabel: messages.chrome.cancel,
    });
    const api = backend();
    if (!confirmed || !api.redeemLimitReset) return;
    setBusy(true);
    try {
      const result = await api.redeemLimitReset(providerId);
      showNotice(text.result(result, messages.meter.errors), result.status === "reset" ? "positive" : "notice");
    } catch (error) {
      console.error("Using a limit reset failed", error);
      showNotice(text.failed, "notice");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="uc-row-action">
      <Button className="is-small" onClick={() => void redeem()} disabled={busy}>
        {busy ? text.redeeming : text.redeem}
      </Button>
    </div>
  );
}
