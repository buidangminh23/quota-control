/**
 * Fixture data mirroring the reference screenshot (Claude Team 5x, Codex, Cursor, Grok), used by
 * `MockBackend` and by component tests. Times are relative to `now` so countdowns stay meaningful.
 */
import type {
  EngineState,
  MetricLine,
  MetricValue,
  Provider,
  ProviderEntry,
  ProviderSnapshot,
  WidgetDescriptor,
  WidgetTemplate,
} from "./types";

const HOUR = 3_600_000;
const WEEK = 7 * 24 * HOUR;

function iso(offsetMs: number, now: number): string {
  return new Date(now + offsetMs).toISOString();
}

function template(title: string, kind: WidgetTemplate["kind"], limit?: number, extra: Partial<WidgetTemplate> = {}): WidgetTemplate {
  return { title, kind, ...(limit === undefined ? {} : { limit }), ...extra };
}

function descriptor(provider: Provider, suffix: string, metricLabel: string, tpl: WidgetTemplate, extra: Partial<WidgetDescriptor> = {}): WidgetDescriptor {
  return {
    id: `${provider.id}.${suffix}`,
    providerId: provider.id,
    metricLabel,
    template: tpl,
    pinnable: true,
    isSpendTile: false,
    ...extra,
  };
}

function spendTiles(provider: Provider, note?: string): WidgetDescriptor[] {
  return [
    ["today", "Today"],
    ["yesterday", "Yesterday"],
    ["last30", "Last 30 Days"],
  ].map(([suffix, title]) =>
    descriptor(
      provider,
      suffix!,
      title!,
      template(title!, "dollars", undefined, { isUsagePeriod: true, ...(note ? { valueTooltipNote: note } : {}) }),
      { isSpendTile: true },
    ),
  );
}

function trend(provider: Provider, note: string): WidgetDescriptor {
  return descriptor(provider, "trend", "Usage Trend", template("Usage Trend", "count", undefined, { isChart: true }), {
    pinnable: false,
    historyResource: { scope: "machineLocal", estimatedCost: true, sourceNote: note },
  });
}

function spend(dollars: number, tokens: number, estimated = true): MetricValue[] {
  return [
    { number: dollars, kind: "dollars", estimated },
    { number: tokens, kind: "count", label: "tokens", estimated: false },
  ];
}

function spendLines(today: [number, number], yesterday: [number, number], last30: [number, number], estimated = true): MetricLine[] {
  return [
    { type: "values", label: "Today", values: spend(...today, estimated) },
    { type: "values", label: "Yesterday", values: spend(...yesterday, estimated) },
    { type: "values", label: "Last 30 Days", values: spend(...last30, estimated) },
  ];
}

function trendLine(now: number, scale: number): MetricLine {
  const points = Array.from({ length: 31 }, (_, index) => {
    const date = new Date(now - (30 - index) * 24 * HOUR);
    const value = Math.round(scale * (0.4 + 0.6 * Math.abs(Math.sin(index * 1.7))));
    return {
      value,
      label: date.toLocaleDateString("en-US", { month: "short", day: "numeric" }),
      valueLabel: `${(value / 1_000_000).toFixed(1)}M tokens`,
    };
  });
  return { type: "chart", label: "Usage Trend", points, note: "From your usage history (estimated)" };
}

const claude: Provider = { id: "claude", displayName: "Claude", icon: "claude", links: [{ label: "Status", url: "https://status.anthropic.com" }] };
const codex: Provider = { id: "codex", displayName: "Codex", icon: "codex" };
const cursor: Provider = { id: "cursor", displayName: "Cursor", icon: "cursor" };
const grok: Provider = { id: "grok", displayName: "Grok", icon: "grok" };

