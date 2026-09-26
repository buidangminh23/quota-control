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
 *   node scripts/release.mjs stage-macos [--dir BUNDLE_DIR]
 *       Copy the macOS updater archive `tauri build` writes (`Quota Control.app.tar.gz`) to a name
 *       with the version and architecture, as the other installers have, so assemble can find it.
 *   node scripts/release.mjs mac [--allow-dirty] [--publish] [--latest]
 *       macOS only: build the app bundle and DMG here, signed with ~/.tauri/quota-control.key (or
 *       TAURI_SIGNING_PRIVATE_KEY), and assemble them into target/release-assets/vX.Y.Z-macos.
 *       --publish adds them to the release, which may already hold the Windows and Linux installers.
 *   node scripts/release.mjs ship [--notes-file FILE]
 *       The usual way to release: push the tag vX.Y.Z for the pushed HEAD (FILE becomes the notes of
 *       its draft), wait while the Release workflow builds and signs the Windows, Linux and macOS
 *       packages and publishes them together, then check latest.json serves all three.
 *
 * Publishing merges with what the release already holds: latest.json keeps the other platforms'
 * entries and SHA256SUMS the other files' lines, so a release can be assembled from several machines.
 * Installed apps only see it once --latest publishes it, and that requires the Windows, Linux and
 * macOS packages alike, so installed copies on every system are offered the same version.
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ASSET_PREFIX = "Quota-Control";
const UPDATER_CONFIG = "src-tauri/tauri.updater.conf.json";
const DEFAULT_KEY = join(homedir(), ".tauri", "quota-control.key");
const PRODUCT = "Quota Control";
const ARCH = { x64: "x86_64", amd64: "x86_64", x86: "i686", i386: "i686", arm64: "aarch64", aarch64: "aarch64", universal: "universal" };
/** The macOS architectures an updater archive built for `arch` serves. */
const DARWIN_ARCHES = { aarch64: ["aarch64"], x86_64: ["x86_64"], universal: ["aarch64", "x86_64"] };
/**
 * Installer kinds, the updater targets each one serves (`{os}-{arch}-{bundle}` first), and the
 * architecture every published release must serve for it. Every release carries all of them, so the
 * Windows, Linux and macOS copies are always offered the same version.
 */
const KINDS = [
  { id: "nsis", pattern: /_(x64|x86|arm64)-setup\.exe$/, targets: (arch) => [`windows-${arch}-nsis`, `windows-${arch}`], required: "x86_64" },
  { id: "deb", pattern: /_(amd64|arm64|i386)\.deb$/, targets: (arch) => [`linux-${arch}-deb`], required: "x86_64" },
  { id: "appimage", pattern: /_(amd64|aarch64|i386)\.AppImage$/, targets: (arch) => [`linux-${arch}-appimage`, `linux-${arch}`], required: "x86_64" },
  {
    id: "app",
    pattern: /_(aarch64|x64|universal)\.app\.tar\.gz$/,
    targets: (arch) => DARWIN_ARCHES[arch].flatMap((darwin) => [`darwin-${darwin}-app`, `darwin-${darwin}`]),
    required: "aarch64",
  },
];
/** Downloads that are not updater packages: the macOS disk image people open by hand. */
const DOWNLOADS = [{ id: "dmg", pattern: /_(aarch64|x64|universal)\.dmg$/ }];
const PUBLISHED_POLL_ATTEMPTS = 12;
const PUBLISHED_POLL_DELAY_MS = 5000;
const SIGNING_SECRET = "TAURI_SIGNING_PRIVATE_KEY";
const RELEASE_WORKFLOW = "release.yml";
const RUN_APPEAR_ATTEMPTS = 24;
const RUN_APPEAR_DELAY_MS = 5000;
const RUN_POLL_DELAY_MS = 30_000;
const RUN_TIMEOUT_MS = 90 * 60_000;
const EVERY_PLATFORM =
  "every release ships Windows, Linux and macOS together so installed copies stay on one version. Release with " +
  "`node scripts/release.mjs ship` (the Release workflow builds all three), or upload the missing packages to the " +
  "draft from the machines that build them (`local --publish` on Windows, `mac --publish` on a Mac) and publish with --latest last";

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

/**
 * The updater targets `document` lacks among those a published release must serve, one per
 * installer kind in `kinds` (every kind unless narrowed).
 */
export function missingTargets(document, kinds = KINDS.map((kind) => kind.id)) {
  return KINDS.filter((kind) => kinds.includes(kind.id))
    .map((kind) => kind.targets(kind.required)[0])
    .filter((target) => !document?.platforms?.[target]);
}

