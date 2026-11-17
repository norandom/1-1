import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';

const platforms = ['windows-x86_64', 'windows-aarch64', 'darwin-aarch64', 'linux-x86_64'];
const targets = { 'windows-x86_64': 'x86_64-pc-windows-msvc', 'windows-aarch64': 'aarch64-pc-windows-msvc', 'darwin-aarch64': 'aarch64-apple-darwin', 'linux-x86_64': 'x86_64-unknown-linux-gnu' };
export function createManifest(version, assets) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) throw new Error('Invalid release version');
  if (!assets.length) throw new Error('No signed update assets');
  const result = { version, notes: '1-1 preview update. See the GitHub release notes.', pub_date: new Date().toISOString(), platforms: {} };
  for (const asset of assets) {
    if (!platforms.includes(asset.platform) || result.platforms[asset.platform]) throw new Error('Unknown or duplicate platform');
    if (!asset.signature?.trim() || basename(asset.name) !== asset.name) throw new Error('Missing signature or invalid asset filename');
    const info = asset.metadata;
    if (!info || info.version !== version || info.target !== targets[asset.platform] || info.asset !== asset.name || !/^[a-f0-9]{64}$/.test(asset.sha256 ?? '') || info.sha256 !== asset.sha256) throw new Error('Build metadata does not match version, platform or installer bytes');
    result.platforms[asset.platform] = { signature: asset.signature.trim(), url: `https://github.com/norandom/1-1/releases/download/v${version}/${encodeURIComponent(asset.name)}` };
  }
  return result;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [version, directory, output] = process.argv.slice(2);
  if (!version || !directory || !output) throw new Error('Usage: node scripts/update-manifest.mjs VERSION ASSET_DIRECTORY OUTPUT_JSON');
  const assets = [];
  const available = readdirSync(directory);
  for (const platform of platforms.filter(p => available.includes(p))) {
    const folder = join(directory, platform);
    const candidates = readdirSync(folder).filter(n => n.endsWith('.sig') && (n.endsWith('.exe.sig') || n.endsWith('.app.tar.gz.sig') || n.endsWith('.AppImage.sig')));
    if (candidates.length !== 1) throw new Error(`Expected one signed installer for ${platform}`);
    const name = candidates[0].slice(0, -4);
    if (!statSync(join(folder, name)).size) throw new Error(`Empty installer for ${platform}`);
    assets.push({ platform, name, signature: readFileSync(join(folder, `${name}.sig`), 'utf8'), sha256: createHash('sha256').update(readFileSync(join(folder, name))).digest('hex'), metadata: JSON.parse(readFileSync(join(folder, 'build-info.json'), 'utf8').replace(/^\uFEFF/, '')) });
  }
  writeFileSync(output, JSON.stringify(createManifest(version, assets), null, 2) + '\n');
}
