/**
 * What the dashboard's update card shows for an `UpdateStatus` (upstream `UpdateBannerCard`, which
 * only covered "Update Available"; here the card also carries the install's progress and the outcome
 * of a check the user started, since the popup has no separate updater window).
 *
 * Background checks stay silent unless they find a release. Closing the card snoozes it: the key
 * includes the check time, so the next check that still finds the release shows it again.
 */
import type { UpdateFailureReason, UpdateFailureStage, UpdateStatus } from "@/lib/types";

export type UpdateBanner =
  | { kind: "checking" }
  | { kind: "available"; version: string; notes?: string }
  | { kind: "downloading"; version: string; fraction: number | null }
  | { kind: "installing"; version: string }
  | { kind: "upToDate"; version: string }
  | { kind: "failed"; stage: UpdateFailureStage; reason: UpdateFailureReason; version?: string };

export function updateBannerOf(status: UpdateStatus | null): UpdateBanner | null {
  if (!status) return null;
  const version = status.available?.version;
  switch (status.phase) {
    case "available":
      return version ? { kind: "available", version, notes: status.available?.notes } : null;
    case "downloading":
      return {
        kind: "downloading",
        version: version ?? "",
        fraction: status.total ? Math.min(1, status.downloaded / status.total) : null,
      };
    case "installing":
      return { kind: "installing", version: version ?? "" };
    case "checking":
      return status.manual ? { kind: "checking" } : null;
    case "upToDate":
      return status.manual ? { kind: "upToDate", version: status.currentVersion } : null;
    case "failed": {
      const failure = status.failure;
      if (!failure || (failure.stage === "check" && !status.manual)) return null;
      return { kind: "failed", stage: failure.stage, reason: failure.reason, version };
    }
    case "idle":
      return null;
  }
}

/** The card can be closed; progress cards stay until the step ends. */
export function isDismissible(banner: UpdateBanner): boolean {
  return banner.kind === "available" || banner.kind === "upToDate" || banner.kind === "failed";
}

/** Identity of the card for snoozing, or `null` when there is nothing to close. */
export function updateBannerKey(status: UpdateStatus | null): string | null {
  const banner = updateBannerOf(status);
  if (!banner || !isDismissible(banner) || !status) return null;
  const stage = banner.kind === "failed" ? banner.stage : "";
  return [banner.kind, status.available?.version ?? "", status.checkedAt ?? "", stage].join("|");
}

/** Results of a check the user started, which should not greet them again the next time the popup opens. */
export function isTransientBanner(banner: UpdateBanner | null): boolean {
  return banner?.kind === "upToDate" || (banner?.kind === "failed" && banner.stage === "check");
}
