/**
 * The cards the Reset tab is made of, shared by its Codex and Claude views: the latest reset, what
 * is announced or hinted at, this app's estimate of the chance of one soon with how long the
 * current wait is against past gaps, a calendar of the last weeks, when in the week and the day
 * announcements land, the history's statistics and the latest resets with links to their posts.
 */
import { Fragment, useMemo, useState } from "react";
import type { InsightsMessages } from "@/i18n/insights";
import type { Language } from "@/i18n";
import type { GlanceResetAuthor, GlanceResetPresentation } from "@/model/glance";
import { announcementPattern, HOUR_BLOCKS, resetCalendar, type CodexReset, type ResetSource } from "@/model/insights/resets";
import { zonedParts } from "@/model/timeZone";
import { useNow } from "@/state/hooks";
import { ResetAuthorAvatar } from "../ui/ResetAuthorAvatar";
import { tooltipProps } from "../ui/tooltip";
import { dateText, LinkButton, numberText, RateBar } from "./parts";

const HISTORY_PREVIEW = 8;

export function PostLink({ source, text, compact = false }: { source: ResetSource; text: InsightsMessages; compact?: boolean }) {
  if (!source.url) return null;
  return compact ? (
    <LinkButton url={source.url} label={text.openPost} />
  ) : (
    <LinkButton url={source.url} label={text.openPost}>
      <span>{text.openPost}</span>
    </LinkButton>
  );
}

export function PostAuthor({ author }: { author: GlanceResetAuthor }) {
  return (
    <span className="uc-reset-author">
      <ResetAuthorAvatar size={22} handle={author.handle} />
      <span>{author.handle}</span>
    </span>
  );
}

export function LatestReset({ latest }: { latest: NonNullable<GlanceResetPresentation["latest"]> }) {
  return (
    <article className="uc-card uc-reset-latest">
      <span className="uc-reset-latest-title">{latest.title}</span>
      {latest.author ? <PostAuthor author={latest.author} /> : null}
      <span className="uc-reset-latest-ago">{latest.ago}</span>
      <span className="uc-reset-meta uc-num">{latest.meta}</span>
      {latest.notes?.map((note) => (
        <span key={note} className="uc-reset-meta">
          {note}
        </span>
      ))}
    </article>
  );
}

export function StatusCards({ cards, text }: { cards: GlanceResetPresentation["statuses"]; text: InsightsMessages }) {
  return <>{cards.map((card) => (
    <article key={card.id} className={`uc-card uc-reset-status${card.kind === "quiet" ? "" : ` is-${card.kind}`}${card.level ? ` is-${card.level}` : ""}`}>
      <span className="uc-reset-status-title">{card.title}</span>
      {card.kind === "watch" && card.meta.length > 1 ? <span className="uc-reset-meta">{card.meta[0]}</span> : null}
      {card.author ? <PostAuthor author={card.author} /> : null}
      {card.excerpt !== undefined ? <p className="uc-reset-post">{card.excerpt}</p> : null}
      {card.meta.slice(card.kind === "watch" && card.meta.length > 1 ? 1 : 0).map((meta, index) => <span key={index} className="uc-reset-meta">{meta}</span>)}
      {card.due ? <span className="uc-reset-meta">{card.due}</span> : null}
      {card.url ? <PostLink source={{ kind: "x_post", url: card.url }} text={text} /> : null}
    </article>
  ))}</>;
}

export function Forecast({ forecast }: { forecast: GlanceResetPresentation["forecast"] }) {
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{forecast.title}</h2>
      <div className="uc-card uc-reset-forecast">
        {forecast.chances.length ? (
          <>
            <div className="uc-reset-horizons">
              {forecast.chances.map((chance) => (
                <div key={chance.days} className="uc-reset-horizon">
                  <span className="uc-reset-horizon-value uc-num">{chance.percent}</span>
                  <RateBar rate={chance.fraction} low={null} high={null} />
                  <span className="uc-reset-horizon-label">{chance.label}</span>
                </div>
              ))}
            </div>
            {forecast.wait !== undefined ? (
              <div className="uc-reset-wait">
                <span className="uc-reset-wait-line">{forecast.wait}</span>
                <RateBar rate={forecast.waitFraction ?? 0} low={null} high={null} />
                <span className="uc-reset-meta">{forecast.median}</span>
              </div>
            ) : null}
            <p className="uc-insight-note">{forecast.sampleNote}</p>
            {forecast.reliability ? <p className="uc-insight-note">{forecast.reliability}</p> : null}
            <p className="uc-insight-note">{forecast.disclaimer}</p>
          </>
        ) : (
          <p className="uc-empty">{forecast.unavailable}</p>
        )}
      </div>
    </section>
  );
}

export function Calendar({ resets, language, text }: { resets: readonly CodexReset[]; language: Language; text: InsightsMessages }) {
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

export function Pattern({ resets, language, text }: { resets: readonly CodexReset[]; language: Language; text: InsightsMessages }) {
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

export function Stats({ presentation }: { presentation: Pick<GlanceResetPresentation, "statsTitle" | "stats"> }) {
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{presentation.statsTitle}</h2>
      <div className="uc-card uc-reset-stats">
        {presentation.stats.map(({ label, value }) => (
          <div key={label} className="uc-reset-stat">
            <span className="uc-reset-stat-label">{label}</span>
            <span className="uc-reset-stat-value uc-num">{value}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

export function History({ presentation, text }: { presentation: Pick<GlanceResetPresentation, "historyTitle" | "history">; text: InsightsMessages }) {
  const [expanded, setExpanded] = useState(false);
  const shown = expanded ? presentation.history : presentation.history.slice(0, HISTORY_PREVIEW);
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{presentation.historyTitle}</h2>
      <div className="uc-card uc-list-card">
        {shown.map((reset) => (
          <article key={reset.id} className="uc-reset-item">
            <div className="uc-reset-item-head">
              {reset.author ? <ResetAuthorAvatar size={18} handle={reset.author.handle} /> : null}
              <span className={`uc-insight-badge${reset.kind === "banked" ? " is-accent" : ""}`}>{reset.kindLabel}</span>
              {reset.provisional ? <span className="uc-insight-badge is-notice">{reset.provisional}</span> : null}
              <span className="uc-reset-item-time uc-num">{reset.when}</span>
              {reset.url ? <PostLink source={{ kind: "x_post", url: reset.url }} text={text} compact /> : null}
            </div>
            <p className="uc-reset-post">{reset.excerpt}</p>
            {reset.scope ? <span className="uc-reset-meta">{reset.scope}</span> : null}
            {reset.observed ? <span className="uc-reset-meta">{reset.observed}</span> : null}
          </article>
        ))}
        {presentation.history.length > HISTORY_PREVIEW ? (
          <button type="button" className="uc-insight-more" onClick={() => setExpanded(!expanded)}>
            {expanded ? text.showLess : text.showMore(presentation.history.length - HISTORY_PREVIEW)}
          </button>
        ) : null}
      </div>
    </section>
  );
}
