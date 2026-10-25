import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import {
  createPreparedFlow,
  createPreparedTransactionBody,
  executePreparedOperation,
  TransactionExecutionError,
  type BuiltInstruction,
  type OperationExecutionOptions,
  type PreparedFlow,
  type PreparedOperation,
  type WalletAdapter,
} from '@usearete/sdk';

import {
  OperationMutationError,
  isWalletRejection,
  useOperationMutation,
  type OperationExecutor,
  type OperationExecutorSource,
  type OperationMutationStatus,
  type UseOperationMutationResult,
} from './use-operation-mutation';
import { AreteSessionProvider, type AnySession } from '../session-provider';

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
const PAYER = 'Payer111111111111111111111111111111111111111';
const CREATE_KEY = 'CreateKey1111111111111111111111111111111111';

function instruction(signers: readonly string[]): BuiltInstruction {
  return {
    programId: MEMO,
    keys: signers.map((pubkey) => ({ pubkey, isSigner: true, isWritable: true })),
    data: new Uint8Array([1]),
  };
}

type CreateArtifacts = { multisig: string };

function twoStepFlow(): PreparedFlow<CreateArtifacts> {
  return createPreparedFlow({
    name: 'createMultisig',
    artifacts: { multisig: 'Multisig1111' },
    transactions: [
      createPreparedTransactionBody({
        name: 'create',
        instructions: [instruction([PAYER, CREATE_KEY])],
      }),
      createPreparedTransactionBody({ name: 'configure', instructions: [instruction([PAYER])] }),
    ],
  });
}

type Activity = { stage: 'signing' } | { stage: 'sending' | 'confirming'; signature: string };

/**
 * A wallet that reports its activity like the adapter-kit adapters do. Each
 * send waits on `gate` while "the prompt is open", so a test can look at the
 * state in between.
 */
function activityWallet() {
  const listeners = new Set<(activity: Activity) => void>();
  const report = (activity: Activity) => listeners.forEach((listener) => listener(activity));
  let release: (() => void) | null = null;
  let reject: ((error: unknown) => void) | null = null;
  let sends = 0;
  const signAndSend = jest.fn(async (
    _instructions: readonly BuiltInstruction[],
    options?: { signers?: readonly unknown[] }
  ) => {
    sends += 1;
    const signature = `sig-${sends}`;
    report({ stage: 'signing' });
    await new Promise<void>((resolve, fail) => {
      release = resolve;
      reject = fail;
    });
    report({ stage: 'sending', signature });
    report({ stage: 'confirming', signature });
    return { signature, slot: 10 + sends, signers: options?.signers };
  });
  const wallet: WalletAdapter & { onActivity(listener: (activity: Activity) => void): () => void } = {
    publicKey: PAYER,
    signAndSend,
    onActivity(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  return {
    wallet,
    signAndSend,
    listenerCount: () => listeners.size,
    approve: () => release?.(),
    decline: (error: unknown) => reject?.(error),
  };
}

/** A session-like executor over the core executor, with `wallet` as its default wallet. */
function executorFor(defaultWallet?: WalletAdapter): OperationExecutor & { wallet?: WalletAdapter } {
  return {
    wallet: defaultWallet,
    execute<TPrepared extends PreparedOperation, TSigner = unknown>(
      prepared: TPrepared,
      options?: OperationExecutionOptions<TSigner, TPrepared>
    ) {
      return executePreparedOperation({
        wallet: defaultWallet,
        async transaction(instructions, transactionOptions) {
          const wallet = transactionOptions?.wallet ?? defaultWallet;
          if (!wallet) throw new Error('no wallet');
          return wallet.signAndSend(instructions, {
            ...(transactionOptions?.send ?? {}),
            signers: transactionOptions?.signers,
          });
        },
      }, prepared, { availableSignerAddresses: [PAYER, CREATE_KEY], ...options });
    },
  };
}

type Hook = UseOperationMutationResult<{ threshold: number }, PreparedFlow<CreateArtifacts>>;

function renderOperationHook(
  prepare: (input: { threshold: number }) => Promise<
    PreparedFlow<CreateArtifacts> | { operation: PreparedFlow<CreateArtifacts>; signers?: readonly unknown[] }
  >,
  options: {
    executor?: OperationExecutorSource;
    wallet?: WalletAdapter;
    allowExecutorWallet?: boolean;
    signers?: readonly unknown[];
    onSuccess?: jest.Mock;
    onError?: jest.Mock;
  },
  wrap: (children: React.ReactElement) => React.ReactElement = (children) => children
) {
  const seen: OperationMutationStatus[] = ['idle'];
  const current: { hook: Hook | null } = { hook: null };
  function Harness() {
    const hook = useOperationMutation(prepare, {
      ...options,
      onStatusChange: (status) => seen.push(status),
    });
    current.hook = hook;
    return null;
  }
  let renderer!: ReactTestRenderer;
  act(() => {
    renderer = create(wrap(React.createElement(Harness)));
  });
  return {
    get hook(): Hook {
      if (!current.hook) throw new Error('not rendered');
      return current.hook;
    },
    seen,
    renderer,
  };
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 5; i += 1) await Promise.resolve();
  });
}

