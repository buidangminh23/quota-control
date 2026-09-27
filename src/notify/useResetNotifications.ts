/**
 * Desktop notifications for Codex resets: one when codex-resets.com records a new reset, one when
 * a reset is announced ahead of time and one when the site sees signs of a reset (its watch). The
 * ids already seen are kept in the webview's storage so a restart does not repeat them, and the
 * first run only records what is there.
 */
import { announceOnIsland } from "@/glance/alerts";
import { useEffect } from "react";
import { insightsFor } from "@/i18n/insights";
import { activeWatch, excerpt, parseResetStatus, type ResetWatch } from "@/model/insights/resets";
import { notify } from "@/platform/system";
import { ensureFeed, useInsights } from "@/state/insights";
import { useApp } from "@/state/store";

const STORAGE_KEY = "quota-control.codex-resets-notified";
/** A reset older than this when first seen is history, not news. */
const FRESH_MS = 48 * 3_600_000;

interface Seen {
  latest: string | null;
  scheduled: string | null;
  /** The watch's level and start, since a watch has no id of its own. */
  watch: string | null;
}

/** A watch is the same one for as long as its level and start do not change. */
export function watchKey(watch: ResetWatch | null): string | null {
  return watch ? `${watch.level}@${watch.observedAt.toISOString()}` : null;
}

function readSeen(): Seen | null {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<Seen>;
    const text = (item: unknown) => (typeof item === "string" ? item : null);
    return { latest: text(value.latest), scheduled: text(value.scheduled), watch: text(value.watch) };
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

export function useResetNotifications(): void {
  const enabled = useApp((state) => state.settings.notifyCodexResets);
  const language = useApp((state) => state.settings.language);
  const body = useInsights((state) => state.feeds.codexResetStatus?.body ?? null);

  useEffect(() => {
    if (enabled) ensureFeed("codexResetStatus");
  }, [enabled]);

  useEffect(() => {
    if (!enabled || !body) return;
    const status = parseResetStatus(body);
    if (!status) return;
    const seen = readSeen();
    const watch = activeWatch(status, new Date());
    const next: Seen = {
      latest: status.latest?.id ?? seen?.latest ?? null,
      scheduled: status.scheduled?.id ?? seen?.scheduled ?? null,
      watch: watchKey(watch) ?? seen?.watch ?? null,
    };
    writeSeen(next);
    if (!seen) return;
    const text = insightsFor(language);
    const send = (title: string, post: string) => {
      announceOnIsland({ title, body: excerpt(post), brand: "codex", severity: "normal" });
      return notify(title, excerpt(post)).catch((error: unknown) => console.error("Sending notification failed", error));
    };
    if (status.latest && status.latest.id !== seen.latest && Date.now() - status.latest.announcedAt.getTime() < FRESH_MS) {
      void send(text.notifyResetTitle, status.latest.text);
    }
    if (status.scheduled && status.scheduled.id !== seen.scheduled) void send(text.notifyScheduledTitle, status.scheduled.text);
    if (watch && watchKey(watch) !== seen.watch) void send(text.notifyWatchTitle(watch.level), watch.text);
  }, [enabled, body, language]);
}
