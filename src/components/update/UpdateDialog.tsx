/**
 * App-update notices as a dialog over the popup, in place of upstream's dashboard card: the version
 * this launch replaced, a release a check found, the download and install progress, and the
 * outcome of a check or install the user started. While the popup is closed the core raises a
 * system notification instead (`src-tauri/src/updates.rs`). Escape and the scrim close a notice;
 * Later snoozes an offer until the next check finds it again (`model/updateBanner.ts`).
 */
import { useEffect, useLayoutEffect, useRef, type ReactNode } from "react";
import type { Messages } from "@/i18n";
import type { PlatformKey } from "@/i18n/messages";
import { backend } from "@/lib/backend";
import { platformKey } from "@/model/platform";
import { updateNoticeOf, type UpdateBanner } from "@/model/updateBanner";
import { useMessages } from "@/state/hooks";
import { acknowledgeUpdate, checkForUpdates, dismissUpdate, installUpdate, useApp } from "@/state/store";
import { REPOSITORY_URL } from "../chrome/about";
import { holdDialog, useDialogRequest } from "../ui/dialog";
import { Spinner } from "../ui/icons";
import { closeMenu } from "../ui/menu";
import { hideTooltip } from "../ui/tooltip";

export function releaseNotesUrl(version: string): string {
  return `${REPOSITORY_URL}/releases/tag/v${version}`;
}

interface NoticeAction {
  label: string;
  prominent?: boolean;
  /** Escape and the scrim choose it, and it starts focused. */
  cancel?: boolean;
  onSelect: () => void;
}

interface Notice {
  title: string;
  busy?: boolean;
  body?: ReactNode;
  actions: NoticeAction[];
}

function openNotes(version: string): void {
  void backend().openUrl(releaseNotesUrl(version));
}

function noticeOf(banner: UpdateBanner, messages: Messages, name: string, platform: PlatformKey): Notice {
  const text = messages.update;
  const hide: NoticeAction = { label: text.hide, cancel: true, onSelect: dismissUpdate };
  switch (banner.kind) {
    case "updated":
      return {
        title: text.updatedTitle(banner.version),
        body: text.updatedMessage(name, banner.from, banner.version),
        actions: [
          {
            label: text.whatsNew,
            onSelect: () => {
              openNotes(banner.version);
              acknowledgeUpdate();
            },
          },
          { label: text.close, prominent: true, cancel: true, onSelect: acknowledgeUpdate },
        ],
      };
    case "available":
      return {
        title: text.availableTitle,
        body: text.availableMessage(name, banner.version),
        actions: [
          { label: text.install, prominent: true, onSelect: installUpdate },
          { label: text.whatsNew, onSelect: () => openNotes(banner.version) },
          { label: text.later, cancel: true, onSelect: dismissUpdate },
        ],
      };
    case "checking":
      return { title: text.checking, busy: true, actions: [hide] };
    case "downloading": {
      const percent = banner.fraction === null ? null : Math.round(banner.fraction * 100);
      return {
        title: text.downloading(banner.version),
        busy: percent === null,
        body: (
          <div className="uc-update-progress">
            <div
              className="uc-meter uc-update-meter"
              role="progressbar"
              aria-label={text.downloading(banner.version)}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percent ?? undefined}
            >
              <div className="uc-meter-fill uc-update-meter-fill" style={{ width: `${percent ?? 0}%` }} />
            </div>
            {percent === null ? null : <span className="uc-update-percent uc-num">{percent}%</span>}
          </div>
        ),
        actions: [hide],
      };
    }
    case "installing":
      return { title: text.installing(banner.version), busy: true, body: text.installingNote(platform), actions: [hide] };
    case "upToDate":
      return {
        title: text.upToDateTitle,
        body: text.upToDateMessage(name, banner.version),
        actions: [{ label: text.close, prominent: true, cancel: true, onSelect: dismissUpdate }],
      };
    case "failed":
      return {
        title: text.failedTitle(banner.stage),
        body: text.failure(banner.reason),
        actions: [
          { label: text.retry, prominent: true, onSelect: banner.stage === "check" ? checkForUpdates : installUpdate },
          { label: text.close, cancel: true, onSelect: dismissUpdate },
        ],
      };
  }
}

export function UpdateDialog() {
  const status = useApp((state) => state.update);
  const dismissed = useApp((state) => state.dismissedUpdate);
  const ready = useApp((state) => state.ready);
  const info = useApp((state) => state.info);
  const covered = useDialogRequest();
  const messages = useMessages();
  const banner = ready ? updateNoticeOf(status, dismissed) : null;
  if (!banner || covered) return null;
  const notice = noticeOf(banner, messages, info?.name ?? messages.chrome.appName, platformKey(info?.platform));
  return <NoticeCard key={banner.kind === "checking" ? banner.kind : `${banner.kind}|${banner.version ?? ""}`} notice={notice} />;
}

function NoticeCard({ notice }: { notice: Notice }) {
  const cancelButton = useRef<HTMLButtonElement>(null);
  const cancel = notice.actions.find((action) => action.cancel);
  const latestCancel = useRef(cancel);
  latestCancel.current = cancel;

  useLayoutEffect(() => {
    hideTooltip();
    closeMenu();
    return holdDialog();
  }, []);

  useEffect(() => {
    cancelButton.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      latestCancel.current?.onSelect();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  return (
    <div className="uc-dialog-scrim" onPointerDown={(event) => event.target === event.currentTarget && cancel?.onSelect()}>
      <div
        className="uc-dialog uc-update-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="uc-update-dialog-title"
        aria-busy={notice.busy || undefined}
        data-update-dialog
      >
        <h2 id="uc-update-dialog-title" className="uc-dialog-title">
          {notice.busy ? <Spinner size={11} /> : null}
          <span>{notice.title}</span>
        </h2>
        {notice.body ? <div className="uc-dialog-message">{notice.body}</div> : null}
        <div className={`uc-dialog-actions${notice.actions.length > 2 ? " is-stacked" : ""}`}>
          {notice.actions.map((action) => (
            <button
              key={action.label}
              ref={action.cancel ? cancelButton : undefined}
              type="button"
              className={`uc-button ${action.prominent ? "is-prominent" : "is-bordered"}`}
              onClick={action.onSelect}
            >
              {action.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
