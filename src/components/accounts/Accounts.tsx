/**
 * The Accounts screen: every connected Claude and Codex account (always all of them, each with its
 * own live status), adding one through the browser or from this computer's CLI login, and the saved
 * in-app chat sessions that open the official Claude / ChatGPT sites in their own windows.
 */
import { useEffect, useRef, useState } from "react";
import { messagesFor, translate, type Language, type Messages } from "@/i18n";
import { backend } from "@/lib/backend";
import type { AccountLogin, AccountProvider, ChatSession, ConnectedAccount, ProviderRuntimeState } from "@/lib/types";
import { brandName, headerNotice } from "@/model/providerText";
import { useLanguage } from "@/state/hooks";
import { openChatFor, reloadAccounts, reloadChats, showNotice, useApp } from "@/state/store";
import { Button } from "../ui/controls";
import { confirmAction } from "../ui/dialog";
import { ChatIcon, CloseIcon, PlusIcon, Spinner } from "../ui/icons";
import { ProviderMark } from "../ui/ProviderMark";
import { tooltipProps } from "../ui/tooltip";

const PROVIDERS: readonly AccountProvider[] = ["claude", "codex"];
const CHAT_PRODUCTS: Record<AccountProvider, string> = { claude: "Claude", codex: "ChatGPT" };

function errorText(error: unknown, language: Language): string {
  const raw = error instanceof Error ? error.message : typeof error === "string" ? error : String(error);
  return translate(raw, language);
}

/** `Claude · Công ty`, or just the brand when the label is the default CLI name. */
function accountTitle(provider: AccountProvider, label: string): string {
  const trimmed = label.trim();
  return trimmed === "" || trimmed.toLowerCase() === provider ? brandName(provider) : `${brandName(provider)} · ${trimmed}`;
}

type Status = "ok" | "refreshing" | "error" | "unknown";

function statusOf(runtime: ProviderRuntimeState | undefined): Status {
  if (!runtime) return "unknown";
  if (runtime.refreshing) return "refreshing";
  if (runtime.error || runtime.snapshot?.errorCategory) return "error";
  return runtime.snapshot ? "ok" : "unknown";
}

function formatDate(iso: string, language: Language): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleDateString(language === "vi" ? "vi-VN" : "en-US");
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
        <span className="uc-list-title uc-truncate">{title}</span>
        <span className="uc-list-subtitle uc-truncate">{messages.accounts.mode(account.credentialMode)}</span>
        <span className={`uc-account-status is-${status}`} {...tooltipProps(notice)}>
          {status === "refreshing" ? <Spinner size={9} /> : <span className="uc-status-dot" />}
          {messages.accounts.status(status)}
        </span>
      </span>
      <button type="button" className="uc-icon-button" aria-label={messages.dashboard.openChat(CHAT_PRODUCTS[account.provider])} onClick={() => void chat()} {...tooltipProps(messages.dashboard.openChat(CHAT_PRODUCTS[account.provider]))}>
        <ChatIcon size={13} />
      </button>
      <button type="button" className="uc-icon-button" aria-label={`${messages.accounts.remove} ${title}`} onClick={() => void remove()} {...tooltipProps(messages.accounts.remove)}>
        <CloseIcon size={11} />
      </button>
    </div>
  );
}

type Flow = { kind: "idle" } | { kind: "busy" } | { kind: "login"; login: AccountLogin; provider: AccountProvider };

