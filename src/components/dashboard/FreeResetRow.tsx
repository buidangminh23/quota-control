/**
 * The first row of a Codex card while codex-resets.com knows a free reset is coming: a countdown,
 * the time in the device's zone underneath and, on hover, the post and how its time was read. It
 * follows the reset tracking, shown while the Reset tab or the reset notification is on (the same
 * settings the core fetches the status for), and opens the Reset tab when that tab is on.
 */
import { useEffect, useMemo } from "react";
import { insightsFor } from "@/i18n/insights";
import { freeResetLines } from "@/model/insights/freeResetLines";
import { parseResetStatus } from "@/model/insights/resets";
import { upcomingReset } from "@/model/insights/upcomingReset";
import { useSettings } from "@/state/hooks";
import { ensureFeed, useInsights } from "@/state/insights";
import { selectDashboardTab } from "@/state/store";
import { ResetAuthorAvatar } from "../ui/ResetAuthorAvatar";
import { tooltipProps } from "../ui/tooltip";

export function FreeResetRow({ now }: { now: Date }) {
  const { showResetsTab, notifyCodexResets, timeFormat, language } = useSettings();
  const tracking = showResetsTab || notifyCodexResets;
  const body = useInsights((state) => state.feeds.codexResetStatus?.body ?? null);
  const status = useMemo(() => parseResetStatus(body), [body]);

  useEffect(() => {
    if (tracking) ensureFeed("codexResetStatus");
  }, [tracking]);

  const next = tracking ? upcomingReset(status, now) : null;
  if (!next) return null;
  const lines = freeResetLines(next, now, timeFormat, language);
  const details = showResetsTab ? `${lines.details}\n${insightsFor(language).freeResetOpenTab}` : lines.details;
  const label = `${lines.title}: ${lines.value}. ${lines.caption}${lines.note ? `. ${lines.note}` : ""}`;
  const className = `uc-row uc-free-reset${lines.awaiting ? " is-awaiting" : ""}${next.origin === "watch" ? " is-watch" : ""}`;
  const content = (
    <>
      <span className="uc-row-primary">
        <span className="uc-free-reset-title">
          <ResetAuthorAvatar size={14} />
          <span>{lines.title}</span>
        </span>
        <span className="uc-row-trailing uc-free-reset-value uc-num">{lines.value}</span>
      </span>
      <span className="uc-row-restore uc-num">{lines.caption}</span>
      {lines.note ? <span className="uc-row-restore">{lines.note}</span> : null}
    </>
  );
  return showResetsTab ? (
    <button type="button" className={className} onClick={() => selectDashboardTab("resets")} aria-label={label} {...tooltipProps(details)}>
      {content}
    </button>
  ) : (
    <div className={className} role="group" aria-label={label} {...tooltipProps(details)}>
      {content}
    </div>
  );
}
