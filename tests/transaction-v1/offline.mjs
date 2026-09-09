import assert from 'node:assert/strict';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFileSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import * as kit from '@solana/kit';

const directory = fileURLToPath(new URL('../fixtures/transaction-v1/', import.meta.url));
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const PAYER = 'AKnL4NNf3DGWZJS6cPknBuEGnVsV4A4m5tgebLHaRSZ9';
const COSIGNER = '9hSR6S7WPtxmTojgo6GG3k4yDPecgJY292j7xrsUGWBu';
const MEMO = 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr';
const cases = {
  legacy: { version: 'legacy', bytes: 172, data: [1, 2, 3] },
  v0: { version: 0, bytes: 174, data: [1, 2, 3] },
  v1: { version: 1, bytes: 177, data: [1, 2, 3] },
  v1_oversize: { version: 1, bytes: 1574, data: new Array(1400).fill(7) },
  v1_two_signatures: { version: 1, bytes: 273, data: [9], signers: [PAYER, COSIGNER] },
  v1_zero_config: {
    version: 1, data: [1, 2, 3],
    config: { priorityFeeLamports: 0n, computeUnitLimit: 0, loadedAccountsDataSizeLimit: 0, heapSize: 32768 },
  },
  v1_max_fee: { version: 1, data: [1, 2, 3], config: { priorityFeeLamports: 18446744073709551615n } },
  v1_4096: { version: 1, bytes: 4096, data: new Array(3922).fill(7) },
  v1_4097: { version: 1, bytes: 4097, data: new Array(3923).fill(7) },
};

function checkCorpus(corpus, provenance) {
  assert.deepEqual(Object.keys(corpus.fixtures).sort(), Object.keys(cases).sort(), 'required fixture cases');
  assert.deepEqual(Object.keys(provenance.fixtures).sort(), Object.keys(cases).sort(), 'fixture digests');
  assert.equal(corpus.payer, PAYER);
  assert.equal(corpus.cosigner, COSIGNER);
  const summaries = [];
  const semanticControls = [];
  for (const [name, expected] of Object.entries(cases)) {
    const fixture = corpus.fixtures[name];
    const wire = Buffer.from(fixture.base64, 'base64');
    assert.equal(wire.toString('base64'), fixture.base64, `${name}: canonical base64`);
    assert.equal(sha256(wire), provenance.fixtures[name].sha256, `${name}: SHA-256`);
    assert.equal(wire.length, fixture.bytes, `${name}: declared size`);
    if (expected.bytes) assert.equal(wire.length, expected.bytes, `${name}: expected size`);
    const transaction = kit.getTransactionDecoder().decode(wire);
    assert.deepEqual(Buffer.from(kit.getTransactionEncoder().encode(transaction)), wire, `${name}: exact wire round trip`);
    const compiled = kit.getCompiledTransactionMessageDecoder().decode(transaction.messageBytes);
    assert.equal(compiled.version, expected.version, `${name}: decoded version`);
    assert.equal(fixture.version, expected.version, `${name}: declared version`);
    const message = kit.decompileTransactionMessage(compiled);
    assert.deepEqual(message.config ?? {}, expected.config ?? {}, `${name}: observed config including presence`);
    assert.equal(message.feePayer.address, PAYER);
    assert.equal(message.lifetimeConstraint.blockhash, '11111111111111111111111111111111');
    const signers = expected.signers ?? [PAYER];
    assert.deepEqual(Object.keys(transaction.signatures), signers, `${name}: signer order`);
    assert.equal(fixture.signatureCount, signers.length, `${name}: required signatures`);
    assert.equal(compiled.header.numSignerAccounts, signers.length);
    assert.equal(kit.getSignatureFromTransaction(transaction), fixture.firstSignature, `${name}: reconciliation signature`);
    for (const address of signers) {
      const publicKey = createPublicKey({
        key: Buffer.concat([
          Buffer.from('302a300506032b6570032100', 'hex'),
          Buffer.from(kit.getAddressEncoder().encode(address)),
        ]), format: 'der', type: 'spki',
      });
      assert.ok(verify(null, transaction.messageBytes, publicKey, transaction.signatures[address]), `${name}: Ed25519 signature for ${address}`);
    }
    assert.deepEqual(compiled.staticAccounts, [...signers, MEMO], `${name}: account order`);
    assert.equal(message.instructions.length, 1);
    const instruction = message.instructions[0];
    assert.equal(instruction.programAddress, MEMO);
    assert.deepEqual(Array.from(instruction.data), expected.data, `${name}: instruction ABI`);
    assert.deepEqual(instruction.accounts ?? [], expected.signers?.map(address => ({ address, role: 3 })) ?? []);
    // Identical instructions across envelope versions; transaction version is
    // never inserted into a program argument, account list or discriminator.
    if (['legacy', 'v0', 'v1'].includes(name)) semanticControls.push(instruction);
    summaries.push({ name, version: compiled.version, bytes: wire.length,
      sha256: sha256(wire), signature: fixture.firstSignature, signaturesVerified: signers.length,
      config: message.config ?? {}, withinWireSizeLimit: wire.length <= (compiled.version === 1 ? 4096 : 1232) });
  }
  assert.deepEqual(semanticControls[0], semanticControls[1]);
  assert.deepEqual(semanticControls[1], semanticControls[2]);
  // This is fixture classification only. Relay/adapter rejection is a separate
  // acceptance gate and must exercise those implementations.
  assert.equal(summaries.find(x => x.name === 'v1_4096').withinWireSizeLimit, true);
  assert.equal(summaries.find(x => x.name === 'v1_4097').withinWireSizeLimit, false);
  return summaries;
}

