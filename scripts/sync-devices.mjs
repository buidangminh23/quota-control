import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { lstat, open, realpath, readFile, unlink } from "node:fs/promises";
import { createServer } from "node:net";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const MAX_BYTES = 1_048_576;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const NIL_UUID = "00000000-0000-0000-0000-000000000000";
const ENVELOPE_KEYS = ["version", "deviceId", "fields", "accounts", "billing"];
const REPORT_KEYS = ["changed", "revision", "settingsChanged", "layoutChanged", "billingApplied", "billingPending", "pendingAccounts"];

function record(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function exactKeys(value, required, optional = []) {
  return record(value) && required.every((key) => Object.hasOwn(value, key))
    && Object.keys(value).every((key) => required.includes(key) || optional.includes(key));
}

function command(value) {
  return exactKeys(value, ["command", "args"]) && typeof value.command === "string"
    && value.command.length > 0 && value.command.length <= 8192 && !value.command.includes("\0")
    && Array.isArray(value.args) && value.args.length <= 128
    && value.args.every((arg) => typeof arg === "string" && arg.length <= 32768 && !arg.includes("\0"));
}

export function validateConfig(value) {
  if (!exactKeys(value, ["version", "peers"], ["timeoutMs"]) || value.version !== 1
    || !Array.isArray(value.peers) || value.peers.length < 2 || value.peers.length > 16
    || (value.timeoutMs !== undefined && (!Number.isInteger(value.timeoutMs) || value.timeoutMs < 100 || value.timeoutMs > 120_000))) {
    throw new Error("Invalid synchronization configuration");
  }
  const names = new Set();
  for (const peer of value.peers) {
    if (!exactKeys(peer, ["name", "export", "import"]) || typeof peer.name !== "string"
      || !/^[\p{L}\p{N} ._-]{1,64}$/u.test(peer.name) || names.has(peer.name)
      || !command(peer.export) || !command(peer.import)) {
      throw new Error("Invalid synchronization peer");
    }
    names.add(peer.name);
  }
  return { ...value, timeoutMs: value.timeoutMs ?? 45_000 };
}

export function validateEnvelope(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length > MAX_BYTES) throw new Error("Invalid synchronization payload");
  const value = JSON.parse(bytes.toString("utf8"));
  if (!exactKeys(value, ENVELOPE_KEYS) || value.version !== 1 || typeof value.deviceId !== "string"
    || !UUID.test(value.deviceId) || value.deviceId === NIL_UUID
    || !record(value.fields) || !record(value.accounts) || !record(value.billing)
    || Object.keys(value.fields).length > 4096 || Object.keys(value.accounts).length > 512 || Object.keys(value.billing).length > 512) {
    throw new Error("Unsupported synchronization payload");
  }
  return value;
}

function validateReport(bytes) {
  const value = JSON.parse(bytes.toString("utf8"));
  if (!exactKeys(value, REPORT_KEYS) || !["changed", "settingsChanged", "layoutChanged"].every((key) => typeof value[key] === "boolean")
    || !["revision", "billingApplied", "billingPending", "pendingAccounts"].every((key) => Number.isSafeInteger(value[key]) && value[key] >= 0)) {
    throw new Error("Invalid synchronization response");
  }
  return value;
}

async function stopTree(child) {
  if (!child.pid) return;
  if (process.platform === "win32") {
    if (child.exitCode !== null || child.signalCode !== null) return;
    await new Promise((done) => {
      const killer = spawn("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], { windowsHide: true, shell: false, stdio: "ignore" });
      const timer = setTimeout(() => { killer.kill(); child.kill(); done(); }, 3000);
      killer.once("error", () => { clearTimeout(timer); child.kill(); done(); });
      killer.once("close", () => { clearTimeout(timer); done(); });
    });
  } else {
    try { process.kill(-child.pid, "SIGKILL"); } catch { child.kill("SIGKILL"); }
  }
}

export async function runCommand(spec, input, { timeoutMs = 45_000, maxBytes = MAX_BYTES } = {}) {
  if (!command(spec) || input.length > MAX_BYTES) return { ok: false, reason: "invalid_command" };
  return new Promise((resolveResult) => {
    let child;
    try {
      child = spawn(spec.command, spec.args, { shell: false, windowsHide: true, detached: process.platform !== "win32", stdio: ["pipe", "pipe", "pipe"] });
    } catch {
      resolveResult({ ok: false, reason: "spawn_failed" });
      return;
    }
    let settled = false;
    let stopping = false;
    let bytes = 0;
    let stderrBytes = 0;
    const chunks = [];
    const finish = (result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.stdin.destroy();
      child.stdout.destroy();
      child.stderr.destroy();
      resolveResult(result);
    };
    const abort = (reason) => {
      if (settled || stopping) return;
      stopping = true;
      clearTimeout(timer);
      void stopTree(child).finally(() => finish({ ok: false, reason }));
    };
    const timer = setTimeout(() => abort("timeout"), timeoutMs);
    child.once("error", () => finish({ ok: false, reason: "spawn_failed" }));
    child.stdout.on("data", (chunk) => {
      bytes += chunk.length;
      if (bytes > maxBytes) abort("output_limit");
      else if (!stopping) chunks.push(chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes > maxBytes) abort("output_limit");
    });
    child.stdin.on("error", () => {});
    child.once("close", (code) => {
      if (stopping) return;
      finish(code === 0 ? { ok: true, stdout: Buffer.concat(chunks) } : { ok: false, reason: "command_failed" });
    });
    child.stdin.end(input);
  });
}

export async function syncDevices(rawConfig, { run = runCommand } = {}) {
  const config = validateConfig(rawConfig);
  const summary = { status: "complete", peerCount: config.peers.length, availablePeers: 0, importsSucceeded: 0, importsFailed: 0, changedImports: 0, failures: [] };
  const exports = await Promise.all(config.peers.map(async (peer, index) => {
    const output = await run(peer.export, Buffer.alloc(0), { timeoutMs: config.timeoutMs });
    if (!output.ok) {
      summary.failures.push({ peer: index, operation: "export", reason: output.reason });
      return null;
    }
    try {
      const envelope = validateEnvelope(output.stdout);
      return { index, bytes: output.stdout, deviceId: envelope.deviceId };
    } catch {
      summary.failures.push({ peer: index, operation: "export", reason: "invalid_payload" });
      return null;
    }
  }));
  const available = exports.filter(Boolean);
  const identities = new Set();
  const distinct = available.filter((entry) => {
    if (identities.has(entry.deviceId)) {
      summary.failures.push({ peer: entry.index, operation: "export", reason: "duplicate_device" });
      return false;
    }
    identities.add(entry.deviceId);
    return true;
  });
  summary.availablePeers = distinct.length;
  await Promise.all(distinct.map(async (destination) => {
    for (const source of distinct) {
      if (source.index === destination.index) continue;
      const output = await run(config.peers[destination.index].import, source.bytes, { timeoutMs: config.timeoutMs });
      let reason;
      if (!output.ok) reason = output.reason;
      else {
        try {
          const report = validateReport(output.stdout);
          summary.importsSucceeded += 1;
          if (report.changed) summary.changedImports += 1;
        } catch { reason = "invalid_response"; }
      }
      if (reason) {
        summary.importsFailed += 1;
        summary.failures.push({ peer: destination.index, operation: "import", reason });
      }
    }
  }));
  summary.failures.sort((a, b) => a.peer - b.peer || a.operation.localeCompare(b.operation));
  if (summary.failures.length > 0 || distinct.length < 2) summary.status = "partial";
  return summary;
}

function alive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return true;
  try { process.kill(pid, 0); return true; } catch (error) { return error.code !== "ESRCH"; }
}

