/**
 * The popup shell: loads the store, applies theme/density/motion to the document, routes between
 * screens and dashboard tabs, reports the content height so the core sizes the window, handles the
 * keyboard shortcuts (Esc, Enter, Ctrl+Tab, Ctrl+R, Ctrl+,, Ctrl+Z, Ctrl+Q) and keeps the taskbar and
 * notifications live.
 */
import { useEffect, useLayoutEffect, useRef, type RefObject } from "react";
import { messagesFor } from "@/i18n";
import { backend } from "@/lib/backend";
import { useUsageNotifications } from "@/notify/useUsageNotifications";
import { useTaskbarStrip } from "@/strip/useTaskbarStrip";
import { useGlance } from "@/glance/useGlance";
import { useDashboardTabs, useIsDark, useTimeZoneWatch } from "@/state/hooks";
import { cycleDashboardTab, dashboardTabs, navigate, refresh, startApp, undoLayout, useApp, type Screen } from "@/state/store";
import { startInsights } from "@/state/insights";
import { useResetNotifications } from "@/notify/useResetNotifications";
import { Accounts } from "./components/accounts/Accounts";
import { DashboardTabs } from "./components/chrome/DashboardTabs";
import { Footer } from "./components/chrome/Footer";
import { goBack, TopBar } from "./components/chrome/TopBar";
import { Customize } from "./components/customize/Customize";
import { Dashboard } from "./components/dashboard/Dashboard";
import { Settings } from "./components/settings/Settings";
import { Pill } from "./components/ui/controls";
import { closeDialog, DialogLayer, isDialogOpen } from "./components/ui/dialog";
import { UpdateDialog } from "./components/update/UpdateDialog";
import { dismissHoverPopovers } from "./components/ui/hoverPopover";
import { closeMenu, isMenuOpen, MenuLayer } from "./components/ui/menu";
import { hideTooltip, TooltipLayer } from "./components/ui/tooltip";

function useDocumentAttributes(): void {
  const dark = useIsDark();
  const density = useApp((state) => state.settings.density);
  const reduceAnimations = useApp((state) => state.settings.reduceAnimations);
  const language = useApp((state) => state.settings.language);
  useLayoutEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = dark ? "dark" : "light";
    root.dataset.density = density;
    root.dataset.reduceMotion = String(reduceAnimations || window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true);
    root.lang = language;
    document.title = messagesFor(language).chrome.appName;
  }, [dark, density, reduceAnimations, language]);
}

function editableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT";
}

function interactiveTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (editableTarget(target) || target.closest("button, a, [role='switch'], [role='menuitem'], [tabindex]") !== null);
}

function toggleScreen(screen: Screen): void {
  navigate(useApp.getState().screen === screen ? "dashboard" : screen);
}

function useKeyboard(): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (isDialogOpen() || isMenuOpen()) return;
      const ctrl = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();
      if (event.key === "Escape") {
        event.preventDefault();
        dismissHoverPopovers();
        if (useApp.getState().screen !== "dashboard") goBack();
        else void backend().hidePopup();
        return;
      }
      if (ctrl && event.key === "Tab") {
        const state = useApp.getState();
        if (state.screen === "dashboard" && dashboardTabs(state).length > 1) {
          event.preventDefault();
          cycleDashboardTab(event.shiftKey ? -1 : 1);
        }
        return;
      }
      if (event.key === "F5" || (ctrl && key === "r")) {
        event.preventDefault();
        refresh();
        return;
      }
      if (ctrl && event.key === ",") {
        event.preventDefault();
        toggleScreen("settings");
        return;
      }
      if (ctrl && key === "q") {
        event.preventDefault();
        void backend().quit();
        return;
      }
      if (ctrl && key === "z" && !editableTarget(event.target)) {
        if (useApp.getState().screen === "customize") {
          event.preventDefault();
          undoLayout();
        }
        return;
      }
      if (event.key === "Enter" && !ctrl && !interactiveTarget(event.target)) {
        const screen = useApp.getState().screen;
        if (screen === "dashboard" || screen === "customize") {
          event.preventDefault();
          toggleScreen("customize");
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

function usePopupHeight(topRef: RefObject<HTMLElement | null>, contentRef: RefObject<HTMLElement | null>, footerRef: RefObject<HTMLElement | null>, view: string): void {
  const last = useRef(0);
  useLayoutEffect(() => {
    let frame = 0;
    const report = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const total = Math.ceil((topRef.current?.offsetHeight ?? 0) + (contentRef.current?.offsetHeight ?? 0) + (footerRef.current?.offsetHeight ?? 0));
        if (total <= 0 || total === last.current) return;
        last.current = total;
        void backend().resizePopup(total).catch((error: unknown) => console.error("Resizing the popup failed", error));
      });
    };
    const observer = new ResizeObserver(report);
    for (const ref of [topRef, contentRef, footerRef]) if (ref.current) observer.observe(ref.current);
    report();
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [topRef, contentRef, footerRef, view]);
}

