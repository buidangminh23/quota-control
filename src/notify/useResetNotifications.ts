/**
 * Desktop notifications for Codex resets: one when codex-resets.com records a new reset and one when
 * a reset is announced ahead of time. The ids already seen are kept in the webview's storage so a
 * restart does not repeat them, and the first run only records what is there.
 */
import { announceOnIsland } from "@/glance/alerts";
import { useEffect } from "react";
import { insightsFor } from "@/i18n/insights";
import { parseResetStatus } from "@/model/insights/resets";
import { notify } from "@/platform/system";
import { ensureFeed, useInsights } from "@/state/insights";
import { useApp } from "@/state/store";

const STORAGE_KEY = "quota-control.codex-resets-notified";
/** A reset older than this when first seen is history, not news. */
const FRESH_MS = 48 * 3_600_000;
const EXCERPT_LENGTH = 160;

interface Seen {
  latest: string | null;
  scheduled: string | null;
}

function readSeen(): Seen | null {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<Seen>;
    return { latest: typeof value.latest === "string" ? value.latest : null, scheduled: typeof value.scheduled === "string" ? value.scheduled : null };
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

/** The post's words without its links, on one line and cut to a notification's length. */
export function excerpt(text: string, length = EXCERPT_LENGTH): string {
  const plain = text.replace(/https?:\/\/\S+/g, "").replace(/\s+/g, " ").trim();
  return plain.length > length ? `${plain.slice(0, length - 1).trimEnd()}…` : plain;
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
    const next: Seen = { latest: status.latest?.id ?? seen?.latest ?? null, scheduled: status.scheduled?.id ?? seen?.scheduled ?? null };
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
  }, [enabled, body, language]);
}
