import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { findDownloads, findInstallers, manifest, mergeManifest, mergeSums, missingTargets, pickRun, signedVersion, stageMacos } from "./release.mjs";

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

  it("serves the macOS updater archive to both darwin targets of its architecture", () => {
    installer("Quota Control_0.2.0_aarch64.app.tar.gz");
    installer("Quota Control_0.2.0_universal.app.tar.gz");
    const found = findInstallers([dir], "0.2.0").sort((a, b) => a.assetName.localeCompare(b.assetName));
    expect(found.map(({ kind, assetName, targets }) => ({ kind, assetName, targets }))).toEqual([
      { kind: "app", assetName: "Quota-Control_0.2.0_aarch64.app.tar.gz", targets: ["darwin-aarch64-app", "darwin-aarch64"] },
      {
        kind: "app",
        assetName: "Quota-Control_0.2.0_universal.app.tar.gz",
        targets: ["darwin-aarch64-app", "darwin-aarch64", "darwin-x86_64-app", "darwin-x86_64"],
      },
    ]);
  });

  it("refuses an unsigned installer and a signature bound to another version", () => {
    writeFileSync(join(dir, "Quota Control_0.2.0_x64-setup.exe"), "binary");
    expect(() => findInstallers([dir], "0.2.0")).toThrow(/has no \.sig/);
    installer("Quota Control_0.2.0_x64-setup.exe", "0.1.0");
    expect(() => findInstallers([dir], "0.2.0")).toThrow(/signed for version 0\.1\.0, expected 0\.2\.0/);
  });
});

describe("findDownloads", () => {
  it("collects this version's disk images without asking for a signature", () => {
    writeFileSync(join(dir, "Quota Control_0.2.0_aarch64.dmg"), "disk image");
    writeFileSync(join(dir, "Quota Control_0.1.0_aarch64.dmg"), "older");
    expect(findDownloads([dir], "0.2.0").map(({ kind, assetName }) => ({ kind, assetName }))).toEqual([
      { kind: "dmg", assetName: "Quota-Control_0.2.0_aarch64.dmg" },
    ]);
  });
});

describe("stageMacos", () => {
  it("copies the updater archive and its signature to a versioned name", () => {
    mkdirSync(join(dir, "macos"));
    writeFileSync(join(dir, "macos", "Quota Control.app.tar.gz"), "archive");
    writeFileSync(join(dir, "macos", "Quota Control.app.tar.gz.sig"), "signature");
    const staged = stageMacos(dir, "0.2.0", "arm64");
    expect(staged).toBe(join(dir, "macos", "Quota Control_0.2.0_aarch64.app.tar.gz"));
    expect(readFileSync(`${staged}.sig`, "utf8")).toBe("signature");
    expect(() => stageMacos(join(dir, "missing"), "0.2.0", "arm64")).toThrow(/is missing/);
  });
});

describe("merging with a release", () => {
  it("keeps the other platforms of the same version and drops those of an older one", () => {
    const windows = { version: "0.2.0", notes: "Notes", pub_date: "a", platforms: { "windows-x86_64": { signature: "w", url: "w" } } };
    const mac = { version: "0.2.0", notes: "", pub_date: "b", platforms: { "darwin-aarch64": { signature: "m", url: "m" } } };
    expect(mergeManifest(windows, mac)).toEqual({
      version: "0.2.0",
      notes: "Notes",
      pub_date: "b",
      platforms: { "windows-x86_64": { signature: "w", url: "w" }, "darwin-aarch64": { signature: "m", url: "m" } },
    });
    expect(mergeManifest({ ...windows, version: "0.1.9" }, mac)).toEqual(mac);
    expect(mergeManifest(null, mac)).toEqual(mac);
  });

  it("keeps one checksum line per file, the new one winning", () => {
    const a = "a".repeat(64);
    const b = "b".repeat(64);
    const c = "c".repeat(64);
    expect(mergeSums(`${a}  setup.exe\n${b}  app.tar.gz\n`, `${c}  app.tar.gz\n`)).toBe(`${a}  setup.exe\n${c}  app.tar.gz\n`);
    expect(mergeSums(null, `${c}  app.dmg\n`)).toBe(`${c}  app.dmg\n`);
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
    expect(missingTargets(document)).toEqual(["linux-x86_64-deb", "linux-x86_64-appimage", "darwin-aarch64-app"]);
  });
});

describe("missingTargets", () => {
  const windowsAndLinux = { "windows-x86_64-nsis": {}, "windows-x86_64": {}, "linux-x86_64-deb": {}, "linux-x86_64-appimage": {}, "linux-x86_64": {} };
  const macos = { "darwin-aarch64-app": {}, "darwin-aarch64": {} };

  it("asks for macOS in every release, including the first one to carry it", () => {
    expect(missingTargets({ version: "0.1.14", platforms: windowsAndLinux })).toEqual(["darwin-aarch64-app"]);
    expect(missingTargets({ version: "0.1.14", platforms: { ...windowsAndLinux, ...macos } })).toEqual([]);
  });

  it("always asks for Windows and Linux", () => {
    expect(missingTargets({ version: "0.2.1", platforms: macos })).toEqual(["windows-x86_64-nsis", "linux-x86_64-deb", "linux-x86_64-appimage"]);
  });

  it("checks only the kinds it is given, and treats a missing manifest as empty", () => {
    expect(missingTargets({ version: "0.2.1", platforms: macos }, ["app"])).toEqual([]);
    expect(missingTargets(null, ["app"])).toEqual(["darwin-aarch64-app"]);
    expect(missingTargets(null, ["nsis", "deb", "appimage"])).toEqual(["windows-x86_64-nsis", "linux-x86_64-deb", "linux-x86_64-appimage"]);
  });
});

describe("pickRun", () => {
  const head = "a".repeat(40);
  const run = (id, overrides) => ({ databaseId: id, event: "push", headBranch: "v0.1.14", headSha: head, createdAt: "2026-09-26T15:00:00Z", ...overrides });

  it("takes the newest run the tag push started at that commit", () => {
    const runs = [run(1), run(3, { createdAt: "2026-09-26T15:30:00Z" }), run(2, { createdAt: "2026-09-26T15:10:00Z" })];
    expect(pickRun(runs, "v0.1.14", head)?.databaseId).toBe(3);
  });

  it("ignores manual runs, other tags and other commits", () => {
    const runs = [run(1, { event: "workflow_dispatch" }), run(2, { headBranch: "v0.1.13" }), run(3, { headSha: "b".repeat(40) })];
    expect(pickRun(runs, "v0.1.14", head)).toBeNull();
  });
});
