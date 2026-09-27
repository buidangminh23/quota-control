/**
 * The Accounts screen: every connected Claude and Codex account (always all of them, each with its
 * own live status and a button that opens its product's official site in the app), signing in to
 * another one through the browser, and the other AI providers (their apps' logins on this computer
 * and API keys). Claude Code and the Codex CLI signed in on this computer are listed automatically.
 */
import { useEffect, useState } from "react";
import { messagesFor, type Language, type Messages } from "@/i18n";
import { backend } from "@/lib/backend";
import type { AccountProvider, ConnectedAccount, ProviderRuntimeState } from "@/lib/types";
import { brandName, headerNotice } from "@/model/providerText";
import { useLanguage } from "@/state/hooks";
import { cancelAccountLogin, openChatFor, reloadAccounts, reloadServices, reopenAccountLogin, showNotice, startAccountLogin, useApp } from "@/state/store";
import { Button } from "../ui/controls";
import { confirmAction } from "../ui/dialog";
import { ChatIcon, CloseIcon, Spinner } from "../ui/icons";
import { ProviderMark } from "../ui/ProviderMark";
import { tooltipProps, truncatedTooltipProps } from "../ui/tooltip";
import { AddServiceKey, detectedApps, ServiceCards } from "./Services";
import { errorText, statusOf } from "./status";

const PROVIDERS: readonly AccountProvider[] = ["claude", "codex"];
const CHAT_PRODUCTS: Record<AccountProvider, string> = { claude: "Claude", codex: "ChatGPT" };
const CLI_PRODUCTS: Record<AccountProvider, string> = { claude: "Claude Code", codex: "Codex CLI" };

/** `Claude · Công ty`, or just the brand when the label is the default CLI name. */
function accountTitle(provider: AccountProvider, label: string): string {
  const trimmed = label.trim();
  return trimmed === "" || trimmed.toLowerCase() === provider ? brandName(provider) : `${brandName(provider)} · ${trimmed}`;
}

function AccountRow({ account, runtime, messages, language }: { account: ConnectedAccount; runtime: ProviderRuntimeState | undefined; messages: Messages; language: Language }) {
  const status = statusOf(runtime);
  const notice = status === "error" ? headerNotice(runtime, language) : null;
  const title = accountTitle(account.provider, account.label);
  const remove = async () => {
    const confirmed = await confirmAction({
      title: messages.accounts.removeTitle(title),
      message: messages.accounts.removeMessage,
      confirmLabel: messages.accounts.removeConfirm,
      cancelLabel: messages.accounts.cancel,
    });
    if (!confirmed) return;
    try {
      await backend().removeAccount(account.id);
      await reloadAccounts();
    } catch (error) {
      showNotice(messages.accounts.failed(errorText(error, language)), "notice");
    }
  };
  const chat = () =>
    openChatFor(account.provider, account.label).catch(() => showNotice(messages.accounts.chatOpenFailed, "notice"));
  return (
    <div className="uc-list-row is-account">
      <span className="uc-list-mark">
        <ProviderMark brand={account.provider} size={18} />
      </span>
      <span className="uc-list-text">
        <span className="uc-list-title uc-truncate" {...truncatedTooltipProps(title)}>
          {title}
        </span>
        <span className="uc-list-subtitle uc-truncate">{messages.accounts.mode(account.credentialMode, CLI_PRODUCTS[account.provider])}</span>
        <span className={`uc-account-status is-${status}`} {...tooltipProps(notice)}>
          {status === "refreshing" ? <Spinner size={9} /> : <span className="uc-status-dot" />}
          {messages.accounts.status(status)}
        </span>
      </span>
      <button type="button" className="uc-icon-button" aria-label={messages.dashboard.openChat(CHAT_PRODUCTS[account.provider])} onClick={() => void chat()} {...tooltipProps(messages.dashboard.openChat(CHAT_PRODUCTS[account.provider]))}>
        <ChatIcon size={13} />
      </button>
      {account.credentialMode === "cli" ? null : (
        <button type="button" className="uc-icon-button" aria-label={`${messages.accounts.remove} ${title}`} onClick={() => void remove()} {...tooltipProps(messages.accounts.remove)}>
          <CloseIcon size={11} />
        </button>
      )}
    </div>
  );
}

