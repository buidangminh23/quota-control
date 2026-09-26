/**
 * The popup footer (upstream `PopoverFooter` + `HeaderView`): app identity over a live "Next update
 * in …" line that refreshes on click, a transient notice in its place, and the Options menu.
 */
import { useRef } from "react";
import { messagesFor, type Messages } from "@/i18n";
import { backend } from "@/lib/backend";
import type { EngineState, UpdateStatus } from "@/lib/types";
import { useLanguage, useNow } from "@/state/hooks";
import { checkForUpdates, installUpdate, navigate, refresh, useApp, type Screen } from "@/state/store";
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

/**
 * The tray menu's update entry (`Updates::menu_label` in `updates.rs`): Install once a check found a
 * release, Check otherwise, and nothing where the build cannot replace itself. The result of either
 * shows on the dashboard's update card; the entry waits while a check or install is running.
 */
export function updateEntry(status: UpdateStatus | null, messages: Messages): MenuEntry | null {
  if (!status?.supported) return null;
  const busy = status.phase === "checking" || status.phase === "downloading" || status.phase === "installing";
  const offer = status.available?.version;
  return offer
    ? { kind: "item", label: messages.chrome.installUpdate(offer), disabled: busy, onSelect: installUpdate }
    : { kind: "item", label: messages.chrome.checkForUpdates, disabled: busy, onSelect: checkForUpdates };
}

export function optionsEntries(messages: Messages, appName: string): MenuEntry[] {
  const update = updateEntry(useApp.getState().update, messages);
  return [
    { kind: "item", label: messages.chrome.customize, shortcut: "Enter", onSelect: () => toggle("customize") },
    { kind: "item", label: messages.chrome.settings, shortcut: "Ctrl+,", onSelect: () => toggle("settings") },
    { kind: "item", label: messages.chrome.accounts, onSelect: () => toggle("accounts") },
    { kind: "separator" },
    ...(update ? [update, { kind: "separator" as const }] : []),
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
