/**
 * The round profile picture of the account a reset was announced from: @thsottiaux, whose posts
 * codex-resets.com reads, and @ClaudeDevs, which claude-resets.com follows. Any other account (a
 * Claude Code team member) gets its initial. Decorative: every place that shows it also names the
 * source in text or in its label.
 */
import claudeDevsUrl from "@/assets/claudedevs.webp";
import thsottiauxUrl from "@/assets/thsottiaux.webp";

export const RESET_AUTHOR_HANDLE = "@thsottiaux";
export const CLAUDE_AUTHOR_HANDLE = "@ClaudeDevs";

const AVATARS: Readonly<Record<string, string>> = {
  [RESET_AUTHOR_HANDLE.toLowerCase()]: thsottiauxUrl,
  [CLAUDE_AUTHOR_HANDLE.toLowerCase()]: claudeDevsUrl,
};

export function ResetAuthorAvatar({ size, handle = RESET_AUTHOR_HANDLE }: { size: number; handle?: string }) {
  const url = AVATARS[handle.toLowerCase()];
  if (url) return <img className="uc-reset-avatar" src={url} alt="" width={size} height={size} draggable={false} />;
  return (
    <span className="uc-reset-avatar is-initial" style={{ width: size, height: size, fontSize: Math.round(size * 0.55) }} aria-hidden="true">
      {handle.replace(/^@/, "").charAt(0).toUpperCase()}
    </span>
  );
}
