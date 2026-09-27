/**
 * Fixture data in the exact shape the Rust core sends: connected-account quota cards
 * (`claude@…`, `codex@…`) and this computer's local-history cards (`claude-local`, `codex-local`)
 * with spend, token and per-model rows. Used by `MockBackend` and component tests; times are relative
 * to `now` so countdowns stay meaningful.
 */
import type {
  AccountProvider,
  ConnectedAccount,
  EngineState,
  MetricChartPoint,
  MetricLine,
  ModelUsageBreakdown,
  ModelUsageEntry,
  Provider,
  ProviderEntry,
  ProviderSnapshot,
  ServiceEntry,
  TokenUsage,
  WidgetDescriptor,
  WidgetTemplate,
} from "./types";

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const SESSION_MS = 5 * HOUR;
const WEEK_MS = 7 * DAY;

export const LOCAL_SOURCE_NOTE =
  "All local sessions on this machine; not account-scoped. Input includes cached tokens; output is model-generated. Costs estimate API-equivalent usage using bundled 2026-07-02 prices, not subscription charges.";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function iso(offsetMs: number, now: number): string {
  return new Date(now + offsetMs).toISOString();
}

function template(title: string, kind: WidgetTemplate["kind"], extra: Partial<WidgetTemplate> = {}): WidgetTemplate {
  return { title, kind, ...extra };
}

function descriptor(provider: Provider, suffix: string, metricLabel: string, tpl: WidgetTemplate, extra: Partial<WidgetDescriptor> = {}): WidgetDescriptor {
  return { id: `${provider.id}.${suffix}`, providerId: provider.id, metricLabel, template: tpl, pinnable: true, isSpendTile: false, ...extra };
}

const percent = (provider: Provider, suffix: string, title: string, extra: Partial<WidgetTemplate> = {}) =>
  descriptor(provider, suffix, title, template(title, "percent", { limit: 100, ...extra }));

export function accountProvider(kind: AccountProvider, hash: string, label: string): Provider {
  const brand = kind === "claude" ? "Claude" : "Codex";
  const links = kind === "claude" ? [{ label: "Status", url: "https://status.anthropic.com" }] : [{ label: "Status", url: "https://status.openai.com" }];
  return { id: `${kind}@${hash}`, displayName: `${brand} · ${label}`, icon: kind, links };
}

export function accountDescriptors(provider: Provider, kind: AccountProvider): WidgetDescriptor[] {
  if (kind === "claude") {
    return [
      percent(provider, "session", "Session", { sessionStartSignal: "missingResetDate" }),
      percent(provider, "weekly", "Weekly"),
      percent(provider, "sonnet", "Sonnet"),
      percent(provider, "fable", "Fable"),
      descriptor(provider, "extra", "Extra usage spent", template("Extra Usage", "dollars", { limit: 100, unboundedValueWord: "spent" })),
    ];
  }
  return [
    percent(provider, "session", "Session"),
    percent(provider, "weekly", "Weekly"),
    percent(provider, "spark", "Spark"),
    percent(provider, "sparkWeekly", "Spark Weekly"),
    descriptor(provider, "credits", "Credits", template("Credits", "dollars")),
    descriptor(provider, "rateLimitResets", "Rate Limit Resets", template("Rate Limit Resets", "count", { selectionKind: "count", unboundedValueWord: "available", traySuffix: "resets", showsResetExpiries: true })),
  ];
}

function localProvider(brand: AccountProvider): Provider {
  return { id: `${brand}-local`, displayName: brand === "claude" ? "Claude Local Usage" : "Codex Local Usage", icon: brand };
}

