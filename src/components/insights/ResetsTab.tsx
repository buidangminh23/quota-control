/**
 * The Reset tab: whether a Codex reset is announced or hinted at (codex-resets.com, which follows
 * @thsottiaux on X), this app's estimate of the chance of one soon, the history's statistics and
 * the latest resets with links to their posts.
 */
import { useMemo, useState } from "react";
import { insightsFor, type InsightsMessages } from "@/i18n/insights";
import type { Language } from "@/i18n";
import { shortTime, type TimeFormat } from "@/model/format";
import { activeWatch, FORECAST_HORIZONS, forecastResets, HALF_LIFE_DAYS, parseResets, parseResetStatus, resetStats, type CodexReset, type ResetSource, type ResetStatus } from "@/model/insights/resets";
import { excerpt } from "@/notify/useResetNotifications";
import { useNow, useSettings } from "@/state/hooks";
import { refreshFeeds, useInsights } from "@/state/insights";
import { Button } from "../ui/controls";
import { useFeeds } from "./data";
import { agoText, dateText, Disclosure, LinkButton, numberText, percentText, RateBar, SourceLine } from "./parts";

const HISTORY_PREVIEW = 8;
const SITE_URL = "https://codex-resets.com";
const FEEDS = ["codexResetStatus", "codexResets"] as const;

function when(date: Date, timeFormat: TimeFormat, language: Language): string {
  return `${shortTime(date, timeFormat, language)} ${dateText(date, language)}`;
}

function PostLink({ source, text, compact = false }: { source: ResetSource; text: InsightsMessages; compact?: boolean }) {
  if (!source.url) return null;
  return compact ? (
    <LinkButton url={source.url} label={text.openPost} />
  ) : (
    <LinkButton url={source.url} label={text.openPost}>
      <span>{text.openPost}</span>
    </LinkButton>
  );
}

/** `12/09`, with the year only when it is not the current one. */
function shortDate(date: Date, now: Date, language: Language): string {
  if (date.getFullYear() !== now.getFullYear()) return dateText(date, language);
  const day = String(date.getDate()).padStart(2, "0");
  const month = String(date.getMonth() + 1).padStart(2, "0");
  return language === "vi" ? `${day}/${month}` : `${month}/${day}`;
}

function StatusCards({ status, resets, language, timeFormat, text }: { status: ResetStatus | null; resets: CodexReset[]; language: Language; timeFormat: TimeFormat; text: InsightsMessages }) {
  const now = useNow();
  const watch = activeWatch(status, now);
  const scheduled = status?.scheduled ?? null;
  const last = resets[0] ?? null;
  const lastAgo = last ? agoText(last.announcedAt, now, language) : null;

  return (
    <>
      {scheduled ? (
        <article className="uc-card uc-reset-status is-scheduled">
          <span className="uc-reset-status-title">{text.scheduledTitle}</span>
          <p className="uc-reset-post">{excerpt(scheduled.text, 280)}</p>
          <span className="uc-reset-meta">
            {[
              text.announcedAgo(agoText(scheduled.announcedAt, now, language) ?? "—"),
              scheduled.scheduledFor ? text.scheduledFor(when(scheduled.scheduledFor, timeFormat, language)) : text.scheduledNoTime,
            ].join(" · ")}
          </span>
          <PostLink source={scheduled.source} text={text} />
        </article>
      ) : null}
      {watch ? (
        <article className={`uc-card uc-reset-status is-watch is-${watch.level}`}>
          <span className="uc-reset-status-title">{text.watchTitle(watch.level)}</span>
          {watch.chancePercent !== null ? <span className="uc-reset-meta">{text.watchChance(`${watch.chancePercent}%`, watch.forecastWindow)}</span> : null}
          <p className="uc-reset-post">{excerpt(watch.text, 280)}</p>
          <span className="uc-reset-meta">{text.watchUntil(when(watch.expiresAt, timeFormat, language))}</span>
          <PostLink source={watch.source} text={text} />
        </article>
      ) : null}
      {!scheduled && !watch && (status || last) ? (
        <article className="uc-card uc-reset-status">
          <span className="uc-reset-status-title">{text.quietTitle}</span>
          {lastAgo ? <span className="uc-reset-meta">{text.lastReset(lastAgo)}</span> : null}
        </article>
      ) : null}
    </>
  );
}

function Forecast({ resets, language, text }: { resets: CodexReset[]; language: Language; text: InsightsMessages }) {
  const now = useNow();
  const forecast = forecastResets(resets, now);
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{text.forecastTitle}</h2>
      <div className="uc-card uc-reset-forecast">
        {forecast ? (
          <>
            <div className="uc-reset-horizons">
              {FORECAST_HORIZONS.map((days) => (
                <div key={days} className="uc-reset-horizon">
                  <span className="uc-reset-horizon-value uc-num">{percentText(language, forecast.chance[days], 0)}</span>
                  <RateBar rate={forecast.chance[days]} low={null} high={null} />
                  <span className="uc-reset-horizon-label">{text.horizon(days)}</span>
                </div>
              ))}
            </div>
            <p className="uc-insight-note">{text.forecastNote(numberText(language, forecast.resets), HALF_LIFE_DAYS)}</p>
            <p className="uc-insight-note">{text.forecastDisclaimer}</p>
          </>
        ) : (
          <p className="uc-empty">{text.forecastUnavailable}</p>
        )}
      </div>
    </section>
  );
}