function selfTest(corpus, provenance) {
  const temporary = mkdtempSync(join(tmpdir(), 'arete-v1-corruption-'));
  const results = [];
  try {
    for (const kind of ['hash', 'signature', 'missing-control']) {
      const corrupted = structuredClone(corpus);
      const digests = structuredClone(provenance);
      if (kind === 'missing-control') delete corrupted.fixtures.legacy;
      else {
        const wire = Buffer.from(corrupted.fixtures.v1_two_signatures.base64, 'base64');
        wire[wire.length - 1] ^= 1; // Corrupt the second signature, not the payer.
        corrupted.fixtures.v1_two_signatures.base64 = wire.toString('base64');
        if (kind === 'signature') digests.fixtures.v1_two_signatures.sha256 = sha256(wire);
      }
      const path = join(temporary, 'case.json');
      writeFileSync(path, JSON.stringify({ corpus: corrupted, provenance: digests }));
      const child = spawnSync(process.execPath, [fileURLToPath(import.meta.url), '--check-corruption', path], {
        encoding: 'utf8', timeout: 15_000,
      });
      if (child.error) throw child.error;
      assert.equal(child.status, 1, `${kind}: corrupted child must exit nonzero`);
      const failure = JSON.parse(child.stdout);
      assert.equal(failure.status, 'failed');
      const expectedError = { hash: 'SHA-256', signature: 'Ed25519 signature', 'missing-control': 'required fixture cases' }[kind];
      assert.ok(failure.error.includes(expectedError), `${kind}: must fail for the intended reason: ${failure.error}`);
      results.push({ case: kind, exitCode: child.status, detected: true });
    }
  } finally { rmSync(temporary, { recursive: true, force: true }); }
  return results;
}

try {
  if (process.argv[2] === '--check-corruption') {
    const { corpus, provenance } = JSON.parse(readFileSync(process.argv[3]));
    checkCorpus(corpus, provenance);
    throw new Error('Corruption was accepted');
  }
  assert.equal(process.argv.length, 2, 'unexpected arguments');
  const provenance = JSON.parse(readFileSync(join(directory, 'provenance.json')));
  assert.equal(provenance.schemaVersion, 1);
  assert.deepEqual(provenance.codec, { name: '@solana/kit', version: '8.2.0' });
  const installed = JSON.parse(readFileSync(new URL('./node_modules/@solana/kit/package.json', import.meta.url)));
  assert.equal(installed.version, provenance.codec.version, 'installed codec version');
  for (const name of ['generate.mjs', 'transactions.json']) {
    assert.equal(sha256(readFileSync(join(directory, name))), provenance.files[name], `${name}: provenance digest`);
  }
  const corpus = JSON.parse(readFileSync(join(directory, 'transactions.json')));
  const fixtures = checkCorpus(corpus, provenance);
  const negativeTests = selfTest(corpus, provenance);
  console.log(JSON.stringify({ status: 'passed', scope: 'offline-codec-fixtures',
    codec: provenance.codec, node: process.version, fixtures, negativeTests,
  }, (_, value) => typeof value === 'bigint' ? value.toString() : value));
} catch (error) {
  console.log(JSON.stringify({ status: 'failed', error: error.message }));
  process.exitCode = 1;
}
