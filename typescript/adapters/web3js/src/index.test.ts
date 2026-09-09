import { afterEach, describe, expect, it, vi } from 'vitest';
import bs58 from 'bs58';
import {
  Keypair,
  SendTransactionError,
  VersionedTransaction,
  type Connection,
  type SignatureStatus,
  type Signer,
} from '@solana/web3.js';
import {
  getTransactionFailureOutcome,
  parseInstructionError,
  type BuiltInstruction,
  type TransactionFailureOutcome,
  type TransactionTransport,
} from '@usearete/sdk';

import {
  connectionAccountLoader,
  createWalletAdapter,
  type VersionedTransactionSigner,
} from './index';

const SYSTEM_PROGRAM = '11111111111111111111111111111111';

function hasSignature(signature: Uint8Array): boolean {
  return signature.some((byte) => byte !== 0);
}

function makeInstruction(signers: readonly string[]): BuiltInstruction {
  return {
    programId: SYSTEM_PROGRAM,
    keys: signers.map((pubkey, index) => ({
      pubkey,
      isSigner: true,
      isWritable: index === 0,
    })),
    data: new Uint8Array(),
  };
}

function createPrimarySigner(keypair: Keypair): VersionedTransactionSigner & { calls: number } {
  return {
    publicKey: keypair.publicKey,
    calls: 0,
    async signTransaction(tx: VersionedTransaction): Promise<VersionedTransaction> {
      this.calls += 1;
      tx.sign([keypair]);
      return tx;
    },
  };
}

interface ConnectionStubOptions {
  sendError?: unknown;
  confirmationError?: unknown;
  confirmationResultError?: unknown;
  status?: SignatureStatus | null;
  feeLamports?: number | null;
  simulationError?: unknown;
}

function createConnectionStub(options: ConnectionStubOptions = {}) {
  let sent: VersionedTransaction | null = null;
  let sendCalls = 0;
  let statusCalls = 0;
  let searchTransactionHistory: boolean | undefined;

  const connection = {
    async getLatestBlockhash() {
      return { blockhash: SYSTEM_PROGRAM, lastValidBlockHeight: 123 };
    },
    async sendRawTransaction(raw: Buffer | Uint8Array) {
      sendCalls += 1;
      sent = VersionedTransaction.deserialize(Buffer.from(raw));
      if (options.sendError) {
        throw options.sendError;
      }
      return 'sig-web3js';
    },
    async confirmTransaction() {
      if (options.confirmationError) {
        throw options.confirmationError;
      }
      return {
        context: { slot: 456 },
        value: { err: options.confirmationResultError ?? null },
      };
    },
    async getSignatureStatuses(
      _signatures: readonly string[],
      config?: { searchTransactionHistory?: boolean }
    ) {
      statusCalls += 1;
      searchTransactionHistory = config?.searchTransactionHistory;
      return {
        context: { slot: 999 },
        value: [options.status ?? null],
      };
    },
    async getFeeForMessage() {
      return {
        context: { slot: 100 },
        value: options.feeLamports === undefined ? 5000 : options.feeLamports,
      };
    },
    async simulateTransaction() {
      return {
        context: { slot: 101 },
        value: {
          err: options.simulationError ?? null,
          logs: ['Program log: inspected'],
          unitsConsumed: 1234,
        },
      };
    },
  } as unknown as Connection;

  return {
    connection,
    getSent: () => sent,
    getSendCalls: () => sendCalls,
    getStatusCalls: () => statusCalls,
    getSearchTransactionHistory: () => searchTransactionHistory,
  };
}

