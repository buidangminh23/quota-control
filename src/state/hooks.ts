/**
 * Selector hooks over the store. Each returns a stable reference (zustand v5 re-renders on identity),
 * so derived values are memoized here rather than rebuilt inside selectors.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { messagesFor, type Language, type Messages } from "@/i18n";
import type { BarKind, PlatformKey } from "@/i18n/messages";
import type { IsEnabled } from "@/model/layout";
import { barKind, platformKey } from "@/model/platform";
import type { AppSettings, DashboardTab } from "@/model/settings";
import type { DisplayOptions } from "@/model/widgetData";
import { checkTimeZone, dashboardTabs, displayOptionsOf, isProviderEnabled, useApp, visibleDashboardTab } from "./store";

/** The platform key the interface words itself by (Finder vs File Explorer, ⌘ vs Win). */
export function usePlatformKey(): PlatformKey {
  return platformKey(useApp((state) => state.info?.platform));
}

/** Where starred metrics show: the macOS menu bar or the taskbar. */
export function useBarKind(): BarKind {
  return barKind(useApp((state) => state.info?.platform));
}

export function useSettings(): AppSettings {
  return useApp((state) => state.settings);
}

export function useLanguage(): Language {
  return useApp((state) => state.settings.language);
}

export function useMessages(): Messages {
  return messagesFor(useLanguage());
}

/**
 * Row display options. The exchange rate and the time zone are dependencies on purpose: money on the
 * Vietnamese UI follows the rate (`setDongRate`) and every clock time the zone (`deviceTimeZone`), so
 * a change hands the rows a new object and they draw again.
 */
export function useDisplay(): DisplayOptions {
  const settings = useSettings();
  const rate = useApp((state) => state.exchangeRate?.usdToVnd ?? null);
  const zone = useApp((state) => state.timeZone);
  return useMemo(() => ({ ...displayOptionsOf(settings) }), [settings, rate, zone]);
}

const TIME_ZONE_CHECK_MS = 60_000;

/** Keep the time zone current: at start, when the popup opens, when the window regains focus and every minute. */
export function useTimeZoneWatch(): void {
  const visible = useApp((state) => state.popupVisible);
  useEffect(() => {
    void checkTimeZone();
  }, [visible]);
  useEffect(() => {
    const check = () => void checkTimeZone();
    const timer = setInterval(check, TIME_ZONE_CHECK_MS);
    window.addEventListener("focus", check);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", check);
    };
  }, []);
}

export function useIsEnabled(): IsEnabled {
  const enabledProviders = useApp((state) => state.enabledProviders);
  return useCallback((providerId: string) => isProviderEnabled({ enabledProviders }, providerId), [enabledProviders]);
}

export interface DashboardTabState {
  /** Whether the tab bar shows: whenever there is more than one tab. */
  tabbed: boolean;
  /** The tabs on screen, left to right. */
  tabs: DashboardTab[];
  /** The tab on screen. */
  tab: DashboardTab;
}

export function useDashboardTabs(): DashboardTabState {
  const key = useApp((state) => dashboardTabs(state).join(","));
  const tab = useApp(visibleDashboardTab);
  return useMemo(() => {
    const tabs = key.split(",") as DashboardTab[];
    return { tabbed: tabs.length > 1, tabs, tab };
  }, [key, tab]);
}

/**
 * The current time, re-read every `intervalMs` while the popup is visible (upstream rows tick every
 * 30s through `TimelineView`, which only schedules while the popover is on screen).
 */
export function useNow(intervalMs = 30_000): Date {
  const visible = useApp((state) => state.popupVisible);
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    if (!visible) return;
    setNow(new Date());
    const timer = setInterval(() => setNow(new Date()), intervalMs);
    return () => clearInterval(timer);
  }, [visible, intervalMs]);
  return now;
}

/**
 * The current time, re-read every `intervalMs` whether or not the popup is visible: for surfaces
 * that stay on screen while it is closed (the menu bar, the Dynamic Island, the desktop widget).
 */
export function useWallClock(intervalMs = 60_000): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = setInterval(() => setNow(new Date()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}

const DARK_QUERY = "(prefers-color-scheme: dark)";

function systemDark(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia(DARK_QUERY).matches;
}

/** Whether the operating system is in dark mode, following changes live. */
export function useSystemDark(): boolean {
  const [dark, setDark] = useState(systemDark);
  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const query = window.matchMedia(DARK_QUERY);
    const onChange = () => setDark(query.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return dark;
}

/** The popup's effective appearance: the Theme setting, with "system" following the OS. */
export function useIsDark(): boolean {
  const theme = useApp((state) => state.settings.theme);
  const system = useSystemDark();
  return theme === "system" ? system : theme === "dark";
}
