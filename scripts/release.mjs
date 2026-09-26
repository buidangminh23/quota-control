#!/usr/bin/env node
/**
 * Release tooling for Quota Control's signed self-updates (see README → Releases).
 *
 *   node scripts/release.mjs version [--tag vX.Y.Z]
 *       Print the app version once package.json, Cargo.toml and tauri.conf.json agree (and match
 *       the tag when one is given).
 *   node scripts/release.mjs assemble --out DIR [--tag vX.Y.Z] [--base-url URL] [--notes-file FILE]
 *                                     [--allow-missing] BUNDLE_DIR...
 *       Collect the signed NSIS, deb and AppImage installers that `tauri build` left under the bundle
 *       folders, rename them without spaces, and write latest.json (the updater manifest) and
 *       SHA256SUMS. Refuses unsigned installers and signatures bound to another version.
 *   node scripts/release.mjs publish --dir DIR --tag vX.Y.Z [--notes-file FILE] [--latest]
 *       Upload DIR to a draft GitHub release. --latest publishes it, which is the moment installed
 *       apps start seeing the update, then reads latest.json back from GitHub.
 *   node scripts/release.mjs local [--skip-linux] [--distro NAME] [--allow-dirty] [--publish] [--latest]
 *       Windows only: build the NSIS installer here and the deb and AppImage in WSL from the committed
 *       tree, signed with %USERPROFILE%\.tauri\quota-control.key (or TAURI_SIGNING_PRIVATE_KEY),
 *       then assemble into target/release-assets/vX.Y.Z (and publish).
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ASSET_PREFIX = "Quota-Control";
const UPDATER_CONFIG = "src-tauri/tauri.updater.conf.json";
const DEFAULT_KEY = join(homedir(), ".tauri", "quota-control.key");
const ARCH = { x64: "x86_64", amd64: "x86_64", x86: "i686", i386: "i686", arm64: "aarch64", aarch64: "aarch64" };
/** Installer kinds, the updater targets each one serves (`{os}-{arch}-{bundle}` first), and whether a release needs it. */
const KINDS = [
  { id: "nsis", pattern: /_(x64|x86|arm64)-setup\.exe$/, targets: (arch) => [`windows-${arch}-nsis`, `windows-${arch}`] },
  { id: "deb", pattern: /_(amd64|arm64|i386)\.deb$/, targets: (arch) => [`linux-${arch}-deb`] },
  { id: "appimage", pattern: /_(amd64|aarch64|i386)\.AppImage$/, targets: (arch) => [`linux-${arch}-appimage`, `linux-${arch}`] },
];
const PUBLISHED_POLL_ATTEMPTS = 12;
const PUBLISHED_POLL_DELAY_MS = 5000;

class ReleaseError extends Error {}

function fail(message) {
  throw new ReleaseError(message);
}

function read(path) {
  return readFileSync(join(ROOT, path), "utf8");
}

function run(command, args, options = {}) {
  console.log(`> ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, { cwd: ROOT, stdio: "inherit", shell: process.platform === "win32" && command === "pnpm", ...options });
  if (result.error) fail(`${command} could not start: ${result.error.message}`);
  if (result.status !== 0) fail(`${command} exited with ${result.status}`);
}

function capture(command, args) {
  const result = spawnSync(command, args, { cwd: ROOT, encoding: "utf8" });
  return { ok: result.status === 0, stdout: (result.stdout ?? "").trim(), stderr: (result.stderr ?? "").trim() };
}

/** Cargo's target folder, honouring CARGO_TARGET_DIR and .cargo/config.toml as `tauri build` does. */
function targetDir() {
  const metadata = capture("cargo", ["metadata", "--format-version", "1", "--no-deps"]);
  if (!metadata.ok) fail(`cargo metadata failed: ${metadata.stderr}`);
  return JSON.parse(metadata.stdout).target_directory;
}

function workspacePackage(key) {
  const section = /^\[workspace\.package\]$([\s\S]*?)(?=^\[)/m.exec(read("Cargo.toml"))?.[1] ?? "";
  return new RegExp(`^${key}\\s*=\\s*"([^"]+)"`, "m").exec(section)?.[1];
}

