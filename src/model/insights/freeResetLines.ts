/**
 * The words of the free-reset row on a Codex card: a countdown on the right, the exact time in the
 * device's zone with its GMT offset underneath, and a hover note with the post and how its time was
 * worked out. A named day keeps the poster's word on a line of its own ("“Ngày mai” theo giờ Mỹ"),
 * because beside the converted time it overflows a 320 px card in 12-hour or English layouts.
 */
import type { Language } from "@/i18n";
import { insightsFor } from "@/i18n/insights";
import { localeOf } from "@/i18n/numbers";
import { compactDuration, timeOnDayLabel, type TimeFormat } from "@/model/format";
import { deviceTimeZone, offsetLabel, zoneName } from "@/model/timeZone";
import { excerpt } from "./resets";
import { timingMoment, type UpcomingReset } from "./upcomingReset";

const POST_LENGTH = 200;

export interface FreeResetLines {
  title: string;
  /** The countdown, or why there is none. */
  value: string;
  /** The time it points at, in the device's zone. */
  caption: string;
  /** The post's own day under an estimated time, else `null`. */
  note: string | null;
  /** The post and how its time was worked out, one paragraph per line. */
  details: string;
  /** Its time has come but codex-resets.com has not confirmed the reset yet. */
  awaiting: boolean;
}

export function freeResetLines(next: UpcomingReset, now: Date, timeFormat: TimeFormat, language: Language): FreeResetLines {
  const text = insightsFor(language);
  const timing = next.timing;
  const at = timingMoment(timing);
  const zone = deviceTimeZone();
  const offset = offsetLabel(at ?? now, zone);
  const zoneText = `${zoneName(at ?? now, zone, localeOf(language))}, ${offset}`;
  const clock = (date: Date) => timeOnDayLabel(date, now, timeFormat, language);
  const left = at && at.getTime() > now.getTime() ? compactDuration((at.getTime() - now.getTime()) / 1000, language) : null;
  const awaiting = at !== null && left === null;
  const chance = next.chancePercent === null ? null : `${next.chancePercent}%`;

  let value: string;
  let caption: string;
  let note: string | null = null;
  let how: string;
  switch (timing.kind) {
    case "exact":
      value = left ? text.freeResetIn(left, false) : text.freeResetAwaiting;
      caption = (awaiting ? text.freeResetDue : text.freeResetAt)(clock(timing.at), offset);
      how = text.freeResetHowExact(timing.from === "site", zoneText);
      break;
    case "day": {
      const day = text.namedDay(timing.day);
      value = left ? text.freeResetIn(left, true) : text.freeResetAwaiting;
      caption = awaiting
        ? text.freeResetDue(clock(timing.at), offset)
        : text.freeResetAround(timeOnDayLabel(timing.at, now, timeFormat, language, false), offset);
      note = text.freeResetDayNote(day);
      how = text.freeResetHowDay(day, zoneText);
      break;
    }
    case "by":
      value = left ? text.freeResetWithin(left) : text.freeResetAwaiting;
      caption = text.freeResetBy(clock(timing.at), offset, chance);
      how = text.freeResetHowBy(chance);
      break;
    case "window":
      value = text.freeResetNoTime;
      caption = text.freeResetWindow(timing.window);
      how = text.freeResetHowWindow(timing.window);
      break;
    case "unknown":
      value = text.freeResetNoTime;
      caption = text.freeResetUntimed;
      how = text.freeResetHowUntimed;
      break;
  }
  const post = excerpt(next.text, POST_LENGTH);
  const origin = next.origin === "scheduled" ? text.freeResetPosted(clock(next.announcedAt), post) : text.freeResetWatched(clock(next.announcedAt), post);
  return { title: text.freeResetTitle(next.origin, next.kind), value, caption, note, details: `${origin}\n${how}`, awaiting };
}