export function fixtureCatalog(): ProviderEntry[] {
  return [
    {
      provider: claude,
      descriptors: [
        descriptor(claude, "session", "Session", template("Session", "percent", 100, { sessionStartSignal: "missingResetDate" })),
        descriptor(claude, "weekly", "Weekly", template("Weekly", "percent", 100)),
        descriptor(claude, "fable", "Fable", template("Fable", "percent", 100)),
        descriptor(claude, "sonnet", "Sonnet", template("Sonnet", "percent", 100)),
        descriptor(claude, "extra", "Extra usage spent", template("Extra Usage", "dollars", 100, { unboundedValueWord: "spent" })),
        descriptor(claude, "rateLimitResets", "Rate Limit Resets", template("Rate Limit Resets", "dollars", undefined, { traySuffix: "resets", showsResetExpiries: true })),
        trend(claude, "From your Claude usage history (estimated)"),
        ...spendTiles(claude),
      ],
    },
    {
      provider: codex,
      descriptors: [
        descriptor(codex, "session", "Session", template("Session", "percent", 100)),
        descriptor(codex, "weekly", "Weekly", template("Weekly", "percent", 100)),
        trend(codex, "From your Codex usage history (estimated)"),
        ...spendTiles(codex),
      ],
    },
    {
      provider: cursor,
      descriptors: [
        descriptor(cursor, "totalUsage", "Total usage", template("Total Usage", "percent", 100)),
        descriptor(cursor, "cursorModels", "Cursor Models", template("Cursor Models", "percent", 100)),
        descriptor(cursor, "otherModels", "Other Models", template("Other Models", "percent", 100)),
        ...spendTiles(cursor, "From your Cursor usage history."),
      ],
    },
    {
      provider: grok,
      descriptors: [descriptor(grok, "weekly", "Weekly", template("Weekly", "percent", 100)), ...spendTiles(grok)],
    },
  ];
}

export function fixtureSnapshots(now = Date.now()): Record<string, ProviderSnapshot> {
  const refreshedAt = iso(-60_000, now);
  return {
    claude: {
      providerID: "claude",
      displayName: "Claude",
      plan: "Team 5x",
      refreshedAt,
      lines: [
        { type: "progress", label: "Session", used: 0, limit: 100, format: { kind: "percent" }, resetsAt: iso(5 * HOUR, now), periodDurationMs: 5 * HOUR },
        { type: "progress", label: "Weekly", used: 64, limit: 100, format: { kind: "percent" }, resetsAt: iso(30 * HOUR, now), periodDurationMs: WEEK },
        { type: "progress", label: "Fable", used: 73, limit: 100, format: { kind: "percent" }, resetsAt: iso(30 * HOUR, now), periodDurationMs: WEEK },
        trendLine(now, 80_000_000),
        ...spendLines([49.85, 35_800_000], [61.3, 43_400_000], [2_500, 2_300_000_000]),
      ],
    },
    codex: {
      providerID: "codex",
      displayName: "Codex",
      plan: "Pro",
      refreshedAt,
      lines: [
        { type: "progress", label: "Session", used: 18, limit: 100, format: { kind: "percent" }, resetsAt: iso(3 * HOUR, now), periodDurationMs: 5 * HOUR },
        { type: "progress", label: "Weekly", used: 41, limit: 100, format: { kind: "percent" }, resetsAt: iso(4 * 24 * HOUR, now), periodDurationMs: WEEK },
        trendLine(now, 50_000_000),
        ...spendLines([12.1, 9_100_000], [31.2, 22_000_000], [812, 610_000_000]),
      ],
    },
    cursor: {
      providerID: "cursor",
      displayName: "Cursor",
      plan: "Pro",
      refreshedAt,
      lines: [
        { type: "progress", label: "Total usage", used: 42, limit: 100, format: { kind: "percent" }, resetsAt: iso(12 * 24 * HOUR, now), periodDurationMs: 30 * 24 * HOUR },
        ...spendLines([3.4, 1_200_000], [12.4, 4_800_000], [210, 90_000_000], false),
      ],
    },
    grok: {
      providerID: "grok",
      displayName: "Grok",
      plan: "SuperGrok",
      refreshedAt,
      lines: [
        { type: "progress", label: "Weekly", used: 12, limit: 100, format: { kind: "percent" }, resetsAt: iso(2 * 24 * HOUR, now), periodDurationMs: WEEK },
        ...spendLines([0.2, 80_000], [0.9, 350_000], [14.3, 5_900_000]),
      ],
    },
  };
}

export function fixtureEngineState(now = Date.now()): EngineState {
  const snapshots = fixtureSnapshots(now);
  return {
    providers: Object.fromEntries(Object.entries(snapshots).map(([id, snapshot]) => [id, { snapshot, refreshing: false }])),
    lastRefreshAt: iso(-2 * 60_000, now),
    refreshIntervalMs: 5 * 60_000,
  };
}
