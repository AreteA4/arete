import { describe, expect, it, vi } from 'vitest';
import {
  createPreparedFlow,
  createPreparedInstruction,
  executePreparedOperation,
  inspectPreparedOperation,
  OperationExecutionError,
  unwrapOperationExecutionError,
} from './operations';
import {
  InstructionError,
  TransactionExecutionError,
} from './instructions/error-parser';

const instruction = {
  programId: 'program',
  keys: [],
  data: new Uint8Array(),
};

describe('executePreparedOperation', () => {
  it('rejects required signers when no signer address can be inferred', async () => {
    const transaction = vi.fn(async () => ({ signature: 'signature' }));
    const operation = createPreparedFlow({
      name: 'test-flow',
      artifacts: undefined,
      transactions: [
        {
          name: 'test-transaction',
          instructions: [instruction],
          requiredSignerAddresses: ['required'],
          errors: [],
        },
      ],
    });

    const result = executePreparedOperation(
      { transaction },
      operation,
      { signers: [{ opaqueSigner: true }] }
    );

    await expect(result).rejects.toMatchObject({
      cause: new Error('Missing signer(s) for test-transaction: required'),
    } satisfies Partial<OperationExecutionError>);
    expect(transaction).not.toHaveBeenCalled();
  });

  it('does not let an opaque signer hide a missing required signer', async () => {
    const transaction = vi.fn(async () => ({ signature: 'signature' }));
    const operation = createPreparedFlow({
      name: 'test-flow',
      artifacts: undefined,
      transactions: [
        {
          name: 'test-transaction',
          instructions: [instruction],
          requiredSignerAddresses: ['wallet', 'missing'],
          errors: [],
        },
      ],
    });

    const result = executePreparedOperation(
      { publicKey: 'wallet', transaction },
      operation,
      { signers: [{ opaqueSigner: true }] }
    );

    await expect(result).rejects.toMatchObject({
      cause: new Error('Missing signer(s) for test-transaction: missing'),
    } satisfies Partial<OperationExecutionError>);
    expect(transaction).not.toHaveBeenCalled();
  });

  it('classifies wallet rejection as not submitted', async () => {
    const rejected = Object.assign(new Error('User rejected the wallet request'), {
      code: 4001,
    });
    const transaction = vi.fn(async () => {
      throw rejected;
    });
    const operation = createPreparedInstruction({
      name: 'wallet-rejection',
      instruction,
      artifacts: undefined,
    });

    const error = await executePreparedOperation({ transaction }, operation)
      .catch((cause: unknown) => cause);

    expect(error).toBeInstanceOf(OperationExecutionError);
    expect(error).toMatchObject({
      outcome: { status: 'not-submitted', phase: 'wallet', cause: rejected },
      completedReceipts: [],
    });
    expect(transaction).toHaveBeenCalledTimes(1);
  });

  it('preserves a known signature when confirmation status is unknown', async () => {
    const timeout = new Error('confirmation timed out');
    const transaction = vi.fn(async () => {
      throw new TransactionExecutionError({
        status: 'submitted-unknown',
        phase: 'confirmation',
        signature: 'known-signature',
        cause: timeout,
      });
    });
    const operation = createPreparedInstruction({
      name: 'confirmation-timeout',
      instruction,
      artifacts: undefined,
    });

    const error = await executePreparedOperation({ transaction }, operation)
      .catch((cause: unknown) => cause) as OperationExecutionError;

    expect(error.outcome).toMatchObject({
      status: 'submitted-unknown',
      phase: 'confirmation',
      signature: 'known-signature',
      cause: timeout,
    });
    expect(error.signature).toBe('known-signature');
    expect(unwrapOperationExecutionError(error)).toEqual(error.outcome);
  });

  it('keeps nested program failures visible with signature and slot', async () => {
    const chainFailure = {
      signature: 'ore-signature',
      slot: 123,
      value: { err: { InstructionError: [0, { Custom: 6000 }] } },
    };
    const transaction = vi.fn(async () => {
      throw chainFailure;
    });
    const operation = createPreparedInstruction({
      name: 'ore-deploy',
      instruction,
      artifacts: undefined,
      errors: [{ code: 6000, name: 'OreProgramError', msg: 'ORE deploy failed' }],
    });

    const error = await executePreparedOperation({ transaction }, operation)
      .catch((cause: unknown) => cause) as OperationExecutionError;
    const underlying = unwrapOperationExecutionError(error);

    expect(error.outcome).toMatchObject({
      status: 'chain-failed',
      signature: 'ore-signature',
      slot: 123,
      programError: { code: 6000, name: 'OreProgramError' },
      cause: chainFailure,
    });
    expect(underlying).toBeInstanceOf(InstructionError);
    expect(underlying).toMatchObject({
      signature: 'ore-signature',
      slot: 123,
      programError: { name: 'OreProgramError', message: 'ORE deploy failed' },
    });
  });

  it('keeps a confirmed receipt when a post-confirmation callback throws', async () => {
    const transaction = vi.fn(async () => ({ signature: 'confirmed-signature', slot: 99 }));
    const callbackCause = new Error('reconciliation failed');
    const onCallbackError = vi.fn();
    const operation = createPreparedInstruction({
      name: 'confirmed-operation',
      instruction,
      artifacts: { confirmed: true },
    });

    const receipt = await executePreparedOperation({ transaction }, operation, {
      onTransactionSuccess: () => {
        throw callbackCause;
      },
      onCallbackError,
    });

    expect(receipt.transaction).toEqual({
      transactionIndex: 0,
      transactionName: 'confirmed-operation',
      signature: 'confirmed-signature',
      slot: 99,
    });
    expect(receipt.callbackErrors).toHaveLength(1);
    expect(receipt.callbackErrors?.[0]).toMatchObject({
      phase: 'transaction-success',
      cause: callbackCause,
      receipt: { signature: 'confirmed-signature', slot: 99 },
    });
    expect(onCallbackError).toHaveBeenCalledWith(receipt.callbackErrors?.[0]);
    expect(transaction).toHaveBeenCalledTimes(1);
  });

  it('preserves completed flow receipts when a later transaction fails', async () => {
    const transaction = vi
      .fn()
      .mockResolvedValueOnce({ signature: 'first-signature', slot: 10 })
      .mockRejectedValueOnce(new TransactionExecutionError({
        status: 'chain-failed',
        phase: 'chain',
        signature: 'second-signature',
        slot: 11,
        cause: new Error('second transaction failed'),
      }));
    const operation = createPreparedFlow({
      name: 'partial-flow',
      artifacts: undefined,
      transactions: [
        { name: 'first', instructions: [instruction] },
        { name: 'second', instructions: [instruction] },
      ],
    });

    const error = await executePreparedOperation({ transaction }, operation)
      .catch((cause: unknown) => cause) as OperationExecutionError;

    expect(error.completedReceipts).toEqual([{
      transactionIndex: 0,
      transactionName: 'first',
      signature: 'first-signature',
      slot: 10,
    }]);
    expect(error.outcome).toMatchObject({
      status: 'chain-failed',
      signature: 'second-signature',
      slot: 11,
    });
    expect(transaction).toHaveBeenCalledTimes(2);
  });
});

