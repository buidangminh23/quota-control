/**
 * Operating-system features the popup reaches through Tauri plugins: launch at login, desktop
 * notifications and revealing the log file. Each degrades to "unsupported" in the browser build, so
 * Settings can hide what the platform cannot do instead of failing on click.
 */
import { isTauri } from "@/lib/backend";

export async function autostartEnabled(): Promise<boolean | null> {
  if (!isTauri()) return null;
  const { isEnabled } = await import("@tauri-apps/plugin-autostart");
  return isEnabled();
}

export async function setAutostart(enabled: boolean): Promise<void> {
  if (!isTauri()) throw new Error("Launch at login is unavailable here");
  const plugin = await import("@tauri-apps/plugin-autostart");
  if (enabled) await plugin.enable();
  else await plugin.disable();
}

export type NotificationAccess = "granted" | "denied" | "unsupported";

export async function notificationAccess(): Promise<NotificationAccess> {
  if (isTauri()) {
    const { isPermissionGranted } = await import("@tauri-apps/plugin-notification");
    return (await isPermissionGranted()) ? "granted" : "denied";
  }
  if (typeof Notification === "undefined") return "unsupported";
  return Notification.permission === "granted" ? "granted" : "denied";
}

export async function requestNotificationAccess(): Promise<NotificationAccess> {
  if (isTauri()) {
    const { requestPermission } = await import("@tauri-apps/plugin-notification");
    return (await requestPermission()) === "granted" ? "granted" : "denied";
  }
  if (typeof Notification === "undefined") return "unsupported";
  return (await Notification.requestPermission()) === "granted" ? "granted" : "denied";
}

export async function notify(title: string, body: string): Promise<void> {
  if (isTauri()) {
    const { sendNotification } = await import("@tauri-apps/plugin-notification");
    sendNotification({ title, body });
    return;
  }
  if (typeof Notification !== "undefined" && Notification.permission === "granted") new Notification(title, { body });
}

export function canRevealFiles(): boolean {
  return isTauri();
}

export async function revealFile(path: string): Promise<void> {
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path);
}
