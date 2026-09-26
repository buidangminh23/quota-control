/**
 * The dashboard's tab bar, in the top bar slot the other screens use for their title: Hạn mức (the
 * accounts' limits) and Token (the total token use). Arrow keys, Home and End move between the tabs
 * (WAI-ARIA tabs pattern with automatic activation); Ctrl+Tab cycles them from anywhere.
 */
import { useEffect, useRef, type KeyboardEvent } from "react";
import { messagesFor } from "@/i18n";
import { DASHBOARD_TABS, type DashboardTab } from "@/model/settings";
import { useLanguage } from "@/state/hooks";
import { selectDashboardTab } from "@/state/store";

export const DASHBOARD_PANEL_ID = "uc-dashboard-panel";

export function dashboardTabId(tab: DashboardTab): string {
  return `uc-dashboard-tab-${tab}`;
}

function targetTab(key: string, current: DashboardTab): DashboardTab | null {
  const index = DASHBOARD_TABS.indexOf(current);
  const last = DASHBOARD_TABS.length - 1;
  switch (key) {
    case "ArrowRight":
      return DASHBOARD_TABS[index === last ? 0 : index + 1]!;
    case "ArrowLeft":
      return DASHBOARD_TABS[index === 0 ? last : index - 1]!;
    case "Home":
      return DASHBOARD_TABS[0]!;
    case "End":
      return DASHBOARD_TABS[last]!;
    default:
      return null;
  }
}

export function DashboardTabs({ tab }: { tab: DashboardTab }) {
  const messages = messagesFor(useLanguage()).dashboard;
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const list = listRef.current;
    if (list?.contains(document.activeElement)) list.querySelector<HTMLButtonElement>(`#${dashboardTabId(tab)}`)?.focus();
  }, [tab]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const next = targetTab(event.key, tab);
    if (!next) return;
    event.preventDefault();
    selectDashboardTab(next);
  };

  return (
    <div className="uc-topbar">
      <div ref={listRef} className="uc-tabs" role="tablist" aria-label={messages.tabsLabel} onKeyDown={onKeyDown}>
        {DASHBOARD_TABS.map((candidate) => {
          const selected = candidate === tab;
          return (
            <button
              key={candidate}
              id={dashboardTabId(candidate)}
              type="button"
              role="tab"
              aria-selected={selected}
              aria-controls={DASHBOARD_PANEL_ID}
              tabIndex={selected ? 0 : -1}
              className={`uc-tab${selected ? " is-selected" : ""}`}
              onClick={() => selectDashboardTab(candidate)}
            >
              {messages.tab(candidate)}
            </button>
          );
        })}
      </div>
    </div>
  );
}
