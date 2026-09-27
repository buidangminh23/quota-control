/**
 * The Reset tab: when the latest Codex reset was, whether another is announced or hinted at
 * (codex-resets.com, which follows @thsottiaux on X), this app's estimate of the chance of one soon with how long the current wait
 * is against past gaps, a calendar of the last weeks, when in the week and the day announcements
 * land, the history's statistics and the latest resets with links to their posts.
 */
import { Fragment, useMemo, useState } from "react";
import { insightsFor, type InsightsMessages } from "@/i18n/insights";
import type { Language } from "@/i18n";
import { compactDuration, shortTime, timeOnDayLabel, type TimeFormat } from "@/model/format";
import {
  activeWatch,
  announcementPattern,
  currentWait,
  excerpt,
  FORECAST_HORIZONS,
  forecastResets,
  HOUR_BLOCKS,
  parseResets,
  parseResetStatus,
  resetCalendar,
  resetStats,
  type CodexReset,
  type ResetSource,
  type ResetStatus,
} from "@/model/insights/resets";
import { zonedParts } from "@/model/timeZone";
import { useNow, useSettings } from "@/state/hooks";
import { useInsights } from "@/state/insights";
import { tooltipProps } from "../ui/tooltip";
import { useFeeds } from "./data";
import { agoText, dateText, Disclosure, FeedStatus, LinkButton, numberText, percentText, RateBar, sinceText, SourceLine } from "./parts";

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

/** `12/09` in the device's zone, with the year only when it is not the current one. */
function shortDate(date: Date, now: Date, language: Language): string {
  const parts = zonedParts(date);
  if (parts.year !== zonedParts(now).year) return dateText(date, language);
  const day = String(parts.day).padStart(2, "0");
  const month = String(parts.month).padStart(2, "0");
  return language === "vi" ? `${day}/${month}` : `${month}/${day}`;
}

/** How far an announced time is: a countdown while ahead, an overdue note once it has passed. */
function dueLine(due: Date, now: Date, language: Language, text: InsightsMessages): string | null {
  if (due.getTime() > now.getTime()) {
    const left = compactDuration((due.getTime() - now.getTime()) / 1000, language);
    return left ? text.scheduledIn(left) : null;
  }
  const ago = agoText(due, now, language);
  return ago ? text.scheduledOverdue(ago) : null;
}

/** The latest reset the way codex-resets.com leads with it: how long ago in large type, then when. */
function LatestReset({ reset, language, timeFormat, text }: { reset: CodexReset; language: Language; timeFormat: TimeFormat; text: InsightsMessages }) {
  const now = useNow();
  const moment = timeOnDayLabel(reset.announcedAt, now, timeFormat, language, false);
  return (
    <article className="uc-card uc-reset-latest">
      <span className="uc-reset-latest-title">{text.latestTitle}</span>
      <span className="uc-reset-latest-ago">{sinceText(reset.announcedAt, now, language)}</span>
      <span className="uc-reset-meta uc-num">{reset.kind === "banked" ? `${moment} · ${text.kind("banked")}` : moment}</span>
    </article>
  );
}

function StatusCards({ status, resets, language, timeFormat, text }: { status: ResetStatus | null; resets: CodexReset[]; language: Language; timeFormat: TimeFormat; text: InsightsMessages }) {
  const now = useNow();
  const watch = activeWatch(status, now);
  const scheduled = status?.scheduled ?? null;
  const last = resets[0] ?? null;
  const due = scheduled?.scheduledFor ? dueLine(scheduled.scheduledFor, now, language, text) : null;

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
          {due ? <span className="uc-reset-meta">{due}</span> : null}
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
        </article>
      ) : null}
    </>
  );
}

function Forecast({ resets, language, timeFormat, text }: { resets: CodexReset[]; language: Language; timeFormat: TimeFormat; text: InsightsMessages }) {
  const now = useNow();
  const forecast = forecastResets(resets, now);
  const wait = currentWait(resets, now);
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
            {wait ? (
              <div className="uc-reset-wait">
                <span className="uc-reset-wait-line">{text.waitLine(text.days(numberText(language, wait.waitedDays, 1)), percentText(language, wait.shorterShare, 0))}</span>
                <RateBar rate={wait.shorterShare} low={null} high={null} />
                <span className="uc-reset-meta">
                  {(wait.medianMark.getTime() > now.getTime() ? text.medianMark : text.medianMarkPassed)(
                    text.days(numberText(language, wait.medianGapDays, 1)),
                    `${shortTime(wait.medianMark, timeFormat, language)} ${shortDate(wait.medianMark, now, language)}`,
                  )}
                </span>
              </div>
            ) : null}
            <p className="uc-insight-note">{text.forecastNote(numberText(language, forecast.resets))}</p>
            <p className="uc-insight-note">{text.forecastDisclaimer}</p>
          </>
        ) : (
          <p className="uc-empty">{text.forecastUnavailable}</p>
        )}
      </div>
    </section>
  );
}

