import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const checker = fileURLToPath(new URL('./check-example-sdk-pins.mjs', import.meta.url));

function check(t, extensions) {
  const dir = mkdtempSync(join(tmpdir(), 'arete-sdk-pins-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const manifest = join(dir, 'stack-manifest.json');
  writeFileSync(manifest, JSON.stringify({ kind: 'stack-manifest', artifactHash: 'current' }));
  const paths = extensions.map((value, index) => {
    const path = join(dir, `extensions-${index}.json`);
    writeFileSync(path, JSON.stringify(value));
    return path;
  });
  return spawnSync(process.execPath, [checker, manifest, ...paths], { encoding: 'utf8' });
}

test('accepts matching TypeScript and Rust source pins', (t) => {
  const pin = { inputKind: 'stack-manifest', inputHash: 'current' };
  assert.equal(check(t, [pin, { ...pin, language: 'rust' }]).status, 0);
});

test('reports every stale pin in one failure', (t) => {
  const result = check(t, [
    { inputKind: 'stack-manifest', inputHash: 'intermediate' },
    { inputKind: 'stack-manifest', inputHash: 'original' },
  ]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /extensions-0.json:.*intermediate/);
  assert.match(result.stderr, /extensions-1.json:.*original/);
  assert.match(result.stderr, /Review extension compatibility/);
});

test('rejects missing pins and the wrong artifact kind', (t) => {
  const result = check(t, [{}, { inputKind: 'program-spec', inputHash: 'current' }]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /missing hash/);
  assert.match(result.stderr, /found program-spec/);
});