function ScreenContent({ screen }: { screen: Screen }) {
  switch (screen) {
    case "dashboard":
      return <Dashboard />;
    case "customize":
      return <Customize />;
    case "settings":
      return <Settings />;
    case "accounts":
      return <Accounts />;
  }
}

export function App() {
  const screen = useApp((state) => state.screen);
  const previous = useApp((state) => state.previousScreen);
  const tabMotion = useApp((state) => state.tabMotion);
  const detail = useApp((state) => state.customizeProviderId);
  const { tabbed, tabs, tab } = useDashboardTabs();
  const visible = useApp((state) => state.popupVisible);
  const notice = useApp((state) => state.notice);
  const topRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const footerRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLElement>(null);

  useEffect(() => startInsights(), []);

  useEffect(() => {
    const release = startApp();
    const blockContextMenu = (event: MouseEvent) => {
      if (!editableTarget(event.target)) event.preventDefault();
    };
    document.addEventListener("contextmenu", blockContextMenu);
    return () => {
      release();
      document.removeEventListener("contextmenu", blockContextMenu);
    };
  }, []);

  useEffect(() => {
    if (visible) return;
    dismissHoverPopovers();
    closeMenu();
    closeDialog();
    hideTooltip();
  }, [visible]);

  const view = screen === "dashboard" ? `dashboard:${tabbed ? tab : "limits-only"}` : `${screen}:${detail ?? ""}`;

  useLayoutEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
    dismissHoverPopovers();
    hideTooltip();
  }, [view]);

  useDocumentAttributes();
  useKeyboard();
  usePopupHeight(topRef, contentRef, footerRef, view);
  useTaskbarStrip();
  useGlance();
  useUsageNotifications();
  useResetNotifications();
  useTimeZoneWatch();

  const direction = tabMotion ?? (screen === "dashboard" && previous !== "dashboard" ? "back" : "forward");
  return (
    <div className="uc-shell" data-screen={screen}>
      {screen !== "dashboard" ? (
        <div ref={topRef}>
          <TopBar screen={screen} />
        </div>
      ) : tabbed ? (
        <div ref={topRef}>
          <DashboardTabs tabs={tabs} tab={tab} />
        </div>
      ) : null}
      <main ref={scrollRef} className="uc-scroll">
        <div ref={contentRef} key={view} className={`uc-content is-entering-${direction}`}>
          <ScreenContent screen={screen} />
        </div>
      </main>
      {screen !== "customize" ? (
        <div ref={footerRef}>
          <Footer screen={screen} />
        </div>
      ) : notice ? (
        <div className="uc-floating-pill" key={notice.id}>
          <Pill text={notice.text} tone={notice.tone} />
        </div>
      ) : null}
      <TooltipLayer />
      <MenuLayer />
      <UpdateDialog />
      <DialogLayer />
    </div>
  );
}
