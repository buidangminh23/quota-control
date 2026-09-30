/**
 * The Reset tab's Claude view: the cards of the Codex view over claude-resets.com's history, and
 * what only Claude's catalog has. A banked reset that can still be applied gets a card with its
 * deadline, which the user can mark as applied; every reset says who it covered and whether that
 * includes the plans connected here; limit changes are listed apart, since they never reset usage;
 * and the last card sets Claude against Codex over the time both were tracked.
 */
import { useMemo, useState } from "react";
import { insightsFor, type InsightsMessages } from "@/i18n/insights";
import type { Language } from "@/i18n";
import { buildClaudePresentation, type ClaudeBankedCard, type ClaudePresentation, type ComparePresentation } from "@/model/insights/claudePresentation";
import { parseClaudeResets } from "@/model/insights/claudeResets";
import { parseResets } from "@/model/insights/resets";
import { useClaudeAccountPlans, useClaudePlans } from "@/state/claudePlans";
import { useNow, useSettings } from "@/state/hooks";
import { useInsights } from "@/state/insights";
import { updateSettings, useApp } from "@/state/store";
import { Button } from "../ui/controls";
import { ProviderMark } from "../ui/ProviderMark";
import { useFeeds } from "./data";
import { Disclosure, FeedStatus, numberText, SourceLine } from "./parts";
import { Calendar, Forecast, History, LatestReset, Pattern, PostAuthor, PostLink, RowAuthor, Stats, StatusCards } from "./ResetParts";

const SITE_URL = "https://claude-resets.com";
const FEEDS = ["claudeResets"] as const;
const COMPARE_FEEDS = ["claudeResets", "codexResets"] as const;
const CHANGES_PREVIEW = 3;
const MARK_SIZE = 12;

/** Mark a banked reset as applied, or take the mark back. */
export function setBankedUsed(resetId: string, used: boolean): void {
  const current = useApp.getState().settings.usedBankedResets.filter((id) => id !== resetId);
  updateSettings({ usedBankedResets: used ? [...current, resetId] : current });
}

/**
 * The banked resets still to apply. The one that is the latest reset leaves its author and words
 * to the latest card above when that card quotes them, and keeps its deadline and how to apply it.
 */
function BankedCards({ cards, quotedAbove, text }: { cards: readonly ClaudeBankedCard[]; quotedAbove: boolean; text: InsightsMessages }) {
  const repeats = (card: ClaudeBankedCard) => quotedAbove && card.sameAsLatest === true;
  return (
    <>
      {cards.map((card) =>
        card.used ? (
          <article key={card.id} className="uc-card uc-reset-status is-used">
            <span className="uc-reset-meta">{text.claude.bankedUsed}</span>
            <Button className="is-small" onClick={() => setBankedUsed(card.resetId, false)}>
              {text.claude.bankedUndo}
            </Button>
          </article>
        ) : (
          <article key={card.id} className="uc-card uc-reset-status is-banked">
            <span className="uc-reset-status-title">{card.title}</span>
            {card.author && !repeats(card) ? <PostAuthor author={card.author} /> : null}
            {card.due ? <span className="uc-reset-banked-left uc-num">{card.due}</span> : null}
            {card.excerpt !== undefined && !repeats(card) ? <p className="uc-reset-post">{card.excerpt}</p> : null}
            {card.meta.map((meta) => (
              <span key={meta} className="uc-reset-meta">
                {meta}
              </span>
            ))}
            <span className="uc-reset-meta">{card.how}</span>
            <div className="uc-reset-actions">
              {card.url ? <PostLink source={{ kind: "x_post", url: card.url }} text={text} /> : null}
              <Button className="is-small" onClick={() => setBankedUsed(card.resetId, true)}>
                {text.claude.bankedMarkUsed}
              </Button>
            </div>
          </article>
        ),
      )}
    </>
  );
}

function Changes({ presentation, text }: { presentation: ClaudePresentation; text: InsightsMessages }) {
  const [expanded, setExpanded] = useState(false);
  if (presentation.changes.length === 0) return null;
  const shown = expanded ? presentation.changes : presentation.changes.slice(0, CHANGES_PREVIEW);
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{presentation.changesTitle}</h2>
      <div className="uc-card uc-list-card">
        {shown.map((change) => (
          <article key={change.id} className="uc-reset-item">
            <div className="uc-reset-item-head">
              <RowAuthor author={change.author} />
              <span className="uc-insight-badge">{text.claude.changeBadge}</span>
              {change.provisional ? <span className="uc-insight-badge is-notice">{change.provisional}</span> : null}
              <span className="uc-reset-item-time uc-num">{change.when}</span>
              {change.url ? <PostLink source={{ kind: "x_post", url: change.url }} text={text} compact /> : null}
            </div>
            <p className="uc-reset-post">{change.excerpt}</p>
            {change.scope ? <span className="uc-reset-meta">{change.scope}</span> : null}
          </article>
        ))}
        {presentation.changes.length > CHANGES_PREVIEW ? (
          <button type="button" className="uc-insight-more" onClick={() => setExpanded(!expanded)}>
            {expanded ? text.showLess : text.showMore(presentation.changes.length - CHANGES_PREVIEW)}
          </button>
        ) : null}
      </div>
      <p className="uc-insight-note">{presentation.changesNote}</p>
    </section>
  );
}

