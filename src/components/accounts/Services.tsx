/**
 * The Accounts screen's other AI providers: the rows of services beyond Claude and Codex (logins
 * other apps keep on this computer, keys in environment variables, keys saved here, accounts signed
 * in to here) that the connected list shows after Claude and Codex, and the panel the Add Account picker opens for a service: every way it connects (a
 * Google or GitHub sign-in in the browser, a key or cookie), or a note for a service that only reads
 * another app's login. The core reads every login, key and token; the popup only sees a saved key's
 * last four characters.
 */
import { useState } from "react";
import { translate, type Language, type Messages } from "@/i18n";
import { backend } from "@/lib/backend";
import type { ProviderRuntimeState, ServiceEntry, SignInMethod } from "@/lib/types";
import { headerNotice } from "@/model/providerText";
import { reloadServices, showNotice, startAccountLogin, useApp } from "@/state/store";
import { Button } from "../ui/controls";
import { confirmAction } from "../ui/dialog";
import { CloseIcon, Spinner } from "../ui/icons";
import { ProviderMark } from "../ui/ProviderMark";
import { tooltipProps, truncatedTooltipProps } from "../ui/tooltip";
import { errorText, statusOf } from "./status";

export interface ServiceCardRow {
  id: string;
  service: ServiceEntry;
  title: string;
  /** An app's login, an environment key, a saved key, or an account signed in to here with Google or GitHub. */
  source: "login" | "env" | "key" | SignInMethod;
  detail: string;
  /** Only keys and sign-ins saved here can be removed here; logins and environment keys belong to their owners. */
  removable: boolean;
}

function titleOf(service: ServiceEntry, label: string | null): string {
  const trimmed = label?.trim() ?? "";
  return trimmed === "" || trimmed === service.name ? service.name : `${service.name} · ${trimmed}`;
}

/** Every card the services have, grouped by provider name. */
export function serviceCardRows(services: readonly ServiceEntry[]): ServiceCardRow[] {
  const rows: ServiceCardRow[] = [];
  for (const service of services) {
    for (const found of service.detected) {
      const env = service.keyEnv.includes(found.origin);
      rows.push({ id: found.id, service, title: titleOf(service, found.label), source: env ? "env" : "login", detail: found.origin, removable: false });
    }
    for (const key of service.keys) {
      rows.push({ id: key.id, service, title: titleOf(service, key.label), source: key.signIn ?? "key", detail: key.hint, removable: true });
    }
  }
  return rows.sort((a, b) => a.title.localeCompare(b.title));
}

export function ServiceRow({ row, runtime, messages, language }: { row: ServiceCardRow; runtime: ProviderRuntimeState | undefined; messages: Messages; language: Language }) {
  const status = statusOf(runtime);
  const notice = status === "error" ? headerNotice(runtime, language) : null;
  const signedIn = row.source === "google" || row.source === "github";
  const remove = async () => {
    const confirmed = await confirmAction(
      signedIn
        ? {
            title: messages.accounts.removeTitle(row.title),
            message: messages.accounts.removeMessage,
            confirmLabel: messages.accounts.removeConfirm,
            cancelLabel: messages.accounts.cancel,
          }
        : {
            title: messages.accounts.removeKeyTitle(row.title),
            message: messages.accounts.removeKeyMessage,
            confirmLabel: messages.accounts.removeKeyConfirm,
            cancelLabel: messages.accounts.cancel,
          },
    );
    if (!confirmed) return;
    try {
      await backend().removeApiKey(row.id);
      await reloadServices();
    } catch (error) {
      showNotice(messages.accounts.failed(errorText(error, language)), "notice");
    }
  };
  return (
    <div className="uc-list-row is-account">
      <span className="uc-list-mark">
        <ProviderMark brand={row.service.id} size={18} />
      </span>
      <span className="uc-list-text">
        <span className="uc-list-title uc-truncate" {...truncatedTooltipProps(row.title)}>
          {row.title}
        </span>
        <span className="uc-list-subtitle uc-truncate">{messages.accounts.serviceSource(row.source, row.detail)}</span>
        <span className={`uc-account-status is-${status}`} {...tooltipProps(notice)}>
          {status === "refreshing" ? <Spinner size={9} /> : <span className="uc-status-dot" />}
          {row.service.startsHidden && !runtime ? messages.accounts.hiddenNote : messages.accounts.status(status)}
        </span>
      </span>
      {row.removable ? (
        <button type="button" className="uc-icon-button" aria-label={`${messages.accounts.remove} ${row.title}`} onClick={() => void remove()} {...tooltipProps(messages.accounts.remove)}>
          <CloseIcon size={11} />
        </button>
      ) : null}
    </div>
  );
}