describe('useOperationMutation', () => {
  it('walks preparing, signing, sending and confirming once per transaction, with every signer', async () => {
    const { wallet, signAndSend, approve, listenerCount } = activityWallet();
    const createKey = { address: CREATE_KEY };
    const hookSigner = { address: 'HookSigner' };
    const onSuccess = jest.fn();
    const view = renderOperationHook(
      async () => ({ operation: twoStepFlow(), signers: [createKey] }),
      { executor: executorFor(), wallet, signers: [hookSigner], onSuccess }
    );
    expect(view.hook.status).toBe('idle');

    let receipt!: Promise<unknown>;
    act(() => {
      receipt = view.hook.run({ threshold: 2 });
    });
    await flush();
    expect(view.hook.status).toBe('signing');
    expect(view.hook.isAwaitingWallet).toBe(true);
    expect(view.hook.transactionIndex).toBe(0);
    expect(view.hook.transactionCount).toBe(2);
    expect(view.hook.artifacts).toEqual({ multisig: 'Multisig1111' });

    await act(async () => {
      approve();
    });
    await flush();
    expect(view.hook.status).toBe('signing');
    expect(view.hook.transactionIndex).toBe(1);
    expect(view.hook.signatures).toEqual(['sig-1']);

    await act(async () => {
      approve();
      await receipt;
    });

    expect(view.hook.status).toBe('done');
    expect(view.hook.result).toMatchObject({ kind: 'flow', signatures: ['sig-1', 'sig-2'] });
    expect(view.seen).toEqual([
      'idle', 'preparing', 'signing', 'sending', 'confirming', 'preparing', 'signing', 'sending',
      'confirming', 'done',
    ]);
    // The hook's, prepare's and the run's signers all reach the wallet.
    expect(signAndSend.mock.calls[0][1]?.signers).toEqual([hookSigner, createKey]);
    expect(onSuccess).toHaveBeenCalledWith(view.hook.result, view.hook.prepared);
    // The activity subscription ends with the run.
    expect(listenerCount()).toBe(0);
  });

  it('reports a declined wallet prompt as wallet-rejected', async () => {
    const { wallet, decline } = activityWallet();
    const onError = jest.fn();
    const view = renderOperationHook(async () => twoStepFlow(), {
      executor: executorFor(), wallet, onError,
    });

    let failure!: Promise<unknown>;
    act(() => {
      failure = view.hook.run({ threshold: 1 }).catch((error: unknown) => error);
    });
    await flush();
    await act(async () => {
      decline(new TransactionExecutionError({
        status: 'not-submitted',
        phase: 'wallet',
        cause: Object.assign(new Error('User rejected the request.'), { code: 4001 }),
      }));
      await failure;
    });

    const error = await failure;
    expect(error).toBeInstanceOf(OperationMutationError);
    expect(view.hook.status).toBe('error');
    expect(view.hook.error?.kind).toBe('wallet-rejected');
    expect(view.hook.error?.failure).toMatchObject({ status: 'not-submitted', phase: 'wallet' });
    expect(onError).toHaveBeenCalledWith(view.hook.error);
  });

  it('classifies each wallet failure by its signer error code', async () => {
    const signerError = (code: string) => Object.assign(new Error(code), {
      name: 'WalletStandardSignerError',
      code,
    });
    for (const [code, kind] of [
      ['wallet_returned_invalid_transaction', 'wallet-modified'],
      ['sign_transaction_unsupported', 'wallet-feature-unsupported'],
      ['chain_unsupported', 'wallet-chain-unsupported'],
      ['transaction_version_unsupported', 'wallet-version-unsupported'],
      ['user_rejected', 'wallet-rejected'],
    ] as const) {
      const wallet: WalletAdapter = {
        publicKey: PAYER,
        signAndSend: async () => {
          throw new TransactionExecutionError({
            status: 'not-submitted', phase: 'wallet', cause: signerError(code),
          });
        },
      };
      const view = renderOperationHook(async () => twoStepFlow(), { executor: executorFor(), wallet });
      await act(async () => {
        await view.hook.run({ threshold: 1 }).catch(() => undefined);
      });
      expect(view.hook.error?.kind).toBe(kind);
    }
  });

  it('keeps the confirmed signatures of a flow that failed on chain', async () => {
    let sends = 0;
    const wallet: WalletAdapter = {
      publicKey: PAYER,
      signAndSend: async () => {
        sends += 1;
        if (sends === 1) return { signature: 'sig-ok' };
        throw new TransactionExecutionError({
          status: 'chain-failed', phase: 'confirmation', signature: 'sig-bad', cause: 'custom 6001',
        });
      },
    };
    const view = renderOperationHook(async () => twoStepFlow(), { executor: executorFor(), wallet });
    await act(async () => {
      await view.hook.run({ threshold: 1 }).catch(() => undefined);
    });

    expect(view.hook.error?.kind).toBe('chain-failed');
    expect(view.hook.error?.signature).toBe('sig-bad');
    expect(view.hook.error?.completedSignatures).toEqual(['sig-ok']);
    expect(view.hook.signatures).toEqual(['sig-ok']);
  });

  it('stays signing until confirmation with a wallet that reports no activity', async () => {
    const wallet: WalletAdapter = {
      publicKey: PAYER,
      signAndSend: async () => ({ signature: 'sig' }),
    };
    const view = renderOperationHook(async () => twoStepFlow(), { executor: executorFor(), wallet });
    await act(async () => {
      await view.hook.run({ threshold: 1 });
    });
    expect(view.seen).toEqual(['idle', 'preparing', 'signing', 'done']);
  });

  it('refuses a second run while one is in progress', async () => {
    const { wallet, approve } = activityWallet();
    const view = renderOperationHook(async () => twoStepFlow(), { executor: executorFor(), wallet });
    let first!: Promise<unknown>;
    act(() => {
      first = view.hook.run({ threshold: 1 });
    });
    await flush();

    await expect(view.hook.run({ threshold: 1 })).rejects.toMatchObject({ kind: 'busy' });
    expect(view.hook.status).toBe('signing');

    await act(async () => {
      approve();
      await flush();
      approve();
      await first;
    });
    expect(view.hook.status).toBe('done');
  });

  it('fails with not-ready without an executor and prepare-failed when prepare throws', async () => {
    const prepare = jest.fn(async () => twoStepFlow());
    const missing = renderOperationHook(prepare, { executor: null });
    await act(async () => {
      await missing.hook.run({ threshold: 1 }).catch(() => undefined);
    });
    expect(missing.hook.error?.kind).toBe('not-ready');
    expect(prepare).not.toHaveBeenCalled();

    const wallet: WalletAdapter = { publicKey: PAYER, signAndSend: async () => ({ signature: 's' }) };
    const broken = renderOperationHook(async () => {
      throw new Error('threshold above members');
    }, { executor: executorFor(), wallet });
    await act(async () => {
      await broken.hook.run({ threshold: 9 }).catch(() => undefined);
    });
    expect(broken.hook.error).toMatchObject({
      kind: 'prepare-failed',
      message: 'threshold above members',
    });
  });

  it('resolves a lazy executor once per run and resets to idle', async () => {
    const wallet: WalletAdapter = { publicKey: PAYER, signAndSend: async () => ({ signature: 's' }) };
    const open = jest.fn(async () => executorFor());
    const view = renderOperationHook(async () => twoStepFlow(), { executor: open, wallet });
    await act(async () => {
      await view.hook.run({ threshold: 1 });
    });
    expect(open).toHaveBeenCalledTimes(1);
    act(() => view.hook.reset());
    expect(view.hook.status).toBe('idle');
    expect(view.hook.result).toBeUndefined();
  });
});

