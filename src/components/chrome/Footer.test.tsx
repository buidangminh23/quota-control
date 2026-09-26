import { messagesFor } from "@/i18n";
import type { UpdateStatus } from "@/lib/types";
import { updateEntry } from "./Footer";

const messages = messagesFor("vi");

function status(patch: Partial<UpdateStatus>): UpdateStatus {
  return { supported: true, currentVersion: "0.1.6", phase: "idle", manual: false, downloaded: 0, ...patch };
}

function summary(entry: ReturnType<typeof updateEntry>): { label: string; disabled: boolean } | null {
  if (entry?.kind !== "item") return null;
  return { label: entry.label, disabled: entry.disabled === true };
}

describe("Options menu update entry", () => {
  it("checks for a new version while nothing is on offer", () => {
    expect(summary(updateEntry(status({}), messages))).toEqual({ label: "Kiểm tra phiên bản mới…", disabled: false });
    expect(summary(updateEntry(status({ phase: "upToDate", checkedAt: "2026-09-26T08:00:00Z" }), messages))).toEqual({
      label: "Kiểm tra phiên bản mới…",
      disabled: false,
    });
    expect(summary(updateEntry(status({ phase: "failed", manual: true, failure: { stage: "check", reason: "network" } }), messages))).toEqual({
      label: "Kiểm tra phiên bản mới…",
      disabled: false,
    });
  });

  it("offers the release a check found, like the tray menu", () => {
    const offer = { version: "0.1.7" };
    expect(summary(updateEntry(status({ phase: "available", available: offer }), messages))).toEqual({ label: "Cài bản mới 0.1.7…", disabled: false });
    expect(summary(updateEntry(status({ phase: "failed", available: offer, failure: { stage: "download", reason: "signature" } }), messages))).toEqual({
      label: "Cài bản mới 0.1.7…",
      disabled: false,
    });
  });

  it("waits while a check or an install is running", () => {
    expect(summary(updateEntry(status({ phase: "checking", manual: true }), messages))?.disabled).toBe(true);
    expect(summary(updateEntry(status({ phase: "downloading", available: { version: "0.1.7" } }), messages))?.disabled).toBe(true);
    expect(summary(updateEntry(status({ phase: "installing", available: { version: "0.1.7" } }), messages))?.disabled).toBe(true);
  });

  it("is left out where the build cannot replace itself or the core has no updater", () => {
    expect(updateEntry(status({ supported: false }), messages)).toBeNull();
    expect(updateEntry(null, messages)).toBeNull();
  });
});
