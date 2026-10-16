import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";
import { acquireConfigLock, MAX_BYTES, runCommand, syncDevices, validateConfig, validateEnvelope } from "./sync-devices.mjs";

const envelope = () => Buffer.from(JSON.stringify({ version: 1, deviceId: randomUUID(), fields: {}, accounts: {}, billing: {} }));
const report = () => Buffer.from(JSON.stringify({ changed: false, revision: 1, settingsChanged: false, layoutChanged: false, billingApplied: 0, billingPending: 0, pendingAccounts: 0 }));
const command = (source) => ({ command: process.execPath, args: ["-e", source] });
const config = (count) => ({ version: 1, peers: Array.from({ length: count }, (_, index) => ({ name: `peer-${index}`, export: { command: "fixture", args: [String(index), "export"] }, import: { command: "fixture", args: [String(index), "import"] } })) });

test("an offline peer does not prevent the available peers from exchanging state", async () => {
  const states = [envelope(), envelope()];
  const imports = [];
  const summary = await syncDevices(config(3), { run: async (spec, input) => {
    const index = Number(spec.args[0]);
    if (index === 2) return { ok: false, reason: "command_failed" };
    if (spec.args[1] === "export") return { ok: true, stdout: states[index] };
    imports.push({ index, input });
    return { ok: true, stdout: report() };
  } });
  assert.equal(summary.status, "partial");
  assert.equal(summary.availablePeers, 2);
  assert.equal(summary.importsSucceeded, 2);
  assert.deepEqual(imports.map(({ index }) => index).sort(), [0, 1]);
  assert.equal(imports.find(({ index }) => index === 0).input, states[1]);
});

test("each destination imports sequentially while healthy destinations keep progressing", async () => {
  const states = [envelope(), envelope(), envelope()];
  const running = new Set();
  const summary = await syncDevices(config(3), { run: async (spec) => {
    const index = Number(spec.args[0]);
    if (spec.args[1] === "export") return { ok: true, stdout: states[index] };
    assert.equal(running.has(index), false);
    running.add(index);
    await new Promise((done) => setTimeout(done, 5));
    running.delete(index);
    return index === 2 ? { ok: false, reason: "command_failed" } : { ok: true, stdout: report() };
  } });
  assert.equal(summary.importsSucceeded, 4);
  assert.equal(summary.importsFailed, 2);
  assert.equal(running.size, 0);
});

test("unknown and malformed exports are rejected without forwarding or logging data", async () => {
  const secret = "PRIVATE-ACCOUNT-CONTENT";
  const raw = { version: 1, deviceId: randomUUID(), fields: {}, accounts: {}, billing: {}, secret };
  const summary = await syncDevices(config(2), { run: async () => ({ ok: true, stdout: Buffer.from(JSON.stringify(raw)) }) });
  assert.equal(summary.availablePeers, 0);
  assert.equal(summary.importsSucceeded, 0);
  assert.equal(JSON.stringify(summary).includes(secret), false);
  assert.throws(() => validateEnvelope(Buffer.from("broken json")));
  assert.throws(() => validateEnvelope(Buffer.alloc(MAX_BYTES + 1)));
  assert.throws(() => validateEnvelope(Buffer.from(JSON.stringify({ ...raw, version: 2 }))));
});

test("duplicate device identities are never mistaken for two devices", async () => {
  const state = envelope();
  const summary = await syncDevices(config(2), { run: async () => ({ ok: true, stdout: state }) });
  assert.equal(summary.availablePeers, 1);
  assert.equal(summary.importsSucceeded, 0);
  assert.equal(summary.failures[0].reason, "duplicate_device");
});

test("configuration permits only bounded explicit process arguments", () => {
  assert.equal(validateConfig(config(2)).timeoutMs, 45_000);
  assert.throws(() => validateConfig(config(1)));
  assert.throws(() => validateConfig({ ...config(2), shell: true }));
  const invalid = config(2);
  invalid.peers[0].export.command = "bad\0command";
  assert.throws(() => validateConfig(invalid));
});

test("subprocess timeouts and oversized output are bounded and do not expose stderr", async () => {
  const started = Date.now();
  const timeout = await runCommand(command("setInterval(() => {}, 1000)"), Buffer.alloc(0), { timeoutMs: 100 });
  assert.deepEqual(timeout, { ok: false, reason: "timeout" });
  assert.ok(Date.now() - started < 5000);
  const large = await runCommand(command("process.stdout.write('x'.repeat(8192)); setInterval(() => {}, 1000)"), Buffer.alloc(0), { timeoutMs: 5000, maxBytes: 1024 });
  assert.deepEqual(large, { ok: false, reason: "output_limit" });
  const failed = await runCommand(command("process.stderr.write('SECRET-TOKEN'); process.exit(7)"), Buffer.alloc(0), { timeoutMs: 5000 });
  assert.deepEqual(failed, { ok: false, reason: "command_failed" });
  assert.equal(JSON.stringify(failed).includes("SECRET"), false);
});

test("process stdin is forwarded verbatim without shell interpretation", async () => {
  const input = Buffer.from("literal `$() & text\nsecond line");
  const output = await runCommand(command("process.stdin.pipe(process.stdout)"), input, { timeoutMs: 5000 });
  assert.equal(output.ok, true);
  assert.deepEqual(output.stdout, input);
});

test("the per-config guard rejects overlaps and recovers an exited owner's lock", async () => {
  const directory = await mkdtemp(join(tmpdir(), "quota-sync-test-"));
  const path = join(directory, "config.json");
  try {
    await writeFile(path, JSON.stringify(config(2)), { mode: 0o600 });
    const release = await acquireConfigLock(path);
    try { await assert.rejects(acquireConfigLock(path)); } finally { await release(); }
    await writeFile(`${path}.lock`, JSON.stringify({ version: 1, pid: 2_147_483_647, nonce: randomUUID() }), { mode: 0o600 });
    const next = await acquireConfigLock(path);
    const owner = JSON.parse(await readFile(`${path}.lock`, "utf8"));
    assert.equal(owner.pid, process.pid);
    await next();
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test("reserved lock ports retry deterministically but an occupied port never permits overlap", async () => {
  const directory = await mkdtemp(join(tmpdir(), "quota-sync-test-"));
  const path = join(directory, "config.json");
  try {
    await writeFile(path, JSON.stringify(config(2)), { mode: 0o600 });
    const attempted = [];
    const createGuard = (failure) => (listener) => {
      const server = createServer(listener);
      const listen = server.listen.bind(server);
      server.listen = (options, done) => {
        attempted.push(options.port);
        if (attempted.length === 1) {
          queueMicrotask(() => server.emit("error", Object.assign(new Error("fixture"), { code: failure })));
          return server;
        }
        return listen(options, done);
      };
      return server;
    };
    const release = await acquireConfigLock(path, { createGuard: createGuard("EACCES") });
    assert.equal(attempted.length, 2);
    assert.ok(attempted.every((port) => port >= 10_000 && port < 20_000));
    await release();
    const firstPort = attempted[0];
    attempted.length = 0;
    await assert.rejects(acquireConfigLock(path, { createGuard: createGuard("EADDRINUSE") }), { code: "EADDRINUSE" });
    assert.deepEqual(attempted, [firstPort]);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
