import assert from 'node:assert/strict';
import { readFile, stat } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';

async function main() {
  function required(name: string): string {
    const value = process.env[name];
    assert(value, `Missing ${name}. See tests/transaction-v1/README.md; this smoke requires an already-running local stack.`);
    return value;
  }
  const relayUrl = required('A4_V1_RELAY_URL');
  const keypairPath = required('A4_V1_KEYPAIR');
  const ingestionLog = required('A4_V1_INGESTION_LOG');
  assert(['localhost', '127.0.0.1', '[::1]'].includes(new URL(relayUrl).hostname), 'Use a local Arete relay');
  const logStart = (await stat(ingestionLog)).size;
  const kit = await import('@solana/kit');
  const { buildInstruction, createTransactionTransport } = await import('@usearete/sdk');
  const { createWalletAdapter } = await import('../src/index');
  const { transferInstruction } = await import('./generated/system-core');
  const signer = await kit.createKeyPairSignerFromBytes(new Uint8Array(JSON.parse(await readFile(keypairPath, 'utf8'))));
  const relay = createTransactionTransport(relayUrl, (input, init) => fetch(input, {
    ...init,
    headers: { ...init.headers, ...(process.env.A4_V1_TOKEN ? { authorization: `Bearer ${process.env.A4_V1_TOKEN}` } : {}) },
    signal: AbortSignal.timeout(30_000),
  }));
  const submissions: { version: number; bytes: number }[] = [];
  const wallet = createWalletAdapter({ signer, transport: {
    ...relay,
    sendTransaction: async (wire, options) => {
      const bytes = Buffer.from(wire, 'base64');
      const tx = kit.getTransactionDecoder().decode(bytes);
      const message = kit.getCompiledTransactionMessageDecoder().decode(tx.messageBytes);
      assert(message.version === 0 || message.version === 1);
      assert(message.version === 1 ? bytes.length > 1232 && bytes.length <= 4096 : bytes.length <= 1232,
        `Unexpected v${message.version} transaction size: ${bytes.length}`);
      submissions.push({ version: message.version, bytes: bytes.length });
      return relay.sendTransaction(wire, options);
    },
  } });
  assert(wallet.supportedTransactionVersions.includes(1), 'The public Kit adapter must support V1');

  // Fresh recipients make stale state impossible. 32 ordinary transfers exceed
  // 1232 bytes without a deployed padding program or hand-built instruction.
  for (const version of [1, 0] as const) {
    const recipients = await Promise.all(Array.from({ length: version === 1 ? 32 : 1 }, () => kit.generateKeyPairSigner()));
    const instructions = recipients.map(recipient => buildInstruction(transferInstruction, {
      from: signer.address, to: recipient.address, lamports: 1_000_000n,
    }));
    const resources = {
      computeUnitLimit: 200_000, loadedAccountsDataSizeLimit: 1_048_576, heapSize: 32_768,
      ...(version === 1 ? { priorityFeeLamports: 7n } : {}),
    };
    // Both public adapter operations fetch fresh blockhashes through Arete.
    const inspected = await wallet.inspectTransaction(instructions, { transactionVersion: version, resources });
    assert.equal(inspected.error, undefined, `v${version} simulation failed: ${JSON.stringify(inspected.error)}`);
    const submissionIndex = submissions.length;
    const result = await wallet.signAndSend(instructions, { transactionVersion: version, resources, confirmationLevel: 'confirmed' });
    assert.equal(submissions.length, submissionIndex + 1, 'The adapter must submit once through Arete');
    const sent = submissions[submissionIndex];
    assert.equal(sent.version, version);
    const status = await relay.getSignatureStatus(result.signature, { commitment: 'confirmed', searchTransactionHistory: true });
    assert(status && ['confirmed', 'finalized'].includes(status.confirmationStatus ?? ''), 'Transaction did not confirm');
    assert.equal(status.err, null, 'Transaction execution failed');

    const deadline = Date.now() + 60_000;
    let observed = false;
    while (Date.now() < deadline) {
      const log = await readFile(ingestionLog);
      assert(log.length >= logStart, 'Ingestion log was rotated during the smoke');
      const rows = log.subarray(logStart).toString('utf8').split('\n').slice(0, -1).flatMap(line => {
        try { return [JSON.parse(line)]; } catch { return []; } // runtime diagnostics are not observations
      });
      const changes = rows.filter(row => row.context?.signature === result.signature && row.state?.signature === result.signature);
      if (recipients.every(recipient => changes.some(row => row.state.to === recipient.address))) {
        for (const row of changes) {
          assert.equal(row.event, 'system::TransferIxState');
          assert.equal(Number(row.state.lamports), 1_000_000);
          if (version === 1) {
            assert.deepEqual(row.context.solana_transaction, {
              version: 1, config: { compute_unit_limit: 200_000, loaded_accounts_data_size_limit: 1_048_576, heap_size: 32_768, priority_fee_lamports: '7' },
            });
          } else {
            // Yellowstone lacks an explicit version field for v0: unknown is
            // expected, and must not inherit V1 metadata from the previous send.
            assert.equal(row.context.solana_transaction, undefined);
          }
        }
        observed = true;
        break;
      }
      await delay(250);
    }
    assert(observed, `Timed out waiting for decoded transfer state and metadata for ${result.signature}`);
    console.log(`v${version}: simulated, submitted via Arete, confirmed and ingested (${sent.bytes} bytes): ${result.signature}`);
  }
}

await main().catch(error => { console.error(error.message); process.exitCode = 1; });
