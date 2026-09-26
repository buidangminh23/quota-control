/**
 * Project: every repository's token use for a period, largest first, each in its own color with its
 * cost and the Claude / Codex split. Worktrees count under their repository; only the name shows.
 */
import { useMemo, useState } from "react";
import { messagesFor } from "@/i18n";
import { formatMoney } from "@/model/currency";
import { colorOf, VIEW_COLORS } from "@/model/palette";
import { TOTAL_SPEND_PERIODS } from "@/model/settings";
import { grandTotal, periodQuery, totalsByKey, USAGE_SOURCES } from "@/model/usage";
import { useSettings } from "@/state/hooks";
import { updateSettings, useApp } from "@/state/store";
import { useToday, useUsageColors, useUsageRows } from "@/state/usage";
import { FolderIcon } from "../ui/icons";
import { Capsules, ChipHeader, RankedBars, tokenCount, totalsText, UsageNote, type RankedBar } from "./parts";

const COLLAPSED_COUNT = 12;

export function ProjectsView() {
  const settings = useSettings();
  const language = settings.language;
  const messages = messagesFor(language).usage;
  const today = useToday();
  const rate = useApp((state) => state.exchangeRate);
  const colors = useUsageColors();
  const [expanded, setExpanded] = useState(false);
  const { rows, failed } = useUsageRows(periodQuery(settings.projectPeriod, today, "project"));

  const bars: RankedBar[] = useMemo(() => {
    if (!rows) return [];
    return totalsByKey(rows)
      .filter((item) => item.total.totalTokens > 0)
      .sort((a, b) => b.total.totalTokens - a.total.totalTokens)
      .map((item) => {
        const label = item.key || messages.unknownProject;
        const split = USAGE_SOURCES.flatMap((source) => {
          const tokens = item.bySource[source]?.totalTokens ?? 0;
          return tokens > 0 ? [`${messages.source(source)} ${tokenCount(tokens, language)}`] : [];
        }).join(" · ");
        const cost = item.total.costUSD !== undefined ? formatMoney(item.total.costUSD, language, rate) : null;
        return {
          key: item.key,
          label,
          color: colorOf(colors.projects, item.key),
          value: item.total.totalTokens,
          valueText: tokenCount(item.total.totalTokens, language),
          detail: cost ? `${cost} · ${split}` : split,
          tooltip: label,
        };
      });
  }, [rows, colors, language, rate, messages]);

  const shown = expanded ? bars : bars.slice(0, COLLAPSED_COUNT);
  return (
    <section className="uc-section">
      <ChipHeader icon={<FolderIcon size={11} />} color={VIEW_COLORS.projects} title={messages.view("projects")} />
      <div className="uc-card uc-chart-card">
        <Capsules
          label={messages.periodLabel}
          options={TOTAL_SPEND_PERIODS.map((value) => ({ value, label: messages.period(value) }))}
          value={settings.projectPeriod}
          onChange={(value) => updateSettings({ projectPeriod: value })}
        />
        {!rows ? (
          <UsageNote text={failed ? messages.failed : messages.loading} />
        ) : bars.length === 0 ? (
          <UsageNote text={messages.noData} />
        ) : (
          <>
            <div className="uc-history-total">
              <span>{`${bars.length} ${messages.view("projects").toLowerCase()}`}</span>
              <span className="uc-num">{totalsText(grandTotal(rows), language, rate)}</span>
            </div>
            <RankedBars bars={shown} />
            {bars.length > COLLAPSED_COUNT ? (
              <button type="button" className="uc-link-button is-block" onClick={() => setExpanded((value) => !value)}>
                {expanded ? messages.showFewer : messages.showAll(bars.length)}
              </button>
            ) : null}
          </>
        )}
      </div>
    </section>
  );
}