describe('useOperationMutation without a wallet', () => {
  it('fails with not-ready and prepares and signs nothing, even when the executor has a wallet', async () => {
    const executorWallet: WalletAdapter = {
      publicKey: PAYER,
      signAndSend: jest.fn(async () => ({ signature: 'sig' })),
    };
    const executor = executorFor(executorWallet);
    const execute = jest.spyOn(executor, 'execute');
    const prepare = jest.fn(async () => twoStepFlow());
    const onError = jest.fn();
    const view = renderOperationHook(prepare, { executor, onError });

    let failure: unknown;
    await act(async () => {
      failure = await view.hook.run({ threshold: 1 }).catch((error: unknown) => error);
    });

    expect(failure).toBeInstanceOf(OperationMutationError);
    expect(view.hook.status).toBe('error');
    expect(view.hook.error?.kind).toBe('not-ready');
    expect(onError).toHaveBeenCalledWith(view.hook.error);
    expect(prepare).not.toHaveBeenCalled();
    expect(execute).not.toHaveBeenCalled();
    expect(executorWallet.signAndSend).not.toHaveBeenCalled();
  });

  it("signs with the executor's wallet only when allowExecutorWallet is set", async () => {
    const executorWallet: WalletAdapter = {
      publicKey: PAYER,
      signAndSend: jest.fn(async () => ({ signature: 'sig' })),
    };
    const view = renderOperationHook(async () => twoStepFlow(), {
      executor: executorFor(executorWallet),
      allowExecutorWallet: true,
    });
    await act(async () => {
      await view.hook.run({ threshold: 1 });
    });
    expect(view.hook.status).toBe('done');
    expect(executorWallet.signAndSend).toHaveBeenCalledTimes(2);

    const without = renderOperationHook(async () => twoStepFlow(), {
      executor: executorFor(),
      allowExecutorWallet: true,
    });
    await act(async () => {
      await without.hook.run({ threshold: 1 }).catch(() => undefined);
    });
    expect(without.hook.error?.kind).toBe('not-ready');
  });

  it('a wallet given to the run is enough', async () => {
    const wallet: WalletAdapter = { publicKey: PAYER, signAndSend: async () => ({ signature: 's' }) };
    const view = renderOperationHook(async () => twoStepFlow(), { executor: executorFor() });
    await act(async () => {
      await view.hook.run({ threshold: 1 }, { wallet });
    });
    expect(view.hook.status).toBe('done');
  });
});

