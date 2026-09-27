/** A card's live status as the Accounts screen shows it, and core error text in the user's language. */
import { translate, type Language } from "@/i18n";
import type { ProviderRuntimeState } from "@/lib/types";

export type Status = "ok" | "refreshing" | "error" | "unknown";

export function statusOf(runtime: ProviderRuntimeState | undefined): Status {
  if (!runtime) return "unknown";
  if (runtime.refreshing) return "refreshing";
  if (runtime.error || runtime.snapshot?.errorCategory) return "error";
  return runtime.snapshot ? "ok" : "unknown";
}

export function errorText(error: unknown, language: Language): string {
  const raw = error instanceof Error ? error.message : typeof error === "string" ? error : String(error);
  return translate(raw, language);
}
