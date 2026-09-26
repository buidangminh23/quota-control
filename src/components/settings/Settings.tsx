/**
 * The Settings screen (upstream `SettingsScreen`): Customize-style cards of rows for language, total
 * spend, launch at login and the global shortcut; appearance; how usage reads; the taskbar strip;
 * pace notifications; the `usagectl` terminal helper; and the log file plus a full reset. Changes
 * apply live and persist to the shared settings document.
 */
import { useEffect, useState, type ReactNode } from "react";
import { LANGUAGES, messagesFor, type Language } from "@/i18n";
import type { NotificationKey, SettingsSectionKey } from "@/i18n/messages";
import { anyNotificationEnabled, type NotificationSettings } from "@/model/settings";
import { autostartEnabled, canRevealFiles, notificationAccess, requestNotificationAccess, revealFile, setAutostart, type NotificationAccess } from "@/platform/system";
import { backend } from "@/lib/backend";
import { useSettings } from "@/state/hooks";
import { navigate, resetAllSettings, showNotice, updateSettings, useApp } from "@/state/store";
import { useTaskbarInfo } from "@/strip/support";
import { CrossLink } from "../customize/Customize";
import { CommandLineRows } from "./CommandLine";
import { ShortcutRecorder } from "./ShortcutRecorder";
import { Button, Picker, Switch } from "../ui/controls";
import { confirmAction } from "../ui/dialog";
import { SlidersIcon, WarningTriangle } from "../ui/icons";

function Section({ title, children, warning }: { title: string; children: ReactNode; warning?: boolean }) {
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">
        {title}
        {warning ? (
          <span className="uc-inline-icon" style={{ color: "var(--uc-orange)" }}>
            <WarningTriangle size={10} />
          </span>
        ) : null}
      </h2>
      <div className="uc-card uc-settings-card">{children}</div>
    </section>
  );
}

function Row({ label, children, note }: { label: string; children: ReactNode; note?: string }) {
  return (
    <div className="uc-settings-row-group">
      <div className="uc-settings-row">
        <span className="uc-settings-label">{label}</span>
        {children}
      </div>
      {note ? <p className="uc-settings-note">{note}</p> : null}
    </div>
  );
}

function InlineNotice({ text }: { text: string }) {
  return <p className="uc-settings-notice">{text}</p>;
}

function platformKey(platform: string | undefined): "windows" | "linux" | "other" {
  return platform === "windows" || platform === "linux" ? platform : "other";
}

function useAutostart(): [boolean | null, (enabled: boolean) => void, string | null] {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const language = useApp((state) => state.settings.language);
  useEffect(() => {
    let alive = true;
    autostartEnabled()
      .then((value) => alive && setEnabled(value))
      .catch(() => alive && setEnabled(null));
    return () => {
      alive = false;
    };
  }, []);
  const change = (next: boolean) => {
    setError(null);
    setEnabled(next);
    setAutostart(next)
      .then(() => autostartEnabled())
      .then((value) => setEnabled(value))
      .catch(() => {
        setEnabled(!next);
        setError(messagesFor(language).settings.launchAtLoginError);
      });
  };
  return [enabled, change, error];
}

function useNotificationAccess(active: boolean): [NotificationAccess, () => void] {
  const [access, setAccess] = useState<NotificationAccess>("granted");
  const visible = useApp((state) => state.popupVisible);
  useEffect(() => {
    let alive = true;
    notificationAccess()
      .then((value) => alive && setAccess(value))
      .catch(() => alive && setAccess("unsupported"));
    return () => {
      alive = false;
    };
  }, [active, visible]);
  const request = () => {
    requestNotificationAccess()
      .then(setAccess)
      .catch(() => setAccess("denied"));
  };
  return [access, request];
}

const NOTIFICATION_KEYS: readonly NotificationKey[] = ["almostOut", "cuttingItClose", "willRunOut"];

