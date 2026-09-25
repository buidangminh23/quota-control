/**
 * Locale-aware number text. Upstream formats through ICU (en_US) with half-to-even rounding on the
 * shortest decimal; `Intl.NumberFormat` with `roundingMode: "halfEven"` is the same ICU pipeline, so
 * English output matches upstream digit for digit. Vietnamese follows CLDR vi-VN: `56.904.995,2`,
 * `35,8 Tr`, `49,85 $`.
 */
import type { Language } from "./language";

const LOCALES: Record<Language, string> = { vi: "vi-VN", en: "en-US" };

const cache = new Map<string, Intl.NumberFormat>();

function formatter(language: Language, options: Intl.NumberFormatOptions): Intl.NumberFormat {
  const key = `${language}|${JSON.stringify(options)}`;
  let instance = cache.get(key);
  if (!instance) {
    instance = new Intl.NumberFormat(LOCALES[language], { roundingMode: "halfEven", ...options });
    cache.set(key, instance);
  }
  return instance;
}

export function localeOf(language: Language): string {
  return LOCALES[language];
}

/** Grouped decimal with `minFraction...maxFraction` digits. */
export function decimal(language: Language, value: number, minFraction: number, maxFraction: number): string {
  return formatter(language, { minimumFractionDigits: minFraction, maximumFractionDigits: maxFraction }).format(value);
}

/** Short compact notation with up to one decimal: `1.2K` / `1,2 N`, `35.8M` / `35,8 Tr`. */
export function compact(language: Language, value: number): string {
  return formatter(language, { notation: "compact", compactDisplay: "short", maximumFractionDigits: 1 }).format(value);
}

/** US dollars with a fixed number of fractional digits: `$2,059.07` / `2.059,07 $`. */
export function dollars(language: Language, value: number, fractionDigits: number): string {
  return formatter(language, {
    style: "currency",
    currency: "USD",
    currencyDisplay: "narrowSymbol",
    minimumFractionDigits: fractionDigits,
    maximumFractionDigits: fractionDigits,
  }).format(value);
}

/** Compact US dollars with up to one decimal: `$2.1K` / `2,1 N $`. */
export function compactDollars(language: Language, value: number): string {
  return formatter(language, {
    style: "currency",
    currency: "USD",
    currencyDisplay: "narrowSymbol",
    notation: "compact",
    compactDisplay: "short",
    maximumFractionDigits: 1,
  }).format(value);
}
