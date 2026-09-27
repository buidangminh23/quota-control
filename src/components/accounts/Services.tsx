/**
 * The Accounts screen's other AI providers: the cards of services beyond Claude and Codex (logins
 * other apps keep on this computer, keys in environment variables, keys saved here) and adding a
 * service with an API key. The core reads every login and key; the popup only sees a saved key's
 * last four characters.
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

function matches(service: ServiceEntry, query: string): boolean {
  const needle = query.trim().toLowerCase();
  return needle === "" || service.name.toLowerCase().includes(needle) || service.id.includes(needle);
}

/** Pick a service that takes an API key, then paste the key (and what the service asks beside it). */
export function AddServiceKey({ messages, language }: { messages: Messages; language: Language }) {
  const services = useApp((state) => state.services);
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [key, setKey] = useState("");
  const [label, setLabel] = useState("");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const keyed = useMemo(() => services.filter((service) => service.takesApiKey).sort((a, b) => a.name.localeCompare(b.name)), [services]);
  const selected = keyed.find((service) => service.id === selectedId) ?? null;

  const choose = (id: string | null) => {
    setSelectedId(id);
    setKey("");
    setLabel("");
    setFields({});
  };

  const save = async () => {
    if (!selected || key.trim() === "" || saving) return;
    setSaving(true);
    try {
      const filled = Object.fromEntries(Object.entries(fields).filter(([, value]) => value.trim() !== ""));
      await backend().addApiKey(selected.id, key.trim(), label.trim() || undefined, filled);
      await reloadServices();
      showNotice(messages.accounts.keySaved(selected.name), "positive");
      choose(null);
      setQuery("");
    } catch (error) {
      showNotice(messages.accounts.failed(errorText(error, language)), "notice");
    } finally {
      setSaving(false);
    }
  };

  if (!selected) {
    const found = keyed.filter((service) => matches(service, query));
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
            if (event.key === "Enter" && found[0]) choose(found[0].id);
          }}
        />
        <div className="uc-service-results" role="listbox" aria-label={messages.accounts.addKey}>
          {found.length === 0 ? <p className="uc-insight-note">{messages.accounts.noServiceMatch}</p> : null}
          {found.map((service) => (
            <button key={service.id} type="button" role="option" aria-selected={false} className="uc-service-result" onClick={() => choose(service.id)}>
              <ProviderMark brand={service.id} size={14} />
              <span className="uc-truncate">{service.name}</span>
            </button>
          ))}
        </div>
      </div>
    );
  }

  const keyPlaceholder = selected.keyLabel === "API key" ? messages.accounts.keyPlaceholder : messages.accounts.pasteValue(translate(selected.keyLabel, language));
  return (
    <div className="uc-card uc-add-account">
      <div className="uc-login-head">
        <ProviderMark brand={selected.id} size={16} />
        <span className="uc-list-title uc-truncate">{selected.name}</span>
        <Button variant="plain" onClick={() => choose(null)} className="is-small uc-key-change">
          {messages.accounts.changeService}
        </Button>
      </div>
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
      {selected.keyFields.map(([field, fieldLabel]) => (
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
        {selected.keyUrl ? (
          <Button onClick={() => void backend().openUrl(selected.keyUrl!)} className="is-small">
            {selected.keyLabel === "API key" ? messages.accounts.getKey : messages.accounts.openServicePage(selected.name)}
          </Button>
        ) : (
          <span />
        )}
        <Button variant="prominent" onClick={() => void save()} disabled={key.trim() === "" || saving} className="is-small">
          {saving ? <Spinner size={10} /> : null}
          <span>{messages.accounts.saveKey}</span>
        </Button>
      </div>
      {selected.keyFormat === "cookie" ? <p className="uc-settings-note is-flush">{messages.accounts.cookieNote}</p> : null}
      {selected.keyFormat === "cookieHeader" ? <p className="uc-settings-note is-flush">{messages.accounts.cookieHeaderNote}</p> : null}
      <p className="uc-settings-note is-flush">{messages.accounts.keyStoredNote}</p>
      {selected.keyEnv.length > 0 ? <p className="uc-settings-note is-flush">{messages.accounts.keyEnvNote(selected.keyEnv.join(", "))}</p> : null}
    </div>
  );
}
