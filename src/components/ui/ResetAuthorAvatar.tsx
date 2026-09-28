/**
 * The round profile picture of @thsottiaux, whose posts on X codex-resets.com reads resets from.
 * Decorative: every place that shows it also names the source in text or in its label.
 */
import avatarUrl from "@/assets/thsottiaux.webp";

export const RESET_AUTHOR_HANDLE = "@thsottiaux";

export function ResetAuthorAvatar({ size }: { size: number }) {
  return <img className="uc-reset-avatar" src={avatarUrl} alt="" width={size} height={size} draggable={false} />;
}
