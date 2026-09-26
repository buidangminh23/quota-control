/**
 * So sánh: up to three models side by side across the user's projects, Epoch AI and Arena. A row
 * marks its best value; a lead counts as clear only when the intervals do not overlap, otherwise
 * the row says the data cannot separate them yet.
 */
import { useMemo, useState } from "react";
import { insightsFor, type InsightsMessages } from "@/i18n/insights";
import type { Language } from "@/i18n";
import { compareCandidates, compareRows, MAX_COMPARED, searchCandidates, type CompareCandidate, type CompareCell, type CompareInput, type CompareRow } from "@/model/insights/compare";
import { benchmarkLabel, type BenchmarkCategory } from "@/model/insights/epoch";
import { modelKey } from "@/model/insights/modelNames";
import { summarizeQuality } from "@/model/insights/quality";
import { useSettings } from "@/state/hooks";
import { setCompared, useInsights } from "@/state/insights";
import { CheckIcon, CloseIcon, PlusIcon } from "../ui/icons";
import { tooltipProps } from "../ui/tooltip";
import { arena3dOf, arenaOf, epochBoardsOf, epochScoresOf, useFeeds } from "./data";
import { numberText } from "./parts";

const SEARCH_LIMIT = 8;

type Group = "mine" | "epoch" | "arena";

function groupOf(row: CompareRow): Group {
  switch (row.kind) {
    case "mineScore":
    case "minePart":
    case "mineTurns":
      return "mine";
    case "eci":
    case "benchmark":
      return "epoch";
    default:
      return "arena";
  }
}

function rowLabel(row: CompareRow, text: InsightsMessages): string {
  switch (row.kind) {
    case "mineScore":
      return text.mineScore;
    case "minePart":
      return text.part(row.part);
    case "mineTurns":
      return text.mineTurns;
    case "eci":
      return text.eciRow;
    case "benchmark":
      return benchmarkLabel(row.benchmark);
    case "arena":
      return text.arenaRow(text.arenaBoard(row.board));
    case "arenaAgent":
      return text.arenaAgentRow;
    case "arena3d":
      return text.arena3dRow;
  }
}

function cellText(row: CompareRow, cell: CompareCell, language: Language, text: InsightsMessages): { main: string; detail: string | null } {
  const range = cell.low !== null && cell.high !== null ? `${numberText(language, cell.low, 1)}–${numberText(language, cell.high, 1)}` : null;
  switch (row.kind) {
    case "mineScore":
    case "eci":
      return { main: numberText(language, cell.value, 1), detail: range };
    case "minePart":
      return { main: `${numberText(language, cell.value, 1)}%`, detail: cell.samples !== undefined ? `n=${numberText(language, cell.samples)}` : null };
    case "mineTurns":
      return { main: numberText(language, cell.value), detail: null };
    case "benchmark":
      return { main: `${numberText(language, cell.value, 1)}%`, detail: null };
    case "arena": {
      const margin = cell.high !== null ? `±${numberText(language, cell.high - cell.value)}` : null;
      return { main: numberText(language, cell.value), detail: [margin, cell.effort ? text.effort(cell.effort) : null].filter(Boolean).join(" · ") || null };
    }
    case "arenaAgent":
      return { main: text.rank(numberText(language, cell.value)), detail: cell.effort ? text.effort(cell.effort) : null };
    case "arena3d":
      return { main: numberText(language, cell.value), detail: null };
  }
}

function hasIntervals(row: CompareRow): boolean {
  return row.cells.filter((cell) => cell && cell.low !== null && cell.high !== null).length > 1;
}

