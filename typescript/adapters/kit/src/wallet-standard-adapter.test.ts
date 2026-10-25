/**
 * `createWalletStandardWalletAdapter` end to end: a fake Wallet Standard
 * wallet that really signs the bytes it is handed, a fake Arete relay, and
 * real kit codecs in between. Covers the prompt callbacks and the adapter's
 * activity stages a UI shows while a transaction is in the wallet.
 */

import { describe, expect, it, vi } from 'vitest';
import {
  createKeyPairSignerFromPrivateKeyBytes,
  getBase64Encoder,
  getSignatureFromTransaction,
  getTransactionDecoder,
  getTransactionEncoder,
  partiallySignTransaction,
} from '@solana/kit';
import type { BuiltInstruction, TransactionTransport } from '@usearete/sdk';
import {
  KitTransactionExecutionError,
  SOLANA_SIGN_AND_SEND_TRANSACTION,
  SOLANA_SIGN_TRANSACTION,
  WalletStandardSignerError,
  createWalletStandardWalletAdapter,
  type KitWalletActivity,
  type WalletStandardSignTransactionInput,
} from './index';

const MEMO_PROGRAM = 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr';
const BLOCKHASH = '11111111111111111111111111111111';

const keypair = await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(7));
const localSigner = await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(8));

function memo(signers: readonly string[]): BuiltInstruction {
  return {
    programId: MEMO_PROGRAM,
    keys: signers.map((pubkey) => ({ pubkey, isSigner: true, isWritable: true })),
    data: new Uint8Array([1, 2, 3]),
  };
}

/** A Wallet Standard wallet with one account; `decline` makes it refuse. */
function fakeWallet(options: { decline?: unknown; features?: readonly string[] } = {}) {
  const account = {
    address: keypair.address,
    publicKey: new Uint8Array(32),
    chains: ['solana:devnet'] as const,
    features: options.features ?? [SOLANA_SIGN_TRANSACTION],
  };
  const signTransactionSpy = vi.fn(
    async (...inputs: readonly WalletStandardSignTransactionInput[]) => {
      if (options.decline) throw options.decline;
      return Promise.all(inputs.map(async (input) => ({
        signedTransaction: getTransactionEncoder().encode(
          await partiallySignTransaction([keypair.keyPair], getTransactionDecoder().decode(input.transaction))
        ) as Uint8Array,
      })));
    }
  );
  const wallet = {
    version: '1.0.0' as const,
    name: 'Fake',
    chains: ['solana:devnet'] as const,
    accounts: [account],
    features: {
      [SOLANA_SIGN_TRANSACTION]: {
        version: '1.0.0',
        supportedTransactionVersions: ['legacy', 0, 1] as const,
        signTransaction: signTransactionSpy,
      },
    },
  };
  return { wallet, account, signTransactionSpy };
}

function fakeRelay() {
  const calls: string[] = [];
  const transport = {
    async getLatestBlockhash() {
      calls.push('latest-blockhash');
      return { blockhash: BLOCKHASH, contextSlot: 1n, lastValidBlockHeight: 999n };
    },
    async sendTransaction(transaction: string) {
      calls.push('send');
      const decoded = getTransactionDecoder().decode(
        getBase64Encoder().encode(transaction) as Uint8Array
      );
      return { signature: getSignatureFromTransaction(decoded) };
    },
    async getSignatureStatus(signature: string) {
      calls.push('status');
      return { signature, confirmationStatus: 'confirmed' as const, err: null, slot: 42n };
    },
    async getBlockHeight() {
      return 10n;
    },
  } as unknown as TransactionTransport;
  return { transport, calls };
}

