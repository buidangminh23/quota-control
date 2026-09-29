/**
 * Desktop notifications for Claude resets: one when claude-resets.com records a new reset (also
 * one it has not reviewed yet, said so in the title), one for a limit change, and one when a banked
 * reset the user has not marked as applied has three days left. The ids already seen are kept in
 * the webview's storage so a restart does not repeat them, and the first run only records what is
 * there; a deadline that is near is announced on the first run too, since it is still news.
 */
import { useEffect } from "react";
import { announceOnIsland } from "@/glance/alerts";
import { insightsFor } from "@/i18n/insights";
import { compactDuration, timeOnDayLabel } from "@/model/format";
import { openBanked, parseClaudeResets } from "@/model/insights/claudeResets";
import { excerpt } from "@/model/insights/resets";
import { notify } from "@/platform/system";
import { useWallClock } from "@/state/hooks";
import { ensureFeed, useInsights } from "@/state/insights";
import { useApp } from "@/state/store";

const STORAGE_KEY = "quota-control.claude-resets-notified";
/** The reset notifications stay together in the system's list. */
const RESETS_GROUP = "resets";
/** An announcement older than this when first seen is history, not news. */
const FRESH_MS = 48 * 3_600_000;
/** A banked reset is announced again this long before its deadline. */
export const EXPIRY_NOTICE_MS = 3 * 24 * 3_600_000;
const CLOCK_MS = 10 * 60_000;
/** More ids than this are forgotten, oldest first. */
const MAX_SEEN = 500;

interface Seen {
  resets: string[];
  changes: string[];
  /** Banked resets whose deadline was announced. */
  expiring: string[];
}

function readSeen(): Seen | null {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<Record<keyof Seen, unknown>>;
    const ids = (item: unknown) => (Array.isArray(item) ? item.filter((id): id is string => typeof id === "string") : []);
    return { resets: ids(value.resets), changes: ids(value.changes), expiring: ids(value.expiring) };
  } catch {
    return null;
  }
}

function writeSeen(seen: Seen): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(seen));
  } catch {
    return;
  }
}

function merged(seen: readonly string[], ids: readonly string[]): string[] {
  return [...new Set([...seen, ...ids])].slice(-MAX_SEEN);
}

export function useClaudeResetNotifications(): void {
  const enabled = useApp((state) => state.settings.notifyClaudeResets);
  const language = useApp((state) => state.settings.language);
  const timeFormat = useApp((state) => state.settings.timeFormat);
  const used = useApp((state) => state.settings.usedBankedResets);
  const body = useInsights((state) => state.feeds.claudeResets?.body ?? null);
  const clock = useWallClock(CLOCK_MS);

  useEffect(() => {
    if (enabled) ensureFeed("claudeResets");
  }, [enabled]);

  useEffect(() => {
    if (!enabled || !body) return;
    const feed = parseClaudeResets(body);
    if (!feed) return;
    const now = new Date();
    const seen = readSeen();
    const expiring = openBanked(feed.resets, now).filter(
      (reset) => reset.usableUntil!.getTime() - now.getTime() <= EXPIRY_NOTICE_MS && !used.includes(reset.id) && !seen?.expiring.includes(reset.id),
    );
    writeSeen({
      resets: merged(seen?.resets ?? [], feed.resets.map((reset) => reset.id)),
      changes: merged(seen?.changes ?? [], feed.changes.map((change) => change.id)),
      expiring: merged(seen?.expiring ?? [], expiring.map((reset) => reset.id)),
    });
    const text = insightsFor(language).claude;
    const send = (kind: string, title: string, message: string) => {
      announceOnIsland({ title, body: message, brand: "claude", severity: "normal" });
      const topic = { id: `claude-resets.${kind}`, group: RESETS_GROUP };
      return notify(title, message, topic).catch((error: unknown) => console.error("Sending notification failed", error));
    };
    const fresh = (date: Date) => now.getTime() - date.getTime() < FRESH_MS;
    for (const reset of expiring) {
      const until = reset.usableUntil!;
      const left = compactDuration((until.getTime() - now.getTime()) / 1000, language) ?? "";
      void send(`expiring.${reset.id}`, text.notifyBankedTitle, text.notifyBankedBody(left, timeOnDayLabel(until, now, timeFormat, language)));
    }
    if (!seen) return;
    for (const reset of feed.resets) {
      if (!seen.resets.includes(reset.id) && fresh(reset.announcedAt)) void send(`reset.${reset.id}`, text.notifyResetTitle(reset.kind, reset.provisional), excerpt(reset.text));
    }
    for (const change of feed.changes) {
      if (!seen.changes.includes(change.id) && fresh(change.announcedAt)) void send(`change.${change.id}`, text.notifyChangeTitle, excerpt(change.text));
    }
  }, [enabled, body, language, timeFormat, used, clock]);
}