function Compare({ compare, language, text }: { compare: ComparePresentation; language: Language; text: InsightsMessages }) {
  const max = Math.max(1, ...compare.months.flatMap((month) => [month.claude, month.codex]));
  const height = (count: number) => `${Math.round((count / max) * 100)}%`;
  const count = (value: number) => (value > 0 ? numberText(language, value) : "");
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{compare.title}</h2>
      <div className="uc-card uc-reset-compare">
        <table className="uc-reset-compare-table">
          <thead>
            <tr>
              <td />
              <th scope="col">
                <span className="uc-reset-compare-head">
                  <ProviderMark brand="claude" size={MARK_SIZE} />
                  {text.resetProvider("claude")}
                </span>
              </th>
              <th scope="col">
                <span className="uc-reset-compare-head">
                  <ProviderMark brand="codex" size={MARK_SIZE} />
                  {text.resetProvider("codex")}
                </span>
              </th>
            </tr>
          </thead>
          <tbody>
            {compare.rows.map((row) => (
              <tr key={row.label}>
                <th scope="row">{row.label}</th>
                <td className="uc-num">{row.claude}</td>
                <td className="uc-num">{row.codex}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="uc-reset-pattern-block">
          <span className="uc-reset-meta">{compare.monthsTitle}</span>
          <div className="uc-reset-bars" role="img" aria-label={`${compare.monthsTitle}: ${compare.months.map((month) => month.summary).join("; ")}`}>
            {compare.months.map((month, index) => (
              <div key={index} className="uc-reset-bar is-pair">
                <span className="uc-reset-bar-count uc-num">
                  <span>{count(month.claude)}</span>
                  <span>{count(month.codex)}</span>
                </span>
                <span className="uc-reset-bar-track">
                  <span className="uc-reset-bar-fill is-claude" style={{ height: height(month.claude) }} />
                  <span className="uc-reset-bar-fill is-codex" style={{ height: height(month.codex) }} />
                </span>
                <span className="uc-reset-bar-label">{month.label}</span>
              </div>
            ))}
          </div>
        </div>
        <p className="uc-insight-note">{compare.since}</p>
      </div>
    </section>
  );
}

export function ClaudeResets() {
  const { language, timeFormat, usedBankedResets } = useSettings();
  const text = insightsFor(language);
  const feeds = useFeeds(COMPARE_FEEDS);
  const errors = useInsights((state) => state.feedErrors);
  const plans = useClaudePlans();
  const accounts = useClaudeAccountPlans();
  const feed = useMemo(() => parseClaudeResets(feeds.claudeResets?.body), [feeds.claudeResets]);
  const codex = useMemo(() => parseResets(feeds.codexResets?.body), [feeds.codexResets]);
  const now = useNow();
  const presentation = useMemo(
    () => (feed ? buildClaudePresentation({ feed, codex, plans, accounts: accounts ?? [], used: usedBankedResets, now, language, timeFormat }) : null),
    [feed, codex, plans, accounts, usedBankedResets, now, language, timeFormat],
  );
  const loaded = feeds.claudeResets !== undefined;
  const empty = loaded && !presentation;
  const error = feeds.claudeResets?.error ?? errors.claudeResets;

  return (
    <>
      {!loaded ? <p className="uc-empty">{text.loading}</p> : null}
      {empty ? <p className="uc-insight-error">{text.failed(error ?? "")}</p> : null}
      {feeds.claudeResets?.error && presentation ? <p className="uc-insight-note">{text.staleNote}</p> : null}
      {presentation?.notices.map((notice) => (
        <p key={notice} className="uc-insight-note">
          {notice}
        </p>
      ))}
      {presentation && feed ? (
        <>
          {presentation.latest ? <LatestReset latest={presentation.latest} text={text} /> : null}
          <BankedCards cards={presentation.banked} quotedAbove={presentation.latest?.excerpt !== undefined} text={text} />
          <StatusCards cards={presentation.statuses} text={text} />
          {feed.resets.length > 0 ? (
            <>
              <Forecast forecast={presentation.forecast} />
              <Calendar resets={feed.resets} language={language} text={text} />
              <Pattern resets={feed.resets} language={language} text={text} />
              <Stats presentation={presentation} />
              <History presentation={presentation} text={text} />
            </>
          ) : null}
          <Changes presentation={presentation} text={text} />
          {presentation.compare ? <Compare compare={presentation.compare} language={language} text={text} /> : null}
        </>
      ) : null}
      <FeedStatus names={FEEDS} shown={feeds.claudeResets} language={language} text={text} />
      <SourceLine text={text.claude.source} url={SITE_URL} linkLabel={text.openLink} />
      <Disclosure title={text.methodTitle}>
        {text.claude.method.map((paragraph) => (
          <p key={paragraph}>{paragraph}</p>
        ))}
      </Disclosure>
    </>
  );
}
