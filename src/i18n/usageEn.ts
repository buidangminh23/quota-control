/** English copy for the Token views and the Prices tab. */
import type { PriceMessages, UsageMessages } from "./usageMessages";

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"] as const;
const WEEKDAYS_LONG = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"] as const;
const WEEKDAYS_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;

export const usageEn: UsageMessages = {
  viewsLabel: "Token views",
  view: (key) => ({ overview: "Overview", history: "History", charts: "Charts", projects: "Projects" })[key],
  periodLabel: "Period",
  period: (key) => ({ today: "Today", last30: "30 days", last365: "1 year", all: "All" })[key],
  loading: "Reading history…",
  importing: "Reading every log for the first time; totals fill in over a few minutes.",
  failed: "Couldn't read the token history.",
  noData: "No usage in this period.",
  source: (key) => ({ claude: "Claude", codex: "Codex" })[key],
  other: "Other",
  unknownProject: "Unknown project",
  tokens: (count) => `${count} tokens`,
  ringByLabel: "Split the ring by",
  ringBy: (key) => ({ source: "Source", model: "Model", project: "Project" })[key],
  ringAria: (total, parts) => `Total ${total} in ${parts} parts`,
  yearsTitle: "By year",
  allTime: "All time",
  since: (date) => `since ${date}`,
  year: (year, current) => (current ? `${year} · this year` : year),
  rateNote: (rate, time, stale) => `Converted at Vietcombank's USD selling rate: ${rate}${time ? ` (${time})` : ""}${stale ? " · old rate" : ""}.`,
  dongUnit: (scale) => ({ billion: "billion VND", million: "million VND", thousand: "thousand VND", one: "VND" })[scale],
  monthTitle: (year, month) => `${MONTHS[month - 1]} ${year}`,
  previousMonth: "Previous month",
  nextMonth: "Next month",
  monthTotal: "Whole month",
  dayTitle: (date) => `${WEEKDAYS_LONG[date.getDay()]}, ${MONTHS[date.getMonth()]} ${date.getDate()}, ${date.getFullYear()}`,
  dayShort: (date) => `${WEEKDAYS_SHORT[date.getDay()]} ${MONTHS[date.getMonth()]!.slice(0, 3)} ${date.getDate()}`,
  back: (label) => `Back to ${label}`,
  bySource: "By source",
  byModel: "By model",
  byProject: "By project",
  noUsage: "No tokens used on this day.",
  chartLabel: "Chart",
  chart: (key) => ({ day: "Day", month: "Month", year: "Year", model: "Model", project: "Project" })[key],
  chartMetricLabel: "Chart unit",
  chartMetric: (key) => ({ tokens: "Tokens", cost: "Cost" })[key],
  chartCaption: (key) =>
    ({
      day: "Last 30 days",
      month: "Last 12 months",
      year: "Every year on record",
      model: "Top models",
      project: "Top projects",
    })[key],
  chartPeak: (value) => `Peak ${value}`,
  showAll: (count) => `Show all (${count})`,
  showFewer: "Show fewer",
  contextTitle: "Context window",
  contextHint: "Claude Code and Codex sessions from the last 24 hours, read from logs on this machine. Message text is never read.",
  contextEmpty: "No sessions in the last 24 hours.",
  contextUsage: (used, window) => `${used} / ${window}`,
  contextUsed: (used) => `${used} tokens`,
  contextSegment: (key) =>
    ({ base: "Session base (system, tools, memory)", conversation: "Conversation", lastTurn: "Latest turn", free: "Free" })[key],
  contextApp: (source) => ({ claude: "Claude Code", codex: "Codex" })[source],
  ago: (duration) => (duration ? `${duration} ago` : "just now"),
};

const PRICE_TERMS: Readonly<Record<string, string>> = {
  Name: "Model",
  "Cached input": "Cached",
  "Cache writes": "Cache write",
  "5m writes": "5m write",
  "1h writes": "1h write",
  "Hits and refreshes": "Cache hit",
  "Output / cost": "Output",
  "Price per minute": "Per minute",
  "Estimated cost": "Estimated",
  Pricing: "Price",
  "with data sharing": "With data sharing",
};

export const pricesEn: PriceMessages = {
  providerLabel: "Provider",
  tierLabel: "Processing tier",
  tier: (key) => ({ standard: "Standard", batch: "Batch", flex: "Flex", fast: "Fast" })[key],
  currencyLabel: "Currency",
  currency: (key) => ({ vnd: "₫", usd: "$" })[key],
  section: (id) =>
    ({
      models: "Models",
      flagship: "Flagship models",
      cyber: "Cyber models",
      live: "Live",
      realtime: "Realtime and audio",
      image: "Image generation",
      transcription: "Transcription and translation",
      tools: "Built-in tools",
      specialized: "Specialized models",
      finetuning: "Fine-tuning",
    })[id] ?? id,
  term: (text) => PRICE_TERMS[text] ?? text,
  unitNote: () => "Prices in USD per million tokens (MTok) unless another unit is shown.",
  source: (host, date) => `Source: ${host} · retrieved ${date}`,
  openSource: (host) => `Open ${host}`,
};