/** Whether `query` finds a provider by its name or id. */
export function matchesProvider(name: string, id: string, query: string): boolean {
  const needle = query.trim().toLowerCase();
  return needle === "" || name.toLowerCase().includes(needle) || id.includes(needle);
}

/** A service that can be added from this screen: one with a browser sign-in, one that takes a key, or one that reads an app's login here. */
export function isAddable(service: ServiceEntry): boolean {
  return service.signIn.length > 0 || service.takesApiKey || service.loginFrom !== null;
}

/** A way to connect a service from its panel: a browser sign-in, or its key or cookie. */
type Way = SignInMethod | "key";

function waysOf(service: ServiceEntry): Way[] {
  return [...service.signIn, ...(service.takesApiKey ? (["key"] as const) : [])];
}

function wayLabel(way: Way, service: ServiceEntry, messages: Messages): string {
  if (way === "google") return messages.accounts.kindGoogle;
  if (way === "github") return messages.accounts.kindGitHub;
  return service.keyFormat === "token" ? messages.accounts.kindApiKey : messages.accounts.kindCookie;
}

/** How a service is added, as the picker lists it: its sign-ins and its key or cookie, else the app whose login it reads. */
export function addKind(service: ServiceEntry, messages: Messages): string {
  const ways = waysOf(service);
  if (ways.length === 0) return service.loginFrom ?? "";
  return ways.map((way) => wayLabel(way, service, messages)).join(" · ");
}

/** Whether the picker opens a panel to connect the service here, rather than a note about its app. */
export function connectsHere(service: ServiceEntry): boolean {
  return waysOf(service).length > 0;
}

function PanelHead({ brand, name, messages, onBack }: { brand: string; name: string; messages: Messages; onBack: () => void }) {
  return (
    <div className="uc-login-head">
      <ProviderMark brand={brand} size={16} />
      <span className="uc-list-title uc-truncate">{name}</span>
      <Button variant="plain" onClick={onBack} className="is-small uc-key-change">
        {messages.accounts.changeService}
      </Button>
    </div>
  );
}

/** A service that only reads another app's login on this computer: signing in to that app adds it. */
export function ServiceAppNote({ service, messages, onBack }: { service: ServiceEntry; messages: Messages; onBack: () => void }) {
  return (
    <div className="uc-card uc-add-account">
      <PanelHead brand={service.id} name={service.name} messages={messages} onBack={onBack} />
      <p className="uc-settings-note is-flush">{messages.accounts.appLoginNote(service.name, service.loginFrom ?? service.name)}</p>
    </div>
  );
}

/**
 * Every way a service connects from this screen: its browser sign-ins (Google, GitHub) and its key or
 * cookie, picked one at a time when there are several.
 */
export function ServicePanel({
  service,
  messages,
  language,
  onBack,
  onSaved,
}: {
  service: ServiceEntry;
  messages: Messages;
  language: Language;
  onBack: () => void;
  onSaved: () => void;
}) {
  const ways = waysOf(service);
  const [way, setWay] = useState<Way>(ways[0] ?? "key");
  return (
    <div className="uc-card uc-add-account">
      <PanelHead brand={service.id} name={service.name} messages={messages} onBack={onBack} />
      {ways.length > 1 ? (
        <div className="uc-capsule-picker" role="radiogroup" aria-label={messages.accounts.methodsLabel}>
          {ways.map((candidate) => (
            <button
              key={candidate}
              type="button"
              role="radio"
              aria-checked={candidate === way}
              className={`uc-capsule-segment${candidate === way ? " is-selected" : ""}`}
              onClick={() => setWay(candidate)}
            >
              {wayLabel(candidate, service, messages)}
            </button>
          ))}
        </div>
      ) : null}
      {way === "key" ? (
        <ServiceKeyFields service={service} messages={messages} language={language} onSaved={onSaved} />
      ) : (
        <ServiceSignIn service={service} method={way} messages={messages} />
      )}
      {service.loginFrom ? <p className="uc-settings-note is-flush">{messages.accounts.alsoAppLogin(service.loginFrom)}</p> : null}
    </div>
  );
}

