/**
 * Dự án: each model's quality on the user's own projects, counted from the local transcripts. A
 * model gets a score only when all three parts have enough samples; the rest are listed with what
 * they still need. Every rate shows its 95% interval.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { insightsFor, type InsightsMessages } from "@/i18n/insights";
import type { Language } from "@/i18n";
import { compact } from "@/i18n/numbers";
import { compactDuration } from "@/model/format";
import { perTurn, QUALITY_PARTS, QUALITY_RANGES, qualityProjects, successRates, summarizeQuality, type ModelQuality } from "@/model/insights/quality";
import { useNow, useSettings } from "@/state/hooks";
import { loadQuality, rescanQuality, setQualityProject, setQualityRange, useInsights } from "@/state/insights";
import { Button } from "../ui/controls";
import { ChevronUpDown } from "../ui/icons";
import { openMenuAt } from "../ui/menu";
import { tooltipProps } from "../ui/tooltip";
import { agoText, Disclosure, numberText, percentText, RateBar, Segmented } from "./parts";

const INSUFFICIENT_PREVIEW = 4;

function ProjectPicker({ language, text }: { language: Language; text: InsightsMessages }) {
  const rows = useInsights((state) => state.quality?.rows);
  const project = useInsights((state) => state.project);
  const ref = useRef<HTMLButtonElement>(null);
  const projects = useMemo(() => qualityProjects(rows ?? []), [rows]);
  const name = (value: string) => (value === "" ? text.unnamedProject : value);
  const current = project === null ? text.allProjects : name(project);
  const open = () => {
    if (!ref.current) return;
    openMenuAt(
      ref.current,
      [
        { kind: "item" as const, label: text.allProjects, checked: project === null, onSelect: () => setQualityProject(null) },
        { kind: "separator" as const },
        ...projects.map((entry) => ({
          kind: "item" as const,
          label: text.projectTurns(name(entry.project), numberText(language, entry.turns)),
          checked: project === entry.project,
          onSelect: () => setQualityProject(entry.project),
        })),
      ],
      { align: "end", checkable: true },
    );
  };
  return (
    <button ref={ref} type="button" className="uc-picker" aria-haspopup="menu" aria-label={`${text.projectLabel}: ${current}`} onClick={open}>
      <span className="uc-truncate">{current}</span>
      <ChevronUpDown size={10} />
    </button>
  );
}

function ScanStatus({ language, text }: { language: Language; text: InsightsMessages }) {
  const info = useInsights((state) => state.qualityInfo);
  const now = useNow();
  const ago = agoText(info?.scannedAt, now, language);
  const status = info?.scanning ? (
    text.scanning
  ) : info?.scannedAt && ago ? (
    <>
      {text.scannedFiles(numberText(language, info.files))}
      <br />
      {text.scannedAgo(ago)}
    </>
  ) : (
    text.notScanned
  );
  return (
    <div className="uc-insight-status">
      <span className="uc-insight-status-text">{status}</span>
      <Button onClick={rescanQuality} className="is-small" disabled={info?.scanning === true} tooltip={text.rescanTooltip}>
        {text.rescan}
      </Button>
    </div>
  );
}

function PartRow({ model, part, language, text }: { model: ModelQuality; part: (typeof QUALITY_PARTS)[number]; language: Language; text: InsightsMessages }) {
  const detail = model.parts[part];
  const interval = detail.interval;
  const samples = text.partSamples(part, numberText(language, detail.samples));
  const range = interval ? `${percentText(language, interval.low)}–${percentText(language, interval.high)}` : null;
  return (
    <div className="uc-quality-part">
      <div className="uc-quality-part-line">
        <span className="uc-quality-part-label" {...tooltipProps(text.partTooltip(part))}>
          {text.part(part)}
        </span>
        <span className="uc-num uc-quality-part-value">{interval ? percentText(language, interval.rate) : "—"}</span>
      </div>
      {interval ? <RateBar rate={interval.rate} low={interval.low} high={interval.high} /> : null}
      <span className="uc-quality-caption">{range ? `${samples} · ${range}` : samples}</span>
    </div>
  );
}

function facts(model: ModelQuality, language: Language, text: InsightsMessages): string[] {
  const averages = perTurn(model.counts);
  const rates = successRates(model.counts);
  const lines: string[] = [];
  if (averages.tokens !== null) lines.push(text.tokensPerTurn(compact(language, averages.tokens)));
  if (averages.seconds !== null) {
    const duration = compactDuration(averages.seconds, language);
    if (duration) lines.push(text.timePerTurn(duration));
  }
  if (rates.checks !== null) lines.push(text.checksPassed(percentText(language, rates.checks, 0)));
  if (rates.shell !== null) lines.push(text.commandsFailed(percentText(language, 1 - rates.shell, 1)));
  if (model.counts.unknownCheckRuns > 0) lines.push(text.unknownChecks(numberText(language, model.counts.unknownCheckRuns)));
  if (model.counts.deniedActions > 0) lines.push(text.denied(numberText(language, model.counts.deniedActions)));
  return lines;
}

function ModelCard({ model, language, text }: { model: ModelQuality; language: Language; text: InsightsMessages }) {
  const score = model.score!;
  return (
    <article className="uc-card uc-quality-card" aria-label={model.name}>
      <header className="uc-quality-head">
        <div className="uc-quality-titles">
          <span className="uc-quality-name uc-truncate">{model.name}</span>
          <span className="uc-quality-meta">
            {text.source(model.source)} · {text.turns(numberText(language, model.counts.turns))}
          </span>
        </div>
        <div className="uc-quality-score" {...tooltipProps(text.scoreTooltip)}>
          <span className="uc-quality-score-value uc-num">{numberText(language, score.value, 1)}</span>
          <span className="uc-quality-score-range uc-num">{text.scoreRange(numberText(language, score.low, 1), numberText(language, score.high, 1))}</span>
        </div>
      </header>
      {QUALITY_PARTS.map((part) => (
        <PartRow key={part} model={model} part={part} language={language} text={text} />
      ))}
      <p className="uc-quality-facts">{facts(model, language, text).join(" · ")}</p>
    </article>
  );
}

function InsufficientList({ models, language, text }: { models: ModelQuality[]; language: Language; text: InsightsMessages }) {
  const [expanded, setExpanded] = useState(false);
  const shown = expanded ? models : models.slice(0, INSUFFICIENT_PREVIEW);
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{text.insufficientTitle(models.length)}</h2>
      <div className="uc-card uc-list-card">
        {shown.map((model) => {
          const needs = QUALITY_PARTS.filter((part) => !model.parts[part].enough).map((part) =>
            text.needs(part, numberText(language, model.parts[part].samples), numberText(language, model.parts[part].minimum)),
          );
          return (
            <div key={model.key} className="uc-list-row uc-insight-row">
              <span className="uc-list-text">
                <span className="uc-insight-row-title uc-truncate">{model.name}</span>
                <span className="uc-list-subtitle">{needs.join(" · ")}</span>
              </span>
              <span className="uc-insight-row-value uc-num">{text.turns(numberText(language, model.counts.turns))}</span>
            </div>
          );
        })}
        {models.length > INSUFFICIENT_PREVIEW ? (
          <button type="button" className="uc-insight-more" onClick={() => setExpanded(!expanded)}>
            {expanded ? text.showLess : text.showMore(models.length - INSUFFICIENT_PREVIEW)}
          </button>
        ) : null}
      </div>
    </section>
  );
}

export function MyQuality() {
  const { language } = useSettings();
  const text = insightsFor(language);
  const quality = useInsights((state) => state.quality);
  const loading = useInsights((state) => state.qualityLoading);
  const error = useInsights((state) => state.qualityError);
  const range = useInsights((state) => state.range);
  const project = useInsights((state) => state.project);

  useEffect(() => loadQuality(), []);

  const rows = quality?.rows;
  const known = useMemo(() => new Set(qualityProjects(rows ?? []).map((entry) => entry.project)), [rows]);
  const selected = project !== null && known.has(project) ? project : null;
  const models = useMemo(() => summarizeQuality(rows ?? [], selected).filter((model) => model.counts.turns > 0), [rows, selected]);
  const scored = models.filter((model) => model.score);
  const insufficient = models.filter((model) => !model.score);

  return (
    <>
      <Segmented value={range} options={QUALITY_RANGES} label={text.range} onChange={setQualityRange} ariaLabel={text.rangeLabel} />
      <div className="uc-insight-field">
        <span className="uc-insight-field-label">{text.projectLabel}</span>
        <ProjectPicker language={language} text={text} />
      </div>
      <ScanStatus language={language} text={text} />
      {error ? <p className="uc-insight-error">{text.failed(error)}</p> : null}
      {!quality && loading ? <p className="uc-empty">{text.loading}</p> : null}
      {quality && models.length === 0 ? <p className="uc-empty">{text.noTurns}</p> : null}
      {scored.map((model) => (
        <ModelCard key={model.key} model={model} language={language} text={text} />
      ))}
      {insufficient.length > 0 ? <InsufficientList models={insufficient} language={language} text={text} /> : null}
      <Disclosure title={text.methodTitle}>
        {text.qualityMethod.map((paragraph) => (
          <p key={paragraph}>{paragraph}</p>
        ))}
      </Disclosure>
    </>
  );
}
