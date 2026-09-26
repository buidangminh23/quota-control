/**
 * The dashboard screen (upstream `DashboardContentView`), split into two tabs. Hạn mức lists every
 * connected account's card, with onboarding hints and a way to add an account when none is connected.
 * Token shows the total token use across providers, then each provider's usage trend and period
 * rows. The update card and a sign-in in progress sit on top of both.
 */
import { useMemo } from "react";
import { messagesFor, type Messages } from "@/i18n";
import { displayGroups, tokenGroups, type ProviderMetrics } from "@/model/layout";
import type { AppSettings } from "@/model/settings";
import { useDashboardTabs, useDisplay, useIsEnabled, useNow, useSettings } from "@/state/hooks";
import { navigate, updateSettings, useApp } from "@/state/store";
import { LoginProgress } from "../accounts/Accounts";
import { DASHBOARD_PANEL_ID, dashboardTabId } from "../chrome/DashboardTabs";
import { Button } from "../ui/controls";
import { CloseIcon } from "../ui/icons";
import { ProviderSection } from "./ProviderSection";
import { TotalSpendCard } from "./TotalSpendCard";
import { UpdateBanner } from "./UpdateBanner";

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

function ProviderSections({ groups }: { groups: ProviderMetrics[] }) {
  const display = useDisplay();
  const engine = useApp((state) => state.engine);
  const now = useNow();
  const interval = engine?.refreshIntervalMs ?? 300_000;
  return groups.map((group) => (
    <ProviderSection
      key={group.provider.id}
      group={group}
      runtime={engine?.providers[group.provider.id]}
      display={display}
      refreshIntervalMs={interval}
      now={now}
    />
  ));
}

function TokensTab() {
  const catalog = useApp((state) => state.catalog);
  const layout = useApp((state) => state.layout);
  const isEnabled = useIsEnabled();
  const groups = useMemo(() => tokenGroups(layout, catalog, isEnabled), [layout, catalog, isEnabled]);
  return (
    <>
      <TotalSpendCard />
      <ProviderSections groups={groups} />
    </>
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
      <UpdateBanner />
      <LoginProgress messages={messages} />
      {tab === "tokens" ? <TokensTab /> : <LimitsTab settings={settings} messages={messages} />}
    </div>
  );
}
