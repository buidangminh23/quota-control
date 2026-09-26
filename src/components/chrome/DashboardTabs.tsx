/**
 * The dashboard's tab bar, in the top bar slot the other screens use for their title: Hạn mức (the
 * accounts' limits), Token (the token use) and Bảng giá (the official price lists). Arrow keys, Home
 * and End move between the tabs (WAI-ARIA tabs pattern with automatic activation); Ctrl+Tab cycles
 * them from anywhere.
 */
import { useEffect, useRef, type CSSProperties, type KeyboardEvent } from "react";
import { messagesFor } from "@/i18n";
import { TAB_COLORS } from "@/model/palette";
import type { DashboardTab } from "@/model/settings";
import { useLanguage } from "@/state/hooks";
import { selectDashboardTab } from "@/state/store";

export const DASHBOARD_PANEL_ID = "uc-dashboard-panel";

export function dashboardTabId(tab: DashboardTab): string {
  return `uc-dashboard-tab-${tab}`;
}

function targetTab(key: string, tabs: readonly DashboardTab[], current: DashboardTab): DashboardTab | null {
  const index = tabs.indexOf(current);
  const last = tabs.length - 1;
  switch (key) {
    case "ArrowRight":
      return tabs[index === last ? 0 : index + 1]!;
    case "ArrowLeft":
      return tabs[index === 0 ? last : index - 1]!;
    case "Home":
      return tabs[0]!;
    case "End":
      return tabs[last]!;
    default:
      return null;
  }
}

export function DashboardTabs({ tabs, tab }: { tabs: readonly DashboardTab[]; tab: DashboardTab }) {
  const messages = messagesFor(useLanguage()).dashboard;
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const list = listRef.current;
    if (list?.contains(document.activeElement)) list.querySelector<HTMLButtonElement>(`#${dashboardTabId(tab)}`)?.focus();
  }, [tab]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const next = targetTab(event.key, tabs, tab);
    if (!next) return;
    event.preventDefault();
    selectDashboardTab(next);
  };

  return (
    <div className="uc-topbar">
      <div ref={listRef} className="uc-tabs" role="tablist" aria-label={messages.tabsLabel} onKeyDown={onKeyDown}>
        {tabs.map((candidate) => {
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
              style={{ "--uc-tab-accent": TAB_COLORS[candidate] } as CSSProperties}
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
