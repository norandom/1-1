import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createManifest } from './update-manifest.mjs';
const entry = (overrides = {}) => ({ platform: 'windows-x86_64', name: '1-1 setup.exe', signature: 'signed-data', sha256: 'a'.repeat(64), metadata: { version: '0.1.0-alpha.3', target: 'x86_64-pc-windows-msvc', asset: '1-1 setup.exe', sha256: 'a'.repeat(64) }, ...overrides });

test('manifest contains only supplied signed platform assets', () => {
  const result = createManifest('0.1.0-alpha.3', [entry()]);
  assert.deepEqual(Object.keys(result.platforms), ['windows-x86_64']);
  assert.equal(result.platforms['windows-x86_64'].url, 'https://github.com/norandom/1-1/releases/download/v0.1.0-alpha.3/1-1%20setup.exe');
  assert.equal(result.platforms['windows-x86_64'].signature, 'signed-data');
});
test('reject stale versions, wrong architectures and modified installer bytes', () => {
  assert.throws(() => createManifest('0.1.0-alpha.4', [entry()]));
  assert.throws(() => createManifest('0.1.0-alpha.3', [entry({ platform: 'windows-aarch64' })]));
  assert.throws(() => createManifest('0.1.0-alpha.3', [entry({ sha256: 'b'.repeat(64) })]));
});
test('reject unsigned, duplicate and unknown platforms and invalid versions', () => {
  for (const assets of [[], [{ platform: 'windows-x86_64', name: 'a.exe', signature: '' }], [{platform:'unknown', name:'a.exe', signature:'s'}], [{platform:'windows-x86_64',name:'a.exe',signature:'s'},{platform:'windows-x86_64',name:'b.exe',signature:'s'}]]) {
    assert.throws(() => createManifest('0.1.0-alpha.3', assets));
  }
  assert.throws(() => createManifest('../bad', [{ platform: 'windows-x86_64', name: 'a.exe', signature:'s' }]));
});
