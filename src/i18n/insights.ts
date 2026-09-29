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
import type { ClaudePlan, ForecastSkill } from "@/model/insights/claudeResets";
import type { ForecastHorizon, ResetKind } from "@/model/insights/resets";
import type { NamedDay, ResetWindow } from "@/model/insights/upcomingReset";
import type { ResetProvider } from "@/model/settings";
import type { BenchmarkView } from "@/state/insights";
import { insightsEn } from "./insightsEn";
import { insightsVi } from "./insightsVi";
import type { Language } from "./language";

/** The rows of the Claude and Codex comparison. */
export type CompareRow = "resets" | "average" | "median" | "longest" | "sinceLast" | "last30";

/** The Claude view of the Reset tab (claude-resets.com) and the row it adds to a Claude card. */
export interface ClaudeResetMessages {
  forecastDisclaimer: string;
  scopeEveryone: string;
  scopeAffected: string;
  scopePaid: string;
  plan(plan: ClaudePlan): string;
  scopePlans(plans: string): string;
  /** Whether a reset covered the plan of an account connected here. */
  yourPlan(plan: string, covered: boolean): string;
  provisional: string;
  provisionalNote: string;
  bankedTitle: string;
  bankedUntil(time: string): string;
  bankedLeft(duration: string): string;
  bankedHow: string;
  bankedMarkUsed: string;
  bankedUsed: string;
  bankedUndo: string;
  detectorBehind: string;
  datasetNote: string;
  changesTitle: string;
  changesNote: string;
  changeBadge: string;
  statChanges: string;
  statEveryone: string;
  statYourPlan(plan: string): string;
  ago(duration: string, date: string): string;
  compareTitle: string;
  compareSince(date: string): string;
  compareRow(row: CompareRow): string;
  compareMonths: string;
  /** The chart's title once the window is longer than the months it shows. */
  compareMonthsRecent(months: string): string;
  compareMonth(month: string, claude: string, codex: string): string;
  source: string;
  method: string[];
  notifyResets: string;
  notifyResetsNote: string;
  notifyResetTitle(kind: ResetKind, provisional: boolean): string;
  notifyChangeTitle: string;
  notifyBankedTitle: string;
  notifyBankedBody(duration: string, time: string): string;
  /** The row on a Claude card while a banked reset can still be applied. */
  cardTitle: string;
  cardLeft(duration: string): string;
  cardCaption(time: string, offset: string): string;
  cardPosted(handle: string, time: string, post: string): string;
  cardHow: string;
}

export interface InsightsMessages {
  source(source: UsageSource): string;
  refresh: string;
  refreshing: string;
  refreshTooltip: string;
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
  scannedFiles(files: string): string;
  scannedAgo(ago: string): string;
  notScanned: string;
  rescan: string;
  rescanTooltip: string;
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
  epochSource: string;
  arenaSource(date: string): string;
  arena3dSource: string;
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
  latestTitle: string;
  forecastTitle: string;
  horizon(days: ForecastHorizon): string;
  forecastNote(count: string): string;
  forecastUnavailable: string;
  forecastDisclaimer: string;
  waitLine(waited: string, percent: string): string;
  medianMark(gap: string, time: string): string;
  medianMarkPassed(gap: string, time: string): string;
  scheduledIn(duration: string): string;
  scheduledOverdue(ago: string): string;
  calendarTitle(weeks: number): string;
  calendarToday: string;
  calendarDay(date: string, kinds: string): string;
  monthShort(month: number): string;
  patternTitle: string;
  patternWeekdays: string;
  patternHours: string;
  patternNote(count: string): string;
  weekdayShort: string[];
  hourBlock(startHour: number): string;
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
  /** The Reset tab's switch between the two trackers. */
  resetProviderLabel: string;
  resetProvider(provider: ResetProvider): string;
  /** How the estimate would have done on its own history: better or worse by `percent`, over `days` holding `resets`. */
  forecastReliability(verdict: ForecastSkill["verdict"], percent: string, days: string, resets: string): string;
  claude: ClaudeResetMessages;

  showBenchmarkTab: string;
  showResetsTab: string;
  notifyCodexResets: string;
  notifyCodexResetsNote: string;
  notifyResetTitle: string;
  notifyScheduledTitle: string;
  notifyWatchTitle(level: "elevated" | "strong"): string;

  /** The row on a Codex card counting down to the next free reset. */
  freeResetTitle(origin: "scheduled" | "watch", kind: ResetKind | null): string;
  freeResetIn(duration: string, estimate: boolean): string;
  freeResetWithin(duration: string): string;
  freeResetNoTime: string;
  freeResetAwaiting: string;
  /** Captions; `time` is a clock time with its day, e.g. `08:00 · T2 28/09`. */
  freeResetAt(time: string, offset: string): string;
  freeResetAround(time: string, offset: string): string;
  freeResetBy(time: string, offset: string, chance: string | null): string;
  freeResetDue(time: string, offset: string): string;
  freeResetWindow(window: ResetWindow): string;
  freeResetUntimed: string;
  namedDay(day: NamedDay): string;
  /** The line under an estimated time naming the post's own day, e.g. `“Ngày mai” theo giờ Mỹ`. */
  freeResetDayNote(day: string): string;
  /** Hover lines: where the reset comes from, then how its time was worked out. */
  freeResetPosted(time: string, post: string): string;
  freeResetWatched(time: string, post: string): string;
  freeResetHowExact(fromSite: boolean, zone: string): string;
  freeResetHowDay(day: string, zone: string): string;
  freeResetHowBy(chance: string | null): string;
  freeResetHowWindow(window: ResetWindow): string;
  freeResetHowUntimed: string;
  freeResetOpenTab: string;

  /** The Codex reset tracker on the macOS island and desktop widgets. */
  glanceTitle: string;
  glanceSource: string;
  glanceChanceTitle: string;
  /** One short line under the chances saying they are an estimate. */
  glanceForecastNote: string;
  /** The time since the last reset, e.g. `Đã 1 ngày 7 giờ chưa có reset`. */
  glanceSinceLast(duration: string): string;
}

const CATALOGS: Record<Language, InsightsMessages> = { vi: insightsVi, en: insightsEn };

export function insightsFor(language: Language): InsightsMessages {
  return CATALOGS[language];
}
