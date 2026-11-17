import { readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { createHash } from 'node:crypto';
const [target, asset] = process.argv.slice(2);
if (!['x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc', 'aarch64-apple-darwin', 'x86_64-unknown-linux-gnu'].includes(target) || !asset) throw new Error('Expected supported build target and installer path');
const { version } = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8').replace(/^\uFEFF/, ''));
const sha256 = createHash('sha256').update(readFileSync(asset)).digest('hex');
writeFileSync(join(dirname(asset), 'build-info.json'), JSON.stringify({ version, target, asset: basename(asset), sha256 }, null, 2) + '\n');