function localDescriptors(provider: Provider): WidgetDescriptor[] {
  const spend = (suffix: string, title: string) =>
    descriptor(provider, suffix, title, template(title, "dollars", { isUsagePeriod: true, valueTooltipNote: LOCAL_SOURCE_NOTE }), { isSpendTile: true });
  const tokens = (suffix: string, title: string) =>
    descriptor(provider, suffix, title, template(title, "count", { selectionKind: "count", isUsagePeriod: true, valueTooltipNote: `Today. ${LOCAL_SOURCE_NOTE}` }));
  return [
    descriptor(provider, "trend", "Usage Trend", template("Usage Trend", "count", { isChart: true }), {
      pinnable: false,
      historyResource: { scope: "machineLocal", estimatedCost: true, sourceNote: LOCAL_SOURCE_NOTE },
    }),
    spend("today", "Today"),
    spend("yesterday", "Yesterday"),
    spend("last30", "Last 30 Days"),
    tokens("inputTokens", "Input Tokens"),
    tokens("outputTokens", "Output Tokens"),
    tokens("cachedInputTokens", "Cached Input Tokens"),
  ];
}

interface FixtureAccount {
  kind: AccountProvider;
  hash: string;
  label: string;
  mode: ConnectedAccount["credentialMode"];
}

const ACCOUNTS: readonly FixtureAccount[] = [
  { kind: "claude", hash: "7c1e", label: "Công ty", mode: "managed_oauth" },
  { kind: "claude", hash: "a93f", label: "Cá nhân", mode: "shared_cli" },
  { kind: "codex", hash: "52d0", label: "codex", mode: "cli" },
];

export function fixtureAccounts(now = Date.now()): ConnectedAccount[] {
  return ACCOUNTS.map((account, index) => ({
    id: `${account.kind}@${account.hash}`,
    provider: account.kind,
    label: account.label,
    connectedAt: iso(-(index + 3) * DAY, now),
    updatedAt: iso(-(index + 1) * HOUR, now),
    credentialMode: account.mode,
  }));
}

export function fixtureCatalog(): ProviderEntry[] {
  const accounts = ACCOUNTS.map((account) => {
    const provider = accountProvider(account.kind, account.hash, account.label);
    return { provider, descriptors: accountDescriptors(provider, account.kind) };
  });
  const locals = (["claude", "codex"] as const).map((brand) => {
    const provider = localProvider(brand);
    return { provider, descriptors: localDescriptors(provider) };
  });
  return [...accounts, ...locals];
}

function progress(label: string, used: number, resetsInMs: number | null, periodMs: number, now: number): MetricLine {
  return {
    type: "progress",
    label,
    used,
    limit: 100,
    format: { kind: "percent" },
    ...(resetsInMs === null ? {} : { resetsAt: iso(resetsInMs, now) }),
    periodDurationMs: periodMs,
  };
}

function usage(input: number, output: number, cached: number, cacheWrite = 0): TokenUsage {
  return { inputTokens: input, outputTokens: output, cachedInputTokens: cached, cacheCreationInputTokens: cacheWrite };
}

function sumUsage(entries: readonly TokenUsage[]): TokenUsage {
  return entries.reduce(
    (total, entry) => usage(total.inputTokens + entry.inputTokens, total.outputTokens + entry.outputTokens, total.cachedInputTokens + entry.cachedInputTokens, total.cacheCreationInputTokens + entry.cacheCreationInputTokens),
    usage(0, 0, 0),
  );
}

function scaledModels(models: readonly ModelUsageEntry[], factor: number): ModelUsageEntry[] {
  return models.map((model) => ({
    model: model.model,
    totalTokens: Math.round(model.totalTokens * factor),
    ...(model.costUSD === undefined ? {} : { costUSD: Math.round(model.costUSD * factor * 100) / 100 }),
    ...(model.tokenUsage
      ? {
          tokenUsage: usage(
            Math.round(model.tokenUsage.inputTokens * factor),
            Math.round(model.tokenUsage.outputTokens * factor),
            Math.round(model.tokenUsage.cachedInputTokens * factor),
            Math.round(model.tokenUsage.cacheCreationInputTokens * factor),
          ),
        }
      : {}),
  }));
}

