/**
 * Which operating system the popup runs on, in the two forms the interface words things by: the
 * platform key (Finder vs File Explorer, ⌘ vs Win) and where starred metrics show (the macOS menu
 * bar or the taskbar).
 */
import type { BarKind, PlatformKey } from "@/i18n/messages";
import type { Platform } from "@/lib/types";

export function platformKey(platform: Platform | string | undefined): PlatformKey {
  return platform === "windows" || platform === "linux" || platform === "macos" ? platform : "other";
}

export function barKind(platform: Platform | string | undefined): BarKind {
  return platform === "macos" ? "menuBar" : "taskbar";
}