describe('useOperationMutation inside an AreteSessionProvider', () => {
  it("runs on the provider's session when no executor is given", async () => {
    const wallet: WalletAdapter = { publicKey: PAYER, signAndSend: async () => ({ signature: 's' }) };
    const session = executorFor() as unknown as AnySession;
    const execute = jest.spyOn(session, 'execute');
    const view = renderOperationHook(
      async () => twoStepFlow(),
      { wallet },
      (children) => React.createElement(AreteSessionProvider, { session }, children)
    );
    await act(async () => {
      await view.hook.run({ threshold: 1 });
    });
    expect(view.hook.status).toBe('done');
    expect(execute).toHaveBeenCalledTimes(1);
  });

  it('is not-ready without a provider or an executor', async () => {
    const wallet: WalletAdapter = { publicKey: PAYER, signAndSend: async () => ({ signature: 's' }) };
    const view = renderOperationHook(async () => twoStepFlow(), { wallet });
    await act(async () => {
      await view.hook.run({ threshold: 1 }).catch(() => undefined);
    });
    expect(view.hook.error?.kind).toBe('not-ready');
  });
});

describe('isWalletRejection', () => {
  it.each([
    [{ code: 4001, message: 'x' }, true],
    [new Error('User rejected the request.'), true],
    [Object.assign(new Error('wrapped'), { cause: { name: 'WalletSignTransactionRejectedError' } }), true],
    [new Error('Blockhash not found'), false],
    [Object.assign(new Error('declined'), { name: 'WalletStandardSignerError', code: 'user_rejected' }), true],
    [Object.assign(new Error('x'), { name: 'WalletStandardSignerError', code: 'chain_unsupported' }), false],
    ['nope', false],
  ])('%p -> %p', (error, expected) => {
    expect(isWalletRejection(error)).toBe(expected);
  });
});
