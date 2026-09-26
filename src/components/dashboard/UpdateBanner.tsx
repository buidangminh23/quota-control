/**
 * The dashboard's update card (upstream `UpdateBannerCard`): a release a background check found,
 * with Install and What's New; the download and install progress after that; and the result of a
 * check started from Settings or the tray menu. Closing it snoozes the offer until the next check
 * finds it again (`model/updateBanner.ts`).
 */
import type { ReactNode } from "react";
import { backend } from "@/lib/backend";
import { isDismissible, updateBannerKey, updateBannerOf, type UpdateBanner as Banner } from "@/model/updateBanner";
import { useMessages } from "@/state/hooks";
import { checkForUpdates, dismissUpdate, installUpdate, useApp } from "@/state/store";
import { REPOSITORY_URL } from "../chrome/about";
import { Button } from "../ui/controls";
import { CloseIcon, Spinner } from "../ui/icons";
import { platformKey } from "@/model/platform";

export function releaseNotesUrl(version: string): string {
  return `${REPOSITORY_URL}/releases/tag/v${version}`;
}


function Card({ title, trailing, dismiss, children }: { title: string; trailing?: ReactNode; dismiss?: { label: string; onClick: () => void }; children?: ReactNode }) {
  return (
    <div className="uc-card uc-hint uc-update-card" data-update-card>
      <div className="uc-hint-head">
        <span className="uc-hint-title">{title}</span>
        {trailing}
        {dismiss ? (
          <button type="button" className="uc-icon-button" aria-label={dismiss.label} onClick={dismiss.onClick}>
            <CloseIcon size={10} />
          </button>
        ) : null}
      </div>
      {children}
    </div>
  );
}

export function UpdateBanner() {
  const status = useApp((state) => state.update);
  const dismissed = useApp((state) => state.dismissedUpdate);
  const info = useApp((state) => state.info);
  const messages = useMessages();
  const text = messages.update;
  const banner: Banner | null = updateBannerOf(status);
  const key = updateBannerKey(status);
  if (!banner || !status || (key !== null && key === dismissed)) return null;
  const name = info?.name ?? messages.chrome.appName;
  const dismiss = isDismissible(banner) ? { label: messages.dashboard.dismiss, onClick: dismissUpdate } : undefined;

  switch (banner.kind) {
    case "available":
      return (
        <Card title={text.availableTitle} dismiss={dismiss}>
          <p className="uc-hint-message">{text.availableMessage(name, banner.version)}</p>
          <div className="uc-update-actions">
            <Button variant="prominent" onClick={installUpdate} className="is-small">
              {text.install}
            </Button>
            <Button variant="plain" onClick={() => void backend().openUrl(releaseNotesUrl(banner.version))} className="is-small">
              {text.whatsNew}
            </Button>
          </div>
        </Card>
      );
    case "downloading": {
      const percent = banner.fraction === null ? null : Math.round(banner.fraction * 100);
      return (
        <Card
          title={text.downloading(banner.version)}
          trailing={percent === null ? <Spinner size={10} /> : <span className="uc-update-percent uc-num">{percent}%</span>}
        >
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
        </Card>
      );
    }
    case "installing":
      return (
        <Card title={text.installing(banner.version)} trailing={<Spinner size={10} />}>
          <p className="uc-hint-message">{text.installingNote(platformKey(info?.platform))}</p>
        </Card>
      );
    case "checking":
      return <Card title={text.checking} trailing={<Spinner size={10} />} />;
    case "upToDate":
      return (
        <Card title={text.upToDateTitle} dismiss={dismiss}>
          <p className="uc-hint-message">{text.upToDateMessage(name, banner.version)}</p>
        </Card>
      );
    case "failed":
      return (
        <Card title={text.failedTitle(banner.stage)} dismiss={dismiss}>
          <p className="uc-hint-message">{text.failure(banner.reason)}</p>
          <div className="uc-update-actions">
            <Button onClick={banner.stage === "check" ? checkForUpdates : installUpdate} className="is-small">
              {text.retry}
            </Button>
          </div>
        </Card>
      );
  }
}
