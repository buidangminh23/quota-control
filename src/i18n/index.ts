import { en } from "./en";
import type { Language } from "./language";
import type { Messages } from "./messages";
import { vi } from "./vi";

export type { Language } from "./language";
export { DEFAULT_LANGUAGE, LANGUAGES, isLanguage } from "./language";
export type { Messages } from "./messages";

const CATALOGS: Record<Language, Messages> = { vi, en };

export function messagesFor(language: Language): Messages {
  return CATALOGS[language];
}

/** Backend English text in `language`, or the source text when the catalog has no entry. */
export function translate(text: string, language: Language): string {
  return CATALOGS[language].term(text) ?? text;
}