function failureOutcome(error: unknown): TransactionFailureOutcome {
  const outcome = getTransactionFailureOutcome(error);
  expect(outcome).not.toBeNull();
  return outcome!;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('createWalletAdapter', () => {
  it('uses invocation-context Arete transport in auto mode without direct fallback', async () => {
    const primary = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const sendTransaction = vi.fn(async () => ({ signature: 'sig-arete' }));
    const transport = {
      getLatestBlockhash: vi.fn(async () => ({
        blockhash: SYSTEM_PROGRAM, contextSlot: 1n, lastValidBlockHeight: 100n,
      })),
      getFeeForMessage: vi.fn(),
      simulateTransaction: vi.fn(),
      sendTransaction,
      getSignatureStatus: vi.fn(async () => ({
        signature: 'sig-arete', slot: 55n, confirmationStatus: 'confirmed', err: null,
      })),
      getBlockHeight: vi.fn(async () => 50n),
    } as unknown as TransactionTransport;
    const wallet = createWalletAdapter({ signer, transport: 'auto' });

    await expect(wallet.signAndSend(
      [makeInstruction([primary.publicKey.toBase58()])],
      { statusPollIntervalMs: 0 },
      { transactionTransport: transport }
    )).resolves.toEqual({ signature: 'sig-arete', slot: 55 });
    expect(signer.calls).toBe(1);
    expect(sendTransaction).toHaveBeenCalledTimes(1);
  });

  it('does not fall back to configured direct RPC after an Arete send error', async () => {
    const primary = Keypair.generate();
    const { connection, getSendCalls } = createConnectionStub();
    const transport = {
      getLatestBlockhash: async () => ({
        blockhash: SYSTEM_PROGRAM, contextSlot: 1n, lastValidBlockHeight: 100n,
      }),
      sendTransaction: vi.fn(async () => { throw new Error('relay unavailable'); }),
    } as unknown as TransactionTransport;
    const wallet = createWalletAdapter({
      connection, signer: createPrimarySigner(primary), transport,
    });

    await expect(wallet.signAndSend([
      makeInstruction([primary.publicKey.toBase58()]),
    ])).rejects.toMatchObject({ outcome: { status: 'submitted-unknown', phase: 'send' } });
    expect(transport.sendTransaction).toHaveBeenCalledTimes(1);
    expect(getSendCalls()).toBe(0);
  });

  it('signs and sends with the primary signer by default', async () => {
    const primary = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSent } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    const result = await wallet.signAndSend([makeInstruction([primary.publicKey.toBase58()])]);

    expect(result).toEqual({ signature: 'sig-web3js', slot: 456 });
    expect(signer.calls).toBe(1);

    const sent = getSent();
    expect(sent).not.toBeNull();
    expect(sent!.message.header.numRequiredSignatures).toBe(1);
    expect(hasSignature(sent!.signatures[0]!)).toBe(true);
  });

  it('uses configured local signers for extra required signatures', async () => {
    const primary = Keypair.generate();
    const extra = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSent } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer, additionalSigners: [extra] });

    expect(wallet.signerAddresses).toEqual([
      primary.publicKey.toBase58(),
      extra.publicKey.toBase58(),
    ]);

    await wallet.signAndSend([
      makeInstruction([primary.publicKey.toBase58(), extra.publicKey.toBase58()]),
    ]);

    const sent = getSent();
    expect(sent).not.toBeNull();
    expect(sent!.message.header.numRequiredSignatures).toBe(2);
    expect(sent!.signatures.every(hasSignature)).toBe(true);
  });

  it('accepts per-send local signers', async () => {
    const primary = Keypair.generate();
    const extra = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSent } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    await wallet.signAndSend(
      [makeInstruction([primary.publicKey.toBase58(), extra.publicKey.toBase58()])],
      { additionalSigners: [extra] as readonly Signer[] }
    );

    const sent = getSent();
    expect(sent).not.toBeNull();
    expect(sent!.message.header.numRequiredSignatures).toBe(2);
    expect(sent!.signatures.every(hasSignature)).toBe(true);
  });

  it('accepts standardized signers in send options', async () => {
    const primary = Keypair.generate();
    const extra = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSent } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    await wallet.signAndSend(
      [makeInstruction([primary.publicKey.toBase58(), extra.publicKey.toBase58()])],
      { signers: [extra] as readonly Signer[] }
    );

    const sent = getSent();
    expect(sent).not.toBeNull();
    expect(sent!.message.header.numRequiredSignatures).toBe(2);
    expect(sent!.signatures.every(hasSignature)).toBe(true);
  });

  it('supports overriding the fee payer with a local signer', async () => {
    const primary = Keypair.generate();
    const feePayer = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSent } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer, additionalSigners: [feePayer] });

    await wallet.signAndSend([makeInstruction([])], { feePayer });

    const sent = getSent();
    expect(sent).not.toBeNull();
    expect(sent!.message.staticAccountKeys[0]!.toBase58()).toBe(feePayer.publicKey.toBase58());
    expect(signer.calls).toBe(0);
    expect(hasSignature(sent!.signatures[0]!)).toBe(true);
  });

  it('fails fast when a required signer cannot be satisfied', async () => {
    const primary = Keypair.generate();
    const missing = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSendCalls } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    await expect(
      wallet.signAndSend([
        makeInstruction([primary.publicKey.toBase58(), missing.publicKey.toBase58()]),
      ])
    ).rejects.toThrow(/Missing signer\(s\) for transaction/);
    expect(getSendCalls()).toBe(0);
  });

  it('classifies wallet rejection as not submitted', async () => {
    const primary = Keypair.generate();
    const rejection = Object.assign(new Error('User rejected the request.'), { code: 4001 });
    const signer: VersionedTransactionSigner = {
      publicKey: primary.publicKey,
      async signTransaction() {
        throw rejection;
      },
    };
    const { connection, getSendCalls } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    const error = await wallet
      .signAndSend([makeInstruction([primary.publicKey.toBase58()])])
      .catch((cause: unknown) => cause);

    expect(failureOutcome(error)).toMatchObject({
      status: 'not-submitted',
      phase: 'wallet',
      cause: rejection,
    });
    expect(getSendCalls()).toBe(0);
  });

  it('accepts a signed transaction returned from another module instance', async () => {
    const primary = Keypair.generate();
    const signer: VersionedTransactionSigner = {
      publicKey: primary.publicKey,
      async signTransaction(tx) {
        tx.sign([primary]);
        return {
          serialize: () => tx.serialize(),
        } as VersionedTransaction;
      },
    };
    const { connection, getSendCalls } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    await expect(
      wallet.signAndSend([makeInstruction([primary.publicKey.toBase58()])])
    ).resolves.toMatchObject({ signature: 'sig-web3js' });
    expect(getSendCalls()).toBe(1);
  });

  it('rejects known legacy-only wallets before signing', async () => {
    const primary = Keypair.generate();
    const signer = createPrimarySigner(primary);
    signer.supportedTransactionVersions = null;
    const { connection, getSendCalls } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    const error = await wallet
      .signAndSend([makeInstruction([primary.publicKey.toBase58()])])
      .catch((cause: unknown) => cause);

    expect(failureOutcome(error)).toMatchObject({
      status: 'not-submitted',
      phase: 'wallet',
    });
    expect(error).toHaveProperty(
      'message',
      'The configured wallet does not support v0 VersionedTransaction signing'
    );
    expect(signer.calls).toBe(0);
    expect(getSendCalls()).toBe(0);
  });

  it('classifies a definite preflight rejection as not submitted', async () => {
    const primary = Keypair.generate();
    const preflightError = new SendTransactionError({
      action: 'send',
      signature: 'preflight-signature',
      transactionMessage: 'simulation failed: custom program error: 0x1771',
      logs: ['Program log: custom program error: 0x1771'],
    });
    const { connection, getSendCalls, getStatusCalls } = createConnectionStub({
      sendError: preflightError,
    });
    const wallet = createWalletAdapter({ connection, signer: createPrimarySigner(primary) });

    const error = await wallet
      .signAndSend([makeInstruction([primary.publicKey.toBase58()])])
      .catch((cause: unknown) => cause);

    expect(failureOutcome(error)).toMatchObject({
      status: 'not-submitted',
      phase: 'send',
      cause: preflightError,
    });
    expect(parseInstructionError(error, [
      { code: 6001, name: 'OreFailure', msg: 'ORE transaction failed' },
    ])).toEqual({
      code: 6001,
      name: 'OreFailure',
      message: 'ORE transaction failed',
    });
    expect(getSendCalls()).toBe(1);
    expect(getStatusCalls()).toBe(0);
  });

  it('classifies an uncertain send once without resubmitting', async () => {
    const primary = Keypair.generate();
    const sendError = new Error('RPC response was lost');
    const {
      connection,
      getSearchTransactionHistory,
      getSendCalls,
      getSent,
      getStatusCalls,
    } = createConnectionStub({ sendError, status: null });
    const wallet = createWalletAdapter({ connection, signer: createPrimarySigner(primary) });

    const error = await wallet
      .signAndSend([makeInstruction([primary.publicKey.toBase58()])])
      .catch((cause: unknown) => cause);
    const outcome = failureOutcome(error);

    expect(outcome).toMatchObject({
      status: 'submitted-unknown',
      phase: 'send',
      cause: sendError,
    });
    const sent = getSent();
    expect(sent).not.toBeNull();
    expect('signature' in outcome && outcome.signature).toBe(
      bs58.encode(sent!.signatures[0]!)
    );
    expect(getSendCalls()).toBe(1);
    expect(getStatusCalls()).toBe(1);
    expect(getSearchTransactionHistory()).toBe(true);
  });

  it('classifies an on-chain status after an uncertain send', async () => {
    const primary = Keypair.generate();
    const sendError = new Error('send connection closed');
    const chainError = { InstructionError: [0, { Custom: 6001 }] };
    const { connection, getSendCalls, getStatusCalls } = createConnectionStub({
      sendError,
      status: {
        slot: 789,
        confirmations: 1,
        confirmationStatus: 'confirmed',
        err: chainError,
      },
    });
    const wallet = createWalletAdapter({ connection, signer: createPrimarySigner(primary) });

    const error = await wallet
      .signAndSend([makeInstruction([primary.publicKey.toBase58()])])
      .catch((cause: unknown) => cause);
    const outcome = failureOutcome(error);

    expect(outcome).toMatchObject({
      status: 'chain-failed',
      phase: 'chain',
      slot: 789,
      cause: sendError,
      programError: { code: 6001, name: 'CustomError6001' },
    });
    expect('signature' in outcome && outcome.signature).toBeTruthy();
    expect(getSendCalls()).toBe(1);
    expect(getStatusCalls()).toBe(1);
  });

  it('preserves signature and slot for confirmation failure', async () => {
    const primary = Keypair.generate();
    const chainError = { InstructionError: [0, { Custom: 6002 }] };
    const { connection, getSendCalls, getStatusCalls } = createConnectionStub({
      confirmationResultError: chainError,
    });
    const wallet = createWalletAdapter({ connection, signer: createPrimarySigner(primary) });

    const error = await wallet
      .signAndSend([makeInstruction([primary.publicKey.toBase58()])])
      .catch((cause: unknown) => cause);

    expect(failureOutcome(error)).toMatchObject({
      status: 'chain-failed',
      signature: 'sig-web3js',
      slot: 456,
      cause: chainError,
      programError: { code: 6002 },
    });
    expect(getSendCalls()).toBe(1);
    expect(getStatusCalls()).toBe(0);
  });

  it('uses one status lookup after confirmation timeout', async () => {
    const primary = Keypair.generate();
    const timeout = new Error('confirmation timeout');
    const { connection, getSendCalls, getStatusCalls } = createConnectionStub({
      confirmationError: timeout,
      status: null,
    });
    const wallet = createWalletAdapter({ connection, signer: createPrimarySigner(primary) });

    const error = await wallet
      .signAndSend([makeInstruction([primary.publicKey.toBase58()])])
      .catch((cause: unknown) => cause);

    expect(failureOutcome(error)).toMatchObject({
      status: 'submitted-unknown',
      phase: 'confirmation',
      signature: 'sig-web3js',
      cause: timeout,
    });
    expect(getSendCalls()).toBe(1);
    expect(getStatusCalls()).toBe(1);
  });

  it('returns a confirmed status discovered after confirmation throws', async () => {
    const primary = Keypair.generate();
    const { connection, getSendCalls, getStatusCalls } = createConnectionStub({
      confirmationError: new Error('confirmation transport closed'),
      status: {
        slot: 800,
        confirmations: 2,
        confirmationStatus: 'confirmed',
        err: null,
      },
    });
    const wallet = createWalletAdapter({ connection, signer: createPrimarySigner(primary) });

    await expect(
      wallet.signAndSend([makeInstruction([primary.publicKey.toBase58()])])
    ).resolves.toEqual({ signature: 'sig-web3js', slot: 800 });
    expect(getSendCalls()).toBe(1);
    expect(getStatusCalls()).toBe(1);
  });
});