function Calendar({ resets, language, text }: { resets: CodexReset[]; language: Language; text: InsightsMessages }) {
  const now = useNow();
  const weeks = useMemo(() => resetCalendar(resets, now), [resets, now]);
  const title = text.calendarTitle(weeks.length);
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{title}</h2>
      <div className="uc-card uc-reset-calendar">
        <div className="uc-reset-cal-grid" role="img" aria-label={title} style={{ gridTemplateColumns: `auto repeat(${weeks.length}, minmax(0, 1fr))` }}>
          <span />
          {weeks.map((week, index) => {
            const month = zonedParts(week[0]!.date).month - 1;
            const starts = index === 0 || month !== zonedParts(weeks[index - 1]![0]!.date).month - 1;
            return (
              <span key={week[0]!.date.getTime()} className="uc-reset-cal-month">
                {starts ? text.monthShort(month) : ""}
              </span>
            );
          })}
          {Array.from({ length: 7 }, (_, weekday) => (
            <Fragment key={weekday}>
              <span className="uc-reset-cal-label">{text.weekdayShort[weekday]}</span>
              {weeks.map((week) => {
                const day = week[weekday]!;
                const kind = day.kinds[day.kinds.length - 1];
                const label = kind ? text.calendarDay(dateText(day.date, language), day.kinds.map((item) => text.kind(item)).join(" + ")) : null;
                return (
                  <span
                    key={day.date.getTime()}
                    className={`uc-reset-cal-cell${kind ? ` is-${kind}` : ""}${day.isToday ? " is-today" : ""}${day.future ? " is-future" : ""}`}
                    {...tooltipProps(label)}
                  />
                );
              })}
            </Fragment>
          ))}
        </div>
        <div className="uc-reset-legend">
          <span>
            <i className="uc-reset-cal-cell is-regular" />
            {text.kind("regular")}
          </span>
          <span>
            <i className="uc-reset-cal-cell is-banked" />
            {text.kind("banked")}
          </span>
          <span>
            <i className="uc-reset-cal-cell is-today" />
            {text.calendarToday}
          </span>
        </div>
      </div>
    </section>
  );
}

function Bars({ counts, label, title, language }: { counts: number[]; label: (index: number) => string; title: string; language: Language }) {
  const max = Math.max(1, ...counts);
  return (
    <div className="uc-reset-pattern-block">
      <span className="uc-reset-meta">{title}</span>
      <div className="uc-reset-bars" role="img" aria-label={`${title}: ${counts.map((count, index) => `${label(index)} ${numberText(language, count)}`).join(", ")}`}>
        {counts.map((count, index) => (
          <div key={index} className={`uc-reset-bar${count === max ? " is-peak" : ""}`}>
            <span className="uc-reset-bar-count uc-num">{count > 0 ? numberText(language, count) : ""}</span>
            <span className="uc-reset-bar-track">
              <span className="uc-reset-bar-fill" style={{ height: `${Math.round((count / max) * 100)}%` }} />
            </span>
            <span className="uc-reset-bar-label">{label(index)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function Pattern({ resets, language, text }: { resets: CodexReset[]; language: Language; text: InsightsMessages }) {
  const pattern = useMemo(() => announcementPattern(resets), [resets]);
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{text.patternTitle}</h2>
      <div className="uc-card uc-reset-pattern">
        <Bars counts={pattern.weekdays} label={(index) => text.weekdayShort[index] ?? ""} title={text.patternWeekdays} language={language} />
        <Bars counts={pattern.hours} label={(index) => text.hourBlock(index * (24 / HOUR_BLOCKS))} title={text.patternHours} language={language} />
        <p className="uc-insight-note">{text.patternNote(numberText(language, pattern.total))}</p>
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
  const errors = useInsights((state) => state.feedErrors);
  const status = useMemo(() => parseResetStatus(feeds.codexResetStatus?.body), [feeds.codexResetStatus]);
  const resets = useMemo(() => parseResets(feeds.codexResets?.body, [status?.latest ?? null]), [feeds.codexResets, status]);
  const loaded = feeds.codexResetStatus !== undefined && feeds.codexResets !== undefined;
  const empty = loaded && !feeds.codexResetStatus?.body && !feeds.codexResets?.body;
  const stale = FEEDS.some((name) => feeds[name]?.error);
  const error = FEEDS.map((name) => feeds[name]?.error ?? errors[name]).find(Boolean);

  return (
    <>
      {!loaded ? <p className="uc-empty">{text.loading}</p> : null}
      {empty ? <p className="uc-insight-error">{text.failed(error ?? "")}</p> : null}
      {stale && !empty ? <p className="uc-insight-note">{text.staleNote}</p> : null}
      {resets[0] ? <LatestReset reset={resets[0]} language={language} timeFormat={timeFormat} text={text} /> : null}
      <StatusCards status={status} resets={resets} language={language} timeFormat={timeFormat} text={text} />
      {resets.length > 0 ? (
        <>
          <Forecast resets={resets} language={language} timeFormat={timeFormat} text={text} />
          <Calendar resets={resets} language={language} text={text} />
          <Pattern resets={resets} language={language} text={text} />
          <Stats resets={resets} language={language} text={text} />
          <History resets={resets} language={language} timeFormat={timeFormat} text={text} />
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
