/**
 * The App Updates card (upstream Settings → Updates): "check automatically", "install automatically",
 * the running version with Check Now or Install, and one line on where the updater stands. A build
 * that cannot replace itself links to the releases page instead of offering a check.
 */
import { messagesFor, type Language, type Messages } from "@/i18n";
import { backend } from "@/lib/backend";
import type { UpdateStatus } from "@/lib/types";
import { shortTime, type TimeFormat } from "@/model/format";
import { useSettings } from "@/state/hooks";
import { checkForUpdates, installUpdate, updateSettings, useApp } from "@/state/store";
import { REPOSITORY_URL } from "../chrome/about";
import { Button, Switch } from "../ui/controls";

const RELEASES_URL = `${REPOSITORY_URL}/releases`;

/** The status line under the version row, and whether it reports a problem. */
export function updateStatusLine(status: UpdateStatus, messages: Messages, timeFormat: TimeFormat, language: Language): { text: string; problem: boolean } | null {
  const text = messages.update;
  if (!status.supported) return { text: text.unsupported, problem: false };
  const version = status.available?.version ?? "";
  switch (status.phase) {
    case "checking":
      return { text: text.checking, problem: false };
    case "available":
      return { text: text.availableStatus(version), problem: false };
    case "downloading": {
      const percent = status.total ? ` ${Math.round((Math.min(status.downloaded, status.total) / status.total) * 100)}%` : "";
      return { text: `${text.downloading(version)}${percent}`, problem: false };
    }
    case "installing":
      return { text: text.installing(version), problem: false };
    case "upToDate":
      return status.checkedAt ? { text: text.lastChecked(shortTime(new Date(status.checkedAt), timeFormat, language)), problem: false } : null;
    case "failed":
      return status.failure ? { text: `${text.failedTitle(status.failure.stage)}. ${text.failure(status.failure.reason)}`, problem: true } : null;
    case "idle":
      return null;
  }
}

/** What the "install automatically" switch shows: it follows the checks, and is off where installing asks for a password. */
export function automaticInstallState(status: UpdateStatus, checks: boolean, installs: boolean): "on" | "off" | "unavailable" {
  if (status.unattended === false) return "unavailable";
  return checks && installs ? "on" : "off";
}

export function UpdateRows() {
  const settings = useSettings();
  const status = useApp((state) => state.update);
  const messages = messagesFor(settings.language);
  const text = messages.update;
  if (!status) return null;
  const busy = status.phase === "checking" || status.phase === "downloading" || status.phase === "installing";
  const offered = status.phase === "available" || (status.phase === "failed" && status.failure?.stage !== "check" && status.available !== undefined);
  const line = updateStatusLine(status, messages, settings.timeFormat, settings.language);
  const installs = automaticInstallState(status, settings.automaticUpdateChecks, settings.automaticUpdateInstalls);

  return (
    <>
      {status.supported ? (
        <div className="uc-settings-row-group">
          <div className="uc-settings-row">
            <span className="uc-settings-label">{text.automaticChecks}</span>
            <Switch checked={settings.automaticUpdateChecks} label={text.automaticChecks} onChange={(on) => updateSettings({ automaticUpdateChecks: on })} />
          </div>
          <p className="uc-settings-note">{text.automaticChecksNote}</p>
        </div>
      ) : null}
      {status.supported ? (
        <div className="uc-settings-row-group">
          <div className="uc-settings-row">
            <span className="uc-settings-label">{text.automaticInstalls}</span>
            <Switch
              checked={installs === "on"}
              label={text.automaticInstalls}
              disabled={installs === "unavailable" || !settings.automaticUpdateChecks}
              onChange={(on) => updateSettings({ automaticUpdateInstalls: on })}
            />
          </div>
          <p className="uc-settings-note">{text.automaticInstallsNote(installs)}</p>
        </div>
      ) : null}
      <div className="uc-settings-row-group">
        <div className="uc-settings-row">
          <span className="uc-settings-label uc-num">{text.version(status.currentVersion)}</span>
          {!status.supported ? (
            <Button onClick={() => void backend().openUrl(RELEASES_URL)} className="is-small">
              {text.openReleases}
            </Button>
          ) : offered ? (
            <Button variant="prominent" onClick={installUpdate} disabled={busy} className="is-small">
              {text.install}
            </Button>
          ) : (
            <Button onClick={checkForUpdates} disabled={busy} className="is-small">
              {text.checkNow}
            </Button>
          )}
        </div>
        {line ? (
          <p className={line.problem ? "uc-settings-notice" : "uc-settings-note"} role="status">
            {line.text}
          </p>
        ) : null}
      </div>
    </>
  );
}