async function regularFile(path) {
  const metadata = await lstat(path);
  if (!metadata.isFile() || metadata.isSymbolicLink()) throw new Error("Unsafe synchronization file");
  return metadata;
}

export async function acquireConfigLock(configPath, { createGuard = createServer } = {}) {
  const canonical = await realpath(configPath);
  await regularFile(canonical);
  const identity = process.platform === "win32" ? canonical.toLowerCase() : canonical;
  let server;
  for (let attempt = 0; attempt < 64; attempt += 1) {
    const port = 10_000 + createHash("sha256").update(`${identity}\0${attempt}`).digest().readUInt32BE(0) % 10_000;
    const candidate = createGuard((socket) => socket.destroy());
    try {
      await new Promise((done, reject) => {
        candidate.once("error", reject);
        candidate.listen({ host: "127.0.0.1", port, exclusive: true }, done);
      });
      server = candidate;
      break;
    } catch (error) {
      candidate.close();
      if (error.code !== "EACCES") throw Object.assign(new Error("Synchronization is already running or its lock is unavailable"), { code: error.code });
    }
  }
  if (!server) throw new Error("Synchronization lock is unavailable");
  const lockPath = `${canonical}.lock`;
  const owner = { version: 1, pid: process.pid, nonce: randomUUID() };
  let owned = false;
  try {
    try {
      const metadata = await regularFile(lockPath);
      if (metadata.size > 1024) throw new Error("Invalid synchronization lock");
      const previous = JSON.parse(await readFile(lockPath, "utf8"));
      if (!exactKeys(previous, ["version", "pid", "nonce"]) || previous.version !== 1 || !UUID.test(previous.nonce) || alive(previous.pid)) {
        throw new Error("Synchronization lock is still owned");
      }
      await unlink(lockPath);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    const file = await open(lockPath, "wx", 0o600);
    try { await file.writeFile(JSON.stringify(owner)); await file.sync(); } finally { await file.close(); }
    owned = true;
    return async () => {
      try {
        const current = JSON.parse(await readFile(lockPath, "utf8"));
        if (current.nonce === owner.nonce && current.pid === process.pid) await unlink(lockPath);
      } finally { await new Promise((done) => server.close(done)); }
    };
  } catch (error) {
    if (owned) await unlink(lockPath).catch(() => {});
    await new Promise((done) => server.close(done));
    throw error;
  }
}

async function readConfig(path) {
  const metadata = await regularFile(path);
  if (metadata.size > 65_536) throw new Error("Oversized synchronization configuration");
  const file = await open(path, "r");
  try {
    const bytes = Buffer.alloc(65_537);
    const { bytesRead } = await file.read(bytes, 0, bytes.length, 0);
    if (bytesRead > 65_536) throw new Error("Oversized synchronization configuration");
    return validateConfig(JSON.parse(bytes.subarray(0, bytesRead).toString("utf8")));
  } finally { await file.close(); }
}

export async function main(args = process.argv.slice(2)) {
  if (args.length !== 3 || args[0] !== "--config" || args[2] !== "--once") {
    process.stderr.write("Usage: node scripts/sync-devices.mjs --config <private-config.json> --once\n");
    return 2;
  }
  let release;
  let code = 2;
  try {
    const path = resolve(args[1]);
    const config = await readConfig(path);
    release = await acquireConfigLock(path);
    const summary = await syncDevices(config);
    process.stdout.write(`${JSON.stringify(summary)}\n`);
    code = summary.status === "complete" ? 0 : 1;
  } catch {
    process.stderr.write("Synchronization did not start; check its private configuration and running-instance lock.\n");
  } finally {
    if (release) {
      try { await release(); } catch {
        process.stderr.write("Synchronization lock could not be released cleanly.\n");
        code = 2;
      }
    }
  }
  return code;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = await main();
}