/** Sign in to a service's account in the browser; the core saves it and the popup comes back. */
function ServiceSignIn({ service, method, messages }: { service: ServiceEntry; method: SignInMethod; messages: Messages }) {
  const error = useApp((state) => state.accountLoginError);
  return (
    <>
      <Button variant="prominent" onClick={() => void startAccountLogin(service.id, method)} className="is-small is-wide">
        {method === "google" ? messages.accounts.signInWithGoogle : messages.accounts.signInWithGitHub}
      </Button>
      <p className="uc-settings-note is-flush">{messages.accounts.serviceSignInNote(service.name, method)}</p>
      {error?.provider === service.id ? <p className="uc-settings-notice is-flush">{messages.accounts.loginFailed(service.name, error.text)}</p> : null}
    </>
  );
}

/** Paste a service's API key or session cookie (and what it asks beside it), then save it. */
function ServiceKeyFields({ service, messages, language, onSaved }: { service: ServiceEntry; messages: Messages; language: Language; onSaved: () => void }) {
  const [key, setKey] = useState("");
  const [label, setLabel] = useState("");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);

  const save = async () => {
    if (key.trim() === "" || saving) return;
    setSaving(true);
    try {
      const filled = Object.fromEntries(Object.entries(fields).filter(([, value]) => value.trim() !== ""));
      await backend().addApiKey(service.id, key.trim(), label.trim() || undefined, filled);
      await reloadServices();
      showNotice(messages.accounts.keySaved(service.name), "positive");
      onSaved();
    } catch (error) {
      showNotice(messages.accounts.failed(errorText(error, language)), "notice");
    } finally {
      setSaving(false);
    }
  };

  const keyPlaceholder = service.keyLabel === "API key" ? messages.accounts.keyPlaceholder : messages.accounts.pasteValue(translate(service.keyLabel, language));
  return (
    <>
      <input
        className="uc-text-field"
        type="password"
        value={key}
        placeholder={keyPlaceholder}
        aria-label={keyPlaceholder}
        autoComplete="off"
        spellCheck={false}
        autoFocus
        onChange={(event) => setKey(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") void save();
        }}
      />
      {service.keyFields.map(([field, fieldLabel]) => (
        <input
          key={field}
          className="uc-text-field"
          type="text"
          value={fields[field] ?? ""}
          placeholder={translate(fieldLabel, language)}
          aria-label={translate(fieldLabel, language)}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => setFields({ ...fields, [field]: event.target.value })}
        />
      ))}
      <input
        className="uc-text-field"
        type="text"
        value={label}
        placeholder={messages.accounts.keyLabel}
        aria-label={messages.accounts.keyLabel}
        maxLength={80}
        onChange={(event) => setLabel(event.target.value)}
      />
      <div className="uc-settings-actions is-split">
        {service.keyUrl ? (
          <Button onClick={() => void backend().openUrl(service.keyUrl!)} className="is-small">
            {service.keyLabel === "API key" ? messages.accounts.getKey : messages.accounts.openServicePage(service.name)}
          </Button>
        ) : (
          <span />
        )}
        <Button variant="prominent" onClick={() => void save()} disabled={key.trim() === "" || saving} className="is-small">
          {saving ? <Spinner size={10} /> : null}
          <span>{messages.accounts.saveKey}</span>
        </Button>
      </div>
      {service.keyFormat === "cookie" ? <p className="uc-settings-note is-flush">{messages.accounts.cookieNote}</p> : null}
      {service.keyFormat === "cookieHeader" ? <p className="uc-settings-note is-flush">{messages.accounts.cookieHeaderNote}</p> : null}
      <p className="uc-settings-note is-flush">{messages.accounts.keyStoredNote}</p>
      {service.keyEnv.length > 0 ? <p className="uc-settings-note is-flush">{messages.accounts.keyEnvNote(service.keyEnv.join(", "))}</p> : null}
    </>
  );
}