function breakdown(models: ModelUsageEntry[]): ModelUsageBreakdown {
  const tokenUsage = sumUsage(models.flatMap((model) => (model.tokenUsage ? [model.tokenUsage] : [])));
  return {
    totalTokens: models.reduce((sum, model) => sum + model.totalTokens, 0),
    totalCostUSD: models.reduce((sum, model) => sum + (model.costUSD ?? 0), 0),
    models,
    sourceNote: LOCAL_SOURCE_NOTE,
    tokenUsage,
  };
}

function periodLines(todayModels: ModelUsageEntry[], yesterdayFactor: number, monthFactor: number): MetricLine[] {
  const periods: Array<[string, number]> = [
    ["Today", 1],
    ["Yesterday", yesterdayFactor],
    ["Last 30 Days", monthFactor],
  ];
  const lines: MetricLine[] = periods.map(([label, factor]) => {
    const models = scaledModels(todayModels, factor);
    const detail = breakdown(models);
    return {
      type: "values",
      label,
      values: [
        { number: detail.totalCostUSD ?? 0, kind: "dollars", estimated: true },
        { number: detail.totalTokens, kind: "count", label: "tokens", estimated: false },
      ],
      modelBreakdown: detail,
    };
  });
  const today = breakdown(todayModels).tokenUsage!;
  const tokenLine = (label: string, count: number): MetricLine => ({ type: "values", label, values: [{ number: count, kind: "count", label: "tokens", estimated: false }] });
  return [
    ...lines,
    tokenLine("Input Tokens", today.inputTokens),
    tokenLine("Output Tokens", today.outputTokens),
    tokenLine("Cached Input Tokens", today.cachedInputTokens),
  ];
}

function trendLine(now: number, scale: number, seed: number): MetricLine {
  const points: MetricChartPoint[] = Array.from({ length: 30 }, (_, index) => {
    const date = new Date(now - (29 - index) * DAY);
    const wave = Math.abs(Math.sin((index + seed) * 1.7)) * 0.7 + (index % 7 === 5 || index % 7 === 6 ? 0.05 : 0.3);
    const value = index % 11 === 3 ? 0 : Math.round(scale * wave);
    return { value, label: `${MONTHS[date.getMonth()]} ${String(date.getDate()).padStart(2, "0")}`, valueLabel: `${value} tokens` };
  });
  return { type: "chart", label: "Usage Trend", points, note: LOCAL_SOURCE_NOTE };
}

const CLAUDE_MODELS: ModelUsageEntry[] = [
  { model: "claude-fable-5-1", totalTokens: 18_200_000, costUSD: 28.4, tokenUsage: usage(17_600_000, 610_000, 15_900_000, 240_000) },
  { model: "claude-haiku-4-5", totalTokens: 1_900_000, costUSD: 0.92, tokenUsage: usage(1_820_000, 84_000, 1_300_000) },
  { model: "claude-opus-5-5", totalTokens: 11_400_000, costUSD: 16.7, tokenUsage: usage(11_050_000, 350_000, 9_800_000, 120_000) },
  { model: "claude-sonnet-5", totalTokens: 4_300_000, costUSD: 3.83, tokenUsage: usage(4_140_000, 160_000, 3_600_000) },
];

const CODEX_MODELS: ModelUsageEntry[] = [
  { model: "gpt-6-astra", totalTokens: 7_600_000, costUSD: 9.4, tokenUsage: usage(7_380_000, 220_000, 6_100_000) },
  { model: "gpt-6-mini", totalTokens: 1_500_000, costUSD: 0.61, tokenUsage: usage(1_450_000, 50_000, 1_100_000) },
];

