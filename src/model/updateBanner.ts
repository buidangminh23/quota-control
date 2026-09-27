/**
 * What the update dialog shows for an `UpdateStatus` (upstream `UpdateBannerCard`, which only
 * covered "Update Available" as a dashboard card): the version this launch replaced, a release a
 * background check found, the install's progress, and the outcome of a check or install the user
 * started.
 *
 * Background checks stay silent unless they find a release. Closing a notice snoozes it: its key
 * includes the check time, so the next check that still finds the release shows it again, and a
 * progress notice stays closed only for its own step.
 */
import type { UpdateFailureReason, UpdateFailureStage, UpdateStatus } from "@/lib/types";

export type UpdateBanner =
  | { kind: "updated"; from: string; version: string }
  | { kind: "checking" }
  | { kind: "available"; version: string; notes?: string }
  | { kind: "downloading"; version: string; fraction: number | null }
  | { kind: "installing"; version: string }
  | { kind: "upToDate"; version: string }
  | { kind: "failed"; stage: UpdateFailureStage; reason: UpdateFailureReason; version?: string };

/** The step in progress or the outcome of the last one, apart from the version this launch replaced. */
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

/** Identity of the step's notice for snoozing, or `null` when there is none. */
export function updateBannerKey(status: UpdateStatus | null): string | null {
  const banner = updateBannerOf(status);
  if (!banner || !status) return null;
  const version = status.available?.version ?? "";
  switch (banner.kind) {
    case "downloading":
    case "installing":
      return [banner.kind, version].join("|");
    case "failed":
      return [banner.kind, version, status.checkedAt ?? "", banner.stage].join("|");
    default:
      return [banner.kind, version, status.checkedAt ?? ""].join("|");
  }
}

function inProgress(banner: UpdateBanner): boolean {
  return banner.kind === "checking" || banner.kind === "downloading" || banner.kind === "installing";
}

/**
 * The notice the dialog shows: a step in progress first, then the version this launch replaced,
 * then the step's outcome, skipping the one the user closed (`dismissed`, an `updateBannerKey`).
 */
export function updateNoticeOf(status: UpdateStatus | null, dismissed: string | null): UpdateBanner | null {
  if (!status) return null;
  const banner = updateBannerOf(status);
  const key = updateBannerKey(status);
  const step = banner && key !== dismissed ? banner : null;
  if (step && inProgress(step)) return step;
  if (status.updatedFrom) return { kind: "updated", from: status.updatedFrom, version: status.currentVersion };
  return step;
}

/** Results of a check the user started, which should not greet them again the next time the popup opens. */
export function isTransientBanner(banner: UpdateBanner | null): boolean {
  return banner?.kind === "upToDate" || (banner?.kind === "failed" && banner.stage === "check");
}
