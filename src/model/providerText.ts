/**
 * Display text for provider cards: titles, legend names, the header notice and the "Outdated" hint.
 * The core sends English display names (`Claude · work`, `Claude Local Usage`) and error text; this is
 * where they become short, localized header text.
 */
import { messagesFor, translate, type Language } from "@/i18n";
import type { ErrorCategory, PlanTerm, Provider, ProviderRuntimeState } from "@/lib/types";
import { compactDuration } from "./format";
import { brandOf, isLocalHistoryCard, layoutFamily } from "./layout";

const BRAND_NAMES: Readonly<Record<string, string>> = {
  abacus: "Abacus AI",
  aiand: "ai&",
  aixy: "Aixy",
  alibaba: "Alibaba Model Studio",
  amp: "Amp",
  anthropic: "Anthropic",
  antigravity: "Antigravity",
  atlascloud: "Atlas Cloud",
  augment: "Augment",
  bedrock: "AWS Bedrock",
  bifrost: "Bifrost",
  cerebras: "Cerebras",
  chutes: "Chutes",
  claude: "Claude",
  clawrouter: "ClawRouter",
  cline: "Cline",
  cloudflare: "Cloudflare Workers AI",
  codebuddy: "CodeBuddy",
  codebuff: "Codebuff",
  coderabbit: "CodeRabbit",
  codex: "Codex",
  commandcode: "Command Code",
  copilot: "Copilot",
  cursor: "Cursor",
  deepgram: "Deepgram",
  deepinfra: "DeepInfra",
  deepseek: "DeepSeek",
  devin: "Devin",
  devpass: "DevPass",
  doubao: "Doubao",
  elevenlabs: "ElevenLabs",
  factory: "Droid",
  fireworks: "Fireworks",
  gitkraken: "GitKraken AI",
  grok: "Grok",
  groq: "Groq",
  helmcode: "Helmcode",
  huggingface: "Hugging Face",
  hyper: "Charm Hyper",
  hyperbolic: "Hyperbolic",
  ibmbob: "IBM Bob",
  jetbrains: "JetBrains AI",
  kilo: "Kilo",
  kimi: "Kimi",
  kiro: "Kiro",
  litellm: "LiteLLM",
  llmman: "llmman",
  llmproxy: "LLM Proxy",
  longcat: "LongCat",
  manus: "Manus",
  mimo: "Xiaomi MiMo",
  minimax: "MiniMax",
  mistral: "Mistral",
  moonshot: "Moonshot",
  muse: "Muse Code",
  nanogpt: "NanoGPT",
  neuralwatt: "Neuralwatt",
  newapi: "OpenAI-compatible relay",
  notion: "Notion AI",
  nous: "Nous Portal",
  novita: "Novita AI",
  ollama: "Ollama",
  openai: "OpenAI",
  opencode: "OpenCode",
  openrouter: "OpenRouter",
  perplexity: "Perplexity",
  poe: "Poe",
  qoder: "Qoder",
  qwen: "Qwen Cloud",
  raycast: "Raycast",
  replicate: "Replicate",
  sakana: "Sakana AI",
  siliconflow: "SiliconFlow",
  stepfun: "StepFun",
  sub2api: "sub2api",
  synthetic: "Synthetic",
  t3chat: "T3 Chat",
  typesafe: "TypeSafe",
  v0: "v0",
  venice: "Venice",
  vercel: "Vercel AI Gateway",
  vertexai: "Vertex AI",
  warp: "Warp",
  wayfinder: "Wayfinder",
  windsurf: "Windsurf",
  xai: "xAI",
  xkiro: "xKiro",
  zai: "Z.ai",
  zed: "Zed",
  zenmux: "ZenMux",
  zoommate: "ZoomMate",
};

export function brandName(brand: string): string {
  return BRAND_NAMES[brand] ?? brand.charAt(0).toUpperCase() + brand.slice(1);
}

/** The brand whose mark and color a card uses (`provider.icon`, falling back to the id). */
export function providerBrand(provider: Provider): string {
  return provider.icon ? brandOf(provider.icon) : brandOf(provider.id);
}

const LABEL_SEPARATOR = " · ";

/** The account label inside an account card's display name (`Claude · work` → `work`). */
export function accountLabelOf(provider: Provider): string | null {
  const index = provider.displayName.indexOf(LABEL_SEPARATOR);
  return index >= 0 ? provider.displayName.slice(index + LABEL_SEPARATOR.length) : null;
}

/** An account card's label when it is an email address; the card header shows it on its own line. */
export function accountEmailOf(provider: Provider): string | null {
  const label = accountLabelOf(provider)?.trim();
  return label && /^[^\s@]+@[^\s@]+$/.test(label) ? label : null;
}

/**
 * The card header title. Account cards drop a label that only repeats the family (the default label
 * is the CLI name, e.g. `Claude · claude`); local-history sections, which only appear on the Token
 * tab, read as the brand.
 */
export function providerTitle(provider: Provider, language: Language): string {
  const brand = providerBrand(provider);
  if (isLocalHistoryCard(provider.id)) return brandName(brand);
  const label = accountLabelOf(provider);
  if (label !== null) {
    const trimmed = label.trim();
    if (trimmed === "" || trimmed.toLowerCase() === layoutFamily(provider.id).toLowerCase()) return brandName(brand);
    return `${brandName(brand)}${LABEL_SEPARATOR}${trimmed}`;
  }
  return translate(provider.displayName, language);
}

