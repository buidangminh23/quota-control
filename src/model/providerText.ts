/**
 * Display text for provider cards: titles, legend names, the header notice and the "Outdated" hint.
 * The core sends English display names (`Claude · work`, `Claude Local Usage`) and error text; this is
 * where they become short, localized header text.
 */
import { messagesFor, translate, type Language } from "@/i18n";
import type { ErrorCategory, Provider, ProviderRuntimeState } from "@/lib/types";
import { compactDuration } from "./format";
import { brandOf, isLocalHistoryCard, layoutFamily } from "./layout";

const BRAND_NAMES: Readonly<Record<string, string>> = {
  antigravity: "Antigravity",
  claude: "Claude",
  codex: "Codex",
  copilot: "Copilot",
  cursor: "Cursor",
  devin: "Devin",
  grok: "Grok",
  ollama: "Ollama",
  opencode: "OpenCode",
  openrouter: "OpenRouter",
  zai: "Z.ai",
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

/** Whether the snapshot on screen is only an error (the first refresh failed): rows read "No data". */
export function isErrorSnapshot(runtime: ProviderRuntimeState | undefined): boolean {
  return runtime?.snapshot?.errorCategory !== undefined;
}

export interface StalenessHint {
  label: string;
  tooltip: string;
}

/** Two refresh intervals: past this a refresh was genuinely missed (upstream `stalenessThreshold`). */
export const STALENESS_INTERVALS = 2;

export function stalenessHint(
  runtime: ProviderRuntimeState | undefined,
  refreshIntervalMs: number,
  now: Date,
  language: Language,
): StalenessHint | null {
  const refreshedAt = runtime?.snapshot?.refreshedAt;
  if (!refreshedAt) return null;
  const age = (now.getTime() - new Date(refreshedAt).getTime()) / 1000;
  if (!(age * 1000 >= refreshIntervalMs * STALENESS_INTERVALS)) return null;
  const duration = compactDuration(age, language);
  if (!duration) return null;
  const meter = messagesFor(language).meter;
  return { label: meter.outdated, tooltip: meter.lastUpdated(duration) };
}
