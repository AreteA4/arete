import { beforeEach, describe, expect, it, vi } from 'vitest';
import { normalizeTransactionError } from '@usearete/sdk';
import type { BuiltInstruction, TransactionTransport } from '@usearete/sdk';

const addSignersToTransactionMessage = vi.fn((signers, message) => ({
  ...message,
  attachedSigners: signers,
}));
const appendTransactionMessageInstructions = vi.fn((instructions, message) => ({
  ...message,
  instructions,
}));
const compileTransaction = vi.fn((message) => ({
  message,
  messageBytes: new Uint8Array([1, 2, 3]),
  signatures: {},
}));
const createTransactionMessage = vi.fn(() => ({ version: 0 }));
const decodeBase64 = vi.fn(() => 'encoded-message');
const getBase64Decoder = vi.fn(() => ({ decode: decodeBase64 }));
const getBase64EncodedWireTransaction = vi.fn(() => 'encoded-transaction');
const getSignatureFromTransaction = vi.fn(() => 'sig-kit');
const getFeeForMessageSend = vi.fn();
const getLatestBlockhashSend = vi.fn();
const getSignatureStatusesSend = vi.fn();
const isSolanaError = vi.fn(
  (cause: unknown, code: number) =>
    typeof cause === 'object'
    && cause !== null
    && (cause as { code?: unknown }).code === code
);
const sendAndConfirm = vi.fn();
const sendAndConfirmTransactionFactory = vi.fn(() => sendAndConfirm);
const setTransactionMessageFeePayer = vi.fn((feePayer, message) => ({
  ...message,
  feePayer: { address: feePayer },
}));
const setTransactionMessageFeePayerSigner = vi.fn((feePayer, message) => ({
  ...message,
  feePayer,
}));
const setTransactionMessageLifetimeUsingBlockhash = vi.fn((blockhash, message) => ({
  ...message,
  blockhash,
}));
const signTransactionMessageWithSigners = vi.fn();
const simulateTransactionSend = vi.fn();

vi.mock('@solana/kit', () => ({
  AccountRole: {
    READONLY: 'READONLY',
    READONLY_SIGNER: 'READONLY_SIGNER',
    WRITABLE: 'WRITABLE',
    WRITABLE_SIGNER: 'WRITABLE_SIGNER',
  },
  SOLANA_ERROR__JSON_RPC__SERVER_ERROR_SEND_TRANSACTION_PREFLIGHT_FAILURE: -32002,
  addSignersToTransactionMessage,
  address: (value: string) => value,
  appendTransactionMessageInstructions,
  compileTransaction,
  createTransactionMessage,
  getBase64Decoder,
  getBase64EncodedWireTransaction,
  getSignatureFromTransaction,
  isSolanaError,
  pipe: (value: unknown, ...fns: Array<(input: unknown) => unknown>) =>
    fns.reduce((current, fn) => fn(current), value),
  sendAndConfirmTransactionFactory,
  setTransactionMessageFeePayer,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
}));

const {
  KitTransactionExecutionError,
  createWalletAdapter,
  fromKitInstruction,
  toKitInstruction,
} = await import('./index');

function makeInstruction(signers: readonly string[]): BuiltInstruction {
  return {
    programId: 'program-111',
    keys: signers.map((pubkey, index) => ({
      pubkey,
      isSigner: true,
      isWritable: index === 0,
    })),
    data: new Uint8Array(),
  };
}

function signatureStatus(overrides: Record<string, unknown> = {}) {
  return {
    confirmationStatus: 'confirmed',
    confirmations: 1n,
    err: null,
    slot: 456n,
    status: { Ok: null },
    ...overrides,
  };
}

function createRpcStub() {
  return {
    getFeeForMessage: vi.fn(() => ({ send: getFeeForMessageSend })),
    getLatestBlockhash: vi.fn(() => ({ send: getLatestBlockhashSend })),
    getSignatureStatuses: vi.fn(() => ({ send: getSignatureStatusesSend })),
    simulateTransaction: vi.fn(() => ({ send: simulateTransactionSend })),
  };
}