export function appVersion(tag) {
  const npm = JSON.parse(read("package.json")).version;
  const tauri = JSON.parse(read("src-tauri/tauri.conf.json")).version;
  const cargo = workspacePackage("version");
  if (!npm || npm !== tauri || npm !== cargo) fail(`versions disagree: package.json ${npm}, tauri.conf.json ${tauri}, Cargo.toml ${cargo}`);
  if (tag && tag !== `v${npm}`) fail(`tag ${tag} does not match version ${npm}`);
  return npm;
}

function repositorySlug() {
  const url = workspacePackage("repository") ?? "";
  const slug = /github\.com\/([^/]+\/[^/]+?)(?:\.git)?\/?$/.exec(url)?.[1];
  if (!slug) fail(`Cargo.toml [workspace.package] repository is not a GitHub URL: ${url}`);
  return slug;
}

/** The version a minisign signature (the base64 `.sig` Tauri writes) was bound to, from its trusted comment. */
export function signedVersion(signature) {
  const text = Buffer.from(signature.trim(), "base64").toString("utf8");
  const comment = text.split(/\r?\n/).find((line) => line.startsWith("trusted comment:"));
  const field = comment
    ?.slice("trusted comment:".length)
    .trim()
    .split("\t")
    .find((part) => part.startsWith("version:"));
  return field?.slice("version:".length);
}

function walk(directory) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? walk(path) : [path];
  });
}

/** Find this version's signed installers under `directories`, one per kind and architecture. */
export function findInstallers(directories, version) {
  const found = new Map();
  for (const path of directories.flatMap(walk)) {
    const name = basename(path);
    if (!name.includes(`_${version}_`)) continue;
    const kind = KINDS.find((candidate) => candidate.pattern.test(name));
    if (!kind) continue;
    const arch = ARCH[kind.pattern.exec(name)[1]];
    const sigPath = `${path}.sig`;
    if (!existsSync(sigPath)) fail(`${name} has no .sig; build with ${UPDATER_CONFIG} and the signing key`);
    const signature = readFileSync(sigPath, "utf8").trim();
    const bound = signedVersion(signature);
    if (bound !== version) fail(`${name} is signed for version ${bound ?? "(none)"}, expected ${version}`);
    const key = `${kind.id}-${arch}`;
    if (found.has(key)) fail(`two ${key} installers found: ${found.get(key).path} and ${path}`);
    const assetName = `${ASSET_PREFIX}_${version}_${name.slice(name.indexOf(`_${version}_`) + version.length + 2)}`;
    found.set(key, { kind: kind.id, arch, path, signature, assetName, targets: kind.targets(arch) });
  }
  return [...found.values()];
}

/** The updater targets a published release must serve: one per installer kind on x86_64. */
export function missingTargets(document) {
  return KINDS.map((kind) => kind.targets("x86_64")[0]).filter((target) => !document.platforms?.[target]);
}

