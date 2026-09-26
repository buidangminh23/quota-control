/**
 * Money on the Token tab and the price list. The core prices usage in US dollars at list price; the
 * Vietnamese UI shows it in đồng at the Vietcombank selling rate the core keeps, the English UI in
 * dollars. Without a usable rate the Vietnamese UI keeps dollars rather than show a made-up figure.
 */
import { messagesFor, type Language } from "@/i18n";
import { compactDollars, decimal, dollars, localeOf } from "@/i18n/numbers";
import type { ExchangeRate } from "@/lib/types";
import { totalSpendRingCenter, type RingCenter } from "./format";

/** `compact`: `6,5 Tr ₫` / `$2.1K`. `full`: every digit, `6.512.345 ₫` / `$2,059.07`. */
export type MoneyStyle = "compact" | "full";

const formatters = new Map<string, Intl.NumberFormat>();

function dongFormatter(language: Language, compact: boolean, fractionDigits: number): Intl.NumberFormat {
  const key = `${language}|${compact}|${fractionDigits}`;
  let formatter = formatters.get(key);
  if (!formatter) {
    formatter = new Intl.NumberFormat(localeOf(language), {
      style: "currency",
      currency: "VND",
      minimumFractionDigits: 0,
      maximumFractionDigits: fractionDigits,
      ...(compact ? { notation: "compact", compactDisplay: "short" } : {}),
    });
    formatters.set(key, formatter);
  }
  return formatter;
}

/** Whether amounts show in đồng: Vietnamese UI and a positive rate. */
export function showsDong(language: Language, rate: ExchangeRate | null): rate is ExchangeRate {
  return language === "vi" && rate !== null && Number.isFinite(rate.usdToVnd) && rate.usdToVnd > 0;
}

/** A dollar amount as display money in the UI's currency. */
export function formatMoney(usd: number, language: Language, rate: ExchangeRate | null, style: MoneyStyle = "compact"): string {
  if (showsDong(language, rate)) {
    const amount = usd * rate.usdToVnd;
    const compact = style === "compact" && Math.abs(amount) >= 1000;
    return dongFormatter(language, compact, compact ? 1 : 0).format(amount);
  }
  if (style === "compact" && Math.abs(usd) >= 1000) return compactDollars(language, usd);
  return dollars(language, usd, 2);
}

/** A đồng amount with the precision a price needs: whole đồng from 100 up, cents below. */
export function formatDongPrice(amount: number, language: Language): string {
  const magnitude = Math.abs(amount);
  return dongFormatter(language, false, magnitude >= 100 ? 0 : magnitude >= 10 ? 1 : 2).format(amount);
}

/** A đồng amount as a bare number for a price table, with the precision `formatDongPrice` uses. */
export function formatDongNumber(amount: number, language: Language): string {
  const magnitude = Math.abs(amount);
  const digits = magnitude >= 100 ? 0 : magnitude >= 10 ? 1 : 2;
  return decimal(language, amount, 0, digits);
}

/** A dollar figure from a price page (`1.25`) in the UI's number style, keeping the page's decimals. */
export function formatPageDollars(text: string, language: Language): string {
  const digits = text.split(".")[1]?.length ?? 0;
  return decimal(language, Number(text.replace(/,/g, "")), digits, digits);
}

/** The rate itself, e.g. `26.170 ₫`. */
export function formatRate(rate: ExchangeRate, language: Language): string {
  return dongFormatter(language, false, 0).format(rate.usdToVnd);
}

/** The ring's two-line center for a money metric: dollars as before, or đồng in tỷ / triệu / nghìn. */
export function moneyRingCenter(usd: number, metric: "cost" | "costPerMtok", language: Language, rate: ExchangeRate | null): RingCenter {
  if (!showsDong(language, rate)) return totalSpendRingCenter(usd, metric, language);
  const messages = messagesFor(language);
  if (metric === "costPerMtok") return { primary: formatMoney(usd, language, rate, "compact"), unit: messages.totalSpend.ringUnit("perMtok") };
  const amount = usd * rate.usdToVnd;
  const magnitude = Math.abs(amount);
  if (magnitude >= 1e9) return { primary: decimal(language, amount / 1e9, 0, 1), unit: messages.usage.dongUnit("billion") };
  if (magnitude >= 1e6) return { primary: decimal(language, amount / 1e6, 0, 1), unit: messages.usage.dongUnit("million") };
  if (magnitude >= 1e3) return { primary: decimal(language, amount / 1e3, 0, 1), unit: messages.usage.dongUnit("thousand") };
  return { primary: decimal(language, amount, 0, 0), unit: messages.usage.dongUnit("one") };
}
