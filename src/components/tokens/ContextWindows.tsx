/**
 * How full each recent Claude Code and Codex session's context window is, like the indicator in the
 * Claude app: `478,3 N / 1 Tr (48%)` over a bar split into the session's base (system prompt, tools,
 * memory), the conversation since, the latest turn and the free space. Numbers come from the local
 * logs through the core; message text is never read.
 */
import { messagesFor, type Language } from "@/i18n";
import type { ContextSegmentKey } from "@/i18n/usageMessages";
import type { ContextWindowSession } from "@/lib/types";
import { compactDuration } from "@/model/format";
import { VIEW_COLORS } from "@/model/palette";
import { modelLabel } from "@/model/usage";
import { useLanguage, useNow } from "@/state/hooks";
import { useContextWindows } from "@/state/usage";
import { ProviderMark } from "../ui/ProviderMark";
import { InfoCircle, WindowIcon } from "../ui/icons";
import { tooltipProps } from "../ui/tooltip";
import { ChipHeader, tokenCount, UsageNote } from "./parts";

export const CONTEXT_SEGMENT_COLORS: Readonly<Record<Exclude<ContextSegmentKey, "free">, string>> = {
  base: "#0A84FF",
  conversation: "#FF9F0A",
  lastTurn: "#30D158",
};

const SEGMENT_KEYS = ["base", "conversation", "lastTurn"] as const;

/** The session's tokens split into its base, the conversation since and the latest turn. */
export function contextSegments(session: ContextWindowSession): Record<(typeof SEGMENT_KEYS)[number], number> {
  const used = Math.max(session.usedTokens, 0);
  const base = Math.min(Math.max(session.baseTokens, 0), used);
  const lastTurn = Math.min(Math.max(session.lastTurnTokens, 0), used - base);
  return { base, conversation: used - base - lastTurn, lastTurn };
}

function percentTone(percent: number): string {
  if (percent >= 90) return "is-critical";
  if (percent >= 75) return "is-warning";
  return "";
}

function SessionRow({ session, now, language }: { session: ContextWindowSession; now: Date; language: Language }) {
  const messages = messagesFor(language).usage;
  const segments = contextSegments(session);
  const window = session.windowTokens && session.windowTokens > 0 ? session.windowTokens : null;
  const scale = window ?? Math.max(session.usedTokens, 1);
  const percent = window ? Math.min((session.usedTokens / window) * 100, 100) : null;
  const used = tokenCount(session.usedTokens, language);
  const seconds = (now.getTime() - new Date(session.updatedAt).getTime()) / 1000;
  const ago = messages.ago(seconds >= 60 ? compactDuration(seconds, language) : null);
  const breakdown = [
    ...SEGMENT_KEYS.map((key) => `${messages.contextSegment(key)}: ${tokenCount(segments[key], language)}`),
    ...(window ? [`${messages.contextSegment("free")}: ${tokenCount(Math.max(window - session.usedTokens, 0), language)}`] : []),
  ].join("\n");
  return (
    <li className="uc-ctx-row">
      <div className="uc-ctx-head">
        <ProviderMark brand={session.source} size={14} />
        <span className="uc-ctx-project uc-truncate">{session.project || messages.unknownProject}</span>
        <span className="uc-ctx-when">{ago}</span>
      </div>
      <div className="uc-ctx-meta uc-truncate">
        {modelLabel(session.model)} · {messages.contextApp(session.source)}
      </div>
      <div className="uc-ctx-usage uc-num">
        {window && percent !== null ? (
          <>
            {messages.contextUsage(used, tokenCount(window, language))}{" "}
            <span className={`uc-ctx-percent ${percentTone(percent)}`}>{`(${Math.round(percent)}%)`}</span>
          </>
        ) : (
          messages.contextUsed(used)
        )}
      </div>
      <div className="uc-ctx-bar" role="img" aria-label={breakdown} {...tooltipProps(breakdown)}>
        {SEGMENT_KEYS.map((key) =>
          segments[key] > 0 ? <span key={key} style={{ width: `${(segments[key] / scale) * 100}%`, background: CONTEXT_SEGMENT_COLORS[key] }} /> : null,
        )}
      </div>
    </li>
  );
}

export function ContextWindowsCard() {
  const sessions = useContextWindows();
  const language = useLanguage();
  const now = useNow(30_000);
  if (sessions === undefined) return null;
  const messages = messagesFor(language).usage;
  return (
    <section className="uc-section">
      <ChipHeader
        icon={<WindowIcon size={11} />}
        color={VIEW_COLORS.context}
        title={messages.contextTitle}
        trailing={
          <span className="uc-inline-icon uc-secondary" aria-label={messages.contextHint} {...tooltipProps(messages.contextHint)}>
            <InfoCircle size={12} />
          </span>
        }
      />
      <div className="uc-card uc-ctx-card">
        {sessions === null ? (
          <UsageNote text={messages.loading} />
        ) : sessions.length === 0 ? (
          <UsageNote text={messages.contextEmpty} />
        ) : (
          <>
            <ul className="uc-ctx-list">
              {sessions.map((session) => (
                <SessionRow key={`${session.source}:${session.sessionId}`} session={session} now={now} language={language} />
              ))}
            </ul>
            <div className="uc-ctx-legend">
              {[...SEGMENT_KEYS, "free" as const].map((key) => (
                <span key={key} className="uc-ctx-legend-item">
                  <span className={`uc-legend-dot${key === "free" ? " is-free" : ""}`} style={key === "free" ? undefined : { background: CONTEXT_SEGMENT_COLORS[key] }} />
                  {messages.contextSegment(key)}
                </span>
              ))}
            </div>
          </>
        )}
      </div>
    </section>
  );
}
