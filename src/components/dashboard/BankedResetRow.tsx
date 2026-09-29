/**
 * The first row of a Claude card while a banked reset announced for its plan can still be applied:
 * how long is left, the deadline in the device's zone underneath and, on hover, the post and where
 * to apply it. It follows the reset tracking, shown while the Reset tab or the Claude reset
 * notification is on (the same settings the core fetches the catalog for), goes away once the user
 * marks the reset as applied, and opens the Reset tab's Claude view when that tab is on.
 */
import { useEffect, useMemo } from "react";
import { insightsFor } from "@/i18n/insights";
import { bankedResetFor, bankedResetLines } from "@/model/insights/bankedResetLines";
import { parseClaudeResets } from "@/model/insights/claudeResets";
import { useSettings } from "@/state/hooks";
import { ensureFeed, useInsights } from "@/state/insights";
import { selectDashboardTab, updateSettings } from "@/state/store";
import { CLAUDE_AUTHOR_HANDLE, ResetAuthorAvatar } from "../ui/ResetAuthorAvatar";
import { tooltipProps } from "../ui/tooltip";

function openClaudeResets(): void {
  updateSettings({ resetsProvider: "claude" });
  selectDashboardTab("resets");
}

export function BankedResetRow({ now, plan }: { now: Date; plan: string | undefined }) {
  const { showResetsTab, notifyClaudeResets, usedBankedResets, timeFormat, language } = useSettings();
  const tracking = showResetsTab || notifyClaudeResets;
  const body = useInsights((state) => state.feeds.claudeResets?.body ?? null);
  const feed = useMemo(() => parseClaudeResets(body), [body]);

  useEffect(() => {
    if (tracking) ensureFeed("claudeResets");
  }, [tracking]);

  const reset = tracking && feed ? bankedResetFor(feed.resets, plan, usedBankedResets, now) : null;
  const lines = reset ? bankedResetLines(reset, now, timeFormat, language) : null;
  if (!reset || !lines) return null;
  const details = showResetsTab ? `${lines.details}\n${insightsFor(language).freeResetOpenTab}` : lines.details;
  const label = `${lines.title}: ${lines.value}. ${lines.caption}`;
  const className = "uc-row uc-free-reset is-banked";
  const content = (
    <>
      <span className="uc-row-primary">
        <span className="uc-free-reset-title">
          <ResetAuthorAvatar size={14} handle={reset.account ? `@${reset.account}` : CLAUDE_AUTHOR_HANDLE} />
          <span>{lines.title}</span>
        </span>
        <span className="uc-row-trailing uc-free-reset-value uc-num">{lines.value}</span>
      </span>
      <span className="uc-row-restore uc-num">{lines.caption}</span>
    </>
  );
  return showResetsTab ? (
    <button type="button" className={className} onClick={openClaudeResets} aria-label={label} {...tooltipProps(details)}>
      {content}
    </button>
  ) : (
    <div className={className} role="group" aria-label={label} {...tooltipProps(details)}>
      {content}
    </div>
  );
}
