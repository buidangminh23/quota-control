import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fetchVerifiedAsset, loadConfig, parseChecksums, pickAsset, sha256, targetKeys } from './lib.mjs';

const config = loadConfig();

test('installer.json names a product, checksum file and valid asset patterns', () => {
  assert.ok(config.product);
  assert.ok(config.checksums);
  assert.ok(Object.values(config.assets).some(Boolean));
  for (const pattern of Object.values(config.assets)) if (pattern) new RegExp(pattern);
});

test('each asset pattern matches its sample file name', () => {
  for (const [key, sample] of Object.entries(config.samples)) {
    const sums = parseChecksums(`${'a'.repeat(64)}  ${sample}\n`);
    assert.equal(pickAsset(config, sums, [key])?.name, sample, key);
  }
});

test('parses checksum files with CRLF and rejects malformed lines', () => {
  assert.equal(parseChecksums(`${'b'.repeat(64)}  x.exe\r\n`).get('x.exe'), 'b'.repeat(64));
  assert.throws(() => parseChecksums('nope  file'), /Malformed/);
});

test('prefers the Debian package on apt systems and the AppImage elsewhere', () => {
  assert.deepEqual(targetKeys('linux', 'x64', true), ['linux-x64-deb', 'linux-x64-appimage']);
  assert.deepEqual(targetKeys('linux', 'x64', false), ['linux-x64-appimage']);
  assert.deepEqual(targetKeys('linux', 'arm64', true), []);
});

test('rejects a download whose checksum does not match', async () => {
  const [key, sample] = Object.entries(config.samples)[0];
  const body = Buffer.from('payload');
  const files = { [config.checksums]: `${sha256(body)}  ${sample}\n`, [sample]: body };
  const fetchImpl = async (url) => {
    const name = decodeURIComponent(url.split('/').pop());
    return name in files ? new Response(files[name]) : new Response('missing', { status: 404 });
  };
  const asset = await fetchVerifiedAsset(config, [key], { baseUrl: 'https://example.test', fetchImpl });
  assert.equal(asset.name, sample);
  files[sample] = Buffer.from('tampered');
  await assert.rejects(fetchVerifiedAsset(config, [key], { baseUrl: 'https://example.test', fetchImpl }), /Checksum mismatch/);
});
