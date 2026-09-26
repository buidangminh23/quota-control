/** The About panel: name, version, the attribution the MIT license asks for, and the repository link. */
import { messagesFor } from "@/i18n";
import { backend } from "@/lib/backend";
import { useApp } from "@/state/store";
import { openDialog } from "../ui/dialog";

export const REPOSITORY_URL = "https://github.com/buidangminh23/quota-control";

export function openAboutDialog(): void {
  const { info, settings } = useApp.getState();
  const messages = messagesFor(settings.language);
  const name = info?.name ?? messages.chrome.appName;
  openDialog({
    title: messages.chrome.identity(name, info?.version ?? ""),
    message: messages.chrome.aboutDescription,
    actions: [
      { label: messages.chrome.close, role: "cancel" },
      { label: messages.chrome.openRepository, role: "default", onSelect: () => void backend().openUrl(REPOSITORY_URL) },
    ],
  });
}
