import { expect, it } from 'vitest';
import {
  createKeyPairSignerFromPrivateKeyBytes,
  decompileTransactionMessage,
  getCompiledTransactionMessageDecoder,
  getTransactionDecoder,
  getSignatureFromTransaction,
} from '@solana/kit';
import { buildInstruction, type TransactionTransport } from '@usearete/sdk';
import { oreLogInstruction } from '../../../../examples/ore-typescript/src/generated/ore-stack-core';
import { createWalletAdapter } from './index';

// Only the relay is a test double: generated builder, adapter, signer and
// upstream codec are real. Adapter edge cases belong in the adapter suite.
it('sends a generated ORE instruction as V1 with the chosen config', async () => {
  const signer = await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(1));
  const captured: string[] = [];
  const transport: TransactionTransport = {
    getLatestBlockhash: async () => ({
      blockhash: '11111111111111111111111111111111',
      contextSlot: 1n, lastValidBlockHeight: 100n,
    }),
    getFeeForMessage: async () => { throw new Error('Unexpected fee request'); },
    simulateTransaction: async () => ({ contextSlot: 1n, err: null, logs: [], unitsConsumed: 1_000n, loadedAccountsDataSize: 1_024n }),
    sendTransaction: async (wire) => {
      captured.push(wire);
      return { signature: getSignatureFromTransaction(getTransactionDecoder().decode(Buffer.from(wire, 'base64'))) };
    },
    getSignatureStatus: async (signature) => ({
      signature, slot: 2n, confirmationStatus: 'confirmed', err: null,
    }),
    getBlockHeight: async () => 1n,
  };
  const wallet = createWalletAdapter({ signer, transport });
  expect(wallet.supportedTransactionVersions, 'The public Kit adapter must support V1').toContain(1);
  const instruction = buildInstruction(oreLogInstruction, { signer: signer.address });
  const config = {
    computeUnitLimit: 200_000, loadedAccountsDataSizeLimit: 1_048_576,
    heapSize: 32_768, priorityFeeLamports: 7n,
  };
  const result = await wallet.signAndSend([instruction], { transactionVersion: 1, resources: config });
  expect(captured).toHaveLength(1);
  const transaction = getTransactionDecoder().decode(Buffer.from(captured[0], 'base64'));
  const message = decompileTransactionMessage(getCompiledTransactionMessageDecoder().decode(transaction.messageBytes));
  expect(message).toMatchObject({ version: 1, config });
  expect(message.instructions).toHaveLength(1);
  expect(message.instructions[0].programAddress).toBe('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv');
  expect(Array.from(message.instructions[0].data ?? [])).toEqual([8]);
  expect(message.instructions[0].accounts).toEqual([{ address: signer.address, role: 3 }]);
  expect(result.signature).toBe(getSignatureFromTransaction(transaction));
});