/** The Total Spend legend name: spend comes from this computer's logs, so the brand name is enough. */
export function spendLegendName(provider: Provider, language: Language): string {
  return isLocalHistoryCard(provider.id) ? brandName(providerBrand(provider)) : providerTitle(provider, language);
}

/** The refresh error to surface on the header, localized by category when the core supplied one. */
export function headerNotice(runtime: ProviderRuntimeState | undefined, language: Language): string | null {
  if (!runtime) return null;
  const snapshot = runtime.snapshot;
  const errors = messagesFor(language).meter.errors;
  const category: ErrorCategory | undefined = snapshot?.errorCategory;
  if (runtime.error) return translate(runtime.error, language);
  if (snapshot && category) {
    const detail = snapshot.lines.find((line) => line.type === "badge" && line.label === "Error");
    const localized = errors[category];
    if (detail && detail.type === "badge") {
      const text = translate(detail.text, language);
      return text === localized ? localized : `${localized}\n${text}`;
    }
    return localized;
  }
  return snapshot?.warning ? translate(snapshot.warning, language) : null;
}

/** How an account card introduces itself on the island, the widget and the strip. */
export interface CardIdentity {
  /** The heading: the brand for an account named by its email, the account title otherwise. */
  name: string;
  /** The email under the heading: the label when it is one, else the address the provider reports. */
  account: string | null;
  plan: string | null;
  /** The header notice's first line (a failed refresh, an error snapshot, a provider warning): the
   * reason behind the header's warning triangle. */
  notice: string | null;
  /** The plan's paid period, as the provider sent it, for the header's right corner. */
  planTerm: PlanTerm | null;
}

export function cardIdentity(provider: Provider, runtime: ProviderRuntimeState | undefined, language: Language): CardIdentity {
  const local = isLocalHistoryCard(provider.id);
  const email = local ? null : accountEmailOf(provider);
  const snapshot = local ? undefined : runtime?.snapshot;
  const unconfirmed = !snapshot?.planCheckedAt && (snapshot?.errorCategory || (runtime?.error && !snapshot?.planTerm?.checkedAt));
  const term = unconfirmed || /^free$/i.test(snapshot?.plan?.trim() ?? "") ? null : snapshot?.planTerm;
  const planTerm = term?.basis === "monthlyFrom" && !term.checkedAt ? { ...term, checkedAt: snapshot?.planCheckedAt ?? snapshot?.refreshedAt } : term;
  return {
    name: email ? brandName(providerBrand(provider)) : providerTitle(provider, language),
    account: email ?? snapshot?.account ?? null,
    plan: snapshot?.plan ? translate(snapshot.plan, language) : null,
    notice: headerNotice(runtime, language)?.split("\n")[0] ?? null,
    planTerm: planTerm ?? null,
  };
}

/** Whether the snapshot on screen is only an error (the first refresh failed): rows read "No data". */
export function isErrorSnapshot(runtime: ProviderRuntimeState | undefined): boolean {
  return runtime?.snapshot?.errorCategory !== undefined;
}

/**
 * Whether the snapshot on screen came from a successful read (the latest, or the last good one while
 * a refresh fails), so a metric it lacks is a limit the account's plan does not have.
 */
export function hasPlanReading(runtime: ProviderRuntimeState | undefined): boolean {
  return runtime?.snapshot !== undefined && !isErrorSnapshot(runtime);
}

export interface StalenessHint {
  label: string;
  tooltip: string;
}

/** Two refresh intervals: past this a refresh was genuinely missed (upstream `stalenessThreshold`). */
export const STALENESS_INTERVALS = 2;

/** How long ago, in seconds, a reading fetched at `refreshedAt` was taken, when it is past
 * `STALENESS_INTERVALS` refresh intervals; `null` while it is fresher, or without a reading. */
function outdatedAge(refreshedAt: string | undefined, refreshIntervalMs: number, now: Date): number | null {
  if (!refreshedAt) return null;
  const age = (now.getTime() - new Date(refreshedAt).getTime()) / 1000;
  return age * 1000 >= refreshIntervalMs * STALENESS_INTERVALS && age > 0 ? age : null;
}

/** Whether a reading fetched at `refreshedAt` is old enough for the card to say `Outdated`. */
export function isOutdated(refreshedAt: string | undefined, refreshIntervalMs: number, now: Date): boolean {
  return outdatedAge(refreshedAt, refreshIntervalMs, now) !== null;
}

export function stalenessHint(
  runtime: ProviderRuntimeState | undefined,
  refreshIntervalMs: number,
  now: Date,
  language: Language,
): StalenessHint | null {
  const age = outdatedAge(runtime?.snapshot?.refreshedAt, refreshIntervalMs, now);
  if (age === null) return null;
  const duration = compactDuration(age, language);
  if (!duration) return null;
  const meter = messagesFor(language).meter;
  return { label: meter.outdated, tooltip: meter.lastUpdated(duration) };
}