describe('createWalletStandardWalletAdapter', () => {
  it('signs in the wallet, relays through the session transport and reports each stage', async () => {
    const { wallet, account, signTransactionSpy } = fakeWallet();
    const { transport, calls } = fakeRelay();
    const timeline: string[] = [];
    const adapter = createWalletStandardWalletAdapter({
      wallet,
      account,
      chain: 'solana:devnet',
      onPromptOpen: (event) => timeline.push(`prompt-open:${event.transactionCount}`),
      onPromptClose: (event) => timeline.push(`prompt-close:${event.signed}`),
      onActivity: (activity) => timeline.push(`activity:${activity.stage}`),
    });

    expect(adapter.publicKey).toBe(keypair.address);
    const result = await adapter.signAndSend(
      [memo([keypair.address])],
      { statusPollIntervalMs: 0 },
      { transactionTransport: transport }
    );

    expect(signTransactionSpy).toHaveBeenCalledTimes(1);
    expect(signTransactionSpy.mock.calls[0][0].chain).toBe('solana:devnet');
    // The wallet is handed the account object it gave out, not a copy.
    expect(signTransactionSpy.mock.calls[0][0].account).toBe(account);
    expect(calls).toEqual(['latest-blockhash', 'send', 'status']);
    expect(result.slot).toBe(42);
    expect(timeline).toEqual([
      'activity:signing',
      'prompt-open:1',
      'prompt-close:true',
      'activity:sending',
      'activity:confirming',
    ]);
  });

  it('reports a declined prompt and rejects with a wallet-phase not-submitted outcome', async () => {
    const declined = Object.assign(new Error('User rejected the request.'), { code: 4001 });
    const { wallet, account } = fakeWallet({ decline: declined });
    const { transport, calls } = fakeRelay();
    const onPromptClose = vi.fn();
    const activity: KitWalletActivity[] = [];
    const adapter = createWalletStandardWalletAdapter({ wallet, account, onPromptClose });
    const unsubscribe = adapter.onActivity((event) => activity.push(event));

    const failure = await adapter.signAndSend(
      [memo([keypair.address])], undefined, { transactionTransport: transport }
    ).catch((error: unknown) => error);

    expect(failure).toBeInstanceOf(KitTransactionExecutionError);
    expect((failure as InstanceType<typeof KitTransactionExecutionError>).outcome)
      .toMatchObject({ status: 'not-submitted', phase: 'wallet' });
    expect(onPromptClose).toHaveBeenCalledWith(
      expect.objectContaining({ signed: false, error: declined })
    );
    expect(activity).toEqual([{ stage: 'signing' }]);
    expect(calls).not.toContain('send');
    unsubscribe();
  });

  it('signs with extra local signers in the same prompt', async () => {
    const { wallet, account, signTransactionSpy } = fakeWallet();
    const { transport } = fakeRelay();
    const adapter = createWalletStandardWalletAdapter({ wallet, account });

    await adapter.signAndSend(
      [memo([keypair.address, localSigner.address])],
      { statusPollIntervalMs: 0, signers: [localSigner] },
      { transactionTransport: transport }
    );

    expect(signTransactionSpy).toHaveBeenCalledTimes(1);
  });

  it('refuses a sign-and-send-only account before anything is built', () => {
    const { wallet, account } = fakeWallet({ features: [SOLANA_SIGN_AND_SEND_TRANSACTION] });

    expect(() => createWalletStandardWalletAdapter({ wallet, account }))
      .toThrow(WalletStandardSignerError);
  });

  it('ignores a throwing observer and stops calling an unsubscribed one', async () => {
    const { wallet, account } = fakeWallet();
    const { transport } = fakeRelay();
    const adapter = createWalletStandardWalletAdapter({
      wallet,
      account,
      onPromptOpen: () => {
        throw new Error('observer bug');
      },
    });
    const listener = vi.fn(() => {
      throw new Error('listener bug');
    });
    const unsubscribe = adapter.onActivity(listener);

    await expect(adapter.signAndSend(
      [memo([keypair.address])], { statusPollIntervalMs: 0 }, { transactionTransport: transport }
    )).resolves.toMatchObject({ slot: 42 });
    expect(listener).toHaveBeenCalledTimes(3);

    unsubscribe();
    await adapter.signAndSend(
      [memo([keypair.address])], { statusPollIntervalMs: 0 }, { transactionTransport: transport }
    );
    expect(listener).toHaveBeenCalledTimes(3);
  });
});