/**
 * The sign-in the core is waiting on. The popup hides while the browser is in front and comes back
 * by itself once the account is saved, so the waiting state lives in the store.
 */
export function LoginProgress({ messages }: { messages: Messages }) {
  const login = useApp((state) => state.accountLogin);
  if (!login) return null;
  const brand = brandName(login.provider);
  return (
    <div className="uc-card uc-add-account">
      <div className="uc-login-flow" role="status">
        <div className="uc-login-head">
          <ProviderMark brand={login.provider} size={16} />
          <span className="uc-list-title">{brand}</span>
        </div>
        <p className="uc-login-waiting">
          <Spinner size={11} />
          <span>{login.phase === "waiting" ? messages.accounts.waiting(brand, login.browser) : messages.accounts.starting}</span>
        </p>
        <p className="uc-settings-note is-flush">{messages.accounts.waitingNote}</p>
        <div className="uc-settings-actions is-split">
          <Button onClick={cancelAccountLogin} className="is-small">
            {messages.accounts.cancel}
          </Button>
          <Button onClick={() => void reopenAccountLogin()} disabled={login.phase !== "waiting"} className="is-small">
            {messages.accounts.openSignInPage}
          </Button>
        </div>
      </div>
    </div>
  );
}

function AddAccount({ messages }: { messages: Messages }) {
  const [provider, setProvider] = useState<AccountProvider>("claude");
  const login = useApp((state) => state.accountLogin);
  const error = useApp((state) => state.accountLoginError);
  if (login) return <LoginProgress messages={messages} />;
  return (
    <div className="uc-card uc-add-account">
      <div className="uc-capsule-picker" role="radiogroup" aria-label={messages.accounts.add}>
        {PROVIDERS.map((candidate) => (
          <button
            key={candidate}
            type="button"
            role="radio"
            aria-checked={candidate === provider}
            className={`uc-capsule-segment${candidate === provider ? " is-selected" : ""}`}
            onClick={() => setProvider(candidate)}
          >
            {brandName(candidate)}
          </button>
        ))}
      </div>
      <Button variant="prominent" onClick={() => void startAccountLogin(provider)} className="is-small is-wide">
        {messages.accounts.signInWithGoogle}
      </Button>
      <p className="uc-settings-note is-flush">{messages.accounts.signInNote(brandName(provider))}</p>
      {error ? <p className="uc-settings-notice is-flush">{messages.accounts.loginFailed(brandName(error.provider), error.text)}</p> : null}
    </div>
  );
}

export function Accounts() {
  const language = useLanguage();
  const messages = messagesFor(language);
  const accounts = useApp((state) => state.accounts);
  const engine = useApp((state) => state.engine);
  const services = useApp((state) => state.services);
  const apps = detectedApps(services);

  useEffect(() => {
    void reloadAccounts();
    void reloadServices();
  }, []);

  return (
    <div className="uc-stack">
      <section className="uc-group">
        <h2 className="uc-group-title">{messages.accounts.connected}</h2>
        <div className="uc-card uc-list-card">
          {accounts.length === 0 ? <p className="uc-card-empty">{messages.accounts.none}</p> : null}
          {accounts.map((account) => (
            <AccountRow key={account.id} account={account} runtime={engine?.providers[account.id]} messages={messages} language={language} />
          ))}
        </div>
      </section>

      <section className="uc-group">
        <h2 className="uc-group-title">{messages.accounts.add}</h2>
        <AddAccount messages={messages} />
        <p className="uc-group-note">{messages.accounts.cliNote}</p>
      </section>

      <section className="uc-group">
        <h2 className="uc-group-title">{messages.accounts.otherServices}</h2>
        <ServiceCards messages={messages} language={language} />
        {apps.length > 0 ? <p className="uc-group-note">{messages.accounts.detectedNote(apps.join(", "))}</p> : null}
      </section>

      <section className="uc-group">
        <h2 className="uc-group-title">{messages.accounts.addKey}</h2>
        <AddServiceKey messages={messages} language={language} />
      </section>
    </div>
  );
}