describe('inspectTransaction', () => {
  it('estimates fee and simulates without invoking the signer', async () => {
    const primary = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSendCalls } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });

    await expect(
      wallet.inspectTransaction!([makeInstruction([primary.publicKey.toBase58()])], {
        minContextSlot: 90,
        innerInstructions: true,
      })
    ).resolves.toEqual({
      feeLamports: 5000,
      feeContextSlot: 100,
      logs: ['Program log: inspected'],
      computeUnitsConsumed: 1234,
      contextSlot: 101,
      error: undefined,
      programError: undefined,
    });
    expect(signer.calls).toBe(0);
    expect(getSendCalls()).toBe(0);
  });

  it('returns a generic parsed custom error for core to enrich', async () => {
    const primary = Keypair.generate();
    const simulationError = { InstructionError: [0, { Custom: 7001 }] };
    const { connection } = createConnectionStub({ simulationError });
    const wallet = createWalletAdapter({
      connection,
      signer: createPrimarySigner(primary),
    });

    await expect(
      wallet.inspectTransaction!([makeInstruction([primary.publicKey.toBase58()])])
    ).resolves.toMatchObject({
      error: simulationError,
      programError: {
        code: 7001,
        name: 'CustomError7001',
        message: 'Unknown error with code 7001',
      },
    });
  });

  it('surfaces the loaded-accounts-data-size reported by simulation', async () => {
    const primary = Keypair.generate();
    const { connection } = createConnectionStub();
    const wallet = createWalletAdapter({
      connection: {
        ...connection,
        async simulateTransaction() {
          return {
            context: { slot: 101 },
            value: { err: null, logs: [], unitsConsumed: 1234, loadedAccountsDataSize: 65_536 },
          };
        },
      } as unknown as Connection,
      signer: createPrimarySigner(primary),
    });

    await expect(
      wallet.inspectTransaction!([makeInstruction([primary.publicKey.toBase58()])])
    ).resolves.toMatchObject({ loadedAccountsDataSize: 65_536 });
  });
});

