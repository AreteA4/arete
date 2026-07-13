import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BuiltInstruction } from '@usearete/sdk';

const addSignersToTransactionMessage = vi.fn((signers, message) => ({
  ...message,
  attachedSigners: signers,
}));
const appendTransactionMessageInstructions = vi.fn((instructions, message) => ({
  ...message,
  instructions,
}));
const createTransactionMessage = vi.fn(() => ({ version: 0 }));
const getSignatureFromTransaction = vi.fn(() => 'sig-kit');
const getLatestBlockhashSend = vi.fn(async () => ({ value: 'latest-blockhash' }));
const sendAndConfirm = vi.fn(async () => undefined);
const sendAndConfirmTransactionFactory = vi.fn(() => sendAndConfirm);
const setTransactionMessageFeePayerSigner = vi.fn((feePayer, message) => ({
  ...message,
  feePayer,
}));
const setTransactionMessageLifetimeUsingBlockhash = vi.fn((blockhash, message) => ({
  ...message,
  blockhash,
}));
const signTransactionMessageWithSigners = vi.fn(async (message) => ({
  ...message,
  signed: true,
}));

vi.mock('@solana/kit', () => ({
  AccountRole: {
    READONLY: 'READONLY',
    READONLY_SIGNER: 'READONLY_SIGNER',
    WRITABLE: 'WRITABLE',
    WRITABLE_SIGNER: 'WRITABLE_SIGNER',
  },
  addSignersToTransactionMessage,
  address: (value: string) => value,
  appendTransactionMessageInstructions,
  createTransactionMessage,
  getSignatureFromTransaction,
  pipe: (value: unknown, ...fns: Array<(input: unknown) => unknown>) =>
    fns.reduce((current, fn) => fn(current), value),
  sendAndConfirmTransactionFactory,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
}));

const { createWalletAdapter } = await import('./index');

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

function createRpcStub() {
  return {
    getLatestBlockhash: vi.fn(() => ({ send: getLatestBlockhashSend })),
  };
}

describe('createWalletAdapter', () => {
  beforeEach(() => {
    addSignersToTransactionMessage.mockClear();
    appendTransactionMessageInstructions.mockClear();
    createTransactionMessage.mockClear();
    getLatestBlockhashSend.mockClear();
    sendAndConfirm.mockClear();
    sendAndConfirmTransactionFactory.mockClear();
    setTransactionMessageFeePayerSigner.mockClear();
    setTransactionMessageLifetimeUsingBlockhash.mockClear();
    signTransactionMessageWithSigners.mockClear();
  });

  it('sends with the primary signer by default', async () => {
    const primary = { address: 'primary-signer' };
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
    });

    const result = await wallet.signAndSend([makeInstruction([primary.address])]);

    expect(result).toEqual({ signature: 'sig-kit' });
    expect(setTransactionMessageFeePayerSigner).toHaveBeenCalledWith(primary, expect.anything());
    expect(addSignersToTransactionMessage).toHaveBeenCalledWith([], expect.anything());
    expect(signTransactionMessageWithSigners).toHaveBeenCalledTimes(1);
    expect(sendAndConfirm).toHaveBeenCalledTimes(1);
  });

  it('uses configured local signers for extra required signatures', async () => {
    const primary = { address: 'primary-signer' };
    const extra = { address: 'extra-signer' };
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
      additionalSigners: [extra as never],
    });

    await wallet.signAndSend([makeInstruction([primary.address, extra.address])]);

    expect(addSignersToTransactionMessage).toHaveBeenCalledWith([extra], expect.anything());
  });

  it('accepts per-send local signers', async () => {
    const primary = { address: 'primary-signer' };
    const extra = { address: 'extra-signer' };
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
    });

    await wallet.signAndSend([makeInstruction([primary.address, extra.address])], {
      additionalSigners: [extra as never],
    });

    expect(addSignersToTransactionMessage).toHaveBeenCalledWith([extra], expect.anything());
  });

  it('accepts standardized signers in send options', async () => {
    const primary = { address: 'primary-signer' };
    const extra = { address: 'extra-signer' };
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
    });

    await wallet.signAndSend([makeInstruction([primary.address, extra.address])], {
      signers: [extra as never],
    });

    expect(addSignersToTransactionMessage).toHaveBeenCalledWith([extra], expect.anything());
  });

  it('supports overriding the fee payer', async () => {
    const primary = { address: 'primary-signer' };
    const feePayer = { address: 'fee-payer' };
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
      additionalSigners: [feePayer as never],
    });

    await wallet.signAndSend([makeInstruction([])], { feePayer: feePayer as never });

    expect(setTransactionMessageFeePayerSigner).toHaveBeenCalledWith(feePayer, expect.anything());
    expect(addSignersToTransactionMessage).toHaveBeenCalledWith([feePayer], expect.anything());
  });

  it('fails fast when a required signer cannot be satisfied', async () => {
    const primary = { address: 'primary-signer' };
    const wallet = createWalletAdapter({
      rpc: createRpcStub() as never,
      rpcSubscriptions: {} as never,
      signer: primary as never,
    });

    await expect(
      wallet.signAndSend([makeInstruction([primary.address, 'missing-signer'])])
    ).rejects.toThrow(/Missing signer\(s\) for transaction/);
    expect(signTransactionMessageWithSigners).not.toHaveBeenCalled();
    expect(sendAndConfirm).not.toHaveBeenCalled();
  });
});

describe('instruction converters', () => {
  it('round-trips BuiltInstruction through IInstruction', async () => {
    const { toKitInstruction, fromKitInstruction } = await import('./index');
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

  it('converts missing kit instruction data to an empty byte array', async () => {
    const { fromKitInstruction } = await import('./index');
    expect(
      fromKitInstruction({ programAddress: 'program-111' as never })
    ).toEqual({ programId: 'program-111', keys: [], data: new Uint8Array(0) });
  });
});
