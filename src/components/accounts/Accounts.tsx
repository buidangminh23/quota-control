/**
 * The Accounts screen: one list of every connected account (Claude and Codex first, then every
 * other AI provider connected here or read from an app's login on this computer, each with its own
 * live status), and adding an
 * account of any provider from one list (Claude and Codex through a Google sign-in in the browser;
 * the other services every way they connect: a Google or GitHub sign-in in the browser, an API key
 * or a session cookie, or signing in to the app whose login they read). Claude Code and the Codex
 * CLI signed in on this computer are listed automatically.
 */
import { useEffect, useMemo, useState } from "react";
import { messagesFor, type Language, type Messages } from "@/i18n";
import { backend } from "@/lib/backend";
import type { AccountProvider, ConnectedAccount, ProviderRuntimeState } from "@/lib/types";
import { brandName, headerNotice } from "@/model/providerText";
import { useLanguage } from "@/state/hooks";
import { cancelAccountLogin, loginBrandIn, reloadAccounts, reloadServices, reopenAccountLogin, showNotice, startAccountLogin, useApp } from "@/state/store";
import { Button } from "../ui/controls";
import { confirmAction } from "../ui/dialog";
import { CloseIcon, PlusIcon, Spinner } from "../ui/icons";
import { ProviderMark } from "../ui/ProviderMark";
import { tooltipProps, truncatedTooltipProps } from "../ui/tooltip";
import { addKind, connectsHere, isAddable, matchesProvider, ServiceAppNote, ServicePanel, serviceCardRows, ServiceRow } from "./Services";
import { errorText, statusOf } from "./status";

const PROVIDERS: readonly AccountProvider[] = ["claude", "codex"];
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
  const services = useApp((state) => state.services);
  if (!login) return null;
  const brand = loginBrandIn(login.provider, services);
  const userCode = login.phase === "waiting" ? login.userCode : undefined;
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
        {userCode ? <UserCode code={userCode} messages={messages} /> : null}
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

/**
 * The code a device sign-in (GitHub) asks the user to type on its page. It is copied to the
 * clipboard as soon as it appears, so pasting it is enough.
 */
function UserCode({ code, messages }: { code: string; messages: Messages }) {
  const [copied, setCopied] = useState(false);
  const copy = () =>
    backend()
      .copyText(code)
      .then(() => setCopied(true))
      .catch(() => setCopied(false));
  useEffect(() => {
    void copy();
  }, [code]);
  return (
    <div className="uc-user-code">
      <span className="uc-user-code-label">{messages.accounts.userCodeLabel}</span>
      <div className="uc-user-code-row">
        <span className="uc-user-code-value">{code}</span>
        <Button onClick={() => void copy()} className="is-small">
          {copied ? messages.settings.copied : messages.accounts.copyCode}
        </Button>
      </div>
      <p className="uc-settings-note is-flush">{messages.accounts.userCodeNote}</p>
    </div>
  );
}

function GoogleSignIn({ provider, messages, onBack }: { provider: AccountProvider; messages: Messages; onBack: () => void }) {
  const error = useApp((state) => state.accountLoginError);
  return (
    <div className="uc-card uc-add-account">
      <div className="uc-login-head">
        <ProviderMark brand={provider} size={16} />
        <span className="uc-list-title uc-truncate">{brandName(provider)}</span>
        <Button variant="plain" onClick={onBack} className="is-small uc-key-change">
          {messages.accounts.changeService}
        </Button>
      </div>
      <Button variant="prominent" onClick={() => void startAccountLogin(provider)} className="is-small is-wide">
        {messages.accounts.signInWithGoogle}
      </Button>
      <p className="uc-settings-note is-flush">{messages.accounts.signInNote(brandName(provider))}</p>
      {error ? <p className="uc-settings-notice is-flush">{messages.accounts.loginFailed(brandName(error.provider), error.text)}</p> : null}
    </div>
  );
}

/**
 * Add an account of any provider from one list: Claude and Codex first (a Google sign-in), then
 * every other service that takes a key or reads an app's login on this computer.
 */
/** A provider the Add Account list offers: picking it opens its panel, its plus button connects it at once. */
interface AddOption {
  id: string;
  name: string;
  kind: string;
  quickLabel: string;
  quick: () => void;
}

