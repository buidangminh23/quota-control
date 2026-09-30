/**
 * The Reset tab: a switch between the two trackers, then the chosen one. Codex resets come from
 * codex-resets.com, which follows @thsottiaux on X; Claude resets from claude-resets.com, which
 * follows @ClaudeDevs (`ClaudeResets`). Both views are drawn from the same cards (`ResetParts`).
 */
import { useMemo } from "react";
import { insightsFor } from "@/i18n/insights";
import { buildResetPresentation } from "@/model/glanceResets";
import { forecastSkill } from "@/model/insights/claudeResets";
import { reliabilityText } from "@/model/insights/claudePresentation";
import { feedOutdated, parseResets, parseResetStatus, resetTrackerOutdated } from "@/model/insights/resets";
import { SOURCE_COLORS } from "@/model/palette";
import { RESET_PROVIDERS, type ResetProvider } from "@/model/settings";
import { dayNumber } from "@/model/timeZone";
import { useNow, useSettings } from "@/state/hooks";
import { useInsights } from "@/state/insights";
import { updateSettings } from "@/state/store";
import { Capsules } from "../tokens/parts";
import { ProviderMark } from "../ui/ProviderMark";
import { ClaudeResets } from "./ClaudeResets";
import { useFeeds } from "./data";
import { Disclosure, FeedStatus, SourceLine } from "./parts";
import { Calendar, Forecast, History, LatestReset, Pattern, Stats, StatusCards } from "./ResetParts";

const SITE_URL = "https://codex-resets.com";
const FEEDS = ["codexResetStatus", "codexResets"] as const;
const MARK_SIZE = 14;

function CodexResets() {
  const { language, timeFormat } = useSettings();
  const text = insightsFor(language);
  const feeds = useFeeds(FEEDS);
  const errors = useInsights((state) => state.feedErrors);
  const status = useMemo(() => parseResetStatus(feeds.codexResetStatus?.body), [feeds.codexResetStatus]);
  const resets = useMemo(() => parseResets(feeds.codexResets?.body, [status?.latest ?? null]), [feeds.codexResets, status]);
  const now = useNow();
  const presentation = useMemo(() => buildResetPresentation({ feeds: { status, resets }, now, language, timeFormat }), [status, resets, now, language, timeFormat]);
  const today = dayNumber(now);
  const reliability = useMemo(() => reliabilityText(forecastSkill(resets, now), language, text), [resets, today, language, text]);
  const loaded = feeds.codexResetStatus !== undefined && feeds.codexResets !== undefined;
  const empty = loaded && !feeds.codexResetStatus?.body && !feeds.codexResets?.body;
  const stale = resetTrackerOutdated({
    status,
    statusStale: feedOutdated(feeds.codexResetStatus),
    historyBody: feeds.codexResets?.body,
    historyStale: feedOutdated(feeds.codexResets),
  });
  const error = FEEDS.map((name) => feeds[name]?.error ?? errors[name]).find(Boolean);

  return (
    <>
      {!loaded ? <p className="uc-empty">{text.loading}</p> : null}
      {empty ? <p className="uc-insight-error">{text.failed(error ?? "")}</p> : null}
      {stale && !empty ? <p className="uc-insight-note">{text.staleNote}</p> : null}
      {presentation.latest ? <LatestReset latest={presentation.latest} text={text} /> : null}
      <StatusCards cards={presentation.statuses} text={text} />
      {resets.length > 0 ? (
        <>
          <Forecast forecast={presentation.forecast.chances.length > 0 ? { ...presentation.forecast, reliability } : presentation.forecast} />
          <Calendar resets={resets} language={language} text={text} />
          <Pattern resets={resets} language={language} text={text} />
          <Stats presentation={presentation} />
          <History presentation={presentation} text={text} />
        </>
      ) : null}
      <FeedStatus names={FEEDS} shown={feeds.codexResetStatus} language={language} text={text} />
      <SourceLine text={text.resetsSource} url={SITE_URL} linkLabel={text.openLink} />
      <Disclosure title={text.methodTitle}>
        {text.resetsMethod.map((paragraph) => (
          <p key={paragraph}>{paragraph}</p>
        ))}
      </Disclosure>
    </>
  );
}

export function ResetsTab() {
  const { language, resetsProvider } = useSettings();
  const text = insightsFor(language);
  return (
    <>
      <Capsules<ResetProvider>
        label={text.resetProviderLabel}
        options={RESET_PROVIDERS.map((provider) => ({
          value: provider,
          label: text.resetProvider(provider),
          icon: <ProviderMark brand={provider} size={MARK_SIZE} />,
          color: SOURCE_COLORS[provider],
        }))}
        value={resetsProvider}
        onChange={(provider) => updateSettings({ resetsProvider: provider })}
      />
      {resetsProvider === "claude" ? <ClaudeResets /> : <CodexResets />}
    </>
  );
}