describe('createWalletAdapter', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getLatestBlockhashSend.mockResolvedValue({
      context: { slot: 100n },
      value: { blockhash: 'latest-blockhash', lastValidBlockHeight: 999n },
    });
    getSignatureStatusesSend.mockResolvedValue({ value: [signatureStatus()] });
    getFeeForMessageSend.mockResolvedValue({
      context: { slot: 400n },
      value: 5_000n,
    });
    simulateTransactionSend.mockResolvedValue({
      context: { slot: 401n },
      value: {
        err: null,
        logs: ['Program log: inspected'],
        unitsConsumed: 200_000n,
      },
    });
    sendAndConfirm.mockResolvedValue(undefined);
    signTransactionMessageWithSigners.mockImplementation(async (message) => ({
      ...message,
      signed: true,
    }));
  });

  it('uses Arete auto mode without constructing a subscription confirmer', async () => {
    const transport = {
      getLatestBlockhash: vi.fn(async () => ({
        blockhash: 'latest-blockhash', contextSlot: 1n, lastValidBlockHeight: 999n,
      })),
      sendTransaction: vi.fn(async () => ({ signature: 'sig-arete' })),
      getSignatureStatus: vi.fn(async () => ({
        signature: 'sig-arete', slot: 88n, confirmationStatus: 'confirmed', err: null,
      })),
      getBlockHeight: vi.fn(async () => 100n),
    } as unknown as TransactionTransport;
    const wallet = createWalletAdapter({
      transport: 'auto', signer: { address: 'primary-signer' } as never,
    });

    await expect(wallet.signAndSend(
      [makeInstruction(['primary-signer'])],
      { statusPollIntervalMs: 0 },
      { transactionTransport: transport }
    )).resolves.toEqual({ signature: 'sig-arete', slot: 88 });
    expect(transport.sendTransaction).toHaveBeenCalledTimes(1);
    expect(sendAndConfirmTransactionFactory).not.toHaveBeenCalled();
    expect(signTransactionMessageWithSigners).toHaveBeenCalledTimes(1);
  });

  it('sends once with the primary signer and returns the landed slot', async () => {
    const primary = { address: 'primary-signer' };
    const rpc = createRpcStub();
    const wallet = createWalletAdapter({
      rpc: rpc as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
    });

    const result = await wallet.signAndSend([makeInstruction([primary.address])]);

    expect(result).toEqual({ signature: 'sig-kit', slot: 456 });
    expect(wallet.signerAddresses).toEqual([primary.address]);
    expect(setTransactionMessageFeePayerSigner).toHaveBeenCalledWith(primary, expect.anything());
    expect(addSignersToTransactionMessage).toHaveBeenCalledWith([], expect.anything());
    expect(signTransactionMessageWithSigners).toHaveBeenCalledTimes(1);
    expect(sendAndConfirm).toHaveBeenCalledTimes(1);
    expect(rpc.getSignatureStatuses).toHaveBeenCalledWith(
      ['sig-kit'],
      { searchTransactionHistory: false }
    );
  });

  it('publishes and uses all configured signer addresses without duplicates', async () => {
    const primary = { address: 'primary-signer' };
    const extra = { address: 'extra-signer' };
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
      additionalSigners: [extra as never, extra as never],
    });

    expect(wallet.signerAddresses).toEqual([primary.address, extra.address]);
    await wallet.signAndSend([makeInstruction([primary.address, extra.address])]);

    expect(addSignersToTransactionMessage).toHaveBeenCalledWith([extra], expect.anything());
  });

  it('accepts per-send local signers and standardized send signers', async () => {
    const primary = { address: 'primary-signer' };
    const extra = { address: 'extra-signer' };
    const standardized = { address: 'standardized-signer' };
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
    });

    await wallet.signAndSend(
      [makeInstruction([primary.address, extra.address, standardized.address])],
      {
        additionalSigners: [extra as never],
        signers: [standardized as never],
      }
    );

    expect(addSignersToTransactionMessage).toHaveBeenCalledWith(
      [extra, standardized],
      expect.anything()
    );
  });

  it('supports a fee payer override while retaining a required primary signer', async () => {
    const primary = { address: 'primary-signer' };
    const feePayer = { address: 'fee-payer' };
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
      additionalSigners: [feePayer as never],
    });

    await wallet.signAndSend(
      [makeInstruction([primary.address])],
      { feePayer: feePayer as never }
    );

    expect(setTransactionMessageFeePayerSigner).toHaveBeenCalledWith(feePayer, expect.anything());
    expect(addSignersToTransactionMessage).toHaveBeenCalledWith([primary], expect.anything());
  });

  it('classifies build failures as not submitted', async () => {
    const primary = { address: 'primary-signer' };
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
    });

    await expect(
      wallet.signAndSend([makeInstruction([primary.address, 'missing-signer'])])
    ).rejects.toMatchObject({
      outcome: { status: 'not-submitted', phase: 'build' },
    });
    expect(signTransactionMessageWithSigners).not.toHaveBeenCalled();
    expect(sendAndConfirm).not.toHaveBeenCalled();
  });

  it('classifies an empty transaction as a build failure', async () => {
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: { address: 'primary-signer' } as never,
    });

    await expect(wallet.signAndSend([])).rejects.toMatchObject({
      outcome: { status: 'not-submitted', phase: 'build' },
    });
    expect(getLatestBlockhashSend).not.toHaveBeenCalled();
    expect(sendAndConfirm).not.toHaveBeenCalled();
  });

  it('classifies signer rejection as not submitted by the wallet', async () => {
    const rejection = new Error('User rejected the transaction');
    signTransactionMessageWithSigners.mockRejectedValueOnce(rejection);
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: { address: 'primary-signer' } as never,
    });

    await expect(
      wallet.signAndSend([makeInstruction(['primary-signer'])])
    ).rejects.toMatchObject({
      cause: rejection,
      outcome: { status: 'not-submitted', phase: 'wallet', cause: rejection },
    });
    expect(sendAndConfirm).not.toHaveBeenCalled();
    expect(getSignatureStatusesSend).not.toHaveBeenCalled();
  });

  it('classifies preflight rejection as not submitted without a status query', async () => {
    const preflightError = { code: -32002, message: 'preflight failed' };
    sendAndConfirm.mockRejectedValueOnce(preflightError);
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: { address: 'primary-signer' } as never,
    });

    await expect(
      wallet.signAndSend([makeInstruction(['primary-signer'])])
    ).rejects.toMatchObject({
      outcome: {
        status: 'not-submitted',
        phase: 'send',
        cause: preflightError,
      },
    });
    expect(sendAndConfirm).toHaveBeenCalledTimes(1);
    expect(getSignatureStatusesSend).not.toHaveBeenCalled();
  });

  it('classifies unknown confirmation once without resubmitting', async () => {
    const confirmationError = Object.assign(new Error('confirmation timed out'), {
      context: { slot: 321n },
    });
    sendAndConfirm.mockRejectedValueOnce(confirmationError);
    getSignatureStatusesSend.mockResolvedValueOnce({ value: [null] });
    const rpc = createRpcStub();
    const wallet = createWalletAdapter({
      rpc: rpc as never,
      rpcSubscriptions: {} as never,
      signer: { address: 'primary-signer' } as never,
    });

    await expect(
      wallet.signAndSend([makeInstruction(['primary-signer'])])
    ).rejects.toMatchObject({
      signature: 'sig-kit',
      outcome: {
        status: 'submitted-unknown',
        phase: 'confirmation',
        signature: 'sig-kit',
        slot: 321,
        cause: confirmationError,
      },
    });
    expect(sendAndConfirm).toHaveBeenCalledTimes(1);
    expect(getSignatureStatusesSend).toHaveBeenCalledTimes(1);
    expect(rpc.getSignatureStatuses).toHaveBeenCalledWith(
      ['sig-kit'],
      { searchTransactionHistory: true }
    );
  });

  it('preserves signature and slot for a chain failure found after uncertainty', async () => {
    const transactionError = { InstructionError: [0, { Custom: 6001 }] };
    sendAndConfirm.mockRejectedValueOnce(new Error('confirmation failed'));
    getSignatureStatusesSend.mockResolvedValueOnce({
      value: [signatureStatus({ err: transactionError, status: { Err: transactionError } })],
    });
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: { address: 'primary-signer' } as never,
    });

    await expect(
      wallet.signAndSend([makeInstruction(['primary-signer'])])
    ).rejects.toMatchObject({
      signature: 'sig-kit',
      slot: 456,
      outcome: {
        status: 'chain-failed',
        phase: 'confirmation',
        signature: 'sig-kit',
        slot: 456,
      },
    });
    expect(getSignatureStatusesSend).toHaveBeenCalledTimes(1);
    expect(sendAndConfirm).toHaveBeenCalledTimes(1);
  });

  it('accepts a confirmation that reached the requested commitment despite a waiter error', async () => {
    sendAndConfirm.mockRejectedValueOnce(new Error('subscription disconnected'));
    getSignatureStatusesSend.mockResolvedValueOnce({
      value: [signatureStatus({ confirmationStatus: 'finalized', slot: 789n })],
    });
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: { address: 'primary-signer' } as never,
    });

    await expect(
      wallet.signAndSend([makeInstruction(['primary-signer'])], {
        confirmationLevel: 'finalized',
      })
    ).resolves.toEqual({ signature: 'sig-kit', slot: 789 });
    expect(sendAndConfirm).toHaveBeenCalledTimes(1);
    expect(getSignatureStatusesSend).toHaveBeenCalledTimes(1);
  });

  it('retains the confirmed result if the landed-slot lookup fails', async () => {
    getSignatureStatusesSend.mockRejectedValueOnce(new Error('status unavailable'));
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: { address: 'primary-signer' } as never,
    });

    await expect(
      wallet.signAndSend([makeInstruction(['primary-signer'])])
    ).resolves.toEqual({ signature: 'sig-kit' });
    expect(sendAndConfirm).toHaveBeenCalledTimes(1);
    expect(getSignatureStatusesSend).toHaveBeenCalledTimes(1);
  });

  it('inspects an unsigned transaction with fee and simulation details', async () => {
    const primary = { address: 'primary-signer' };
    const rpc = createRpcStub();
    const wallet = createWalletAdapter({
      rpc: rpc as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
    });

    await expect(
      wallet.inspectTransaction?.([makeInstruction([primary.address])], {
        commitment: 'finalized',
        feePayer: 'inspection-fee-payer',
        minContextSlot: 300n,
      })
    ).resolves.toEqual({
      feeLamports: 5_000,
      logs: ['Program log: inspected'],
      computeUnitsConsumed: 200_000,
      contextSlot: 401,
      error: undefined,
      feeContextSlot: 400,
    });

    expect(setTransactionMessageFeePayer).toHaveBeenCalledWith(
      'inspection-fee-payer',
      expect.anything()
    );
    expect(signTransactionMessageWithSigners).not.toHaveBeenCalled();
    expect(sendAndConfirm).not.toHaveBeenCalled();
    expect(rpc.getFeeForMessage).toHaveBeenCalledWith('encoded-message', {
      commitment: 'finalized',
      minContextSlot: 300n,
    });
    expect(rpc.simulateTransaction).toHaveBeenCalledWith('encoded-transaction', {
      commitment: 'finalized',
      encoding: 'base64',
      minContextSlot: 300n,
      sigVerify: false,
    });
  });

  it('returns the raw simulation error for core program-error parsing', async () => {
    const transactionError = { InstructionError: [0, { Custom: 7001 }] };
    getFeeForMessageSend.mockResolvedValueOnce({
      context: { slot: 500n },
      value: null,
    });
    simulateTransactionSend.mockResolvedValueOnce({
      context: { slot: 501n },
      value: { err: transactionError, logs: null },
    });
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: { address: 'primary-signer' } as never,
    });

    await expect(
      wallet.inspectTransaction?.([makeInstruction(['primary-signer'])])
    ).resolves.toMatchObject({
      feeLamports: undefined,
      contextSlot: 501,
      error: transactionError,
    });
    expect(signTransactionMessageWithSigners).not.toHaveBeenCalled();
  });

  it('normalizes its structural outcome through the current core API', () => {
    const transactionError = { InstructionError: [0, { Custom: 8001 }] };
    const adapterError = new KitTransactionExecutionError({
      status: 'chain-failed',
      phase: 'confirmation',
      signature: 'sig-normalized',
      slot: 808,
      cause: transactionError,
    });

    expect(normalizeTransactionError(adapterError, [{
      code: 8001,
      name: 'NormalizedFailure',
      msg: 'normalized by core',
    }])).toMatchObject({
      programError: {
        code: 8001,
        name: 'NormalizedFailure',
        message: 'normalized by core',
      },
      outcome: {
        status: 'chain-failed',
        signature: 'sig-normalized',
        slot: 808,
      },
    });
  });
});

describe('instruction converters', () => {
  it('round-trips BuiltInstruction through IInstruction', () => {
    const original: BuiltInstruction = {
      programId: 'program-111',
      keys: [
        { pubkey: 'signer-writable', isSigner: true, isWritable: true },
        { pubkey: 'signer-readonly', isSigner: true, isWritable: false },
        { pubkey: 'plain-writable', isSigner: false, isWritable: true },
        { pubkey: 'plain-readonly', isSigner: false, isWritable: false },
      ],
      data: new Uint8Array([9, 8, 7]),
    };

    const kitInstruction = toKitInstruction(original);
    expect(kitInstruction.accounts?.map((account) => account.role)).toEqual([
      'WRITABLE_SIGNER',
      'READONLY_SIGNER',
      'WRITABLE',
      'READONLY',
    ]);
    expect(fromKitInstruction(kitInstruction)).toEqual(original);
  });

  it('converts missing kit instruction data to an empty byte array', () => {
    expect(
      fromKitInstruction({ programAddress: 'program-111' as never })
    ).toEqual({ programId: 'program-111', keys: [], data: new Uint8Array(0) });
  });
});