function ModelPicker({ candidates, selected, text }: { candidates: CompareCandidate[]; selected: string[]; text: InsightsMessages }) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const full = selected.length >= MAX_COMPARED;
  const matches = useMemo(() => searchCandidates(candidates, query, selected, SEARCH_LIMIT), [candidates, query, selected]);
  const add = (key: string) => {
    setCompared([...selected, key]);
    setQuery("");
    setOpen(false);
  };
  return (
    <div className="uc-compare-picker">
      <div className="uc-compare-chips">
        {selected.map((key) => {
          const name = candidates.find((candidate) => candidate.key === key)?.name ?? key;
          return (
            <span key={key} className="uc-compare-chip">
              <span className="uc-truncate">{name}</span>
              <button type="button" className="uc-icon-button" aria-label={text.removeModel(name)} onClick={() => setCompared(selected.filter((item) => item !== key))}>
                <CloseIcon size={8} />
              </button>
            </span>
          );
        })}
        {!full ? (
          <button type="button" className="uc-compare-add" aria-expanded={open} onClick={() => setOpen(!open)}>
            <PlusIcon size={9} />
            <span>{text.addModel}</span>
          </button>
        ) : null}
      </div>
      {full ? <p className="uc-insight-note">{text.maxModels(MAX_COMPARED)}</p> : null}
      {open && !full ? (
        <div className="uc-compare-search">
          <input
            className="uc-text-field"
            type="search"
            value={query}
            placeholder={text.searchPlaceholder}
            aria-label={text.searchPlaceholder}
            autoFocus
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && matches[0]) add(matches[0].key);
            }}
          />
          <div className="uc-compare-results">
            {matches.length === 0 ? <p className="uc-insight-note">{text.noMatch}</p> : null}
            {matches.map((candidate) => (
              <button key={candidate.key} type="button" className="uc-compare-result" onClick={() => add(candidate.key)}>
                <span className="uc-truncate">{candidate.name}</span>
                {candidate.sources.includes("mine") ? <span className="uc-insight-badge is-accent">{text.inUse}</span> : null}
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function RowView({ row, language, text }: { row: CompareRow; language: Language; text: InsightsMessages }) {
  const verdict = hasIntervals(row) && row.best.length > 0 ? (row.decided ? text.decided : text.undecided) : null;
  return (
    <div className="uc-compare-row">
      <div className="uc-compare-row-label">
        <span className="uc-truncate">{rowLabel(row, text)}</span>
        {verdict ? (
          <span className={`uc-compare-verdict${row.decided ? " is-decided" : ""}`} {...tooltipProps(verdict)} aria-label={verdict}>
            {row.decided ? <CheckIcon size={9} /> : "≈"}
          </span>
        ) : null}
      </div>
      <div className="uc-compare-cells" style={{ gridTemplateColumns: `repeat(${row.cells.length}, minmax(0, 1fr))` }}>
        {row.cells.map((cell, index) => {
          if (!cell) {
            return (
              <span key={index} className="uc-compare-cell is-empty">
                —
              </span>
            );
          }
          const { main, detail } = cellText(row, cell, language, text);
          const best = row.best.includes(index);
          return (
            <span key={index} className={`uc-compare-cell${best ? " is-best" : ""}`} {...tooltipProps(cell.sourceName ?? null)}>
              <span className="uc-num uc-compare-main">{main}</span>
              {detail ? <span className="uc-num uc-compare-detail">{detail}</span> : null}
            </span>
          );
        })}
      </div>
    </div>
  );
}

export function CompareModels() {
  const { language } = useSettings();
  const text = insightsFor(language);
  const rows = useInsights((state) => state.quality?.rows);
  const range = useInsights((state) => state.range);
  const compared = useInsights((state) => state.compared);
  const feeds = useFeeds(["epochScores", "epochBenchmarks", "arena", "arena3d"]);

  const quality = useMemo(() => summarizeQuality(rows ?? [], null), [rows]);
  const input: CompareInput = useMemo(
    () => ({
      quality,
      epoch: epochScoresOf(feeds.epochScores),
      boards: epochBoardsOf(feeds.epochBenchmarks),
      arena: arenaOf(feeds.arena),
      arena3d: arena3dOf(feeds.arena3d),
    }),
    [quality, feeds.epochScores, feeds.epochBenchmarks, feeds.arena, feeds.arena3d],
  );
  const candidates = useMemo(() => compareCandidates(input), [input]);
  const selected = useMemo(() => {
    if (compared) return compared;
    const scored = quality.filter((model) => model.score).slice(0, MAX_COMPARED);
    return [...new Set((scored.length >= 2 ? scored : quality.slice(0, MAX_COMPARED)).map((model) => modelKey(model.name)))];
  }, [compared, quality]);
  const table = useMemo(() => (selected.length > 0 ? compareRows(selected, input) : []), [selected, input]);
  const names = selected.map((key) => candidates.find((candidate) => candidate.key === key)?.name ?? key);

  let lastGroup: Group | null = null;
  let lastCategory: BenchmarkCategory | null = null;
  return (
    <>
      <ModelPicker candidates={candidates} selected={selected} text={text} />
      {selected.length === 0 ? <p className="uc-empty">{text.pickModels}</p> : null}
      {selected.length > 0 && table.length === 0 ? <p className="uc-empty">{text.noCompareRows}</p> : null}
      {table.length > 0 ? (
        <div className="uc-card uc-compare-table">
          <div className="uc-compare-cells uc-compare-names" style={{ gridTemplateColumns: `repeat(${selected.length}, minmax(0, 1fr))` }}>
            {names.map((name, index) => (
              <span key={selected[index]} className="uc-compare-name" {...tooltipProps(name)}>
                {name}
              </span>
            ))}
          </div>
          {table.map((row, index) => {
            const group = groupOf(row);
            const heading = group !== lastGroup ? (group === "mine" ? `${text.compareGroup(group)} · ${text.range(range)}` : text.compareGroup(group)) : null;
            lastGroup = group;
            const category = row.kind === "benchmark" ? row.category : null;
            const subheading = category && category !== lastCategory ? text.category(category) : null;
            if (category) lastCategory = category;
            return (
              <div key={`${row.kind}-${index}`}>
                {heading ? <h3 className="uc-compare-group">{heading}</h3> : null}
                {subheading ? <h4 className="uc-compare-subgroup">{subheading}</h4> : null}
                <RowView row={row} language={language} text={text} />
              </div>
            );
          })}
        </div>
      ) : null}
      <p className="uc-insight-note">{text.compareNote}</p>
    </>
  );
}
