/**
 * The Command Line card (upstream `commandLineSection`): install or remove the `usagectl` terminal
 * helper, and where other apps can read the same limits over the loopback HTTP API.
 */
import { useEffect, useState } from "react";
import { messagesFor } from "@/i18n";
import { backend } from "@/lib/backend";
import type { CliStatus } from "@/lib/types";
import { useApp } from "@/state/store";
import { Button } from "../ui/controls";

const LOCAL_API_URL = "http://127.0.0.1:6736/v1/limits";

export function CommandLineRows() {
  const language = useApp((state) => state.settings.language);
  const visible = useApp((state) => state.popupVisible);
  const text = messagesFor(language).settings;
  const [status, setStatus] = useState<CliStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!visible) return;
    let alive = true;
    backend()
      .cliStatus?.()
      .then((next) => alive && setStatus(next))
      .catch(() => alive && setStatus(null));
    return () => {
      alive = false;
    };
  }, [visible]);

  const change = (install: boolean) => {
    const api = backend();
    const action = install ? api.installCli : api.uninstallCli;
    if (!action) return;
    setBusy(true);
    setFailed(false);
    action
      .call(api)
      .then(setStatus)
      .catch(() => setFailed(true))
      .finally(() => setBusy(false));
  };

  const state = status?.state;
  const note = status ? text.cliStatus(status.state, status.location) : null;
  return (
    <>
      <div className="uc-settings-row-group">
        <div className="uc-settings-row">
          <span className="uc-settings-label">{text.terminalHelper}</span>
          {state === "installed" ? (
            <Button onClick={() => change(false)} disabled={busy} className="is-small">
              {text.uninstallCli}
            </Button>
          ) : state === "notInstalled" ? (
            <Button onClick={() => change(true)} disabled={busy} className="is-small">
              {text.installCli}
            </Button>
          ) : null}
        </div>
        <p className="uc-settings-note">{text.cliNote}</p>
        {note ? <p className={state === "conflict" ? "uc-settings-notice" : "uc-settings-note"}>{note}</p> : null}
        {failed ? <p className="uc-settings-notice">{text.cliFailed}</p> : null}
      </div>
      <p className="uc-settings-note uc-settings-api">{text.localApiNote(LOCAL_API_URL)}</p>
    </>
  );
}