function Stats({ resets, language, text }: { resets: CodexReset[]; language: Language; text: InsightsMessages }) {
  const now = useNow();
  const stats = resetStats(resets, now);
  const days = (value: number | null) => (value === null ? "—" : text.days(numberText(language, value, 1)));
  const rows: [string, string][] = [
    [text.statTotal, `${numberText(language, stats.total)} (${text.statKinds(numberText(language, stats.regular), numberText(language, stats.banked))})`],
    [text.statSinceLast, days(stats.daysSinceLast)],
    [text.statLast30, numberText(language, stats.last30Days)],
    [text.statAverage, days(stats.averageGapDays)],
    [text.statMedian, days(stats.medianGapDays)],
    [
      text.statLongest,
      stats.longestGap ? `${days(stats.longestGap.days)} · ${text.gapRange(shortDate(stats.longestGap.from, now, language), shortDate(stats.longestGap.to, now, language))}` : "—",
    ],
  ];
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{text.statsTitle}</h2>
      <div className="uc-card uc-reset-stats">
        {rows.map(([label, value]) => (
          <div key={label} className="uc-reset-stat">
            <span className="uc-reset-stat-label">{label}</span>
            <span className="uc-reset-stat-value uc-num">{value}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function History({ resets, language, timeFormat, text }: { resets: CodexReset[]; language: Language; timeFormat: TimeFormat; text: InsightsMessages }) {
  const [expanded, setExpanded] = useState(false);
  const shown = expanded ? resets : resets.slice(0, HISTORY_PREVIEW);
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{text.historyTitle}</h2>
      <div className="uc-card uc-list-card">
        {shown.map((reset) => (
          <article key={reset.id} className="uc-reset-item">
            <div className="uc-reset-item-head">
              <span className={`uc-insight-badge${reset.kind === "banked" ? " is-accent" : ""}`}>{text.kind(reset.kind)}</span>
              <span className="uc-reset-item-time uc-num">{when(reset.announcedAt, timeFormat, language)}</span>
              <PostLink source={reset.source} text={text} compact />
            </div>
            <p className="uc-reset-post">{excerpt(reset.text, 220)}</p>
            {reset.source.kind === "observed" ? <span className="uc-reset-meta">{text.observed}</span> : null}
          </article>
        ))}
        {resets.length > HISTORY_PREVIEW ? (
          <button type="button" className="uc-insight-more" onClick={() => setExpanded(!expanded)}>
            {expanded ? text.showLess : text.showMore(resets.length - HISTORY_PREVIEW)}
          </button>
        ) : null}
      </div>
    </section>
  );
}

export function ResetsTab() {
  const { language, timeFormat } = useSettings();
  const text = insightsFor(language);
  const feeds = useFeeds(FEEDS);
  const refreshing = useInsights((state) => FEEDS.some((name) => state.refreshing[name]));
  const errors = useInsights((state) => state.feedErrors);
  const now = useNow();
  const status = useMemo(() => parseResetStatus(feeds.codexResetStatus?.body), [feeds.codexResetStatus]);
  const resets = useMemo(() => parseResets(feeds.codexResets?.body, [status?.latest ?? null]), [feeds.codexResets, status]);
  const loaded = feeds.codexResetStatus !== undefined && feeds.codexResets !== undefined;
  const empty = loaded && !feeds.codexResetStatus?.body && !feeds.codexResets?.body;
  const stale = FEEDS.some((name) => feeds[name]?.error);
  const checked = agoText(feeds.codexResetStatus?.checkedAt ?? feeds.codexResetStatus?.fetchedAt, now, language);
  const error = FEEDS.map((name) => feeds[name]?.error ?? errors[name]).find(Boolean);

  return (
    <>
      {!loaded ? <p className="uc-empty">{text.loading}</p> : null}
      {empty ? <p className="uc-insight-error">{text.failed(error ?? "")}</p> : null}
      {stale && !empty ? <p className="uc-insight-note">{text.staleNote}</p> : null}
      <StatusCards status={status} resets={resets} language={language} timeFormat={timeFormat} text={text} />
      {resets.length > 0 ? (
        <>
          <Forecast resets={resets} language={language} text={text} />
          <Stats resets={resets} language={language} text={text} />
          <History resets={resets} language={language} timeFormat={timeFormat} text={text} />
        </>
      ) : null}
      <div className="uc-insight-status">
        <span className="uc-insight-status-text">{checked ? text.fetchedAgo(checked) : ""}</span>
        <Button onClick={() => void refreshFeeds(FEEDS)} className="is-small" disabled={refreshing}>
          {refreshing ? text.refreshing : text.refresh}
        </Button>
      </div>
      <SourceLine text={text.resetsSource} url={SITE_URL} linkLabel={text.openLink} />
      <Disclosure title={text.methodTitle}>
        {text.resetsMethod.map((paragraph) => (
          <p key={paragraph}>{paragraph}</p>
        ))}
      </Disclosure>
    </>
  );
}
