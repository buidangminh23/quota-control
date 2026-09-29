/**
 * Operating-system features the popup reaches through the core: launch at login and desktop
 * notifications. macOS does both its own way (a login item of the app, the system's notification
 * center); the core answers "not here" elsewhere, and the Tauri plugins do the work as before.
 * Each degrades to "unsupported" in the browser build, so Settings can hide what the platform
 * cannot do instead of failing on click.
 */
import { backend, isTauri } from "@/lib/backend";
import type { SystemNotificationAccess } from "@/lib/types";

export async function autostartEnabled(): Promise<boolean | null> {
  if (!isTauri()) return null;
  const system = (await backend().systemLaunchAtLogin?.()) ?? null;
  if (system !== null) return system;
  const { isEnabled } = await import("@tauri-apps/plugin-autostart");
  return isEnabled();
}

export async function setAutostart(enabled: boolean): Promise<void> {
  if (!isTauri()) throw new Error("Launch at login is unavailable here");
  const system = (await backend().setSystemLaunchAtLogin?.(enabled)) ?? null;
  if (system !== null) return;
  const plugin = await import("@tauri-apps/plugin-autostart");
  if (enabled) await plugin.enable();
  else await plugin.disable();
}

export type NotificationAccess = "granted" | "denied" | "unsupported";

/** A user who was never asked has not allowed notifications yet: Settings offers to ask. */
function known(access: SystemNotificationAccess): NotificationAccess {
  return access === "granted" ? "granted" : "denied";
}

export async function notificationAccess(): Promise<NotificationAccess> {
  if (isTauri()) {
    const system = (await backend().systemNotificationAccess?.()) ?? null;
    if (system !== null) return known(system);
    const { isPermissionGranted } = await import("@tauri-apps/plugin-notification");
    return (await isPermissionGranted()) ? "granted" : "denied";
  }
  if (typeof Notification === "undefined") return "unsupported";
  return Notification.permission === "granted" ? "granted" : "denied";
}

export async function requestNotificationAccess(): Promise<NotificationAccess> {
  if (isTauri()) {
    const system = (await backend().requestSystemNotificationAccess?.()) ?? null;
    if (system !== null) return known(system);
    const { requestPermission } = await import("@tauri-apps/plugin-notification");
    return (await requestPermission()) === "granted" ? "granted" : "denied";
  }
  if (typeof Notification === "undefined") return "unsupported";
  return (await Notification.requestPermission()) === "granted" ? "granted" : "denied";
}

/** What a notification is about (`id`) and whose it is (`group`), for systems that keep track. */
export interface NotificationTopic {
  id: string;
  group: string;
}

export async function notify(title: string, body: string, topic?: NotificationTopic): Promise<void> {
  if (isTauri()) {
    if (await backend().sendSystemNotification?.({ title, body, ...topic })) return;
    const { sendNotification } = await import("@tauri-apps/plugin-notification");
    sendNotification({ title, body });
    return;
  }
  if (typeof Notification !== "undefined" && Notification.permission === "granted") new Notification(title, { body });
}

