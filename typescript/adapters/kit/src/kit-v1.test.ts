/**
 * Real-codec behaviour tests for the Kit adapter.
 *
 * Nothing here mocks `@solana/kit`: every assertion is made against bytes
 * this adapter actually produced, decoded by kit's own codec. The mocked
 * suite in `index.test.ts` covers submission and confirmation outcomes; this
 * one covers what the compiled message *is*.
 */

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import {
  createKeyPairSignerFromPrivateKeyBytes,
  getBase64Encoder,
  getCompiledTransactionMessageDecoder,
  getSignatureFromTransaction,
  getTransactionDecoder,
  type TransactionSigner,
} from '@solana/kit';
import { TransactionTransportError } from '@usearete/sdk';
import type { BuiltInstruction, TransactionTransport } from '@usearete/sdk';
import {
  MAX_COMPUTE_UNIT_LIMIT,
  MAX_LOADED_ACCOUNTS_DATA_SIZE,
  KitTransactionExecutionError,
  createWalletAdapter,
} from './index';

const FIXTURES = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL('../../../../tests/fixtures/transaction-v1/transactions.json', import.meta.url)
    ),
    'utf8'
  )
) as {
  payer: string;
  cosigner: string;
  fixtures: Record<string, {
    version: 'legacy' | 0 | 1;
    signatureCount: number;
    firstSignature: string;
    bytes: number;
    base64: string;
  }>;
};

const MEMO_PROGRAM = 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr';
const FIXTURE_BLOCKHASH = '11111111111111111111111111111111';
const COMPUTE_BUDGET_PROGRAM = 'ComputeBudget111111111111111111111111111111';

const payer = await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(1));
const cosigner = await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(2));

function memo(data: readonly number[], signers: readonly string[] = []): BuiltInstruction {
  return {
    programId: MEMO_PROGRAM,
    keys: signers.map((pubkey) => ({ pubkey, isSigner: true, isWritable: true })),
    data: new Uint8Array(data),
  };
}

interface FakeTransportOptions {
  unitsConsumed?: bigint;
  loadedAccountsDataSize?: bigint;
  simulationError?: unknown;
  /** A signature of the relay's own, instead of echoing the signed one. */
  relaySignature?: string;
  /** Fail the submission with this, after recording the dispatch. */
  sendError?: unknown;
  /** Record the dispatch, then never answer. */
  stallSend?: boolean;
  /** Answer the submission, then never answer a status poll. */
  stallStatus?: boolean;
}

/** A promise that never settles, for the stalled-backend probes. */
function pending<T>(): Promise<T> {
  // Executor form: `Promise.withResolvers` needs Node 22, and this package
  // targets the Node 20.18 floor Kit 8 declares.
  return new Promise<T>(() => {});
}

/** Recording Arete relay: `calls` is how "never sent" is asserted. */
function fakeTransport(options: FakeTransportOptions = {}) {
  const calls: string[] = [];
  const simulated: string[] = [];
  const sent: string[] = [];
  const transport = {
    async getLatestBlockhash() {
      calls.push('latest-blockhash');
      return { blockhash: FIXTURE_BLOCKHASH, contextSlot: 1n, lastValidBlockHeight: 999n };
    },
    async getFeeForMessage() {
      calls.push('fee');
      return { feeLamports: 5_000n, contextSlot: 400n };
    },
    async simulateTransaction(transaction: string) {
      calls.push('simulate');
      simulated.push(transaction);
      return {
        contextSlot: 401n,
        err: options.simulationError ?? null,
        logs: ['Program log: ok'],
        unitsConsumed: 'unitsConsumed' in options ? options.unitsConsumed : 1_200n,
        loadedAccountsDataSize:
          'loadedAccountsDataSize' in options ? options.loadedAccountsDataSize : 4_096n,
      };
    },
    async sendTransaction(transaction: string) {
      calls.push('send');
      sent.push(transaction);
      if (options.stallSend) return pending<{ signature: string }>();
      if (options.sendError) throw options.sendError;
      const decoded = getTransactionDecoder().decode(
        getBase64Encoder().encode(transaction) as Uint8Array
      );
      return { signature: options.relaySignature ?? getSignatureFromTransaction(decoded) };
    },
    async getSignatureStatus(signature: string) {
      calls.push('status');
      if (options.stallStatus) return pending<unknown>();
      return { signature, confirmationStatus: 'confirmed' as const, err: null, slot: 42n };
    },
    async getBlockHeight() {
      calls.push('height');
      return 10n;
    },
  } as unknown as TransactionTransport;
  return { transport, calls, simulated, sent };
}

