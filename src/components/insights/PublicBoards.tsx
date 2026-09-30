/**
 * Công khai: public leaderboards across every kind of work. Epoch AI's index and its benchmarks by
 * category (code, agents, real work, math, science, knowledge, reasoning, writing, vision and 3D,
 * games, security, learning), Arena's human-preference boards (text, code, agent, search, documents,
 * vision, image and video generation and editing) and 3D Arena. Models the user runs are marked.
 */
import { useMemo, useRef, useState } from "react";
import { insightsFor, type InsightsMessages } from "@/i18n/insights";
import type { Language } from "@/i18n";
import type { PublicFeedSnapshot } from "@/lib/insightsTypes";
import type { ArenaBoard } from "@/model/insights/arena";
import { BENCHMARK_CATEGORIES, benchmarkLabel, benchmarkUrl, type BenchmarkBoard, type BenchmarkCategory } from "@/model/insights/epoch";
import { effortVariant, modelKey } from "@/model/insights/modelNames";
import { useSettings } from "@/state/hooks";
import { setPublicBoard, useInsights, type PublicBoardChoice } from "@/state/insights";
import { ChevronUpDown } from "../ui/icons";
import { openMenuAt, type MenuEntry } from "../ui/menu";
import { arena3dOf, arenaOf, epochBoardsOf, epochScoresOf, useFeeds } from "./data";
import { dayText, FeedStatus, LinkButton, numberText, percentText, SourceLine } from "./parts";

const LIST_PREVIEW = 10;
const RESULT_PREVIEW = 5;
const ARENA_3D_URL = "https://huggingface.co/spaces/3d-arena/3d-arena";
const ECI_URL = "https://epoch.ai/benchmarks/eci";

/** Whether `name` is one of the user's models, or that model at a stated effort. */
function usedBy(myKeys: ReadonlySet<string>, name: string): boolean {
  const key = modelKey(name);
  if (myKeys.has(key)) return true;
  for (const mine of myKeys) if (effortVariant(mine, key)) return true;
  return false;
}

function FeedState({ snapshot, text }: { snapshot: PublicFeedSnapshot | undefined; text: InsightsMessages }) {
  const error = useInsights((state) => (snapshot ? state.feedErrors[snapshot.name] : undefined));
  if (!snapshot) return <p className="uc-empty">{text.loading}</p>;
  if (!snapshot.body) return <p className="uc-insight-error">{text.failed(snapshot.error ?? error ?? "")}</p>;
  if (snapshot.stale) return <p className="uc-insight-note">{text.staleNote}</p>;
  return null;
}

function Badges({ inUse, open, text }: { inUse: boolean; open?: boolean; text: InsightsMessages }) {
  return (
    <>
      {inUse ? <span className="uc-insight-badge is-accent">{text.inUse}</span> : null}
      {open ? <span className="uc-insight-badge">{text.openWeights}</span> : null}
    </>
  );
}

function ShowMore({ total, expanded, onToggle, preview, text }: { total: number; expanded: boolean; onToggle: () => void; preview: number; text: InsightsMessages }) {
  if (total <= preview) return null;
  return (
    <button type="button" className="uc-insight-more" onClick={onToggle}>
      {expanded ? text.showLess : text.showMore(total - preview)}
    </button>
  );
}

