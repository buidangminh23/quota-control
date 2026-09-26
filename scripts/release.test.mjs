import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { findInstallers, manifest, missingTargets, signedVersion } from "./release.mjs";

function signature(trustedComment) {
  const text = ["untrusted comment: signature from tauri secret key", "RUTkeyid", `trusted comment: ${trustedComment}`, "globalsig", ""].join("\n");
  return Buffer.from(text, "utf8").toString("base64");
}

let dir;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "qc-release-"));
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

function installer(name, signedFor = "0.2.0") {
  writeFileSync(join(dir, name), "binary");
  writeFileSync(join(dir, `${name}.sig`), signature(`timestamp:1790000000\tfile:${name}\tversion:${signedFor}`));
}

describe("signedVersion", () => {
  it("reads the version Tauri binds into the trusted comment", () => {
    expect(signedVersion(signature("timestamp:1\tfile:app.exe\tversion:0.2.0"))).toBe("0.2.0");
    expect(signedVersion(signature("timestamp:1\tfile:app.exe"))).toBeUndefined();
  });
});

describe("findInstallers", () => {
  it("renames each kind without spaces and lists the updater targets it serves", () => {
    installer("Quota Control_0.2.0_x64-setup.exe");
    installer("Quota Control_0.2.0_amd64.deb");
    installer("Quota Control_0.2.0_amd64.AppImage");
    writeFileSync(join(dir, "Quota Control_0.1.0_x64-setup.exe"), "older build");
    const found = findInstallers([dir], "0.2.0").sort((a, b) => a.kind.localeCompare(b.kind));
    expect(found.map(({ kind, assetName, targets }) => ({ kind, assetName, targets }))).toEqual([
      { kind: "appimage", assetName: "Quota-Control_0.2.0_amd64.AppImage", targets: ["linux-x86_64-appimage", "linux-x86_64"] },
      { kind: "deb", assetName: "Quota-Control_0.2.0_amd64.deb", targets: ["linux-x86_64-deb"] },
      { kind: "nsis", assetName: "Quota-Control_0.2.0_x64-setup.exe", targets: ["windows-x86_64-nsis", "windows-x86_64"] },
    ]);
  });

  it("refuses an unsigned installer and a signature bound to another version", () => {
    writeFileSync(join(dir, "Quota Control_0.2.0_x64-setup.exe"), "binary");
    expect(() => findInstallers([dir], "0.2.0")).toThrow(/has no \.sig/);
    installer("Quota Control_0.2.0_x64-setup.exe", "0.1.0");
    expect(() => findInstallers([dir], "0.2.0")).toThrow(/signed for version 0\.1\.0, expected 0\.2\.0/);
  });
});

describe("manifest", () => {
  it("points every target at its release asset", () => {
    const document = manifest({
      version: "0.2.0",
      notes: "",
      pubDate: "2026-09-26T08:00:00Z",
      baseUrl: "https://github.com/buidangminh23/quota-control/releases/download/v0.2.0/",
      installers: [{ assetName: "Quota-Control_0.2.0_x64-setup.exe", signature: "c2ln", targets: ["windows-x86_64-nsis", "windows-x86_64"] }],
    });
    expect(document).toEqual({
      version: "0.2.0",
      notes: "",
      pub_date: "2026-09-26T08:00:00Z",
      platforms: {
        "windows-x86_64-nsis": { signature: "c2ln", url: "https://github.com/buidangminh23/quota-control/releases/download/v0.2.0/Quota-Control_0.2.0_x64-setup.exe" },
        "windows-x86_64": { signature: "c2ln", url: "https://github.com/buidangminh23/quota-control/releases/download/v0.2.0/Quota-Control_0.2.0_x64-setup.exe" },
      },
    });
    expect(missingTargets(document)).toEqual(["linux-x86_64-deb", "linux-x86_64-appimage"]);
  });
});
