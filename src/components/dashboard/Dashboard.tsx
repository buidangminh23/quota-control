/**
 * The dashboard screen (upstream `DashboardContentView`), split into tabs. Hạn mức lists every
 * connected account's card, with onboarding hints and a way to add an account when none is connected.
 * Token holds the usage views (overview, history, charts, projects) and Bảng giá the official price
 * lists. The update card and a sign-in in progress sit on top of every tab.
 */
import { useMemo } from "react";
import { messagesFor, type Messages } from "@/i18n";
import { displayGroups } from "@/model/layout";
import type { AppSettings } from "@/model/settings";
import { useDashboardTabs, useIsEnabled, useSettings } from "@/state/hooks";
import { navigate, updateSettings, useApp } from "@/state/store";
import { LoginProgress } from "../accounts/Accounts";
import { BenchmarkTab } from "../insights/BenchmarkTab";
import { ResetsTab } from "../insights/ResetsTab";
import { DASHBOARD_PANEL_ID, dashboardTabId } from "../chrome/DashboardTabs";
import { Button } from "../ui/controls";
import { CloseIcon } from "../ui/icons";
import { PricesTab } from "../prices/PricesTab";
import { TokensTab } from "../tokens/TokensTab";
import { ProviderSections } from "./ProviderSections";

function HintCard({ title, message, action, onAction, onDismiss, dismissLabel }: { title: string; message: string; action: string; onAction: () => void; onDismiss: () => void; dismissLabel: string }) {
  return (
    <div className="uc-card uc-hint">
      <div className="uc-hint-head">
        <span className="uc-hint-title">{title}</span>
        <button type="button" className="uc-icon-button" aria-label={dismissLabel} onClick={onDismiss}>
          <CloseIcon size={10} />
        </button>
      </div>
      <p className="uc-hint-message">{message}</p>
      <Button variant="prominent" onClick={onAction} className="is-small">
        {action}
      </Button>
    </div>
  );
}

function LimitsTab({ settings, messages }: { settings: AppSettings; messages: Messages }) {
  const catalog = useApp((state) => state.catalog);
  const layout = useApp((state) => state.layout);
  const accounts = useApp((state) => state.accounts);
  const signingIn = useApp((state) => state.accountLogin !== null);
  const ready = useApp((state) => state.ready);
  const isEnabled = useIsEnabled();
  const groups = useMemo(() => displayGroups(layout, catalog, isEnabled), [layout, catalog, isEnabled]);
  const noAccounts = ready && accounts.length === 0 && !signingIn;
  const showAccountsHint = noAccounts && !settings.accountsHintDismissed;
  const showAccountsRow = noAccounts && settings.accountsHintDismissed;
  const showCustomizeHint = ready && !showAccountsHint && !settings.customizeHintDismissed;

  return (
    <>
      {showAccountsRow ? (
        <div className="uc-card uc-list-card">
          <div className="uc-list-row">
            <span className="uc-list-text">
              <span className="uc-list-title">{messages.dashboard.noAccountsShort}</span>
            </span>
            <Button variant="prominent" onClick={() => navigate("accounts")} className="is-small">
              {messages.dashboard.addAccount}
            </Button>
          </div>
        </div>
      ) : null}
      {showAccountsHint ? (
        <HintCard
          title={messages.dashboard.noAccountsTitle}
          message={messages.dashboard.noAccountsMessage}
          action={messages.dashboard.addAccount}
          onAction={() => navigate("accounts")}
          onDismiss={() => updateSettings({ accountsHintDismissed: true })}
          dismissLabel={messages.dashboard.dismiss}
        />
      ) : null}
      {showCustomizeHint ? (
        <HintCard
          title={messages.dashboard.welcomeTitle}
          message={messages.dashboard.welcomeMessage}
          action={messages.dashboard.openCustomize}
          onAction={() => {
            updateSettings({ customizeHintDismissed: true });
            navigate("customize");
          }}
          onDismiss={() => updateSettings({ customizeHintDismissed: true })}
          dismissLabel={messages.dashboard.dismiss}
        />
      ) : null}
      {groups.length > 0 ? (
        <ProviderSections groups={groups} />
      ) : noAccounts ? null : (
        <p className="uc-empty">{ready ? messages.dashboard.emptyState : ""}</p>
      )}
    </>
  );
}

export function Dashboard() {
  const settings = useSettings();
  const messages = messagesFor(settings.language);
  const { tabbed, tab } = useDashboardTabs();
  const panel = tabbed ? { role: "tabpanel", id: DASHBOARD_PANEL_ID, "aria-labelledby": dashboardTabId(tab) } : {};

  return (
    <div className="uc-stack" {...panel}>
      <LoginProgress messages={messages} />
      {tab === "tokens" ? (
        <TokensTab />
      ) : tab === "prices" ? (
        <PricesTab />
      ) : tab === "benchmark" ? (
        <BenchmarkTab />
      ) : tab === "resets" ? (
        <ResetsTab />
      ) : (
        <LimitsTab settings={settings} messages={messages} />
      )}
    </div>
  );
}