function EciBoard({ myKeys, language, text }: { myKeys: ReadonlySet<string>; language: Language; text: InsightsMessages }) {
  const feeds = useFeeds(["epochScores"]);
  const snapshot = feeds.epochScores;
  const [expanded, setExpanded] = useState(false);
  const models = epochScoresOf(snapshot);
  const shown = expanded ? models : models.slice(0, LIST_PREVIEW);
  return (
    <>
      <FeedState snapshot={snapshot} text={text} />
      {models.length > 0 ? (
        <div className="uc-card uc-list-card">
          {shown.map((model, index) => (
            <div key={model.name} className="uc-list-row uc-insight-row">
              <span className="uc-insight-rank uc-num">{index + 1}</span>
              <span className="uc-list-text">
                <span className="uc-insight-row-title uc-truncate">{model.name}</span>
                <span className="uc-list-subtitle uc-insight-row-meta">
                  <span className="uc-truncate">{[model.organization, model.released ? dayText(model.released, language) : ""].filter(Boolean).join(" · ")}</span>
                  <Badges inUse={usedBy(myKeys, model.name)} open={model.openWeights} text={text} />
                </span>
              </span>
              <span className="uc-insight-row-value">
                <span className="uc-num">{numberText(language, model.eci, 1)}</span>
                {model.low !== null && model.high !== null ? (
                  <span className="uc-insight-row-range uc-num">
                    {numberText(language, model.low, 1)}–{numberText(language, model.high, 1)}
                  </span>
                ) : null}
              </span>
            </div>
          ))}
          <ShowMore total={models.length} expanded={expanded} onToggle={() => setExpanded(!expanded)} preview={LIST_PREVIEW} text={text} />
        </div>
      ) : null}
      <p className="uc-insight-note">{text.eciNote}</p>
      <FeedStatus names={["epochScores"]} shown={snapshot} language={language} text={text} tooltip={text.refreshTooltip} />
      <SourceLine text={text.epochSource} url={ECI_URL} linkLabel={text.openLink} />
    </>
  );
}

function BenchmarkCard({ board, myKeys, language, text }: { board: BenchmarkBoard; myKeys: ReadonlySet<string>; language: Language; text: InsightsMessages }) {
  const [expanded, setExpanded] = useState(false);
  const shown = expanded ? board.results : board.results.slice(0, RESULT_PREVIEW);
  const url = benchmarkUrl(board.name);
  const description = language === "vi" ? board.info.vi : board.info.en;
  return (
    <article className={`uc-card uc-benchmark-card${board.dated ? " is-dated" : ""}`} aria-label={benchmarkLabel(board.name)}>
      <header className="uc-benchmark-head">
        <span className="uc-benchmark-name">{benchmarkLabel(board.name)}</span>
        <span className="uc-benchmark-count">{text.modelCount(board.results.length)}</span>
        {url ? <LinkButton url={url} label={text.openLink} /> : null}
      </header>
      {description ? <p className="uc-benchmark-description">{description}</p> : null}
      {board.dated ? (
        <p className="uc-benchmark-dated">{board.info.supersededBy ? text.superseded(benchmarkLabel(board.info.supersededBy)) : text.datedNote}</p>
      ) : null}
      <div className="uc-benchmark-results">
        {shown.map((result, index) => (
          <div key={result.model} className="uc-benchmark-result">
            <span className="uc-insight-rank uc-num">{index + 1}</span>
            <span className="uc-benchmark-model">
              <span className="uc-truncate">{result.model}</span>
              <Badges inUse={usedBy(myKeys, result.model)} text={text} />
            </span>
            <span className="uc-num uc-benchmark-value">{percentText(language, result.performance)}</span>
          </div>
        ))}
      </div>
      <ShowMore total={board.results.length} expanded={expanded} onToggle={() => setExpanded(!expanded)} preview={RESULT_PREVIEW} text={text} />
    </article>
  );
}

function CategoryBoard({ category, myKeys, language, text }: { category: BenchmarkCategory; myKeys: ReadonlySet<string>; language: Language; text: InsightsMessages }) {
  const feeds = useFeeds(["epochBenchmarks"]);
  const snapshot = feeds.epochBenchmarks;
  const boards = epochBoardsOf(snapshot).filter((board) => board.info.category === category);
  return (
    <>
      <FeedState snapshot={snapshot} text={text} />
      {snapshot?.body && boards.length === 0 ? <p className="uc-empty">{text.noResults}</p> : null}
      {boards.map((board) => (
        <BenchmarkCard key={board.name} board={board} myKeys={myKeys} language={language} text={text} />
      ))}
      <FeedStatus names={["epochBenchmarks"]} shown={snapshot} language={language} text={text} tooltip={text.refreshTooltip} />
      <SourceLine text={text.epochSource} />
    </>
  );
}

