// Explicit maintenance command. Acceptance never regenerates its expectations.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const fixtures = new URL('../fixtures/transaction-v1/', import.meta.url);
const generator = new URL('generate.mjs', fixtures);
const transactions = new URL('transactions.json', fixtures);
const installed = JSON.parse(readFileSync(new URL('./node_modules/@solana/kit/package.json', import.meta.url)));
assert.equal(installed.version, '8.2.0', 'regeneration requires the pinned codec');
const source = readFileSync(generator, 'utf8').replace(
  "'@solana/kit'", JSON.stringify(import.meta.resolve('@solana/kit')),
);
const generated = spawnSync(process.execPath, ['--input-type=module', '--eval', source], {
  encoding: 'utf8', timeout: 30_000,
});
if (generated.error) throw generated.error;
assert.equal(generated.status, 0, generated.stderr);
const previous = JSON.parse(readFileSync(transactions));
const next = JSON.parse(generated.stdout);
// These are consumed by the relay's existing tests. Never silently replace them.
for (const name of ['legacy', 'v0', 'v1', 'v1_oversize', 'v1_two_signatures']) {
  assert.deepEqual(next.fixtures[name], previous.fixtures[name], `${name} changed`);
}
writeFileSync(transactions, generated.stdout);
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
writeFileSync(new URL('provenance.json', fixtures), JSON.stringify({
  schemaVersion: 1,
  codec: { name: '@solana/kit', version: '8.2.0' },
  files: {
    'generate.mjs': sha256(readFileSync(generator)),
    'transactions.json': sha256(readFileSync(transactions)),
  },
  fixtures: Object.fromEntries(Object.entries(next.fixtures).map(([name, fixture]) => [
    name, { sha256: sha256(Buffer.from(fixture.base64, 'base64')) },
  ])),
}, null, 2) + '\n');
