/**
 * The words of the plan-period corner in a card header: the time left on top, the day the period
 * ends underneath, and a hover note with the exact time in the device's zone and where the date
 * comes from.
 */
import { messagesFor, type Language } from "@/i18n";
import type { PlanTerm } from "@/lib/types";
import { restoreDayOf, timeOnDayLabel, type TimeFormat } from "./format";
import { isPlanTermSoon, planTermEnd, planTermLeft } from "./planTerm";
import { calendarDaysBetween, offsetLabel } from "./timeZone";

export interface PlanTermLines {
  left: string;
  day: string;
  note: string;
  /** Three whole days or fewer left, or the stated end has passed. */
  soon: boolean;
}

export function planTermLines(term: PlanTerm, now: Date, timeFormat: TimeFormat, language: Language): PlanTermLines | null {
  const end = planTermEnd(term, now);
  if (!end) return null;
  const messages = messagesFor(language);
  const text = messages.dashboard;
  const left = planTermLeft(end.endsAt, now);
  const time = timeOnDayLabel(end.endsAt, now, timeFormat, language, false);
  const offset = offsetLabel(end.endsAt);
  let note: string;
  if (end.startedAt) note = text.planTermEstimateNote(time, offset, messages.format.calendarDate(end.startedAt));
  else if (left.kind === "due") note = text.planTermEndedNote(time, offset);
  else note = text.planTermStatedNote(time, offset, end.checkedAt ? messages.format.calendarDate(end.checkedAt) : null);
  const relative = calendarDaysBetween(now, end.endsAt) >= 0;
  return {
    left: text.planTermLeft(left, end.estimated),
    day: text.planTermDay(restoreDayOf(end.endsAt, now, relative), end.estimated, left.kind === "due"),
    note,
    soon: isPlanTermSoon(left),
  };
}
