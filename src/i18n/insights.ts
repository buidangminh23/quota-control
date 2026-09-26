/**
 * Text for the Benchmark and Reset tabs, kept in its own catalog so the main one stays focused on
 * limits and tokens. Same rules as `messages.ts`: parameterized entries are functions so each
 * language controls word order.
 */
import type { UsageSource } from "@/lib/types";
import type { ArenaBoardSlug } from "@/model/insights/arena";
import type { BenchmarkCategory } from "@/model/insights/epoch";
import type { EffortLevel } from "@/model/insights/modelNames";
import type { QualityPartKey, QualityRange } from "@/model/insights/quality";
import type { ForecastHorizon, ResetKind } from "@/model/insights/resets";
import type { BenchmarkView } from "@/state/insights";
import { insightsEn } from "./insightsEn";
import { insightsVi } from "./insightsVi";
import type { Language } from "./language";

export interface InsightsMessages {
  source(source: UsageSource): string;
  refresh: string;
  refreshing: string;
  loading: string;
  failed(detail: string): string;
  fetchedAgo(ago: string): string;
  justNow: string;
  staleNote: string;
  openLink: string;

  viewsLabel: string;
  view(view: BenchmarkView): string;

  rangeLabel: string;
  range(range: QualityRange): string;
  projectLabel: string;
  allProjects: string;
  unnamedProject: string;
  projectTurns(project: string, turns: string): string;
  scanning: string;
  scanned(files: string, ago: string): string;
  notScanned: string;
  rescan: string;
  noTurns: string;
  score: string;
  scoreRange(low: string, high: string): string;
  scoreTooltip: string;
  turns(count: string): string;
  part(part: QualityPartKey): string;
  partSamples(part: QualityPartKey, count: string): string;
  partTooltip(part: QualityPartKey): string;
  tokensPerTurn(value: string): string;
  timePerTurn(value: string): string;
  checksPassed(percent: string): string;
  commandsFailed(percent: string): string;
  unknownChecks(count: string): string;
  denied(count: string): string;
  insufficientTitle(count: number): string;
  needs(part: QualityPartKey, have: string, minimum: string): string;
  methodTitle: string;
  qualityMethod: string[];

  boardLabel: string;
  epochGroup: string;
  arenaGroup: string;
  eciBoard: string;
  category(category: BenchmarkCategory): string;
  arenaBoard(board: ArenaBoardSlug): string;
  arena3dBoard: string;
  eciNote: string;
  benchmarkCount(count: number): string;
  modelCount(count: number): string;
  datedNote: string;
  superseded(name: string): string;
  noResults: string;
  inUse: string;
  openWeights: string;
  votes(count: string): string;
  sessions(count: string): string;
  arenaNote: string;
  agentNote: string;
  arena3dNote: string;
  epochSource(ago: string): string;
  arenaSource(date: string): string;
  arena3dSource(ago: string): string;
  showMore(count: number): string;
  showLess: string;

  addModel: string;
  searchPlaceholder: string;
  noMatch: string;
  removeModel(name: string): string;
  maxModels(count: number): string;
  pickModels: string;
  compareGroup(group: "mine" | "epoch" | "arena"): string;
  mineScore: string;
  mineTurns: string;
  eciRow: string;
  arenaRow(board: string): string;
  arenaAgentRow: string;
  arena3dRow: string;
  effort(level: EffortLevel): string;
  rank(value: string): string;
  decided: string;
  undecided: string;
  compareNote: string;
  noCompareRows: string;

  scheduledTitle: string;
  scheduledFor(time: string): string;
  scheduledNoTime: string;
  announcedAgo(ago: string): string;
  watchTitle(level: "elevated" | "strong"): string;
  watchChance(percent: string, window: string): string;
  watchUntil(time: string): string;
  quietTitle: string;
  lastReset(ago: string): string;
  forecastTitle: string;
  horizon(days: ForecastHorizon): string;
  forecastNote(count: string, halfLife: number): string;
  forecastUnavailable: string;
  forecastDisclaimer: string;
  statsTitle: string;
  statTotal: string;
  statKinds(regular: string, banked: string): string;
  statSinceLast: string;
  statAverage: string;
  statMedian: string;
  statLongest: string;
  statLast30: string;
  days(value: string): string;
  gapRange(from: string, to: string): string;
  historyTitle: string;
  kind(kind: ResetKind): string;
  observed: string;
  openPost: string;
  resetsSource: string;
  resetsMethod: string[];

  showBenchmarkTab: string;
  showResetsTab: string;
  notifyCodexResets: string;
  notifyCodexResetsNote: string;
  notifyResetTitle: string;
  notifyScheduledTitle: string;
}

const CATALOGS: Record<Language, InsightsMessages> = { vi: insightsVi, en: insightsEn };

export function insightsFor(language: Language): InsightsMessages {
  return CATALOGS[language];
}