function ArenaEntries({ board, myKeys, language, text }: { board: ArenaBoard; myKeys: ReadonlySet<string>; language: Language; text: InsightsMessages }) {
  const [expanded, setExpanded] = useState(false);
  if (board.slug === "agent") {
    return (
      <div className="uc-card uc-list-card">
        {board.agentEntries.map((entry) => (
          <div key={entry.model} className="uc-list-row uc-insight-row is-top">
            <span className="uc-insight-rank uc-num">{entry.rank}</span>
            <span className="uc-list-text">
              <span className="uc-insight-row-title uc-truncate">{entry.model}</span>
              <span className="uc-list-subtitle uc-insight-row-meta">
                <span className="uc-truncate">{[entry.vendor, entry.sessions !== null ? text.sessions(numberText(language, entry.sessions)) : ""].filter(Boolean).join(" · ")}</span>
                <Badges inUse={usedBy(myKeys, entry.model)} text={text} />
              </span>
              <span className="uc-insight-dimensions">
                {entry.dimensions.map((dimension) => (
                  <span key={dimension.name} className="uc-num">
                    {dimension.name} {numberText(language, dimension.score, 2)}
                    {dimension.ci !== null ? ` ±${numberText(language, dimension.ci, 2)}` : ""}
                  </span>
                ))}
              </span>
            </span>
          </div>
        ))}
      </div>
    );
  }
  const shown = expanded ? board.entries : board.entries.slice(0, LIST_PREVIEW);
  return (
    <div className="uc-card uc-list-card">
      {shown.map((entry) => (
        <div key={entry.model} className="uc-list-row uc-insight-row">
          <span className="uc-insight-rank uc-num">{entry.rank}</span>
          <span className="uc-list-text">
            <span className="uc-insight-row-title uc-truncate">{entry.model}</span>
            <span className="uc-list-subtitle uc-insight-row-meta">
              <span className="uc-truncate">{[entry.vendor, entry.votes !== null ? text.votes(numberText(language, entry.votes)) : ""].filter(Boolean).join(" · ")}</span>
              <Badges inUse={usedBy(myKeys, entry.model)} open={entry.openWeights} text={text} />
            </span>
          </span>
          <span className="uc-insight-row-value">
            <span className="uc-num">{numberText(language, entry.score)}</span>
            {entry.ci !== null ? <span className="uc-insight-row-range uc-num">±{numberText(language, entry.ci)}</span> : null}
          </span>
        </div>
      ))}
      <ShowMore total={board.entries.length} expanded={expanded} onToggle={() => setExpanded(!expanded)} preview={LIST_PREVIEW} text={text} />
    </div>
  );
}

function ArenaBoardView({ slug, myKeys, language, text }: { slug: string; myKeys: ReadonlySet<string>; language: Language; text: InsightsMessages }) {
  const feeds = useFeeds(["arena"]);
  const snapshot = feeds.arena;
  const arena = arenaOf(snapshot);
  const board = arena?.boards.find((candidate) => candidate.slug === slug);
  return (
    <>
      <FeedState snapshot={snapshot} text={text} />
      {snapshot?.body && !board ? <p className="uc-empty">{text.noResults}</p> : null}
      {board ? <ArenaEntries board={board} myKeys={myKeys} language={language} text={text} /> : null}
      <p className="uc-insight-note">{slug === "agent" ? text.agentNote : text.arenaNote}</p>
      <FeedStatus names={["arena"]} shown={snapshot} language={language} text={text} tooltip={text.refreshTooltip} />
      {arena ? <SourceLine text={text.arenaSource(arena.date ? dayText(arena.date, language) : "—")} url={board?.sourceUrl} linkLabel={text.openLink} /> : null}
    </>
  );
}

function Arena3dView({ myKeys, language, text }: { myKeys: ReadonlySet<string>; language: Language; text: InsightsMessages }) {
  const feeds = useFeeds(["arena3d"]);
  const snapshot = feeds.arena3d;
  const entries = arena3dOf(snapshot);
  const [expanded, setExpanded] = useState(false);
  const shown = expanded ? entries : entries.slice(0, LIST_PREVIEW);
  return (
    <>
      <FeedState snapshot={snapshot} text={text} />
      {entries.length > 0 ? (
        <div className="uc-card uc-list-card">
          {shown.map((entry) => (
            <div key={entry.model} className="uc-list-row uc-insight-row">
              <span className="uc-insight-rank uc-num">{entry.rank}</span>
              <span className="uc-list-text">
                <span className="uc-insight-row-title uc-truncate">{entry.model}</span>
                <span className="uc-list-subtitle uc-insight-row-meta">
                  {entry.votes !== null ? <span className="uc-truncate">{text.votes(numberText(language, entry.votes))}</span> : null}
                  <Badges inUse={usedBy(myKeys, entry.model)} open={entry.openSource} text={text} />
                </span>
              </span>
              <span className="uc-insight-row-value uc-num">{numberText(language, entry.score)}</span>
            </div>
          ))}
          <ShowMore total={entries.length} expanded={expanded} onToggle={() => setExpanded(!expanded)} preview={LIST_PREVIEW} text={text} />
        </div>
      ) : null}
      <p className="uc-insight-note">{text.arena3dNote}</p>
      <FeedStatus names={["arena3d"]} shown={snapshot} language={language} text={text} tooltip={text.refreshTooltip} />
      <SourceLine text={text.arena3dSource} url={ARENA_3D_URL} linkLabel={text.openLink} />
    </>
  );
}

