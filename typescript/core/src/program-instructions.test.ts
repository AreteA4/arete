import { describe, expect, it } from 'vitest';

import {
  createPreparedFlow,
  createPreparedInstruction,
  createPreparedTransaction,
  describePreparedOperation,
  executePreparedOperation,
  toJsonValue,
} from './operations';
import {
  flowOperation,
  instructionOperation,
  transactionOperation,
} from './program-instructions';

const instruction = {
  programId: 'program-111',
  keys: [{ pubkey: 'signer-1', isSigner: true, isWritable: true }],
  data: new Uint8Array([1]),
} as const;

const secondInstruction = {
  programId: 'program-222',
  keys: [{ pubkey: 'signer-2', isSigner: true, isWritable: false }],
  data: new Uint8Array([2]),
} as const;

describe('operation helpers', () => {
  it('creates a connected instruction operation', async () => {
    const operation = instructionOperation(async (value: { id: string }) =>
      createPreparedInstruction({
        name: 'close',
        instruction,
        artifacts: { id: value.id },
      })
    );

    const prepared = await operation.prepare({ id: 'demo' });
    expect(prepared.kind).toBe('instruction');
    expect(prepared.instruction).toEqual(instruction);
    expect(prepared.transaction.instructions).toEqual([instruction]);
    expect(prepared.artifacts).toEqual({ id: 'demo' });
  });

  it('creates a connected transaction operation', async () => {
    const operation = transactionOperation(async () =>
      createPreparedTransaction({
        name: 'wallet-swap',
        instructions: [instruction, instruction],
        artifacts: { ok: true },
      })
    );

    const prepared = await operation.prepare({});
    expect(prepared.kind).toBe('transaction');
    expect(prepared.transaction.instructions).toHaveLength(2);
    expect(prepared.plan.transactions).toHaveLength(1);
  });

  it('composes prepared instructions in order and inherits their metadata', () => {
    const first = createPreparedInstruction({
      name: 'first',
      instruction,
      artifacts: { child: 1 },
      requiredSignerAddresses: ['prepared-signer-1'],
      errors: [{ code: 6001, name: 'FirstError', msg: 'first failed' }],
    });
    const second = createPreparedInstruction({
      name: 'second',
      instruction: secondInstruction,
      artifacts: { child: 2 },
      errors: [{ code: 6002, name: 'SecondError', msg: 'second failed' }],
    });

    const prepared = createPreparedTransaction({
      name: 'prepared-batch',
      instructions: [first, second],
      artifacts: { parent: true },
    });

    expect(prepared.transaction.instructions).toEqual([
      first.instruction,
      second.instruction,
    ]);
    expect(prepared.transaction.requiredSignerAddresses).toEqual([
      'prepared-signer-1',
      'signer-2',
    ]);
    expect(prepared.transaction.errors).toEqual([
      { code: 6001, name: 'FirstError', msg: 'first failed' },
      { code: 6002, name: 'SecondError', msg: 'second failed' },
    ]);
    expect(prepared.artifacts).toEqual({ parent: true });
  });

  it('composes mixed instructions and honors outer metadata overrides', () => {
    const child = createPreparedInstruction({
      name: 'child',
      instruction: secondInstruction,
      artifacts: { child: true },
      requiredSignerAddresses: ['child-signer'],
      errors: [{ code: 6001, name: 'ChildError', msg: 'child failed' }],
    });
    const inherited = createPreparedTransaction({
      name: 'mixed',
      instructions: [instruction, child, instruction],
      artifacts: { source: 'outer' },
    });

    expect(inherited.transaction.instructions).toEqual([
      instruction,
      child.instruction,
      instruction,
    ]);
    expect(inherited.transaction.requiredSignerAddresses).toEqual([
      'signer-1',
      'child-signer',
    ]);
    expect(inherited.transaction.errors).toEqual(child.transaction.errors);

    const overridden = createPreparedTransaction({
      name: 'overridden',
      instructions: [instruction, child],
      artifacts: { source: 'outer' },
      requiredSignerAddresses: [],
      errors: [],
    });
    expect(overridden.transaction.requiredSignerAddresses).toEqual([]);
    expect(overridden.transaction.errors).toEqual([]);
  });

  it('atomically composes prepared instructions and transactions', () => {
    const initialize = createPreparedTransaction({
      name: 'initialize',
      instructions: [instruction],
      artifacts: { initialized: true },
      errors: [{ code: 6001, name: 'InitializeError', msg: 'initialize failed' }],
    });
    const finalize = createPreparedInstruction({
      name: 'finalize',
      instruction: secondInstruction,
      artifacts: { finalized: true },
      requiredSignerAddresses: ['finalizer'],
    });

    const prepared = createPreparedTransaction({
      name: 'atomic-composition',
      operations: [initialize, finalize],
      artifacts: { complete: true },
    });

    expect(prepared.transaction.instructions).toEqual([
      instruction,
      secondInstruction,
    ]);
    expect(prepared.transaction.requiredSignerAddresses).toEqual([
      'signer-1',
      'finalizer',
    ]);
    expect(prepared.transaction.errors).toEqual([
      { code: 6001, name: 'InitializeError', msg: 'initialize failed' },
    ]);
  });

  it('requires exactly one transaction composition source', () => {
    expect(() => createPreparedTransaction({
      name: 'invalid',
      instructions: [instruction],
      operations: [],
      artifacts: {},
    } as never)).toThrow(/exactly one of instructions or operations/);
  });

  it('creates a connected flow operation', async () => {
    const operation = flowOperation(async () =>
      createPreparedFlow({
        name: 'deposit-flow',
        artifacts: { done: true },
        transactions: [
          {
            name: 'first',
            instructions: [instruction],
            requiredSignerAddresses: ['signer-1'],
            errors: [],
          },
          {
            name: 'second',
            instructions: [instruction],
            requiredSignerAddresses: ['signer-1'],
            errors: [],
          },
        ],
      })
    );

    const prepared = await operation.prepare({});
    expect(prepared.kind).toBe('flow');
    expect(prepared.plan.transactions).toHaveLength(2);
    expect(prepared.artifacts).toEqual({ done: true });
  });

  it('accepts configured signer addresses exposed by the wallet', async () => {
    const prepared = createPreparedInstruction({
      name: 'close',
      instruction,
      artifacts: {},
    });
    const wallet = {
      publicKey: 'wallet-1',
      signerAddresses: ['wallet-1', 'signer-1'],
      async signAndSend() {
        return { signature: 'sig-1' };
      },
    };

    await expect(executePreparedOperation({
      wallet,
      transaction: async () => ({ signature: 'sig-1' }),
    }, prepared)).resolves.toMatchObject({ kind: 'instruction' });
  });

  it('returns ordered top-level signatures for transaction receipts', async () => {
    const prepared = createPreparedTransaction({
      name: 'transaction-receipt',
      instructions: [instruction],
      artifacts: { ok: true },
    });
    const receipt = await executePreparedOperation({
      publicKey: 'signer-1',
      transaction: async () => ({ signature: 'transaction-signature', slot: 42 }),
    }, prepared);
    const firstSignature: string = receipt.signatures[0];

    expect(firstSignature).toBe('transaction-signature');
    expect(receipt).toEqual({
      kind: 'transaction',
      operationName: 'transaction-receipt',
      artifacts: { ok: true },
      signatures: ['transaction-signature'],
      transaction: {
        transactionIndex: 0,
        transactionName: 'transaction-receipt',
        signature: 'transaction-signature',
        slot: 42,
      },
    });
  });

  it('returns flow signatures in transaction execution order', async () => {
    const prepared = createPreparedFlow({
      name: 'ordered-flow',
      artifacts: { ok: true },
      transactions: [
        { name: 'first', instructions: [instruction] },
        { name: 'second', instructions: [secondInstruction] },
      ],
    });
    let transactionIndex = 0;
    const receipt = await executePreparedOperation({
      transaction: async () => ({
        signature: ['first-signature', 'second-signature'][transactionIndex++]!,
      }),
    }, prepared, {
      availableSignerAddresses: ['signer-1', 'signer-2'],
    });

    expect(receipt.signatures).toEqual(['first-signature', 'second-signature']);
    expect(receipt.transactions.map((transaction) => transaction.signature)).toEqual([
      'first-signature',
      'second-signature',
    ]);
  });

  it('describes prepared operations as JSON-safe values', () => {
    const prepared = createPreparedInstruction({
      name: 'json-safe',
      instruction,
      artifacts: {
        amount: 42n,
        bytes: new Uint8Array([1, 2, 3]),
        nested: [undefined, Number.POSITIVE_INFINITY, { omitted: undefined }],
      },
    });

    const description = describePreparedOperation(prepared);
    expect(description.artifacts).toEqual({
      amount: '42',
      bytes: [1, 2, 3],
      nested: [null, null, {}],
    });
    expect(() => JSON.stringify(description)).not.toThrow();
  });

  it('rejects circular values during JSON conversion', () => {
    const circular: Record<string, unknown> = {};
    circular.self = circular;
    expect(() => toJsonValue(circular)).toThrow(/circular value/);
  });
});