function AddAccount({ messages, language }: { messages: Messages; language: Language }) {
  const services = useApp((state) => state.services);
  const login = useApp((state) => state.accountLogin);
  const [query, setQuery] = useState("");
  const [choice, setChoice] = useState<string | null>(null);
  const addable = useMemo(() => services.filter(isAddable).sort((a, b) => a.name.localeCompare(b.name)), [services]);
  if (login) return <LoginProgress messages={messages} />;
  const back = () => setChoice(null);
  const google = PROVIDERS.find((provider) => provider === choice);
  if (google) return <GoogleSignIn provider={google} messages={messages} onBack={back} />;
  const service = addable.find((candidate) => candidate.id === choice);
  if (service) {
    const saved = () => {
      back();
      setQuery("");
    };
    return connectsHere(service) ? (
      <ServicePanel key={service.id} service={service} messages={messages} language={language} onBack={back} onSaved={saved} />
    ) : (
      <ServiceAppNote service={service} messages={messages} onBack={back} />
    );
  }
  const options: AddOption[] = [
    ...PROVIDERS.filter((provider) => matchesProvider(brandName(provider), provider, query)).map((provider) => ({
      id: provider,
      name: brandName(provider),
      kind: messages.accounts.kindGoogle,
      quickLabel: messages.accounts.quickSignIn(brandName(provider), messages.accounts.kindGoogle),
      quick: () => {
        setChoice(provider);
        void startAccountLogin(provider);
      },
    })),
    ...addable
      .filter((candidate) => matchesProvider(candidate.name, candidate.id, query))
      .map((candidate) => {
        const method = candidate.signIn[0];
        const methodName = method === "github" ? messages.accounts.kindGitHub : messages.accounts.kindGoogle;
        return {
          id: candidate.id,
          name: candidate.name,
          kind: addKind(candidate, messages),
          quickLabel: method ? messages.accounts.quickSignIn(candidate.name, methodName) : messages.accounts.quickAdd(candidate.name),
          quick: () => {
            setChoice(candidate.id);
            if (method) void startAccountLogin(candidate.id, method);
          },
        };
      }),
  ];
  return (
    <div className="uc-card uc-add-account">
      <input
        className="uc-text-field"
        type="search"
        value={query}
        placeholder={messages.accounts.searchService}
        aria-label={messages.accounts.searchService}
        onChange={(event) => setQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && options[0]) setChoice(options[0].id);
        }}
      />
      {options.length === 0 ? (
        <p className="uc-insight-note">{messages.accounts.noServiceMatch}</p>
      ) : (
        <ul className="uc-service-results" aria-label={messages.accounts.add}>
          {options.map((option) => (
            <li key={option.id} className="uc-service-result">
              <button type="button" aria-label={option.name} className="uc-service-pick" onClick={() => setChoice(option.id)}>
                <ProviderMark brand={option.id} size={14} />
                <span className="uc-service-text">
                  <span className="uc-service-name uc-truncate" {...truncatedTooltipProps(option.name)}>
                    {option.name}
                  </span>
                  <span className="uc-service-kind uc-truncate" {...truncatedTooltipProps(option.kind)}>
                    {option.kind}
                  </span>
                </span>
              </button>
              <button type="button" className="uc-icon-button" aria-label={option.quickLabel} onClick={option.quick} {...tooltipProps(option.quickLabel)}>
                <PlusIcon size={14} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function Accounts() {
  const language = useLanguage();
  const messages = messagesFor(language);
  const accounts = useApp((state) => state.accounts);
  const engine = useApp((state) => state.engine);
  const services = useApp((state) => state.services);
  const serviceRows = useMemo(() => serviceCardRows(services), [services]);

  useEffect(() => {
    void reloadAccounts();
    void reloadServices();
  }, []);

  return (
    <div className="uc-stack">
      <section className="uc-group">
        <h2 className="uc-group-title">{messages.accounts.connected}</h2>
        <div className="uc-card uc-list-card">
          {accounts.length === 0 && serviceRows.length === 0 ? <p className="uc-card-empty">{messages.accounts.none}</p> : null}
          {accounts.map((account) => (
            <AccountRow key={account.id} account={account} runtime={engine?.providers[account.id]} messages={messages} language={language} />
          ))}
          {serviceRows.map((row) => (
            <ServiceRow key={row.id} row={row} runtime={engine?.providers[row.id]} messages={messages} language={language} />
          ))}
        </div>
      </section>

      <section className="uc-group">
        <h2 className="uc-group-title">{messages.accounts.add}</h2>
        <AddAccount messages={messages} language={language} />
        <p className="uc-group-note">{messages.accounts.cliNote}</p>
      </section>
    </div>
  );
}