function adapter(transport: TransactionTransport, signers: readonly TransactionSigner[] = []) {
  return createWalletAdapter({
    transport,
    signer: payer,
    additionalSigners: signers,
    defaultCommitment: 'confirmed',
  });
}

/** The signature the submitted bytes actually carry. */
function submittedSignature(sent: readonly string[]): string {
  return getSignatureFromTransaction(decodeWire(sent[0]).transaction);
}

function decodeWire(wire: string) {
  const bytes = getBase64Encoder().encode(wire) as Uint8Array;
  const transaction = getTransactionDecoder().decode(bytes);
  return {
    bytes,
    transaction,
    message: getCompiledTransactionMessageDecoder().decode(transaction.messageBytes),
  };
}

// ---------------------------------------------------------------------------
// The corpus and this Kit release agree
// ---------------------------------------------------------------------------

describe('@solana/kit 8 codec against the shared V1 corpus', () => {
  it.each(['legacy', 'v0', 'v1', 'v1_oversize', 'v1_two_signatures'])(
    'decodes the %s fixture to its recorded version, size and signatures',
    (name) => {
      const fixture = FIXTURES.fixtures[name];
      const { bytes, transaction, message } = decodeWire(fixture.base64);

      expect(bytes.length).toBe(fixture.bytes);
      expect(message.version).toBe(fixture.version);
      expect(Object.keys(transaction.signatures)).toHaveLength(fixture.signatureCount);
      expect(Object.keys(transaction.signatures)[0]).toBe(FIXTURES.payer);
    }
  );

  it('records a V1 payload past the legacy ceiling', () => {
    expect(FIXTURES.fixtures.v1_oversize.bytes).toBeGreaterThan(1232);
  });
});

// ---------------------------------------------------------------------------
// One planner: what each version's compiled message actually carries
// ---------------------------------------------------------------------------