export function Settings() {
  const settings = useSettings();
  const info = useApp((state) => state.info);
  const language = settings.language;
  const messages = messagesFor(language);
  const text = messages.settings;
  const platform = platformKey(info?.platform);
  const [autostart, changeAutostart, autostartError] = useAutostart();
  const notificationsOn = anyNotificationEnabled(settings);
  const [access, requestAccess] = useNotificationAccess(notificationsOn);
  const [logError, setLogError] = useState<string | null>(null);
  const [shortcutError, setShortcutError] = useState<string | null>(null);
  const [shortcutGeneration, setShortcutGeneration] = useState(0);
  const stripSupported = useTaskbarInfo()?.supported === true;
  const shortcutSupported = typeof backend().setGlobalShortcut === "function";
  const cliSupported = typeof backend().cliStatus === "function";
  const section = (key: SettingsSectionKey) => text.section(key);

  const setNotification = (key: NotificationKey, on: boolean) => {
    const notifications: NotificationSettings = { ...settings.notifications, [key]: on };
    updateSettings({ notifications });
    if (on && access !== "granted") requestAccess();
  };

  const copyLogPath = () => {
    if (!info?.logFile) return;
    setLogError(null);
    backend()
      .copyText(info.logFile)
      .then(() => showNotice(text.copied, "positive"))
      .catch(() => setLogError(text.logActionFailed));
  };

  const revealLog = () => {
    if (!info?.logFile) return;
    setLogError(null);
    revealFile(info.logFile).catch(() => setLogError(text.logActionFailed));
  };

  const resetEverything = async () => {
    const confirmed = await confirmAction({
      title: text.resetAllSettingsTitle,
      message: text.resetAllSettingsMessage,
      confirmLabel: text.resetAllSettingsConfirm,
      cancelLabel: messages.chrome.cancel,
    });
    if (!confirmed) return;
    resetAllSettings();
    if (autostart === false) changeAutostart(true);
    await backend()
      .setGlobalShortcut?.(null)
      .catch(() => undefined);
    setShortcutError(null);
    setShortcutGeneration((generation) => generation + 1);
  };

  return (
    <div className="uc-stack">
      <Section title={section("general")}>
        <Row label={text.language}>
          <Picker<Language> value={language} options={LANGUAGES} label={(option) => messagesFor(option).language} onChange={(value) => updateSettings({ language: value })} ariaLabel={text.language} />
        </Row>
        <Row label={text.showTotalSpend}>
          <Switch checked={settings.showTotalSpend} label={text.showTotalSpend} onChange={(on) => updateSettings({ showTotalSpend: on })} />
        </Row>
        {autostart !== null ? (
          <Row label={text.launchAtLogin(platform)}>
            <Switch checked={autostart} label={text.launchAtLogin(platform)} onChange={changeAutostart} />
          </Row>
        ) : null}
        {autostartError ? <InlineNotice text={autostartError} /> : null}
        {shortcutSupported ? (
          <Row label={text.globalShortcut}>
            <ShortcutRecorder key={shortcutGeneration} platform={platform} onError={setShortcutError} />
          </Row>
        ) : null}
        {shortcutError ? <InlineNotice text={shortcutError} /> : null}
      </Section>

      <Section title={section("appearance")}>
        <Row label={text.theme}>
          <Picker value={settings.theme} options={["system", "light", "dark"] as const} label={text.themeOption} onChange={(value) => updateSettings({ theme: value })} ariaLabel={text.theme} />
        </Row>
        <Row label={text.density}>
          <Picker value={settings.density} options={["regular", "compact"] as const} label={text.densityOption} onChange={(value) => updateSettings({ density: value })} ariaLabel={text.density} />
        </Row>
        <Row label={text.timeFormat}>
          <Picker value={settings.timeFormat} options={["auto", "12h", "24h"] as const} label={text.timeFormatOption} onChange={(value) => updateSettings({ timeFormat: value })} ariaLabel={text.timeFormat} />
        </Row>
        <Row label={text.reduceAnimations}>
          <Switch checked={settings.reduceAnimations} label={text.reduceAnimations} onChange={(on) => updateSettings({ reduceAnimations: on })} />
        </Row>
      </Section>

      <Section title={section("usageDisplay")}>
        <Row label={text.showUsageAs}>
          <Picker value={settings.displayMode} options={["remaining", "used"] as const} label={messages.meter.displayMode} onChange={(value) => updateSettings({ displayMode: value })} ariaLabel={text.showUsageAs} />
        </Row>
        <Row label={text.resetTimes}>
          <Picker value={settings.resetDisplayMode} options={["relative", "absolute"] as const} label={text.resetTimesOption} onChange={(value) => updateSettings({ resetDisplayMode: value })} ariaLabel={text.resetTimes} />
        </Row>
        <Row label={text.alwaysShowPacing} note={text.alwaysShowPacingNote}>
          <Switch checked={settings.alwaysShowPacing} label={text.alwaysShowPacing} onChange={(on) => updateSettings({ alwaysShowPacing: on })} />
        </Row>
      </Section>

      <Section title={section("taskbar")}>
        <Row label={text.showOnTaskbar} note={text.taskbarNote(stripSupported)}>
          <Switch checked={settings.showTaskbarStrip} label={text.showOnTaskbar} onChange={(on) => updateSettings({ showTaskbarStrip: on })} />
        </Row>
        {stripSupported ? (
          <Row label={text.iconStyle}>
            <Picker value={settings.iconStyle} options={["text", "bars"] as const} label={text.iconStyleOption} onChange={(value) => updateSettings({ iconStyle: value })} ariaLabel={text.iconStyle} disabled={!settings.showTaskbarStrip} />
          </Row>
        ) : null}
      </Section>

      <Section title={section("notifications")} warning={notificationsOn && access === "denied"}>
        {NOTIFICATION_KEYS.map((key) => (
          <Row key={key} label={text.notification(key)} note={text.notificationNote(key)}>
            <Switch checked={settings.notifications[key]} label={text.notification(key)} onChange={(on) => setNotification(key, on)} />
          </Row>
        ))}
        {notificationsOn && access === "denied" ? (
          <div className="uc-settings-row-group">
            <InlineNotice text={text.notificationsDenied} />
            <div className="uc-settings-actions">
              <Button onClick={requestAccess} className="is-small">
                {text.allowNotifications}
              </Button>
            </div>
          </div>
        ) : null}
      </Section>

      {cliSupported ? (
        <Section title={section("commandLine")}>
          <CommandLineRows />
        </Section>
      ) : null}

      <Section title={section("advanced")}>
        {info?.logFile ? (
          <div className="uc-settings-actions is-column">
            <Button onClick={copyLogPath} className="is-small is-wide">
              {text.copyLogPath}
            </Button>
            {canRevealFiles() ? (
              <Button onClick={revealLog} className="is-small is-wide">
                {text.revealLog(platform)}
              </Button>
            ) : null}
          </div>
        ) : null}
        {logError ? <InlineNotice text={logError} /> : null}
        <div className="uc-settings-actions">
          <Button variant="destructive" onClick={() => void resetEverything()} className="is-small is-wide">
            {text.resetAllSettings}
          </Button>
        </div>
      </Section>

      <CrossLink
        icon={<SlidersIcon size={15} />}
        title={messages.customize.customizeLinkTitle}
        subtitle={messages.customize.customizeLinkSubtitle}
        onClick={() => navigate("customize")}
      />
    </div>
  );
}
