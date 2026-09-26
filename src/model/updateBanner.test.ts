import type { UpdateStatus } from "@/lib/types";
import { isTransientBanner, updateBannerKey, updateBannerOf } from "./updateBanner";

function status(patch: Partial<UpdateStatus>): UpdateStatus {
  return { supported: true, currentVersion: "0.1.0", phase: "idle", manual: false, downloaded: 0, ...patch };
}

const OFFER = { version: "0.2.0", notes: "Faster refresh" };

describe("updateBannerOf", () => {
  it("stays quiet about background checks until they find a release", () => {
    expect(updateBannerOf(null)).toBeNull();
    expect(updateBannerOf(status({ phase: "checking" }))).toBeNull();
    expect(updateBannerOf(status({ phase: "upToDate" }))).toBeNull();
    expect(updateBannerOf(status({ phase: "failed", failure: { stage: "check", reason: "network" } }))).toBeNull();
    expect(updateBannerOf(status({ phase: "available", available: OFFER }))).toEqual({ kind: "available", version: "0.2.0", notes: "Faster refresh" });
  });

  it("reports the outcome of a check the user started", () => {
    expect(updateBannerOf(status({ phase: "checking", manual: true }))).toEqual({ kind: "checking" });
    expect(updateBannerOf(status({ phase: "upToDate", manual: true }))).toEqual({ kind: "upToDate", version: "0.1.0" });
    expect(updateBannerOf(status({ phase: "failed", manual: true, failure: { stage: "check", reason: "network" } }))).toEqual({
      kind: "failed",
      stage: "check",
      reason: "network",
      version: undefined,
    });
  });

  it("follows the install and always reports a failed download or install", () => {
    expect(updateBannerOf(status({ phase: "downloading", available: OFFER, downloaded: 25, total: 100 }))).toEqual({
      kind: "downloading",
      version: "0.2.0",
      fraction: 0.25,
    });
    expect(updateBannerOf(status({ phase: "downloading", available: OFFER, downloaded: 25 }))).toMatchObject({ fraction: null });
    expect(updateBannerOf(status({ phase: "installing", available: OFFER }))).toEqual({ kind: "installing", version: "0.2.0" });
    expect(updateBannerOf(status({ phase: "failed", available: OFFER, failure: { stage: "download", reason: "signature" } }))).toMatchObject({
      kind: "failed",
      stage: "download",
      version: "0.2.0",
    });
  });
});

describe("updateBannerKey", () => {
  it("snoozes an offer until a later check finds it again", () => {
    const first = status({ phase: "available", available: OFFER, checkedAt: "2026-09-26T04:00:00Z" });
    const later = { ...first, checkedAt: "2026-09-26T10:00:00Z" };
    expect(updateBannerKey(first)).not.toBeNull();
    expect(updateBannerKey(first)).toBe(updateBannerKey({ ...first }));
    expect(updateBannerKey(later)).not.toBe(updateBannerKey(first));
  });

  it("cannot close a card while an install runs", () => {
    expect(updateBannerKey(status({ phase: "downloading", available: OFFER }))).toBeNull();
    expect(updateBannerKey(status({ phase: "installing", available: OFFER }))).toBeNull();
  });

  it("treats a manual result, not an offer, as transient", () => {
    expect(isTransientBanner(updateBannerOf(status({ phase: "upToDate", manual: true })))).toBe(true);
    expect(isTransientBanner(updateBannerOf(status({ phase: "failed", manual: true, failure: { stage: "check", reason: "network" } })))).toBe(true);
    expect(isTransientBanner(updateBannerOf(status({ phase: "available", available: OFFER })))).toBe(false);
  });
});