/** The newest Release run that a push of `tag` at commit `head` started, among `gh run list` rows. */
export function pickRun(runs, tag, head) {
  return (
    runs
      .filter((run) => run.event === "push" && run.headBranch === tag && run.headSha === head)
      .sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt))[0] ?? null
  );
}

/** This version's disk images under `directories`, renamed like the installers. */
export function findDownloads(directories, version) {
  const found = new Map();
  for (const path of directories.flatMap(walk)) {
    const name = basename(path);
    if (!name.includes(`_${version}_`)) continue;
    const kind = DOWNLOADS.find((candidate) => candidate.pattern.test(name));
    if (!kind) continue;
    const assetName = `${ASSET_PREFIX}_${version}_${name.slice(name.indexOf(`_${version}_`) + version.length + 2)}`;
    if (found.has(assetName)) fail(`two ${assetName} downloads found: ${found.get(assetName).path} and ${path}`);
    found.set(assetName, { kind: kind.id, path, assetName });
  }
  return [...found.values()];
}

/** `incoming` with the other platforms `existing` already serves for the same version. */
export function mergeManifest(existing, incoming) {
  if (!existing || existing.version !== incoming.version) return incoming;
  return { ...incoming, notes: incoming.notes || existing.notes || "", platforms: { ...existing.platforms, ...incoming.platforms } };
}

/** SHA256SUMS lines of both, one per file, `incoming` winning for a file in both. */
export function mergeSums(existing, incoming) {
  const lines = new Map();
  for (const line of `${existing ?? ""}\n${incoming}`.split(/\r?\n/)) {
    const match = /^([0-9a-f]{64})\s+\*?(.+)$/.exec(line.trim());
    if (match) lines.set(match[2], `${match[1]}  ${match[2]}`);
  }
  return `${[...lines.values()].join("\n")}\n`;
}

/**
 * Copy `Quota Control.app.tar.gz` (and its signature) under BUNDLE_DIR/macos to a name carrying the
 * version and architecture. The minisign signature covers the file's bytes and version, not its name.
 */