describe('OperationExecutionError', () => {
  const operation = createPreparedInstruction({
    name: 'error-context',
    instruction,
    artifacts: undefined,
  });

  it('creates and unwraps a fallback outcome when none is explicit', () => {
    const cause = new Error('host failed without structured context');
    const error = new OperationExecutionError({
      operation,
      failedTransaction: operation.transaction,
      failedTransactionIndex: 0,
      completedReceipts: [],
      cause,
    });

    expect(error.outcome).toEqual({
      status: 'not-submitted',
      phase: 'send',
      cause,
    });
    expect(error.message).toBe(
      "Operation 'error-context' failed at transaction 1 (error-context): host failed without structured context"
    );
    expect(unwrapOperationExecutionError(error)).toBe(error.outcome);
  });

  it('uses structured cause context over a contradictory explicit outcome', () => {
    const originalCause = new Error('confirmation timed out');
    const transactionError = new TransactionExecutionError({
      status: 'submitted-unknown',
      phase: 'confirmation',
      signature: 'submitted-signature',
      cause: originalCause,
    });
    const error = new OperationExecutionError({
      operation,
      failedTransaction: operation.transaction,
      failedTransactionIndex: 0,
      completedReceipts: [],
      outcome: {
        status: 'not-submitted',
        phase: 'build',
        cause: new Error('stale explicit outcome'),
      },
      cause: transactionError,
    });

    expect(error.outcome).toBe(transactionError.outcome);
    expect(error.signature).toBe('submitted-signature');
    expect(unwrapOperationExecutionError(error)).toBe(transactionError.outcome);
  });

  it('recursively exposes a nested InstructionError', () => {
    const originalCause = new Error('ORE instruction failed');
    const instructionError = new InstructionError(
      'OreProgramError (6000): ORE failed',
      { code: 6000, name: 'OreProgramError', message: 'ORE failed' },
      originalCause
    );
    const inner = new OperationExecutionError({
      operation,
      failedTransaction: operation.transaction,
      failedTransactionIndex: 0,
      completedReceipts: [],
      cause: instructionError,
    });
    const outer = new OperationExecutionError({
      operation,
      failedTransaction: operation.transaction,
      failedTransactionIndex: 0,
      completedReceipts: [],
      cause: inner,
    });

    expect(unwrapOperationExecutionError(outer)).toBe(instructionError);
    expect(outer.outcome).toEqual(instructionError.outcome);
  });
});

