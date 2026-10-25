/**
 * `useWalletStandardWallet` against a fake Wallet Standard wallet registered
 * the way `@wallet-standard/react` registers real ones, with the real
 * adapter-kit bridge and kit codecs, driving `useOperationMutation`.
 */

import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import {
  createKeyPairSignerFromPrivateKeyBytes,
  getBase64Encoder,
  getSignatureFromTransaction,
  getTransactionDecoder,
  getTransactionEncoder,
  partiallySignTransaction,
} from '@solana/kit';
import type { Wallet, WalletAccount } from '@wallet-standard/base';
import type { UiWalletAccount } from '@wallet-standard/ui-core';
import { getOrCreateUiWalletAccountForStandardWalletAccount } from '@wallet-standard/ui-registry';
import {
  createPreparedInstruction,
  executePreparedOperation,
  type OperationExecutionOptions,
  type PreparedInstruction,
  type PreparedOperation,
  type TransactionTransport,
  type WalletAdapter,
} from '@usearete/sdk';

import { useOperationMutation, type OperationExecutor } from './hooks/use-operation-mutation';
import {
  useWalletStandardWallet,
  type UseWalletStandardWalletResult,
  type WalletStandardSelection,
} from './wallet-standard';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const originalConsoleError = console.error;
beforeAll(() => {
  jest.spyOn(console, 'error').mockImplementation((message: unknown, ...args: unknown[]) => {
    if (typeof message === 'string' && message.startsWith('react-test-renderer is deprecated')) return;
    originalConsoleError(message, ...args);
  });
});
afterAll(() => {
  jest.restoreAllMocks();
});

const MEMO = 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr';
const SIGN_TRANSACTION = 'solana:signTransaction';

async function fakeWallet(features: readonly `${string}:${string}`[] = [SIGN_TRANSACTION]) {
  const keypair = await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(3));
  let release: (() => void) | null = null;
  let decline: ((error: unknown) => void) | null = null;
  const account: WalletAccount = {
    address: keypair.address,
    publicKey: new Uint8Array(32),
    chains: ['solana:devnet'],
    features,
  };
  const signTransaction = jest.fn(async (...inputs: readonly {
    account: unknown;
    transaction: Uint8Array;
  }[]) => {
    await new Promise<void>((resolve, reject) => {
      release = resolve;
      decline = reject;
    });
    return Promise.all(inputs.map(async (input) => ({
      signedTransaction: getTransactionEncoder().encode(
        await partiallySignTransaction(
          [keypair.keyPair],
          getTransactionDecoder().decode(input.transaction)
        )
      ) as Uint8Array,
    })));
  });
  const wallet: Wallet = {
    version: '1.0.0',
    name: 'Fake Wallet',
    icon: 'data:image/svg+xml;base64,',
    chains: ['solana:devnet'],
    accounts: [account],
    features: {
      [SIGN_TRANSACTION]: {
        version: '1.0.0',
        supportedTransactionVersions: ['legacy', 0, 1],
        signTransaction,
      },
    },
  };
  // What `@wallet-standard/react`'s `useWallets()` hands the app.
  const uiAccount = getOrCreateUiWalletAccountForStandardWalletAccount(wallet, account);
  return {
    keypair,
    wallet,
    account,
    uiAccount,
    signTransaction,
    approve: () => release?.(),
    decline: (error: unknown) => decline?.(error),
  };
}

function fakeRelay(): TransactionTransport {
  return {
    async getLatestBlockhash() {
      return {
        blockhash: '11111111111111111111111111111111',
        contextSlot: 1n,
        lastValidBlockHeight: 999n,
      };
    },
    async sendTransaction(transaction: string) {
      const decoded = getTransactionDecoder().decode(
        getBase64Encoder().encode(transaction) as Uint8Array
      );
      return { signature: getSignatureFromTransaction(decoded) };
    },
    async getSignatureStatus(signature: string) {
      return { signature, confirmationStatus: 'confirmed', err: null, slot: 42n };
    },
    async getBlockHeight() {
      return 10n;
    },
  } as unknown as TransactionTransport;
}

/** A session-like executor whose transactions go through `relay`. */
function relayExecutor(relay: TransactionTransport): OperationExecutor {
  return {
    execute<TPrepared extends PreparedOperation, TSigner = unknown>(
      prepared: TPrepared,
      options?: OperationExecutionOptions<TSigner, TPrepared>
    ) {
      return executePreparedOperation({
        async transaction(instructions, transactionOptions) {
          const wallet = transactionOptions?.wallet as WalletAdapter;
          return wallet.signAndSend(
            instructions,
            { ...(transactionOptions?.send ?? {}), statusPollIntervalMs: 0 } as never,
            { transactionTransport: relay }
          );
        },
      }, prepared, options);
    },
  };
}