function boardName(choice: PublicBoardChoice, text: InsightsMessages): string {
  switch (choice.kind) {
    case "eci":
      return `${text.epochGroup} · ${text.eciBoard}`;
    case "epochCategory":
      return `${text.epochGroup} · ${text.category(choice.category as BenchmarkCategory)}`;
    case "arena":
      return `${text.arenaGroup} · ${text.arenaBoard(choice.board as ArenaBoard["slug"])}`;
    case "arena3d":
      return text.arena3dBoard;
  }
}

function same(a: PublicBoardChoice, b: PublicBoardChoice): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

function BoardPicker({ choice, categories, arenaBoards, text }: { choice: PublicBoardChoice; categories: BenchmarkCategory[]; arenaBoards: ArenaBoard["slug"][]; text: InsightsMessages }) {
  const ref = useRef<HTMLButtonElement>(null);
  const item = (next: PublicBoardChoice, label: string): MenuEntry => ({ kind: "item", label, checked: same(choice, next), onSelect: () => setPublicBoard(next) });
  const open = () => {
    if (!ref.current) return;
    openMenuAt(
      ref.current,
      [
        {
          kind: "submenu",
          label: text.epochGroup,
          entries: [item({ kind: "eci" }, text.eciBoard), { kind: "separator" }, ...categories.map((category) => item({ kind: "epochCategory", category }, text.category(category)))],
        },
        { kind: "submenu", label: text.arenaGroup, entries: arenaBoards.map((board) => item({ kind: "arena", board }, text.arenaBoard(board))) },
        item({ kind: "arena3d" }, text.arena3dBoard),
      ],
      { align: "start", checkable: true },
    );
  };
  const label = boardName(choice, text);
  return (
    <button ref={ref} type="button" className="uc-picker uc-board-picker" aria-haspopup="menu" aria-label={`${text.boardLabel}: ${label}`} onClick={open}>
      <span className="uc-truncate">{label}</span>
      <ChevronUpDown size={10} />
    </button>
  );
}

export function PublicBoards() {
  const { language } = useSettings();
  const text = insightsFor(language);
  const choice = useInsights((state) => state.board);
  const rows = useInsights((state) => state.quality?.rows);
  const feeds = useFeeds(["epochBenchmarks", "arena"]);
  const myKeys = useMemo(() => new Set((rows ?? []).map((row) => modelKey(row.model))), [rows]);
  const categories = useMemo(() => {
    const present = new Set(epochBoardsOf(feeds.epochBenchmarks).map((board) => board.info.category));
    return BENCHMARK_CATEGORIES.filter((category) => category !== "other" || present.has(category));
  }, [feeds.epochBenchmarks]);
  const arenaBoards = useMemo(() => (arenaOf(feeds.arena)?.boards ?? []).map((board) => board.slug), [feeds.arena]);

  return (
    <>
      <BoardPicker choice={choice} categories={categories} arenaBoards={arenaBoards} text={text} />
      {choice.kind === "eci" ? <EciBoard myKeys={myKeys} language={language} text={text} /> : null}
      {choice.kind === "epochCategory" ? <CategoryBoard category={choice.category as BenchmarkCategory} myKeys={myKeys} language={language} text={text} /> : null}
      {choice.kind === "arena" ? <ArenaBoardView slug={choice.board} myKeys={myKeys} language={language} text={text} /> : null}
      {choice.kind === "arena3d" ? <Arena3dView myKeys={myKeys} language={language} text={text} /> : null}
    </>
  );
}