describe('inspectPreparedOperation', () => {
  it('uses unsigned adapter inspection and never invokes signing', async () => {
    const signAndSend = vi.fn();
    const inspectTransaction = vi.fn(async () => ({
      feeLamports: 5000,
      contextSlot: 50,
      logs: ['Program log: failed'],
      error: { InstructionError: [0, { Custom: 6001 }] },
    }));
    const wallet = { publicKey: 'wallet', signAndSend, inspectTransaction };
    const operation = createPreparedInstruction({
      name: 'inspect-me',
      instruction,
      artifacts: { amount: 1n },
      errors: [{ code: 6001, name: 'InspectionFailure', msg: 'would fail' }],
    });

    const result = await inspectPreparedOperation(wallet, operation);

    expect(result.description).toMatchObject({
      kind: 'instruction',
      name: 'inspect-me',
      artifacts: { amount: '1' },
    });
    expect(result.transaction).toMatchObject({ feeLamports: 5000, contextSlot: 50 });
    expect(result.programError).toEqual({
      code: 6001,
      name: 'InspectionFailure',
      message: 'would fail',
    });
    expect(inspectTransaction).toHaveBeenCalledWith([instruction], undefined);
    expect(signAndSend).not.toHaveBeenCalled();
  });

  it('rejects flows before invoking adapter inspection', async () => {
    const inspectTransaction = vi.fn();
    const wallet = { publicKey: 'wallet', signAndSend: vi.fn(), inspectTransaction };
    const flow = createPreparedFlow({
      name: 'unsupported-flow',
      artifacts: undefined,
      transactions: [{ name: 'only-stage', instructions: [instruction] }],
    });

    await expect(inspectPreparedOperation(wallet, flow)).rejects.toThrow(
      "Cannot inspect flow 'unsupported-flow': flow inspection is not supported"
    );
    expect(inspectTransaction).not.toHaveBeenCalled();
    expect(wallet.signAndSend).not.toHaveBeenCalled();
  });

  it('reports any multi-transaction operation accurately', async () => {
    const inspectTransaction = vi.fn();
    const wallet = { publicKey: 'wallet', signAndSend: vi.fn(), inspectTransaction };
    const operation = createPreparedInstruction({
      name: 'malformed-multi-transaction',
      instruction,
      artifacts: undefined,
    });
    const multiTransactionOperation = {
      ...operation,
      plan: {
        ...operation.plan,
        transactions: [operation.transaction, operation.transaction] as const,
      },
    };

    await expect(
      inspectPreparedOperation(wallet, multiTransactionOperation)
    ).rejects.toThrow(
      "Cannot inspect operation 'malformed-multi-transaction': multi-transaction operation inspection is not supported"
    );
    expect(inspectTransaction).not.toHaveBeenCalled();
  });
});