function memoInstruction(signer: string): PreparedInstruction<{ note: string }> {
  return createPreparedInstruction({
    name: 'memo',
    artifacts: { note: 'hello' },
    instruction: {
      programId: MEMO,
      keys: [{ pubkey: signer, isSigner: true, isWritable: true }],
      data: new Uint8Array([104, 105]),
    },
  });
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 20; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

describe('useWalletStandardWallet', () => {
  it('signs a prepared operation in the connected wallet and reports the open prompt', async () => {
    const fake = await fakeWallet();
    const relay = fakeRelay();
    const executor = relayExecutor(relay);
    const current: {
      wallet?: UseWalletStandardWalletResult;
      operation?: ReturnType<typeof useOperationMutation<void, PreparedInstruction<{ note: string }>>>;
    } = {};
    function Harness({ account }: { account: UiWalletAccount }) {
      const wallet = useWalletStandardWallet(account, { chain: 'solana:devnet' });
      const operation = useOperationMutation(
        async () => memoInstruction(fake.keypair.address),
        { executor, wallet: wallet.wallet }
      );
      current.wallet = wallet;
      current.operation = operation;
      return null;
    }
    let renderer!: ReactTestRenderer;
    act(() => {
      renderer = create(React.createElement(Harness, { account: fake.uiAccount }));
    });
    expect(current.wallet?.address).toBe(fake.keypair.address);
    expect(current.wallet?.error).toBeNull();
    const adapter = current.wallet?.wallet;
    expect(adapter?.publicKey).toBe(fake.keypair.address);

    let run!: Promise<unknown>;
    act(() => {
      run = current.operation!.run();
    });
    await flush();
    expect(fake.signTransaction).toHaveBeenCalledTimes(1);
    // The wallet is asked with its own account object, not the UI handle.
    expect(fake.signTransaction.mock.calls[0][0].account).toBe(fake.account);
    expect(current.wallet?.isPromptOpen).toBe(true);
    expect(current.operation?.status).toBe('signing');
    expect(current.operation?.isAwaitingWallet).toBe(true);

    await act(async () => {
      fake.approve();
      await run;
    });
    expect(current.wallet?.isPromptOpen).toBe(false);
    expect(current.operation?.status).toBe('done');
    expect(current.operation?.artifacts).toEqual({ note: 'hello' });
    expect(current.operation?.result?.transaction.slot).toBe(42);

    // Re-rendering with the same account keeps the same adapter.
    act(() => {
      renderer.update(React.createElement(Harness, { account: fake.uiAccount }));
    });
    expect(current.wallet?.wallet).toBe(adapter);
  });

  it('closes the prompt and reports wallet-rejected when the user declines', async () => {
    const fake = await fakeWallet();
    const executor = relayExecutor(fakeRelay());
    const current: {
      wallet?: UseWalletStandardWalletResult;
      operation?: ReturnType<typeof useOperationMutation<void, PreparedInstruction<{ note: string }>>>;
    } = {};
    function Harness() {
      const wallet = useWalletStandardWallet(
        { wallet: fake.wallet, account: fake.account } satisfies WalletStandardSelection,
        { chain: 'solana:devnet' }
      );
      current.wallet = wallet;
      current.operation = useOperationMutation(
        async () => memoInstruction(fake.keypair.address),
        { executor, wallet: wallet.wallet }
      );
      return null;
    }
    act(() => {
      create(React.createElement(Harness));
    });

    let run!: Promise<unknown>;
    act(() => {
      run = current.operation!.run().catch((error: unknown) => error);
    });
    await flush();
    expect(current.wallet?.isPromptOpen).toBe(true);

    await act(async () => {
      fake.decline(Object.assign(new Error('User rejected the request.'), { code: 4001 }));
      await run;
    });
    expect(current.wallet?.isPromptOpen).toBe(false);
    expect(current.operation?.error?.kind).toBe('wallet-rejected');
  });

  it('explains an account that can only sign and send', async () => {
    const fake = await fakeWallet(['solana:signAndSendTransaction']);
    let result!: UseWalletStandardWalletResult;
    function Harness() {
      result = useWalletStandardWallet(fake.uiAccount);
      return null;
    }
    act(() => {
      create(React.createElement(Harness));
    });
    expect(result.wallet).toBeUndefined();
    expect(result.error).toMatchObject({
      name: 'WalletStandardSignerError',
      code: 'sign_transaction_unsupported',
    });
  });

  it('is empty without an account', () => {
    let result!: UseWalletStandardWalletResult;
    function Harness() {
      result = useWalletStandardWallet(null);
      return null;
    }
    act(() => {
      create(React.createElement(Harness));
    });
    expect(result).toEqual({ wallet: undefined, address: null, isPromptOpen: false, error: null });
  });
});