export function manifest({ version, notes, pubDate, installers, baseUrl }) {
  const platforms = {};
  for (const installer of installers) {
    for (const target of installer.targets) {
      platforms[target] = { signature: installer.signature, url: `${baseUrl}${encodeURIComponent(installer.assetName)}` };
    }
  }
  return { version, notes, pub_date: pubDate, platforms };
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function assemble(options, bundleDirs) {
  const version = appVersion(options.tag);
  const tag = options.tag ?? `v${version}`;
  if (!options.out) fail("assemble needs --out DIR");
  if (bundleDirs.length === 0) fail("assemble needs at least one bundle folder");
  const installers = findInstallers(bundleDirs.map((dir) => resolve(dir)), version);
  const kinds = new Set(installers.map((installer) => installer.kind));
  const missing = KINDS.map((kind) => kind.id).filter((id) => !kinds.has(id));
  if (installers.length === 0) fail(`no signed ${version} installers under ${bundleDirs.join(", ")}`);
  if (missing.length > 0 && !options["allow-missing"]) {
    fail(`missing ${missing.join(", ")}: installed apps of that kind could not update from this release (pass --allow-missing for a partial test build)`);
  }
  const out = resolve(options.out);
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  const sums = [];
  for (const installer of installers) {
    const target = join(out, installer.assetName);
    copyFileSync(installer.path, target);
    writeFileSync(`${target}.sig`, `${installer.signature}\n`);
    sums.push(`${sha256(target)}  ${installer.assetName}`);
  }
  const baseUrl = options["base-url"] ?? `https://github.com/${repositorySlug()}/releases/download/${tag}/`;
  const notes = options["notes-file"] ? readFileSync(resolve(options["notes-file"]), "utf8").trim() : "";
  const pubDate = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
  const document = manifest({ version, notes, pubDate, installers, baseUrl: baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/` });
  writeFileSync(join(out, "latest.json"), `${JSON.stringify(document, null, 2)}\n`);
  writeFileSync(join(out, "SHA256SUMS"), `${sums.join("\n")}\n`);
  console.log(`Assembled ${tag} in ${relative(ROOT, out) || out}:`);
  for (const installer of installers) {
    console.log(`  ${installer.assetName}  ${(statSync(installer.path).size / 1_048_576).toFixed(1)} MiB  ${installer.targets.join(", ")}`);
  }
  if (missing.length > 0) console.log(`  (partial build: no ${missing.join(", ")})`);
  return { out, tag, version };
}

async function publish(options) {
  if (!options.dir || !options.tag) fail("publish needs --dir DIR and --tag vX.Y.Z");
  const version = appVersion(options.tag);
  const slug = repositorySlug();
  const dir = resolve(options.dir);
  const files = readdirSync(dir).map((name) => join(dir, name));
  if (!files.some((file) => basename(file) === "latest.json")) fail(`${dir} has no latest.json; run assemble first`);
  const document = JSON.parse(readFileSync(join(dir, "latest.json"), "utf8"));
  if (document.version !== version) fail(`${dir} holds ${document.version}, not ${version}`);
  const missing = missingTargets(document);
  if (missing.length > 0) fail(`latest.json has no ${missing.join(", ")}; a release must serve every installer kind`);
  const existing = capture("gh", ["release", "view", options.tag, "-R", slug, "--json", "isDraft"]);
  if (existing.ok && !JSON.parse(existing.stdout).isDraft) fail(`${options.tag} is already published; bump the version for a new release`);
  if (!existing.ok) {
    const create = ["release", "create", options.tag, "-R", slug, "--draft", "--title", `Quota Control ${version}`];
    create.push(...(options["notes-file"] ? ["--notes-file", resolve(options["notes-file"])] : ["--generate-notes"]));
    const remoteTag = capture("git", ["ls-remote", "--tags", "origin", `refs/tags/${options.tag}`]);
    if (!remoteTag.stdout) {
      const head = capture("git", ["rev-parse", "HEAD"]).stdout;
      if (!capture("git", ["branch", "-r", "--contains", head]).stdout) fail(`HEAD ${head.slice(0, 7)} is not pushed; push it before publishing`);
      create.push("--target", head);
    }
    run("gh", create);
  }
  run("gh", ["release", "upload", options.tag, "-R", slug, "--clobber", ...files]);
  if (!options.latest) {
    console.log(`Draft ${options.tag} is ready. Publish it with: gh release edit ${options.tag} -R ${slug} --draft=false --latest`);
    return;
  }
  run("gh", ["release", "edit", options.tag, "-R", slug, "--draft=false", "--latest"]);
  await verifyPublished(slug, version);
}

async function verifyPublished(slug, version) {
  const url = `https://github.com/${slug}/releases/latest/download/latest.json`;
  for (let attempt = 1; attempt <= PUBLISHED_POLL_ATTEMPTS; attempt += 1) {
    const response = await fetch(url, { headers: { accept: "application/json" } }).catch(() => null);
    const body = response?.ok ? await response.json().catch(() => null) : null;
    if (body?.version === version) {
      for (const [target, platform] of Object.entries(body.platforms)) {
        const asset = await fetch(platform.url, { method: "HEAD" }).catch(() => null);
        if (!asset?.ok) fail(`${target} points at ${platform.url}, which answers ${asset?.status ?? "nothing"}`);
      }
      console.log(`Installed apps now see ${version}: ${url} lists ${Object.keys(body.platforms).join(", ")}.`);
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, PUBLISHED_POLL_DELAY_MS));
  }
  fail(`${url} still does not announce ${version}`);
}

function windowsToWsl(path) {
  const match = /^([A-Za-z]):[\\/](.*)$/.exec(path);
  if (!match) fail(`cannot map ${path} into WSL`);
  return `/mnt/${match[1].toLowerCase()}/${match[2].replace(/\\/g, "/")}`;
}

const LINUX_BUILD = `#!/bin/bash
set -euo pipefail
STAGE="$1"
SRC="$HOME/build/qc-release-src"
export CARGO_TARGET_DIR="$HOME/build/qc-target"
export CI=true
rm -rf "$SRC" "$STAGE/bundle" "$CARGO_TARGET_DIR/release/bundle/deb" "$CARGO_TARGET_DIR/release/bundle/appimage"
mkdir -p "$SRC" "$CARGO_TARGET_DIR" "$STAGE/bundle"
tar -xf "$STAGE/source.tar" -C "$SRC"
cd "$SRC"
pnpm install --frozen-lockfile
pnpm tauri build --ci --bundles deb,appimage --config ${UPDATER_CONFIG}
find "$CARGO_TARGET_DIR/release/bundle/deb" "$CARGO_TARGET_DIR/release/bundle/appimage" -maxdepth 1 -type f \\
  \\( -name '*.deb' -o -name '*.deb.sig' -o -name '*.AppImage' -o -name '*.AppImage.sig' \\) -exec cp {} "$STAGE/bundle/" \\;
`;

function buildLinux(env, distro, target) {
  const stage = join(target, "release-assets", "wsl");
  rmSync(stage, { recursive: true, force: true });
  mkdirSync(stage, { recursive: true });
  run("git", ["archive", "--format=tar", "-o", join(stage, "source.tar"), "HEAD"]);
  writeFileSync(join(stage, "build.sh"), LINUX_BUILD);
  const wslStage = windowsToWsl(stage);
  const distroArgs = distro ? ["-d", distro] : [];
  const forwarded = ["TAURI_SIGNING_PRIVATE_KEY/u", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD/u"].join(":");
  run("wsl.exe", [...distroArgs, "-e", "bash", "-l", `${wslStage}/build.sh`, wslStage], { env: { ...env, WSLENV: forwarded } });
  return join(stage, "bundle");
}

async function local(options) {
  if (process.platform !== "win32") fail("local releases run on Windows; the Linux packages are built in WSL");
  if (options["skip-linux"] && options.publish) fail("--skip-linux makes a partial test build, which cannot be published");
  const version = appVersion();
  if (!options["allow-dirty"] && capture("git", ["status", "--porcelain"]).stdout) {
    fail("commit your changes first: the Linux packages are built from the committed tree");
  }
  const env = { ...process.env, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "" };
  if (!env.TAURI_SIGNING_PRIVATE_KEY) {
    if (!existsSync(DEFAULT_KEY)) fail(`no signing key: set TAURI_SIGNING_PRIVATE_KEY or create ${DEFAULT_KEY}`);
    env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(DEFAULT_KEY, "utf8").trim();
  }
  const target = targetDir();
  const bundles = [join(target, "release", "bundle", "nsis")];
  rmSync(bundles[0], { recursive: true, force: true });
  run("pnpm", ["tauri", "build", "--ci", "--bundles", "nsis", "--config", UPDATER_CONFIG], { env });
  if (!options["skip-linux"]) bundles.push(buildLinux(env, options.distro ?? process.env.QUOTA_CONTROL_WSL_DISTRO, target));
  const tag = `v${version}`;
  const assembled = assemble({ ...options, tag, out: join(target, "release-assets", tag), "allow-missing": options["skip-linux"] }, bundles);
  if (options.publish) await publish({ ...options, tag, dir: assembled.out });
}

async function main() {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      tag: { type: "string" },
      out: { type: "string" },
      dir: { type: "string" },
      "base-url": { type: "string" },
      "notes-file": { type: "string" },
      distro: { type: "string" },
      "allow-missing": { type: "boolean" },
      "allow-dirty": { type: "boolean" },
      "skip-linux": { type: "boolean" },
      publish: { type: "boolean" },
      latest: { type: "boolean" },
    },
  });
  const [command, ...rest] = positionals;
  switch (command) {
    case "version":
      console.log(appVersion(values.tag));
      return;
    case "assemble":
      assemble(values, rest);
      return;
    case "publish":
      await publish(values);
      return;
    case "local":
      await local(values);
      return;
    default:
      fail("usage: node scripts/release.mjs version|assemble|publish|local [options] (see the header of this file)");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main().catch((error) => {
    console.error(`release: ${error instanceof ReleaseError ? error.message : error.stack}`);
    process.exit(1);
  });
}
