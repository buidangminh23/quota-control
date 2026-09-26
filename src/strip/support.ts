/**
 * The native taskbar strip is optional: the core hosts it where it can (a window in the Windows
 * taskbar, the tray title on Linux) and exposes `taskbarInfo`/`setTaskbarStrip` on the backend. Where
 * the backend has neither, the strip reports unsupported and the tray-icon glyph covers it.
 */
import { useSyncExternalStore } from "react";
import { backend, type Unsubscribe } from "@/lib/backend";
import type { StripFrame, TaskbarInfo } from "@/lib/types";

let info: TaskbarInfo | null = null;
const listeners = new Set<() => void>();

function publish(next: TaskbarInfo | null): void {
  info = next;
  for (const listener of listeners) listener();
}

/** Ask the core for taskbar support once and follow its changes; a no-op when the core has no strip. */
export function watchTaskbarInfo(): Unsubscribe {
  const api = backend();
  if (!api.taskbarInfo) return () => {};
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

export function pushStripFrame(frame: StripFrame | null): Promise<void> {
  const api = backend();
  return api.setTaskbarStrip ? api.setTaskbarStrip(frame) : Promise.resolve();
}