describe('version planning', () => {
  it('defaults to v0 when the caller names no version', async () => {
    const { transport, sent } = fakeTransport();

    await adapter(transport).signAndSend([memo([1, 2, 3])]);

    expect(decodeWire(sent[0]).message.version).toBe(0);
  });

  it.each([['legacy', 'legacy'], [0, 0], [1, 1]] as const)(
    'compiles the explicitly requested version %s',
    async (requested, expected) => {
      const { transport, sent } = fakeTransport();

      await adapter(transport).signAndSend([memo([1, 2, 3])], {
        transactionVersion: requested,
        resources: requested === 1 ? { priorityFeeLamports: 5_000n } : {},
      });

      expect(decodeWire(sent[0]).message.version).toBe(expected);
    }
  );

  it('carries the V1 budget inline and adds no ComputeBudget instruction', async () => {
    const { transport, sent } = fakeTransport();

    await adapter(transport).signAndSend([memo([1, 2, 3])], {
      transactionVersion: 1,
      resources: {
        computeUnitLimit: 20_000,
        loadedAccountsDataSizeLimit: 65_536,
        heapSize: 64 * 1024,
        priorityFeeLamports: 5_000n,
      },
    });

    const { message } = decodeWire(sent[0]);
    expect(message.version).toBe(1);
    expect(message.numInstructions).toBe(1);
    expect(message.staticAccounts).not.toContain(COMPUTE_BUDGET_PROGRAM);
    // Every configured field lands in the config mask, in wire order.
    expect(message.configValues.map((value: { value: unknown }) => value.value)).toEqual([
      5_000n,
      20_000,
      65_536,
      64 * 1024,
    ]);
  });

  it('renders the same budget as ComputeBudget instructions on v0', async () => {
    const { transport, sent } = fakeTransport();

    await adapter(transport).signAndSend([memo([1, 2, 3])], {
      transactionVersion: 0,
      resources: { computeUnitLimit: 20_000, computeUnitPriceMicroLamports: 7n },
    });

    const { message } = decodeWire(sent[0]);
    expect(message.version).toBe(0);
    expect(message.staticAccounts).toContain(COMPUTE_BUDGET_PROGRAM);
    expect(message.instructions).toHaveLength(3);
  });

  it('keeps a u64 priority fee exact', async () => {
    const { transport, sent } = fakeTransport();
    const fee = (1n << 64n) - 1n;

    await adapter(transport).signAndSend([memo([1])], {
      transactionVersion: 1,
      resources: {
        priorityFeeLamports: fee,
        computeUnitLimit: 1_000,
        loadedAccountsDataSizeLimit: 32_768,
      },
    });

    expect(decodeWire(sent[0]).message.configValues[0].value).toBe(fee);
  });

  it('signs every required signer in message order', async () => {
    const { transport, sent } = fakeTransport();

    const result = await adapter(transport, [cosigner]).signAndSend(
      [memo([9], [payer.address, cosigner.address])],
      {
        transactionVersion: 1,
        resources: { computeUnitLimit: 1_000, loadedAccountsDataSizeLimit: 32_768 },
      }
    );

    const { transaction } = decodeWire(sent[0]);
    expect(Object.keys(transaction.signatures)).toEqual([payer.address, cosigner.address]);
    expect(result.signature).toBe(payer.address === '' ? '' : result.signature);
    expect(Object.values(transaction.signatures).every((s) => s !== null)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// V1 budget resolution
// ---------------------------------------------------------------------------

describe('V1 budget estimation', () => {
  it('simulates a provisional message declaring the protocol maxima', async () => {
    // Signature verification being off does not lift a resource limit, so a
    // provisional message carrying the SIMD-0385 minimum could never produce
    // an ordinary successful estimate.
    const { transport, simulated } = fakeTransport();

    await adapter(transport).signAndSend([memo([1])], { transactionVersion: 1 });

    const provisional = decodeWire(simulated[0]).message;
    expect(provisional.configValues.map((value: { value: unknown }) => value.value)).toEqual([
      MAX_COMPUTE_UNIT_LIMIT,
      MAX_LOADED_ACCOUNTS_DATA_SIZE,
    ]);
  });

  it('derives both budgets with headroom and protocol bounds', async () => {
    const { transport, sent } = fakeTransport({
      unitsConsumed: 1_234n,
      loadedAccountsDataSize: 4_096n,
    });

    await adapter(transport).signAndSend([memo([1])], { transactionVersion: 1 });

    // 1234 + 20% = 1480.8 -> 1481; 4096 bytes -> one 32 KiB page + one page.
    expect(decodeWire(sent[0]).message.configValues.map((v: { value: unknown }) => v.value))
      .toEqual([1_481, 65_536]);
  });

  it('bounds a derived budget by the protocol maximum', async () => {
    const { transport, sent } = fakeTransport({
      unitsConsumed: BigInt(MAX_COMPUTE_UNIT_LIMIT),
      loadedAccountsDataSize: BigInt(MAX_LOADED_ACCOUNTS_DATA_SIZE),
    });

    await adapter(transport).signAndSend([memo([1])], { transactionVersion: 1 });

    expect(decodeWire(sent[0]).message.configValues.map((v: { value: unknown }) => v.value))
      .toEqual([MAX_COMPUTE_UNIT_LIMIT, MAX_LOADED_ACCOUNTS_DATA_SIZE]);
  });

  it('never raises an explicit budget and still measures the other', async () => {
    const { transport, sent, simulated } = fakeTransport({ unitsConsumed: 1_234n });

    await adapter(transport).signAndSend([memo([1])], {
      transactionVersion: 1,
      resources: { computeUnitLimit: 500 },
    });

    // The caller's own value is what the provisional message declares too.
    expect(decodeWire(simulated[0]).message.configValues[0].value).toBe(500);
    expect(decodeWire(sent[0]).message.configValues.map((v: { value: unknown }) => v.value))
      .toEqual([500, 65_536]);
  });

  it('skips the round trip entirely when both budgets are explicit', async () => {
    const { transport, calls } = fakeTransport();

    await adapter(transport).signAndSend([memo([1])], {
      transactionVersion: 1,
      resources: { computeUnitLimit: 500, loadedAccountsDataSizeLimit: 32_768 },
    });

    expect(calls.filter((call) => call === 'simulate')).toHaveLength(0);
  });

  it.each([
    ['unitsConsumed', { unitsConsumed: undefined }, 'computeUnitLimit'],
    ['loadedAccountsDataSize', { loadedAccountsDataSize: undefined }, 'loadedAccountsDataSizeLimit'],
  ])('refuses to sign when the simulation reports no %s', async (_metric, options, named) => {
    const { transport, calls } = fakeTransport(options as FakeTransportOptions);

    await expect(
      adapter(transport).signAndSend([memo([1])], { transactionVersion: 1 })
    ).rejects.toMatchObject({
      outcome: { status: 'not-submitted', phase: 'build' },
      cause: { message: expect.stringContaining(named) },
    });
    expect(calls).not.toContain('send');
  });
});

// ---------------------------------------------------------------------------
// Wire limits and V1 structural caps
// ---------------------------------------------------------------------------

describe('limits', () => {
  const explicitV1 = {
    transactionVersion: 1 as const,
    resources: { computeUnitLimit: 1_000, loadedAccountsDataSizeLimit: 32_768 },
  };

  it('accepts a V1 payload over the legacy ceiling', async () => {
    const { transport, sent } = fakeTransport();

    await adapter(transport).signAndSend([memo(new Array(1_400).fill(7))], explicitV1);

    expect(decodeWire(sent[0]).bytes.length).toBeGreaterThan(1232);
  });

  it('rejects the same payload for v0', async () => {
    const { transport, calls } = fakeTransport();

    await expect(
      adapter(transport).signAndSend([memo(new Array(1_400).fill(7))], { transactionVersion: 0 })
    ).rejects.toMatchObject({
      outcome: { status: 'not-submitted', phase: 'build' },
      cause: { message: expect.stringContaining('1232-byte limit') },
    });
    expect(calls).not.toContain('send');
  });

  it('rejects a V1 payload over 4096 bytes', async () => {
    const { transport, calls } = fakeTransport();

    await expect(
      adapter(transport).signAndSend([memo(new Array(4_200).fill(7))], explicitV1)
    ).rejects.toMatchObject({
      cause: { message: expect.stringContaining('4096-byte limit') },
    });
    expect(calls).not.toContain('send');
  });

  // The three structural caps are the codec's own, so these prove the
  // rejection reaches the caller as a build failure with nothing sent.
  it('rejects more than 64 V1 instructions before signing', async () => {
    const { transport, calls } = fakeTransport();

    await expect(
      adapter(transport).signAndSend(
        Array.from({ length: 65 }, () => memo([1])),
        explicitV1
      )
    ).rejects.toMatchObject({
      outcome: { status: 'not-submitted', phase: 'build' },
      cause: { message: expect.stringContaining('maximum allowed is 64') },
    });
    expect(calls).not.toContain('send');
  });

  it('rejects more than 64 V1 accounts before signing', async () => {
    const { transport, calls } = fakeTransport();
    const accounts = await Promise.all(
      Array.from({ length: 70 }, async (_unused, index) => ({
        pubkey: (
          await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(index + 3))
        ).address as string,
        isSigner: false,
        isWritable: false,
      }))
    );

    await expect(
      adapter(transport).signAndSend(
        [{ programId: MEMO_PROGRAM, keys: accounts, data: new Uint8Array([1]) }],
        explicitV1
      )
    ).rejects.toMatchObject({
      outcome: { status: 'not-submitted', phase: 'build' },
      cause: { message: expect.stringContaining('maximum allowed is 64') },
    });
    expect(calls).not.toContain('send');
  });

  it('rejects more than 12 V1 signatures before signing', async () => {
    const { transport, calls } = fakeTransport();
    const signers = await Promise.all(
      Array.from({ length: 13 }, (_unused, index) =>
        createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(index + 3)))
    );

    await expect(
      createWalletAdapter({ transport, signer: payer, additionalSigners: signers }).signAndSend(
        [memo([1], signers.map((signer) => signer.address))],
        explicitV1
      )
    ).rejects.toMatchObject({
      cause: { message: expect.stringContaining('maximum allowed is 12') },
    });
    expect(calls).not.toContain('send');
  });

  it('refuses an oversized payload during inspection, before any backend call', async () => {
    // Inspecting a payload the send path deterministically rejects must not
    // answer with metrics or a backend-specific simulation error.
    const { transport, calls } = fakeTransport();

    await expect(
      adapter(transport).inspectTransaction([memo(new Array(4_200).fill(7))], explicitV1)
    ).rejects.toThrow(/4096-byte limit/);
    expect(calls).not.toContain('simulate');
    expect(calls).not.toContain('fee');
  });

  it('refuses an oversized provisional message before simulating it', async () => {
    const { transport, calls } = fakeTransport();

    await expect(
      adapter(transport).signAndSend(
        [memo(new Array(4_200).fill(7))],
        { transactionVersion: 1 }
      )
    ).rejects.toMatchObject({
      outcome: { status: 'not-submitted', phase: 'build' },
      cause: { message: expect.stringContaining('4096-byte limit') },
    });
    expect(calls).not.toContain('simulate');
  });
});

// ---------------------------------------------------------------------------
// Inputs the typed configuration owns
// ---------------------------------------------------------------------------

describe('rejected inputs', () => {
  it('refuses a caller-supplied ComputeBudget instruction on every version', async () => {
    const { transport, calls } = fakeTransport();
    const computeBudget: BuiltInstruction = {
      programId: COMPUTE_BUDGET_PROGRAM,
      keys: [],
      data: new Uint8Array([2, 0x20, 0x4e, 0, 0]),
    };

    for (const transactionVersion of ['legacy', 0, 1] as const) {
      await expect(
        adapter(transport).signAndSend([computeBudget], { transactionVersion })
      ).rejects.toMatchObject({
        cause: { message: expect.stringContaining('ComputeBudget') },
      });
    }
    expect(calls).not.toContain('send');
  });

  it('refuses address lookup table inputs', async () => {
    const { transport, calls } = fakeTransport();

    await expect(
      adapter(transport).signAndSend([memo([1])], {
        transactionVersion: 1,
        addressLookupTables: ['7Zb1bGi3pbZUeorUQrCLuJvQvJoWYFsGRe4uSNsWjqCG'],
      } as never)
    ).rejects.toMatchObject({
      cause: { message: expect.stringContaining('lookup tables') },
    });
    expect(calls).not.toContain('send');
  });

  it('refuses an empty instruction list', async () => {
    const { transport, calls } = fakeTransport();

    await expect(adapter(transport).signAndSend([])).rejects.toMatchObject({
      outcome: { status: 'not-submitted', phase: 'build' },
    });
    expect(calls).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// Unsigned inspection
// ---------------------------------------------------------------------------

describe('inspection', () => {
  /** Fails the test if inspection ever reaches a signer. */
  function signerSpy(): TransactionSigner {
    return {
      address: payer.address,
      signTransactions: vi.fn(() => {
        throw new Error('a signer was reached: inspection must never sign');
      }),
    } as unknown as TransactionSigner;
  }

  it('never signs, sends or prompts, and reports the budget it applied', async () => {
    const { transport, calls } = fakeTransport();
    const spy = signerSpy();

    const inspection = await createWalletAdapter({ transport, signer: spy })
      .inspectTransaction([memo([1, 2, 3])], { transactionVersion: 1 });

    expect(spy.signTransactions).not.toHaveBeenCalled();
    expect(calls).not.toContain('send');
    expect(inspection.transactionVersion).toBe(1);
    expect(inspection.feeLamports).toBe(5_000);
    expect(inspection.computeUnitsConsumed).toBe(1_200);
    expect(inspection.loadedAccountsDataSize).toBe(4_096);
    // The provisional maxima are what the reported metrics describe.
    expect(inspection.resources).toEqual({
      computeUnitLimit: String(MAX_COMPUTE_UNIT_LIMIT),
      loadedAccountsDataSizeLimit: String(MAX_LOADED_ACCOUNTS_DATA_SIZE),
    });
  });

  it('inspects the payload it would submit once the budgets are pinned', async () => {
    const inspecting = fakeTransport();
    const sending = fakeTransport();
    const resources = { computeUnitLimit: 1_000, loadedAccountsDataSizeLimit: 32_768 };

    await adapter(inspecting.transport).inspectTransaction([memo([1, 2, 3])], {
      transactionVersion: 1,
      resources,
    });
    await adapter(sending.transport).signAndSend([memo([1, 2, 3])], {
      transactionVersion: 1,
      resources,
    });

    const inspected = decodeWire(inspecting.simulated[0]);
    const submitted = decodeWire(sending.sent[0]);
    expect(inspected.bytes.length).toBe(submitted.bytes.length);
    expect(inspected.transaction.messageBytes).toEqual(submitted.transaction.messageBytes);
  });

  it('keeps an unmeasured metric missing rather than zero', async () => {
    const { transport } = fakeTransport({ loadedAccountsDataSize: undefined });

    const inspection = await adapter(transport).inspectTransaction([memo([1])]);

    expect(inspection.loadedAccountsDataSize).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// The locally derived signature, and the shared submission deadline
// ---------------------------------------------------------------------------

describe('relay submission', () => {
  const explicitV1 = {
    transactionVersion: 1 as const,
    resources: { computeUnitLimit: 1_000, loadedAccountsDataSizeLimit: 32_768 },
  };
  const impostor = '4bMuqmB1nZ1nH6WHhqnbGdrWvGFtcH1sZLYNHFDaCmoxfPnCnSKmyVW3xQ3DFXbEsdKrLtC2K1e2A1SmXKKbrKPu';

  /** The rejection, so `sent` is populated before it is read. */
  async function failedSend(
    transport: TransactionTransport,
    options: Record<string, unknown> = {}
  ) {
    return adapter(transport)
      .signAndSend([memo([1])], { ...explicitV1, ...options })
      .then(
        () => { throw new Error('expected the send to fail'); },
        (cause: KitTransactionExecutionError) => cause
      );
  }

  it('reconciles the signature it signed, not the one the relay echoed', async () => {
    // Adopting the echo would poll a different transaction entirely and
    // report its status as this one's.
    const { transport, calls, sent } = fakeTransport({ relaySignature: impostor });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const result = await adapter(transport).signAndSend([memo([1])], explicitV1);

    expect(result.signature).toBe(submittedSignature(sent));
    expect(result.signature).not.toBe(impostor);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining(impostor));
    expect(calls.filter((call) => call === 'send')).toHaveLength(1);
    warn.mockRestore();
  });

  it('keeps that signature on an ambiguous error carrying another', async () => {
    const { transport, sent } = fakeTransport({
      sendError: new TransactionTransportError(504, {
        code: 'upstream_timeout',
        message: 'Submission outcome is unknown',
        retryable: false,
        submission_state: 'unknown',
        signature: impostor,
      }),
    });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const error = await failedSend(transport);

    expect(error.outcome).toMatchObject({
      status: 'submitted-unknown',
      signature: submittedSignature(sent),
    });
    expect(warn).toHaveBeenCalledWith(expect.stringContaining(impostor));
    warn.mockRestore();
  });

  it('still reports a proven non-dispatch as not submitted', async () => {
    const { transport } = fakeTransport({
      sendError: new TransactionTransportError(400, {
        code: 'invalid_transaction',
        message: 'blockhash not found',
        retryable: false,
        submission_state: 'not_submitted',
      }),
    });

    const error = await failedSend(transport);

    expect(error.outcome).toMatchObject({ status: 'not-submitted', phase: 'send' });
  });

  it('ends a stalled submission at the deadline with the signed signature', async () => {
    const { transport, calls, sent } = fakeTransport({ stallSend: true });

    const error = await failedSend(transport, {
      confirmationTimeoutMs: 30,
      statusPollIntervalMs: 1,
    });

    expect(error.outcome).toMatchObject({
      status: 'submitted-unknown',
      phase: 'send',
      signature: submittedSignature(sent),
    });
    expect(calls.filter((call) => call === 'send')).toHaveLength(1);
    expect(calls).not.toContain('status');
  });

  it('ends a stalled status request at the same deadline', async () => {
    const { transport, calls, sent } = fakeTransport({ stallStatus: true });

    const error = await failedSend(transport, {
      confirmationTimeoutMs: 30,
      statusPollIntervalMs: 1,
    });

    expect(error.outcome).toMatchObject({
      status: 'submitted-unknown',
      phase: 'confirmation',
      signature: submittedSignature(sent),
    });
    expect(calls.filter((call) => call === 'send')).toHaveLength(1);
  });
});
