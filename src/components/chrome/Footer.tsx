/**
 * The popup footer (upstream `PopoverFooter` + `HeaderView`): app identity over a live "Next update
 * in …" line that refreshes on click, a transient notice in its place, and the Options menu.
 */
import { useRef } from "react";
import { messagesFor, type Messages } from "@/i18n";
import { backend } from "@/lib/backend";
import type { EngineState } from "@/lib/types";
import { displayGroups } from "@/model/layout";
import { providerTitle } from "@/model/providerText";
import { copyScreenshot } from "@/share/screenshot";
import { useLanguage, useNow } from "@/state/hooks";
import { navigate, refresh, showNotice, useApp, type Screen } from "@/state/store";
import { Pill } from "../ui/controls";
import { openAboutDialog } from "./about";
import { ChevronDown, Spinner } from "../ui/icons";
import { openMenuAt, type MenuEntry } from "../ui/menu";
import { tooltipProps } from "../ui/tooltip";

export function isEngineUpdating(engine: EngineState | null): boolean {
  return Object.values(engine?.providers ?? {}).some((runtime) => runtime.refreshing);
}

/** `Next update in 4m` / `Cập nhật sau 4 phút`, or `Updating…` while any provider refreshes. */
export function nextUpdateText(engine: EngineState | null, now: Date, messages: Messages): string {
  if (isEngineUpdating(engine)) return messages.chrome.updating;
  const interval = engine?.refreshIntervalMs ?? 300_000;
  const base = engine?.lastRefreshAt ? new Date(engine.lastRefreshAt).getTime() : now.getTime();
  const remaining = Math.max(0, base + interval - now.getTime());
  const seconds = Math.ceil(remaining / 1000);
  if (seconds >= 60) return messages.chrome.nextUpdateMinutes(Math.ceil(seconds / 60));
  return messages.chrome.nextUpdateSeconds(seconds);
}

function NextUpdate({ messages }: { messages: Messages }) {
  const engine = useApp((state) => state.engine);
  const now = useNow(1000);
  const updating = isEngineUpdating(engine);
  return (
    <button type="button" className="uc-footer-update uc-num" disabled={updating} onClick={() => refresh()} {...tooltipProps(messages.chrome.refreshNow)}>
      <span>{nextUpdateText(engine, now, messages)}</span>
      {updating ? <Spinner size={9} /> : null}
    </button>
  );
}

function toggle(screen: Screen): void {
  navigate(useApp.getState().screen === screen ? "dashboard" : screen);
}

function shareEntries(messages: Messages): MenuEntry[] {
  const state = useApp.getState();
  const enabled = state.enabledProviders;
  const groups = displayGroups(state.layout, state.catalog, (id) => enabled === null || enabled.includes(id));
  if (groups.length === 0) return [{ kind: "item", label: messages.chrome.noEnabledProviders, disabled: true, onSelect: () => {} }];
  const language = state.settings.language;
  return groups.map((group) => ({
    kind: "item" as const,
    label: providerTitle(group.provider, language),
    onSelect: () => {
      const element = document.querySelector<HTMLElement>(`[data-provider-section="${CSS.escape(group.provider.id)}"]`);
      void copyScreenshot(element).then((copied) =>
        showNotice(copied ? messages.chrome.copiedToClipboard : messages.chrome.copyFailed, copied ? "positive" : "notice"),
      );
    },
  }));
}

export function optionsEntries(messages: Messages, appName: string): MenuEntry[] {
  return [
    { kind: "item", label: messages.chrome.customize, shortcut: "Enter", onSelect: () => toggle("customize") },
    { kind: "item", label: messages.chrome.settings, shortcut: "Ctrl+,", onSelect: () => toggle("settings") },
    { kind: "item", label: messages.chrome.accounts, onSelect: () => toggle("accounts") },
    { kind: "separator" },
    { kind: "submenu", label: messages.chrome.shareScreenshot, entries: shareEntries(messages) },
    { kind: "separator" },
    { kind: "item", label: messages.chrome.about(appName), onSelect: () => openAboutDialog() },
    { kind: "item", label: messages.chrome.quit(appName), shortcut: "Ctrl+Q", destructive: true, onSelect: () => void backend().quit() },
  ];
}

export function Footer({ screen }: { screen: Screen }) {
  const language = useLanguage();
  const messages = messagesFor(language);
  const info = useApp((state) => state.info);
  const notice = useApp((state) => state.notice);
  const optionsRef = useRef<HTMLButtonElement>(null);
  const name = info?.name ?? messages.chrome.appName;
  return (
    <footer className="uc-footer">
      {notice?.tone === "positive" ? (
        <div className="uc-footer-pill" key={notice.id}>
          <Pill text={notice.text} tone="positive" />
        </div>
      ) : null}
      <div className="uc-footer-identity">
        <span>{messages.chrome.identity(name, info?.version ?? "")}</span>
        {notice?.tone === "notice" ? (
          <span key={notice.id} className="uc-footer-notice" role="status">
            {notice.text}
          </span>
        ) : (
          <NextUpdate messages={messages} />
        )}
      </div>
      {screen === "dashboard" ? (
        <button
          ref={optionsRef}
          type="button"
          className="uc-options-button"
          aria-haspopup="menu"
          onClick={() => optionsRef.current && openMenuAt(optionsRef.current, optionsEntries(messages, name), { placement: "above", align: "end" })}
        >
          <span>{messages.chrome.options}</span>
          <ChevronDown size={10} />
        </button>
      ) : null}
    </footer>
  );
}