describe('transaction v1 options', () => {
  it('transaction_v1 request is rejected without prompting the wallet', async () => {
    const primary = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSendCalls } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });
    const instructions = [makeInstruction([primary.publicKey.toBase58()])];

    expect(wallet.supportedTransactionVersions).toEqual([0]);
    await expect(wallet.signAndSend(instructions, { transactionVersion: 1 }))
      .rejects.toMatchObject({
        name: 'TransactionOptionsError',
        code: 'unsupported_transaction_version',
        requestedVersion: 1,
        supportedVersions: [0],
      });
    await expect(wallet.inspectTransaction!(instructions, { transactionVersion: 1 }))
      .rejects.toMatchObject({ code: 'unsupported_transaction_version' });
    expect(signer.calls).toBe(0);
    expect(getSendCalls()).toBe(0);
  });

  it('v1_contract resource options are rejected, leaving raw sends untouched', async () => {
    const primary = Keypair.generate();
    const signer = createPrimarySigner(primary);
    const { connection, getSendCalls } = createConnectionStub();
    const wallet = createWalletAdapter({ connection, signer });
    const instructions = [makeInstruction([primary.publicKey.toBase58()])];

    for (const resources of [
      { computeUnitLimit: 200_000 },
      { loadedAccountsDataSizeLimit: '65536' },
      { heapSize: 262_144 },
      { computeUnitPriceMicroLamports: 1_000n },
    ]) {
      await expect(wallet.signAndSend(instructions, { resources })).rejects.toMatchObject({
        name: 'TransactionOptionsError',
        code: 'unsupported_resource_option',
        option: Object.keys(resources)[0],
      });
    }
    // V1-only, so the version binding rejects it before the capability does.
    await expect(wallet.signAndSend(instructions, {
      resources: { priorityFeeLamports: 5_000n },
    })).rejects.toMatchObject({
      code: 'unsupported_resource_option',
      option: 'priorityFeeLamports',
    });
    await expect(wallet.signAndSend(instructions, {
      resources: { computeUnits: 1 } as never,
    })).rejects.toMatchObject({ code: 'unsupported_resource_option', option: 'computeUnits' });
    expect(signer.calls).toBe(0);
    expect(getSendCalls()).toBe(0);

    await expect(wallet.signAndSend(instructions)).resolves.toMatchObject({
      signature: 'sig-web3js',
    });
    expect(getSendCalls()).toBe(1);
  });
});

