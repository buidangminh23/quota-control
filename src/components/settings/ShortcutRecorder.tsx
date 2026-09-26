/**
 * The Global Shortcut field (upstream `ShortcutRecorderField`): click it and press a combo to set the
 * shortcut that toggles the popup from anywhere; the ✕ clears it and turns the shortcut off. While
 * it listens, the current combo is released so pressing it records instead of hiding the popup.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { messagesFor } from "@/i18n";
import { backend } from "@/lib/backend";
import { recordKey, shortcutKeys } from "@/model/shortcut";
import { useApp } from "@/state/store";
import { CloseIcon } from "../ui/icons";
import { tooltipProps } from "../ui/tooltip";
import type { PlatformKey } from "@/i18n/messages";

type Platform = PlatformKey;

export function ShortcutRecorder({ platform, onError }: { platform: Platform; onError: (message: string | null) => void }) {
  const language = useApp((state) => state.settings.language);
  const visible = useApp((state) => state.popupVisible);
  const text = messagesFor(language).settings;
  const [shortcut, setShortcut] = useState<string | null>(null);
  const [recording, setRecording] = useState(false);
  const recordingRef = useRef(false);

  useEffect(() => {
    let alive = true;
    backend()
      .globalShortcut?.()
      .then((saved) => alive && setShortcut(saved))
      .catch(() => alive && setShortcut(null));
    return () => {
      alive = false;
    };
  }, []);

  const stop = useCallback(() => {
    if (!recordingRef.current) return;
    recordingRef.current = false;
    setRecording(false);
    void backend().pauseGlobalShortcut?.(false).catch(() => undefined);
  }, []);

  const start = () => {
    onError(null);
    recordingRef.current = true;
    setRecording(true);
    void backend().pauseGlobalShortcut?.(true).catch(() => undefined);
  };

  const save = useCallback(
    (next: string | null) => {
      const api = backend();
      if (!api.setGlobalShortcut) return;
      api
        .setGlobalShortcut(next)
        .then((saved) => {
          setShortcut(saved);
          onError(null);
        })
        .catch(() => onError(text.shortcutUnavailable))
        .finally(stop);
    },
    [onError, stop, text.shortcutUnavailable],
  );

  useEffect(() => {
    if (!recording) return;
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      const result = recordKey(event);
      if (result.kind === "cancel") stop();
      else if (result.kind === "needsModifier") onError(text.shortcutNeedsModifier(platform));
      else if (result.kind === "unsupported") onError(text.shortcutUnsupported);
      else if (result.kind === "combo") save(result.accelerator);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, onError, platform, save, stop, text]);

  useEffect(() => {
    if (!visible) stop();
  }, [visible, stop]);

  useEffect(() => stop, [stop]);

  const keys = shortcut ? shortcutKeys(shortcut, platform) : [];
  const label = recording ? text.pressShortcut : keys.length > 0 ? keys.join("+") : text.recordShortcut;
  return (
    <div className="uc-shortcut">
      <button
        type="button"
        className={`uc-picker uc-shortcut-field${recording ? " is-recording" : ""}${keys.length === 0 && !recording ? " is-empty" : ""}`}
        aria-label={`${text.globalShortcut}: ${label}`}
        aria-pressed={recording}
        onClick={() => (recording ? stop() : start())}
        onBlur={stop}
        {...tooltipProps(text.globalShortcutTooltip)}
      >
        <span className="uc-truncate">{label}</span>
      </button>
      {shortcut && !recording ? (
        <button type="button" className="uc-shortcut-clear" aria-label={text.clearShortcut} onClick={() => save(null)} {...tooltipProps(text.clearShortcut)}>
          <CloseIcon size={9} />
        </button>
      ) : null}
    </div>
  );
}
