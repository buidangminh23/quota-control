#!/usr/bin/env node
/**
 * Builds the macOS desktop widget (a WidgetKit app extension) that `tauri build` copies into
 * `Quota Control.app/Contents/PlugIns` (see `bundle.macOS.files` in src-tauri/tauri.macos.conf.json).
 *
 *   node scripts/macos-widget.mjs build [--arch aarch64|x86_64|universal] [--out DIR]
 *
 * Runs as the bundle step's `beforeBundleCommand`, which passes the architecture in TAURI_ENV_ARCH
 * (or `universal` through TAURI_ENV_TARGET_TRIPLE). The extension is signed with the identity Tauri
 * signs the app with (APPLE_SIGNING_IDENTITY, else `bundle.macOS.signingIdentity`), ad-hoc when that
 * is `-` or unset, always sandboxed with its own entitlements: WidgetKit refuses unsandboxed
 * widgets, and the app's signature must not replace them. A real identity gets a secure timestamp,
 * which notarization requires of nested code. Like Xcode's
 * app extension targets it starts in Foundation's `NSExtensionMain`; started at its own `main`,
 * ExtensionFoundation traps before the widget runs.
 */
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SOURCES = join(ROOT, "src-tauri", "macos");
const DEFAULT_OUT = join(ROOT, "src-tauri", "gen", "macos");
const NAME = "QuotaControlWidget";
const MINIMUM_MACOS = "14.0";
const SWIFT_ARCH = { aarch64: "arm64", arm64: "arm64", x86_64: "x86_64", x64: "x86_64" };

class WidgetError extends Error {}

function fail(message) {
  throw new WidgetError(message);
}

function run(command, args) {
  const result = spawnSync(command, args, { cwd: ROOT, stdio: ["ignore", "inherit", "inherit"] });
  if (result.error) fail(`${command} could not start: ${result.error.message}`);
  if (result.status !== 0) fail(`${command} exited with ${result.status}`);
}

function capture(command, args) {
  const result = spawnSync(command, args, { cwd: ROOT, encoding: "utf8" });
  if (result.status !== 0) fail(`${command} ${args.join(" ")} failed: ${(result.stderr ?? "").trim()}`);
  return result.stdout.trim();
}

function swiftFiles(folder) {
  return capture("find", [join(SOURCES, folder), "-name", "*.swift", "-type", "f"])
    .split("\n")
    .filter(Boolean)
    .sort();
}

/** The Swift architectures to build: TAURI_ENV_* from the bundler, or --arch, or this Mac's. */
export function architectures(requested, environment = process.env) {
  const triple = environment.TAURI_ENV_TARGET_TRIPLE ?? "";
  const choice = requested ?? (triple.startsWith("universal") ? "universal" : environment.TAURI_ENV_ARCH) ?? process.arch;
  if (choice === "universal") return ["arm64", "x86_64"];
  const arch = SWIFT_ARCH[choice];
  if (!arch) fail(`unsupported architecture ${choice}`);
  return [arch];
}

/** The signing identity, resolved the way Tauri resolves the app's: environment, then config. */
export function signingIdentity(environment = process.env) {
  if (environment.APPLE_SIGNING_IDENTITY) return environment.APPLE_SIGNING_IDENTITY;
  for (const name of ["tauri.macos.conf.json", "tauri.conf.json"]) {
    try {
      const config = JSON.parse(readFileSync(join(ROOT, "src-tauri", name), "utf8"));
      const identity = config?.bundle?.macOS?.signingIdentity;
      if (typeof identity === "string" && identity.length > 0) return identity;
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
    }
  }
  return "-";
}

export function appVersion() {
  const version = JSON.parse(readFileSync(join(ROOT, "src-tauri", "tauri.conf.json"), "utf8")).version;
  if (!/^\d+\.\d+\.\d+/.test(version ?? "")) fail(`tauri.conf.json has no usable version: ${version}`);
  return version;
}

function build(options) {
  if (process.platform !== "darwin") fail("the widget builds on macOS only");
  const out = resolve(options.out ?? DEFAULT_OUT);
  const bundle = join(out, `${NAME}.appex`);
  const contents = join(bundle, "Contents");
  const work = join(out, "build");
  rmSync(bundle, { recursive: true, force: true });
  rmSync(work, { recursive: true, force: true });
  mkdirSync(join(contents, "MacOS"), { recursive: true });
  mkdirSync(work, { recursive: true });

  const sdk = capture("xcrun", ["--sdk", "macosx", "--show-sdk-path"]);
  const sources = [...swiftFiles("Shared"), ...swiftFiles("Widget")];
  const slices = architectures(options.arch).map((arch) => {
    const output = join(work, `${NAME}-${arch}`);
    run("xcrun", [
      "swiftc", "-parse-as-library", "-application-extension", "-module-name", NAME, "-swift-version", "5",
      "-target", `${arch}-apple-macos${MINIMUM_MACOS}`, "-sdk", sdk, "-O",
      "-framework", "Foundation", "-Xlinker", "-e", "-Xlinker", "_NSExtensionMain",
      "-o", output, ...sources,
    ]);
    return output;
  });
  const binary = join(contents, "MacOS", NAME);
  if (slices.length === 1) run("cp", [slices[0], binary]);
  else run("lipo", ["-create", "-output", binary, ...slices]);

  const version = appVersion();
  const plist = readFileSync(join(SOURCES, "Widget", "Info.plist"), "utf8").replaceAll("__VERSION__", version);
  writeFileSync(join(contents, "Info.plist"), plist);

  const identity = signingIdentity();
  run("codesign", [
    "--force", "--sign", identity, "--options", "runtime", identity === "-" ? "--timestamp=none" : "--timestamp",
    "--entitlements", join(SOURCES, "Widget", `${NAME}.entitlements`), bundle,
  ]);
  run("codesign", ["--verify", "--strict", bundle]);
  rmSync(work, { recursive: true, force: true });
  console.log(`Built ${bundle} (${version}, ${architectures(options.arch).join("+")}, signed ${identity === "-" ? "ad-hoc" : identity})`);
}

function main() {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: { arch: { type: "string" }, out: { type: "string" } },
  });
  if (positionals[0] !== "build") fail("usage: node scripts/macos-widget.mjs build [--arch aarch64|x86_64|universal] [--out DIR]");
  build(values);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(`macos-widget: ${error instanceof WidgetError ? error.message : error.stack}`);
    process.exit(1);
  }
}