describe('connectionAccountLoader', () => {
  it('adapts a web3.js connection to the AccountLoader interface', async () => {
    const address = Keypair.generate().publicKey.toBase58();
    const getAccountInfo = async (publicKey: PublicKey) => {
      expect(publicKey.toBase58()).toBe(address);
      return { data: Buffer.from([1, 2, 3]) };
    };

    const loader = connectionAccountLoader({ getAccountInfo } as unknown as Connection);
    await expect(loader.getAccount(address)).resolves.toEqual({
      data: Uint8Array.from([1, 2, 3]),
    });
  });

  it('returns null when the connection misses', async () => {
    const address = Keypair.generate().publicKey.toBase58();
    const loader = connectionAccountLoader({
      async getAccountInfo() {
        return null;
      },
    } as unknown as Connection);

    await expect(loader.getAccount(address)).resolves.toBeNull();
  });
});

describe('instruction converters', () => {
  it('round-trips BuiltInstruction through TransactionInstruction', async () => {
    const { toTransactionInstruction, fromTransactionInstruction } = await import('./index');
    const signer = Keypair.generate().publicKey.toBase58();
    const writable = Keypair.generate().publicKey.toBase58();
    const original: BuiltInstruction = {
      programId: SYSTEM_PROGRAM,
      keys: [
        { pubkey: signer, isSigner: true, isWritable: false },
        { pubkey: writable, isSigner: false, isWritable: true },
      ],
      data: new Uint8Array([1, 2, 3, 255]),
    };

    const web3Instruction = toTransactionInstruction(original);
    expect(web3Instruction.programId.toBase58()).toBe(SYSTEM_PROGRAM);
    expect(web3Instruction.keys[0]!.pubkey.toBase58()).toBe(signer);
    expect(web3Instruction.keys[0]!.isSigner).toBe(true);

    const roundTripped = fromTransactionInstruction(web3Instruction);
    expect(roundTripped).toEqual(original);
  });

  it('does not depend on an ambient browser Buffer global', async () => {
    vi.stubGlobal('Buffer', undefined);
    const original: BuiltInstruction = {
      programId: SYSTEM_PROGRAM,
      keys: [],
      data: new Uint8Array([4, 5, 6]),
    };

    const web3Instruction = (await import('./index')).toTransactionInstruction(original);

    expect(new Uint8Array(web3Instruction.data)).toEqual(original.data);
  });
});
