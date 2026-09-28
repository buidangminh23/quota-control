/**
 * Number and date wording shared by the Benchmark and Reset tabs and the Codex reset tracker the
 * macOS island and widgets draw.
 */
import type { Language } from "@/i18n";
import { decimal } from "@/i18n/numbers";
import { deviceTimeZone, zonedParts } from "@/model/timeZone";

export function percentText(language: Language, rate: number, digits = 1): string {
  return `${decimal(language, rate * 100, digits, digits)}%`;
}

export function numberText(language: Language, value: number, digits = 0): string {
  return decimal(language, value, digits, digits);
}

/** A calendar date in the device's zone (or `timeZone`): `26/09/2026` or `9/26/2026`. */
export function dateText(date: Date, language: Language, timeZone: string = deviceTimeZone()): string {
  return date.toLocaleDateString(language === "vi" ? "vi-VN" : "en-US", { day: "2-digit", month: "2-digit", year: "numeric", timeZone });
}

/** `12/09` in the device's zone, with the year only when it is not the current one. */
export function shortDate(date: Date, now: Date, language: Language): string {
  const parts = zonedParts(date);
  if (parts.year !== zonedParts(now).year) return dateText(date, language);
  const day = String(parts.day).padStart(2, "0");
  const month = String(parts.month).padStart(2, "0");
  return language === "vi" ? `${day}/${month}` : `${month}/${day}`;
}
