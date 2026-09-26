/**
 * The dashboard screen (upstream `DashboardContentView`): the update card, a sign-in in progress,
 * onboarding hints, the Total Spend card and every enabled provider's section. Connected accounts
 * are always listed; with none connected, a hint (or, once dismissed, a one-line row) leads to the
 * Accounts screen.
 */
import { useMemo } from "react";
import { messagesFor } from "@/i18n";
import { displayGroups, spendCapableProviders } from "@/model/layout";
import { useDisplay, useIsEnabled, useNow, useSettings } from "@/state/hooks";
import { navigate, updateSettings, useApp } from "@/state/store";
import { LoginProgress } from "../accounts/Accounts";
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

export function Dashboard() {
  const settings = useSettings();
  const display = useDisplay();
  const messages = messagesFor(settings.language);
  const catalog = useApp((state) => state.catalog);
  const layout = useApp((state) => state.layout);
  const engine = useApp((state) => state.engine);
  const accounts = useApp((state) => state.accounts);
  const signingIn = useApp((state) => state.accountLogin !== null);
  const ready = useApp((state) => state.ready);
  const isEnabled = useIsEnabled();
  const now = useNow();
  const groups = useMemo(() => displayGroups(layout, catalog, isEnabled), [layout, catalog, isEnabled]);
  const hasSpend = useMemo(() => spendCapableProviders(layout, catalog, isEnabled).length > 0, [layout, catalog, isEnabled]);
  const interval = engine?.refreshIntervalMs ?? 300_000;
  const noAccounts = ready && accounts.length === 0 && !signingIn;
  const showAccountsHint = noAccounts && !settings.accountsHintDismissed;
  const showAccountsRow = noAccounts && settings.accountsHintDismissed;
  const showCustomizeHint = ready && !showAccountsHint && !settings.customizeHintDismissed;

  return (
    <div className="uc-stack">
      <UpdateBanner />
      <LoginProgress messages={messages} />
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
      {settings.showTotalSpend && hasSpend ? <TotalSpendCard /> : null}
      {groups.length === 0 ? (
        <p className="uc-empty">{ready ? messages.dashboard.emptyState : ""}</p>
      ) : (
        groups.map((group) => (
          <ProviderSection
            key={group.provider.id}
            group={group}
            runtime={engine?.providers[group.provider.id]}
            display={display}
            refreshIntervalMs={interval}
            now={now}
          />
        ))
      )}
    </div>
  );
}