function AddAccount({ messages, language }: { messages: Messages; language: Language }) {
  const [provider, setProvider] = useState<AccountProvider>("claude");
  const [label, setLabel] = useState("");
  const [code, setCode] = useState("");
  const [flow, setFlow] = useState<Flow>({ kind: "idle" });
  const [error, setError] = useState<string | null>(null);
  const pending = useRef<string | null>(null);
  const brand = brandName(provider);

  useEffect(
    () => () => {
      if (pending.current) void backend().cancelAccountLogin(pending.current).catch(() => undefined);
    },
    [],
  );

  const labelArgument = () => label.trim() || undefined;

  const finished = async (account: ConnectedAccount) => {
    pending.current = null;
    setFlow({ kind: "idle" });
    setLabel("");
    setCode("");
    await reloadAccounts();
    showNotice(messages.accounts.added(accountTitle(account.provider, account.label)), "positive");
  };

  const failed = (reason: unknown) => {
    const flowId = pending.current;
    pending.current = null;
    if (flowId) void backend().cancelAccountLogin(flowId).catch(() => undefined);
    setFlow({ kind: "idle" });
    setError(messages.accounts.failed(errorText(reason, language)));
  };

  const importCurrent = async () => {
    setError(null);
    setFlow({ kind: "busy" });
    try {
      await finished(await backend().importCurrentAccount(provider, labelArgument()));
    } catch (reason) {
      failed(reason);
    }
  };

  const signIn = async () => {
    setError(null);
    setFlow({ kind: "busy" });
    let flowId: string | null = null;
    try {
      const login = await backend().beginAccountLogin(provider, labelArgument());
      flowId = login.flowId;
      pending.current = flowId;
      setFlow({ kind: "login", login, provider });
      await backend().openUrl(login.authorizationUrl);
      if (login.callbackMode === "loopback") await finished(await backend().completeAccountLogin(login.flowId));
    } catch (reason) {
      if (flowId !== null && pending.current !== flowId) return;
      failed(reason);
    }
  };

  const complete = async () => {
    if (flow.kind !== "login") return;
    setError(null);
    const flowId = flow.login.flowId;
    setFlow({ kind: "busy" });
    try {
      const account = await backend().completeAccountLogin(flowId, code.trim());
      await finished(account);
    } catch (reason) {
      pending.current = null;
      failed(reason);
    }
  };

  const cancel = () => {
    const flowId = pending.current;
    pending.current = null;
    setFlow({ kind: "idle" });
    setCode("");
    if (flowId) void backend().cancelAccountLogin(flowId).catch(() => undefined);
  };

  const busy = flow.kind === "busy";
  return (
    <div className="uc-card uc-add-account">
      {flow.kind === "login" ? (
        <div className="uc-login-flow">
          <div className="uc-login-head">
            <ProviderMark brand={flow.provider} size={16} />
            <span className="uc-list-title">{messages.accounts.signIn(brandName(flow.provider))}</span>
          </div>
          {flow.login.callbackMode === "manual" ? (
            <>
              <p className="uc-settings-note is-flush">{messages.accounts.pasteCode}</p>
              <input
                className="uc-text-field"
                value={code}
                placeholder={messages.accounts.codePlaceholder}
                onChange={(event) => setCode(event.target.value)}
                onKeyDown={(event) => event.key === "Enter" && code.trim() && void complete()}
                autoFocus
                spellCheck={false}
                autoComplete="off"
                aria-label={messages.accounts.codePlaceholder}
              />
            </>
          ) : (
            <p className="uc-login-waiting">
              <Spinner size={11} />
              <span>{messages.accounts.waitingForBrowser}</span>
            </p>
          )}
          <Button onClick={() => void backend().openUrl(flow.login.authorizationUrl)} className="is-small is-wide">
            {messages.accounts.openSignInPage}
          </Button>
          <div className="uc-settings-actions is-split">
            <Button onClick={cancel} className="is-small">
              {messages.accounts.cancel}
            </Button>
            {flow.login.callbackMode === "manual" ? (
              <Button variant="prominent" onClick={() => void complete()} disabled={!code.trim()} className="is-small">
                {messages.accounts.complete}
              </Button>
            ) : null}
          </div>
        </div>
      ) : (
        <>
          <div className="uc-capsule-picker" role="radiogroup" aria-label={messages.accounts.add}>
            {PROVIDERS.map((candidate) => (
              <button
                key={candidate}
                type="button"
                role="radio"
                aria-checked={candidate === provider}
                className={`uc-capsule-segment${candidate === provider ? " is-selected" : ""}`}
                onClick={() => setProvider(candidate)}
                disabled={busy}
              >
                {brandName(candidate)}
              </button>
            ))}
          </div>
          <input
            className="uc-text-field"
            value={label}
            placeholder={messages.accounts.labelPlaceholder}
            onChange={(event) => setLabel(event.target.value)}
            maxLength={60}
            disabled={busy}
            spellCheck={false}
            aria-label={messages.accounts.labelPlaceholder}
          />
          <div className="uc-settings-actions is-stacked">
            <Button variant="prominent" onClick={() => void signIn()} disabled={busy} className="is-small is-wide">
              {busy ? <Spinner size={10} /> : null}
              <span>{messages.accounts.signIn(brand)}</span>
            </Button>
            <Button onClick={() => void importCurrent()} disabled={busy} className="is-small is-wide">
              {messages.accounts.importCurrent(brand)}
            </Button>
          </div>
        </>
      )}
      {error ? <p className="uc-settings-notice is-flush">{error}</p> : null}
    </div>
  );
}

function ChatRow({ chat, messages, language }: { chat: ChatSession; messages: Messages; language: Language }) {
  const open = () => backend().openChatSession(chat.id).catch(() => showNotice(messages.accounts.chatOpenFailed, "notice"));
  return (
    <div className="uc-list-row is-account">
      <span className="uc-list-mark">
        <ProviderMark brand={chat.provider} size={16} />
      </span>
      <span className="uc-list-text">
        <span className="uc-list-title uc-truncate">
          {chat.label.trim().toLowerCase() === chat.provider ? CHAT_PRODUCTS[chat.provider] : `${CHAT_PRODUCTS[chat.provider]} · ${chat.label}`}
        </span>
        <span className="uc-list-subtitle">{messages.accounts.createdOn(formatDate(chat.createdAt, language))}</span>
      </span>
      <Button onClick={() => void open()} className="is-small">
        {messages.accounts.open}
      </Button>
    </div>
  );
}

export function Accounts() {
  const language = useLanguage();
  const messages = messagesFor(language);
  const accounts = useApp((state) => state.accounts);
  const chats = useApp((state) => state.chats);
  const engine = useApp((state) => state.engine);

  useEffect(() => {
    void reloadAccounts();
    void reloadChats();
  }, []);

  const newChat = (provider: AccountProvider) =>
    backend()
      .createChatSession(provider)
      .then(() => reloadChats())
      .catch(() => showNotice(messages.accounts.chatOpenFailed, "notice"));

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
        <AddAccount messages={messages} language={language} />
      </section>

      <section className="uc-group">
        <h2 className="uc-group-title">{messages.accounts.chats}</h2>
        <div className="uc-card uc-list-card">
          {chats.length === 0 ? <p className="uc-card-empty">{messages.accounts.noChats}</p> : null}
          {chats.map((chat) => (
            <ChatRow key={chat.id} chat={chat} messages={messages} language={language} />
          ))}
          <div className="uc-settings-actions is-column">
            {PROVIDERS.map((provider) => (
              <Button key={provider} onClick={() => void newChat(provider)} className="is-small is-wide">
                <PlusIcon size={10} />
                <span>{messages.accounts.newChat(CHAT_PRODUCTS[provider])}</span>
              </Button>
            ))}
          </div>
        </div>
        <p className="uc-group-note">{messages.accounts.chatsNote}</p>
        <p className="uc-group-note">{messages.accounts.chatsAuthNote}</p>
      </section>
    </div>
  );
}