export function fixtureSnapshots(now = Date.now()): Record<string, ProviderSnapshot> {
  const refreshedAt = iso(-60_000, now);
  return {
    "claude@7c1e": {
      providerID: "claude@7c1e",
      displayName: "Claude · Công ty",
      plan: "Max 5x",
      planTerm: { basis: "monthlyFrom", startedAt: iso(-27 * DAY, now) },
      refreshedAt,
      lines: [
        progress("Session", 12, 3.2 * HOUR, SESSION_MS, now),
        progress("Weekly", 58, 2.1 * DAY, WEEK_MS, now),
        progress("Sonnet", 22, 2.1 * DAY, WEEK_MS, now),
        progress("Fable", 88, 2.1 * DAY, WEEK_MS, now),
        { type: "progress", label: "Extra usage spent", used: 12.4, limit: 50, format: { kind: "dollars" } },
      ],
    },
    "claude@a93f": {
      providerID: "claude@a93f",
      displayName: "Claude · Cá nhân",
      plan: "Pro",
      refreshedAt,
      lines: [progress("Session", 0, null, SESSION_MS, now), progress("Weekly", 33, 4.5 * DAY, WEEK_MS, now), { type: "badge", label: "Extra usage spent", text: "Disabled" }],
    },
    "codex@52d0": {
      providerID: "codex@52d0",
      displayName: "Codex · codex",
      plan: "Pro",
      planTerm: { basis: "stated", endsAt: iso(20.5 * DAY, now), checkedAt: iso(-2 * DAY, now) },
      refreshedAt,
      lines: [
        progress("Session", 82, 1.4 * HOUR, SESSION_MS, now),
        progress("Weekly", 64, 3.3 * DAY, WEEK_MS, now),
        progress("Spark", 9, 1.4 * HOUR, SESSION_MS, now),
        progress("Spark Weekly", 17, 3.3 * DAY, WEEK_MS, now),
        { type: "values", label: "Credits", values: [{ number: 30.88, kind: "dollars", estimated: false }, { number: 772, kind: "count", label: "credits", estimated: false }] },
        { type: "values", label: "Rate Limit Resets", values: [{ number: 2, kind: "count", label: "available", estimated: false }], expiriesAt: [iso(1.6 * DAY, now), iso(9 * DAY, now)] },
      ],
    },
    "claude-local": {
      providerID: "claude-local",
      displayName: "Claude Local Usage",
      refreshedAt,
      lines: [trendLine(now, 38_000_000, 0), ...periodLines(CLAUDE_MODELS, 1.23, 47)],
    },
    "codex-local": {
      providerID: "codex-local",
      displayName: "Codex Local Usage",
      refreshedAt,
      lines: [trendLine(now, 9_000_000, 3), ...periodLines(CODEX_MODELS, 2.6, 71)],
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

/** Services beyond Claude and Codex as the core lists them: one found through its app's login, one with a saved key. */
export function fixtureServices(): ServiceEntry[] {
  const service = (id: string, name: string, extra: Partial<ServiceEntry> = {}): ServiceEntry => ({
    id,
    name,
    loginFrom: null,
    takesApiKey: true,
    keyLabel: "API key",
    keyFormat: "token",
    keyUrl: `https://example.invalid/${id}/keys`,
    keyEnv: [`${id.toUpperCase()}_API_KEY`],
    keyFields: [],
    startsHidden: false,
    detected: [],
    keys: [],
    ...extra,
  });
  return [
    service("gemini", "Gemini", {
      loginFrom: "Gemini CLI",
      takesApiKey: false,
      keyUrl: null,
      keyEnv: [],
      detected: [{ id: `gemini@${"1".repeat(64)}`, service: "gemini", label: "minh@example.com", origin: "Gemini CLI" }],
    }),
    service("openrouter", "OpenRouter", {
      keys: [{ id: `openrouter@${"2".repeat(64)}`, service: "openrouter", label: "OpenRouter", addedAt: "2026-09-20T08:00:00Z", hint: "9f3a" }],
    }),
    service("deepseek", "DeepSeek"),
    service("perplexity", "Perplexity", { keyLabel: "Session cookie (__Secure-authjs.session-token)", keyFormat: "cookie", keyEnv: [] }),
    service("longcat", "LongCat", { keyLabel: "Cookie header", keyFormat: "cookieHeader", keyEnv: [] }),
    service("xai", "xAI", { keyFields: [["teamId", "Team ID"]] }),
    service("ollama", "Ollama", { loginFrom: "Ollama", takesApiKey: false, keyUrl: null, keyEnv: [], startsHidden: true }),
  ];
}
