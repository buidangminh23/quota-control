/**
 * Local calendar days as `YYYY-MM-DD` text, the key the usage ledger files every day under. Days are
 * compared as strings, which sorts them in time order.
 */

/** Today's (or `date`'s) day in the machine's time zone. */
export function localDay(date: Date = new Date()): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

/** The local noon of a day, so adding days never trips over a daylight-saving change. */
export function dayDate(day: string): Date {
  const [year, month, date] = day.split("-").map(Number);
  return new Date(year!, month! - 1, date!, 12);
}

export function addDays(day: string, count: number): string {
  const date = dayDate(day);
  date.setDate(date.getDate() + count);
  return localDay(date);
}

/** Every day from `from` to `to`, both included, oldest first. */
export function daysBetween(from: string, to: string): string[] {
  const days: string[] = [];
  for (let day = from; day <= to; day = addDays(day, 1)) days.push(day);
  return days;
}

/** `YYYY-MM` of a day. */
export function monthOf(day: string): string {
  return day.slice(0, 7);
}

/** The month `count` months after `month` (`YYYY-MM`), negative for earlier months. */
export function addMonths(month: string, count: number): string {
  const [year, index] = month.split("-").map(Number);
  const total = year! * 12 + (index! - 1) + count;
  return `${Math.floor(total / 12)}-${String((total % 12) + 1).padStart(2, "0")}`;
}

/** The first and last day of a month (`YYYY-MM`). */
export function monthRange(month: string): { from: string; to: string } {
  return { from: `${month}-01`, to: addDays(`${addMonths(month, 1)}-01`, -1) };
}
