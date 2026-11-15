#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { download, fetchVerifiedAsset, loadConfig, targetKeys } from './lib.mjs';

const config = loadConfig();
const pkg = JSON.parse(readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), 'package.json'), 'utf8'));

const HELP = `${config.product} installer

Usage:
  npx ${pkg.name}                     Download, verify and install the latest ${config.product}
  npx ${pkg.name} --download <folder> Only download and verify the installer into <folder>
  npx ${pkg.name} --version           Print this package version

Downloads: ${config.releasesPage}
`;

function hasCommand(command) {
  return spawnSync('sh', ['-c', `command -v ${command}`], { stdio: 'ignore' }).status === 0;
}

function run(command, args) {
  const result = spawnSync(command, args, { stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with ${result.status}`);
}

async function installWithMacScript() {
  const dir = mkdtempSync(path.join(tmpdir(), 'npx-installer-'));
  try {
    const file = path.join(dir, 'install-macos.sh');
    writeFileSync(file, await download(config.macScript), { mode: 0o755 });
    run('bash', [file]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function installAppImage(asset) {
  const binDir = path.join(homedir(), '.local', 'bin');
  mkdirSync(binDir, { recursive: true });
  const target = path.join(binDir, config.appImageName);
  writeFileSync(target, asset.data);
  chmodSync(target, 0o755);
  console.log(`Installed ${target}`);
}

async function install(keys, baseUrl) {
  const asset = await fetchVerifiedAsset(config, keys, { baseUrl });
  console.log(`Verified ${asset.name} (SHA-256 ${asset.sha256.slice(0, 12)}...)`);
  if (asset.key === 'linux-x64-appimage') return installAppImage(asset);
  const dir = mkdtempSync(path.join(tmpdir(), 'npx-installer-'));
  const file = path.join(dir, asset.name);
  writeFileSync(file, asset.data, { mode: 0o755 });
  try {
    if (asset.key === 'win32-x64') run(file, []);
    else if (asset.key === 'linux-x64-deb') run('sudo', ['apt-get', 'install', '-y', file]);
    else throw new Error(`Do not know how to install ${asset.name}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

async function downloadOnly(keys, folder, baseUrl) {
  const asset = await fetchVerifiedAsset(config, keys, { baseUrl });
  mkdirSync(folder, { recursive: true });
  const file = path.resolve(folder, asset.name);
  writeFileSync(file, asset.data);
  console.log(`Downloaded and verified ${file}`);
}

async function main(argv) {
  if (argv.includes('-h') || argv.includes('--help')) return void process.stdout.write(HELP);
  if (argv.includes('--version')) return void console.log(pkg.version);
  const baseUrl = process.env.NPX_INSTALLER_BASE_URL || config.baseUrl;
  const linux = process.platform === 'linux';
  const keys = targetKeys(process.platform, process.arch, linux && hasCommand('apt-get') && hasCommand('dpkg'));
  if (!keys.length) throw new Error(`${config.product} has no build for ${process.platform}/${process.arch}`);
  const index = argv.indexOf('--download');
  if (index !== -1) {
    if (!argv[index + 1]) throw new Error('--download needs a folder');
    return downloadOnly(keys, argv[index + 1], baseUrl);
  }
  if (process.platform === 'darwin' && config.macScript) return installWithMacScript();
  return install(keys, baseUrl);
}

main(process.argv.slice(2)).catch((error) => {
  console.error(`${config.product}: ${error.message}`);
  process.exit(1);
});
