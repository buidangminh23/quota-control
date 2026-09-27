/**
 * The Accounts screen's other AI providers: the cards of services beyond Claude and Codex (logins
 * other apps keep on this computer, keys in environment variables, keys saved here), and the panels
 * the Add Account picker opens for them: a key or cookie form, or a note for a service that reads
 * another app's login. The core reads every login and key; the popup only sees a saved key's last
 * four characters.
 */
import { useMemo, useState } from "react";
import { translate, type Language, type Messages } from "@/i18n";
import { backend } from "@/lib/backend";
import type { ProviderRuntimeState, ServiceEntry } from "@/lib/types";
import { headerNotice } from "@/model/providerText";
import { reloadServices, showNotice, useApp } from "@/state/store";
import { Button } from "../ui/controls";
import { confirmAction } from "../ui/dialog";
import { CloseIcon, Spinner } from "../ui/icons";
import { ProviderMark } from "../ui/ProviderMark";
import { tooltipProps, truncatedTooltipProps } from "../ui/tooltip";
import { errorText, statusOf } from "./status";

interface ServiceCardRow {
  id: string;
  service: ServiceEntry;
  title: string;
  source: "login" | "env" | "key";
  detail: string;
  /** Only keys saved here can be removed here; logins and environment keys belong to their owners. */
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
      rows.push({ id: key.id, service, title: titleOf(service, key.label), source: "key", detail: key.hint, removable: true });
    }
  }
  return rows.sort((a, b) => a.title.localeCompare(b.title));
}

function ServiceRow({ row, runtime, messages, language }: { row: ServiceCardRow; runtime: ProviderRuntimeState | undefined; messages: Messages; language: Language }) {
  const status = statusOf(runtime);
  const notice = status === "error" ? headerNotice(runtime, language) : null;
  const remove = async () => {
    const confirmed = await confirmAction({
      title: messages.accounts.removeKeyTitle(row.title),
      message: messages.accounts.removeKeyMessage,
      confirmLabel: messages.accounts.removeKeyConfirm,
      cancelLabel: messages.accounts.cancel,
    });
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

/** The services' cards, or a line saying there are none yet. */
export function ServiceCards({ messages, language }: { messages: Messages; language: Language }) {
  const services = useApp((state) => state.services);
  const engine = useApp((state) => state.engine);
  const rows = useMemo(() => serviceCardRows(services), [services]);
  return (
    <div className="uc-card uc-list-card">
      {rows.length === 0 ? <p className="uc-card-empty">{messages.accounts.otherServicesNone}</p> : null}
      {rows.map((row) => (
        <ServiceRow key={row.id} row={row} runtime={engine?.providers[row.id]} messages={messages} language={language} />
      ))}
    </div>
  );
}

/** The apps whose logins on this computer become cards by themselves, for the note under the list. */
export function detectedApps(services: readonly ServiceEntry[]): string[] {
  return [...new Set(services.flatMap((service) => (service.loginFrom ? [service.loginFrom] : [])))].sort((a, b) => a.localeCompare(b));
}

/** Whether `query` finds a provider by its name or id. */
export function matchesProvider(name: string, id: string, query: string): boolean {
  const needle = query.trim().toLowerCase();
  return needle === "" || name.toLowerCase().includes(needle) || id.includes(needle);
}

/** A service that can be added from this screen: one that takes a key, or reads an app's login here. */
export function isAddable(service: ServiceEntry): boolean {
  return service.takesApiKey || service.loginFrom !== null;
}

/** How a service is added, as the picker lists it: API key, cookie, or the app whose login it reads. */
export function addKind(service: ServiceEntry, messages: Messages): string {
  if (!service.takesApiKey) return service.loginFrom ?? "";
  return service.keyFormat === "token" ? messages.accounts.kindApiKey : messages.accounts.kindCookie;
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

/** Paste a service's API key or session cookie (and what it asks beside it), then save it. */
export function ServiceKeyForm({
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
    <div className="uc-card uc-add-account">
      <PanelHead brand={service.id} name={service.name} messages={messages} onBack={onBack} />
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
      {service.loginFrom ? <p className="uc-settings-note is-flush">{messages.accounts.alsoAppLogin(service.loginFrom)}</p> : null}
    </div>
  );
}