export function stageMacos(bundleDir, version, arch = process.arch) {
  const folder = join(bundleDir, "macos");
  const archive = join(folder, `${PRODUCT}.app.tar.gz`);
  if (!existsSync(archive)) fail(`${archive} is missing; build the app bundle with ${UPDATER_CONFIG}`);
  const suffix = arch === "arm64" || arch === "aarch64" ? "aarch64" : arch === "universal" ? "universal" : "x64";
  const staged = join(folder, `${PRODUCT}_${version}_${suffix}.app.tar.gz`);
  copyFileSync(archive, staged);
  if (existsSync(`${archive}.sig`)) copyFileSync(`${archive}.sig`, `${staged}.sig`);
  return staged;
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
  const downloads = findDownloads(bundleDirs.map((dir) => resolve(dir)), version);
  const kinds = new Set(installers.map((installer) => installer.kind));
  const expected = options.expect ?? KINDS.map((kind) => kind.id);
  const missing = expected.filter((id) => !kinds.has(id));
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
  for (const download of downloads) {
    const target = join(out, download.assetName);
    copyFileSync(download.path, target);
    sums.push(`${sha256(target)}  ${download.assetName}`);
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
  for (const download of downloads) {
    console.log(`  ${download.assetName}  ${(statSync(download.path).size / 1_048_576).toFixed(1)} MiB  download`);
  }
  if (missing.length > 0) console.log(`  (partial build: no ${missing.join(", ")})`);
  return { out, tag, version };
}

async function publish(options) {
  if (!options.dir || !options.tag) fail("publish needs --dir DIR and --tag vX.Y.Z");
  const version = appVersion(options.tag);
  const slug = repositorySlug();
  const dir = resolve(options.dir);
  const listed = () => readdirSync(dir).filter((name) => !name.startsWith(".")).map((name) => join(dir, name));
  if (!listed().some((file) => basename(file) === "latest.json")) fail(`${dir} has no latest.json; run assemble first`);
  const incoming = JSON.parse(readFileSync(join(dir, "latest.json"), "utf8"));
  if (incoming.version !== version) fail(`${dir} holds ${incoming.version}, not ${version}`);
  const existing = capture("gh", ["release", "view", options.tag, "-R", slug, "--json", "isDraft"]);
  if (existing.ok && !JSON.parse(existing.stdout).isDraft) fail(`${options.tag} is already published; bump the version for a new release`);
  if (existing.ok) {
    checkDraftTarget(slug, options.tag);
    mergeWithRelease(slug, options.tag, dir);
  }
  const document = JSON.parse(readFileSync(join(dir, "latest.json"), "utf8"));
  const missing = missingTargets(document);
  if (missing.length > 0 && options.latest) fail(`latest.json has no ${missing.join(", ")}; ${EVERY_PLATFORM}`);
  if (missing.length > 0) console.log(`The draft has no ${missing.join(", ")} yet; add them before publishing.`);
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
  run("gh", ["release", "upload", options.tag, "-R", slug, "--clobber", ...listed()]);
  if (!options.latest) {
    if (missing.length > 0) {
      console.log(`Draft ${options.tag} still lacks ${missing.join(", ")}. Build them from the same commit with \`node scripts/release.mjs local --publish\` (Windows) or \`node scripts/release.mjs mac --publish\` (macOS), then publish with --latest.`);
    } else {
      console.log(`Draft ${options.tag} is ready. Publish it with: gh release edit ${options.tag} -R ${slug} --draft=false --latest`);
    }
    return;
  }
  run("gh", ["release", "edit", options.tag, "-R", slug, "--draft=false", "--latest"]);
  await verifyPublished(slug, version);
}

/**
 * Stop when the draft was started from another commit: its tag (or target) is what gets released,
 * so installers built here from different sources would ship under the wrong tag.
 */
function checkDraftTarget(slug, tag) {
  const head = capture("git", ["rev-parse", "HEAD"]).stdout;
  let expected = remoteTagCommit(tag);
  if (!expected) {
    const view = capture("gh", ["release", "view", tag, "-R", slug, "--json", "targetCommitish"]);
    const target = view.ok ? JSON.parse(view.stdout).targetCommitish ?? "" : "";
    expected = /^[0-9a-f]{40}$/.test(target) ? target : target ? capture("git", ["rev-parse", `origin/${target}`]).stdout : "";
  }
  if (expected && head && expected !== head) {
    fail(`the draft ${tag} targets ${expected.slice(0, 7)}, but this checkout is at ${head.slice(0, 7)}; build from that commit`);
  }
}

/** Fold the draft's latest.json and SHA256SUMS into DIR's, so uploading keeps the other platforms. */
function mergeWithRelease(slug, tag, dir) {
  const view = capture("gh", ["release", "view", tag, "-R", slug, "--json", "assets"]);
  if (!view.ok) fail(`cannot list the assets of ${tag}: ${view.stderr}`);
  const names = new Set(JSON.parse(view.stdout).assets.map((asset) => asset.name));
  const wanted = ["latest.json", "SHA256SUMS"].filter((name) => names.has(name));
  if (wanted.length === 0) return;
  const previous = join(dir, ".release");
  rmSync(previous, { recursive: true, force: true });
  mkdirSync(previous, { recursive: true });
  const download = capture("gh", ["release", "download", tag, "-R", slug, "-D", previous, ...wanted.flatMap((name) => ["-p", name]), "--clobber"]);
  if (!download.ok) {
    rmSync(previous, { recursive: true, force: true });
    fail(`cannot download ${wanted.join(" and ")} from ${tag}, so nothing was uploaded: ${download.stderr}`);
  }
  const read = (name) => (existsSync(join(previous, name)) ? readFileSync(join(previous, name), "utf8") : null);
  const manifestText = read("latest.json");
  const merged = mergeManifest(manifestText ? JSON.parse(manifestText) : null, JSON.parse(readFileSync(join(dir, "latest.json"), "utf8")));
  writeFileSync(join(dir, "latest.json"), `${JSON.stringify(merged, null, 2)}\n`);
  writeFileSync(join(dir, "SHA256SUMS"), mergeSums(read("SHA256SUMS"), readFileSync(join(dir, "SHA256SUMS"), "utf8")));
  rmSync(previous, { recursive: true, force: true });
}

/** The commit a tag on origin points at (through an annotated tag), or "" when origin has no such tag. */
function remoteTagCommit(tag) {
  const remote = capture("git", ["ls-remote", "--tags", "origin", `refs/tags/${tag}`, `refs/tags/${tag}^{}`]).stdout;
  const lines = remote.split("\n").filter(Boolean).map((line) => line.split(/\s+/));
  return (lines.find(([, ref]) => ref.endsWith("^{}")) ?? lines[0])?.[0] ?? "";
}

/**
 * The tag's release: null when there is none, otherwise whether it is still a draft and the
 * latest.json it holds for `version` (null when it holds none).
 */
function releaseState(slug, tag, version) {
  const view = capture("gh", ["release", "view", tag, "-R", slug, "--json", "isDraft,assets"]);
  if (!view.ok) {
    if (/not found/i.test(view.stderr)) return null;
    fail(`cannot read the ${tag} release: ${view.stderr}`);
  }
  const { isDraft, assets } = JSON.parse(view.stdout);
  if (!assets.some((asset) => asset.name === "latest.json")) return { isDraft, document: null };
  const folder = mkdtempSync(join(tmpdir(), "qc-release-"));
  try {
    const download = capture("gh", ["release", "download", tag, "-R", slug, "-D", folder, "-p", "latest.json", "--clobber"]);
    if (!download.ok) fail(`cannot download latest.json from ${tag}: ${download.stderr}`);
    const document = JSON.parse(readFileSync(join(folder, "latest.json"), "utf8"));
    return { isDraft, document: document.version === version ? document : null };
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
}

/**
 * Stop before a long build when --latest cannot succeed from this machine: every installer kind it
 * does not build (`built` lists those it does) must already be in the tag's draft.
 */
function requireRestInDraft(options, version, built) {
  if (!options.publish || !options.latest) return;
  const tag = `v${version}`;
  const state = releaseState(repositorySlug(), tag, version);
  if (state && !state.isDraft) fail(`${tag} is already published; bump the version for a new release`);
  const missing = missingTargets(state?.document, KINDS.map((kind) => kind.id).filter((id) => !built.includes(id)));
  if (missing.length > 0) {
    fail(`this machine does not build ${missing.join(", ")} and the ${tag} draft does not hold ${missing.length > 1 ? "them" : "it"} yet; ${EVERY_PLATFORM}`);
  }
}

function sleep(milliseconds) {
  return new Promise((done) => setTimeout(done, milliseconds));
}

/** Wait for the Release run the tag push started, printing each job as it ends; fail unless it succeeds. */
async function waitForRun(slug, tag, head) {
  let started = null;
  for (let attempt = 1; attempt <= RUN_APPEAR_ATTEMPTS && !started; attempt += 1) {
    const list = capture("gh", ["run", "list", "-R", slug, "--workflow", RELEASE_WORKFLOW, "--event", "push", "--limit", "20", "--json", "databaseId,event,headBranch,headSha,createdAt,url"]);
    started = list.ok ? pickRun(JSON.parse(list.stdout), tag, head) : null;
    if (!started) await sleep(RUN_APPEAR_DELAY_MS);
  }
  if (!started) fail(`no Release run started for ${tag}; see https://github.com/${slug}/actions`);
  console.log(`Waiting for the Release run: ${started.url}`);
  const reported = new Set();
  const deadline = Date.now() + RUN_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const view = capture("gh", ["run", "view", String(started.databaseId), "-R", slug, "--json", "status,conclusion,jobs"]);
    if (view.ok) {
      const { status, conclusion, jobs } = JSON.parse(view.stdout);
      for (const job of jobs.filter((job) => job.status === "completed" && !reported.has(job.name))) {
        reported.add(job.name);
        console.log(`  ${job.name}: ${job.conclusion}`);
      }
      if (status === "completed") {
        if (conclusion !== "success") fail(`the Release run ended ${conclusion}, so nothing was published: ${started.url}`);
        return;
      }
    }
    await sleep(RUN_POLL_DELAY_MS);
  }
  fail(`the Release run is still going after ${RUN_TIMEOUT_MS / 60_000} minutes: ${started.url}`);
}

/**
 * Release the pushed HEAD through the Release workflow, which builds and signs every system's
 * packages from that one commit and publishes them together.
 */
async function ship(options) {
  const version = appVersion();
  const tag = `v${version}`;
  const slug = repositorySlug();
  if (capture("git", ["status", "--porcelain"]).stdout) fail("commit your changes first: the workflow builds the pushed commit");
  const secrets = capture("gh", ["secret", "list", "-R", slug, "--json", "name"]);
  if (!secrets.ok) fail(`cannot list the repository secrets: ${secrets.stderr}`);
  if (!JSON.parse(secrets.stdout).some((secret) => secret.name === SIGNING_SECRET)) {
    fail(`the ${SIGNING_SECRET} secret is not set, so the workflow cannot sign the installers; the owner sets it once (README → Releases → Signing key)`);
  }
  run("git", ["fetch", "--quiet", "origin"]);
  const head = capture("git", ["rev-parse", "HEAD"]).stdout;
  if (!capture("git", ["branch", "-r", "--contains", head]).stdout) fail(`HEAD ${head.slice(0, 7)} is not pushed; push it before shipping`);
  const tagged = remoteTagCommit(tag);
  if (tagged && tagged !== head) fail(`${tag} already points at ${tagged.slice(0, 7)}, not HEAD ${head.slice(0, 7)}; bump the version`);
  const state = releaseState(slug, tag, version);
  if (state && !state.isDraft) fail(`${tag} is already published; bump the version for a new release`);
  if (options["notes-file"]) {
    const notes = resolve(options["notes-file"]);
    if (state) run("gh", ["release", "edit", tag, "-R", slug, "--notes-file", notes]);
    else run("gh", ["release", "create", tag, "-R", slug, "--draft", "--title", `${PRODUCT} ${version}`, "--notes-file", notes, "--target", head]);
  }
  if (!tagged) run("git", ["push", "origin", `${head}:refs/tags/${tag}`]);
  await waitForRun(slug, tag, head);
  await verifyPublished(slug, version);
}

async function verifyPublished(slug, version) {
  const url = `https://github.com/${slug}/releases/latest/download/latest.json`;
  for (let attempt = 1; attempt <= PUBLISHED_POLL_ATTEMPTS; attempt += 1) {
    const response = await fetch(url, { headers: { accept: "application/json" } }).catch(() => null);
    const body = response?.ok ? await response.json().catch(() => null) : null;
    if (body?.version === version) {
      const missing = missingTargets(body);
      if (missing.length > 0) fail(`${url} announces ${version} without ${missing.join(", ")}, so those copies stay behind`);
      for (const [target, platform] of Object.entries(body.platforms)) {
        const asset = await fetch(platform.url, { method: "HEAD" }).catch(() => null);
        if (!asset?.ok) fail(`${target} points at ${platform.url}, which answers ${asset?.status ?? "nothing"}`);
      }
      console.log(`Installed apps now see ${version}: ${url} lists ${Object.keys(body.platforms).join(", ")}.`);
      return;
    }
    await sleep(PUBLISHED_POLL_DELAY_MS);
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
  requireRestInDraft(options, version, ["nsis", "deb", "appimage"]);
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
  const expect = options["skip-linux"] ? ["nsis"] : ["nsis", "deb", "appimage"];
  const assembled = assemble({ ...options, tag, out: join(target, "release-assets", tag), expect }, bundles);
  if (options.publish) await publish({ ...options, tag, dir: assembled.out });
}

/**
 * Apple's tools first on PATH: the bundler runs `xattr -crs`, which the `xattr` a pip install can put
 * ahead of /usr/bin does not understand. CI skips the DMG's Finder styling, which would otherwise ask
 * for permission to control Finder.
 */
function macBuildEnv() {
  return {
    ...process.env,
    PATH: ["/usr/bin", "/bin", "/usr/sbin", "/sbin", process.env.PATH ?? ""].join(":"),
    CI: "true",
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "",
  };
}

async function mac(options) {
  if (process.platform !== "darwin") fail("mac releases run on macOS");
  const version = appVersion();
  if (!options["allow-dirty"] && capture("git", ["status", "--porcelain"]).stdout) {
    fail("commit your changes first: a release is built from the committed tree");
  }
  requireRestInDraft(options, version, ["app"]);
  const env = macBuildEnv();
  if (!env.TAURI_SIGNING_PRIVATE_KEY) {
    if (!existsSync(DEFAULT_KEY)) fail(`no signing key: set TAURI_SIGNING_PRIVATE_KEY or create ${DEFAULT_KEY}`);
    env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(DEFAULT_KEY, "utf8").trim();
  }
  const bundle = join(targetDir(), "release", "bundle");
  rmSync(join(bundle, "macos"), { recursive: true, force: true });
  rmSync(join(bundle, "dmg"), { recursive: true, force: true });
  run("pnpm", ["tauri", "build", "--ci", "--bundles", "app,dmg", "--config", UPDATER_CONFIG], { env });
  stageMacos(bundle, version);
  const tag = `v${version}`;
  const out = join(targetDir(), "release-assets", `${tag}-macos`);
  const assembled = assemble({ ...options, tag, out, expect: ["app"] }, [join(bundle, "macos"), join(bundle, "dmg")]);
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
    case "stage-macos":
      console.log(stageMacos(resolve(values.dir ?? join(targetDir(), "release", "bundle")), appVersion(values.tag)));
      return;
    case "mac":
      await mac(values);
      return;
    case "ship":
      await ship(values);
      return;
    default:
      fail("usage: node scripts/release.mjs version|assemble|publish|local|stage-macos|mac|ship [options] (see the header of this file)");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main().catch((error) => {
    console.error(`release: ${error instanceof ReleaseError ? error.message : error.stack}`);
    process.exit(1);
  });
}
