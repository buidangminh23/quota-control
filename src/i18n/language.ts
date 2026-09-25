/** The popup's display language. Vietnamese is the default; English is the alternative. */
export type Language = "vi" | "en";

export const DEFAULT_LANGUAGE: Language = "vi";

export const LANGUAGES: readonly Language[] = ["vi", "en"];

export function isLanguage(value: unknown): value is Language {
  return value === "vi" || value === "en";
}
