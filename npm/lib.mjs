import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));

export function loadConfig(file = path.join(HERE, 'installer.json')) {
  return JSON.parse(readFileSync(file, 'utf8'));
}

export function parseChecksums(text) {
  const entries = new Map();
  for (const rawLine of text.replace(/\r/g, '').split('\n')) {
    const line = rawLine.trim();
    if (!line) continue;
    const match = /^([0-9a-f]{64})\s+\*?(\S+)$/i.exec(line);
    if (!match) throw new Error(`Malformed checksum line: ${line}`);
    entries.set(match[2], match[1].toLowerCase());
  }
  return entries;
}

export function targetKeys(platform, arch, hasApt) {
  if (platform === 'win32' && arch === 'x64') return ['win32-x64'];
  if (platform === 'darwin') return [`darwin-${arch}`];
  if (platform === 'linux' && arch === 'x64') {
    return hasApt ? ['linux-x64-deb', 'linux-x64-appimage'] : ['linux-x64-appimage'];
  }
  return [];
}

export function pickAsset(config, checksums, keys) {
  for (const key of keys) {
    const source = config.assets?.[key];
    if (!source) continue;
    const pattern = new RegExp(source);
    const names = [...checksums.keys()].filter((name) => pattern.test(name));
    if (names.length > 1) throw new Error(`More than one ${key} file matches in ${config.checksums}`);
    if (names.length === 1) return { key, name: names[0], sha256: checksums.get(names[0]) };
  }
  return null;
}

export function sha256(buffer) {
  return createHash('sha256').update(buffer).digest('hex');
}

export async function download(url, fetchImpl = globalThis.fetch) {
  const response = await fetchImpl(url, { redirect: 'follow', signal: AbortSignal.timeout(300_000) });
  if (!response.ok) throw new Error(`Download failed (${response.status}) for ${url}`);
  return Buffer.from(await response.arrayBuffer());
}

export async function fetchVerifiedAsset(config, keys, { baseUrl = config.baseUrl, fetchImpl } = {}) {
  const sums = parseChecksums((await download(`${baseUrl}/${config.checksums}`, fetchImpl)).toString('utf8'));
  const asset = pickAsset(config, sums, keys);
  if (!asset) throw new Error(`${config.product} has no build for this computer yet. See ${config.releasesPage}`);
  const data = await download(`${baseUrl}/${asset.name}`, fetchImpl);
  if (sha256(data) !== asset.sha256) throw new Error(`Checksum mismatch for ${asset.name}`);
  return { ...asset, data };
}
