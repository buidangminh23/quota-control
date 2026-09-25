/**
 * The native taskbar strip is optional: the core exposes it when it can embed a window in the
 * taskbar (Windows) or set a tray title (Linux). The popup detects the methods on the backend at run
 * time, so the strip lights up as soon as the core ships them and the tray-icon glyph covers the rest.
 */
import { useSyncExternalStore } from "react";
import { backend, type Unsubscribe } from "@/lib/backend";

export interface TaskbarInfo {
  supported: boolean;
  /** Physical pixel height of the taskbar band. */
  height: number;
  scale: number;
  /** The taskbar's own theme (Windows "system" theme), not the app theme. */
  theme: "light" | "dark";
  edge: "bottom" | "top" | "left" | "right";
}

export interface StripFrame {
  png: Uint8Array;
  width: number;
  height: number;
  text: string;
  tooltip: string;
}

interface StripApi {
  setTaskbarStrip(frame: StripFrame | null): Promise<void>;
  taskbarInfo(): Promise<TaskbarInfo>;
  onTaskbarInfo?(listener: (info: TaskbarInfo) => void): Unsubscribe;
}

function stripApi(): StripApi | null {
  const candidate = backend() as unknown as Partial<StripApi>;
  return typeof candidate.setTaskbarStrip === "function" && typeof candidate.taskbarInfo === "function" ? (candidate as StripApi) : null;
}

let info: TaskbarInfo | null = null;
const listeners = new Set<() => void>();

function publish(next: TaskbarInfo | null): void {
  info = next;
  for (const listener of listeners) listener();
}

/** Ask the core for taskbar support once and follow its changes; a no-op when the core has no strip. */
export function watchTaskbarInfo(): Unsubscribe {
  const api = stripApi();
  if (!api) return () => {};
  let alive = true;
  api
    .taskbarInfo()
    .then((next) => alive && publish(next))
    .catch((error: unknown) => console.error("Reading taskbar info failed", error));
  const stop = api.onTaskbarInfo?.((next) => publish(next));
  return () => {
    alive = false;
    stop?.();
  };
}

export function useTaskbarInfo(): TaskbarInfo | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => info,
    () => info,
  );
}

export function taskbarStripSupported(): boolean {
  return info?.supported === true;
}

export function pushStripFrame(frame: StripFrame | null): Promise<void> {
  const api = stripApi();
  return api ? api.setTaskbarStrip(frame) : Promise.resolve();
}
