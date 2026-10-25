import { useCallback, useEffect, useRef, useState } from 'react';
import {
  getTransactionFailureOutcome,
  OperationExecutionError,
  type OperationExecutionOptions,
  type OperationReceiptFor,
  type PreparedOperation,
  type ProgramError,
  type SendOptions,
  type TransactionFailureOutcome,
  type WalletAdapter,
} from '@usearete/sdk';

import { useOptionalAreteSession } from '../session-provider';

/**
 * Where an operation run is:
 *
 * - `idle`: nothing has run (or `reset()` was called).
 * - `preparing`: `prepare` is building the operation.
 * - `signing`: a transaction is waiting for its signatures, which for a
 *   browser wallet means its approval prompt is open ("check your wallet").
 * - `sending`: signed; the transaction is being submitted.
 * - `confirming`: submitted; waiting for the commitment.
 * - `done`: every transaction of the operation confirmed; `result` is set.
 * - `error`: the run failed; `error` says how.
 *
 * `sending` and `confirming` are reported by wallets that publish their
 * activity (the `@usearete/adapter-kit` adapters do, through `onActivity`).
 * With any other wallet a transaction stays `signing` until it confirms.
 * A flow walks `signing` → `sending` → `confirming` once per transaction.
 */
export type OperationMutationStatus =
  | 'idle'
  | 'preparing'
  | 'signing'
  | 'sending'
  | 'confirming'
  | 'done'
  | 'error';

/** Something that executes a prepared operation: a connected client or a session. */
export interface OperationExecutor {
  execute<TPrepared extends PreparedOperation, TSigner = unknown>(
    prepared: TPrepared,
    options?: OperationExecutionOptions<TSigner, TPrepared>
  ): Promise<OperationReceiptFor<TPrepared>>;
}

/**
 * The executor, or a function that resolves it when a run starts (for
 * example a session opened on first use). `null` while it is not available
 * yet: a run then fails with `not-ready`. Omitted (or `undefined`), the hook
 * uses the nearest `AreteSessionProvider`'s session, waiting for it to open.
 */
export type OperationExecutorSource =
  | OperationExecutor
  | (() => OperationExecutor | Promise<OperationExecutor>)
  | null
  | undefined;

/** A prepared operation with signers made while preparing it (a create key). */
export interface PreparedOperationWithSigners<TPrepared extends PreparedOperation> {
  readonly operation: TPrepared;
  readonly signers?: readonly unknown[];
}

/** Builds the operation for one run, e.g. `(input) => program.flows.x.y.prepare(input)`. */
export type OperationPrepareFn<TInput, TPrepared extends PreparedOperation> = (
  input: TInput
) => TPrepared | PreparedOperationWithSigners<TPrepared>
  | Promise<TPrepared | PreparedOperationWithSigners<TPrepared>>;

/** Per-run settings; each also has a hook-level default. */
export interface OperationRunOptions {
  /**
   * The wallet that signs. Without one the run fails with `not-ready` and
   * nothing is prepared or signed, unless the hook sets
   * `allowExecutorWallet`.
   */
  readonly wallet?: WalletAdapter;
  /** Extra local signers for this run, added to the hook's and `prepare`'s. */
  readonly signers?: readonly unknown[];
  /** Send options forwarded to the wallet (transaction version, resources). */
  readonly send?: SendOptions;
}

export interface UseOperationMutationOptions<TPrepared extends PreparedOperation>
  extends OperationRunOptions {
  /** What runs the operation; defaults to the nearest `AreteSessionProvider`'s session. */
  readonly executor?: OperationExecutorSource;
  /**
   * Sign with the executor's own wallet (`session.wallet`, the client's
   * wallet) when neither the run nor the hook names one. Off by default, so
   * a run never signs with a wallet the component did not pass.
   */
  readonly allowExecutorWallet?: boolean;
  readonly onSuccess?: (
    result: OperationReceiptFor<TPrepared>,
    prepared: TPrepared
  ) => void | Promise<void>;
  readonly onError?: (error: OperationMutationError<TPrepared>) => void | Promise<void>;
  /**
   * Called on every status change of a run, as it happens (React may batch
   * the re-renders of quick changes, this callback never skips one).
   * `transactionIndex` is the transaction of the operation it is about.
   */
  readonly onStatusChange?: (
    status: OperationMutationStatus,
    context: { readonly transactionIndex: number | null }
  ) => void;
}

/**
 * How a run failed:
 *
 * - `busy`: a run of this hook is already in progress; nothing was started.
 * - `not-ready`: no executor, or no wallet, to run with; nothing was
 *   prepared or signed.
 * - `prepare-failed`: `prepare` threw; nothing was signed.
 * - `wallet-rejected`: the user declined (or closed) the wallet prompt.
 * - `wallet-feature-unsupported`: the wallet cannot sign without sending
 *   (it lacks `solana:signTransaction`), so Arete cannot relay for it.
 * - `wallet-chain-unsupported`: the wallet (or the account) does not sign
 *   for the chain it was asked to.
 * - `wallet-version-unsupported`: the wallet signs, but not this
 *   transaction version (a v0-only wallet and a V1 transaction).
 * - `wallet-modified`: the wallet changed the transaction it was asked to
 *   sign (for example to add a priority fee), so it was not sent.
 * - `not-submitted`: failed before anything reached the chain.
 * - `submitted-unknown`: sent, but its confirmation did not arrive; check
 *   the chain before trying again.
 * - `chain-failed`: the transaction failed on chain; see `programError`.
 */
export type OperationMutationErrorKind =
  | 'busy'
  | 'not-ready'
  | 'prepare-failed'
  | 'wallet-rejected'
  | 'wallet-feature-unsupported'
  | 'wallet-chain-unsupported'
  | 'wallet-version-unsupported'
  | 'wallet-modified'
  | 'not-submitted'
  | 'submitted-unknown'
  | 'chain-failed';

/** The typed failure of a {@link useOperationMutation} run. */
export class OperationMutationError<
  TPrepared extends PreparedOperation = PreparedOperation,
> extends Error {
  readonly kind: OperationMutationErrorKind;
  /** The transaction outcome, when the failure happened at or after signing. */
  readonly failure: TransactionFailureOutcome | null;
  /** The operation that was running, once prepared. */
  readonly prepared: TPrepared | null;
  /** Signatures of the transactions that confirmed before the failure. */
  readonly completedSignatures: readonly string[];
  /** The failing transaction's signature, when it was signed. */
  readonly signature?: string;
  readonly programError?: ProgramError;
  readonly cause: unknown;

  constructor(input: {
    kind: OperationMutationErrorKind;
    message: string;
    cause: unknown;
    failure?: TransactionFailureOutcome | null;
    prepared?: TPrepared | null;
    completedSignatures?: readonly string[];
  }) {
    super(input.message);
    this.name = 'OperationMutationError';
    this.kind = input.kind;
    this.cause = input.cause;
    this.failure = input.failure ?? null;
    this.prepared = input.prepared ?? null;
    this.completedSignatures = [...(input.completedSignatures ?? [])];
    this.signature = this.failure && 'signature' in this.failure
      ? this.failure.signature
      : undefined;
    this.programError = this.failure?.status === 'chain-failed'
      ? this.failure.programError
      : undefined;
  }
}

/** The `cause` chain of `error`, outermost first. */
function causeChain(error: unknown): unknown[] {
  const chain: unknown[] = [];
  let current = error;
  while (current !== undefined && current !== null && chain.length < 10 && !chain.includes(current)) {
    chain.push(current);
    current = typeof current === 'object' ? (current as { cause?: unknown }).cause : undefined;
  }
  return chain;
}

/** Error codes wallets report a declined prompt with (EIP-1193 4001 and kin). */
const USER_REJECTION_CODES: readonly unknown[] = [4001, 'ACTION_REJECTED', 'USER_REJECTED'];

/**
 * Whether `error` (or anything in its `cause` chain) is a wallet prompt the
 * user declined or closed. Wallets report this in a few ways: EIP-1193 code
 * 4001 (Phantom, Solflare), a `...RejectedError`, or the message.
 */
export function isWalletRejection(error: unknown): boolean {
  return causeChain(error).some((item) => {
    if (typeof item !== 'object' || item === null) return false;
    const { code, name, message } = item as { code?: unknown; name?: unknown; message?: unknown };
    if (USER_REJECTION_CODES.includes(code)) return true;
    if (name === 'WalletStandardSignerError') return code === 'user_rejected';
    if (typeof name === 'string' && /rejected/i.test(name)) return true;
    return typeof message === 'string'
      && /user (rejected|declined|denied|cancel)|rejected the request|request (was )?(rejected|declined)/i
        .test(message);
  });
}

/** The `code` of a `WalletStandardSignerError` in the cause chain, if any. */
function walletSignerErrorCode(error: unknown): string | null {
  for (const item of causeChain(error)) {
    if (
      item instanceof Error
      && item.name === 'WalletStandardSignerError'
      && typeof (item as { code?: unknown }).code === 'string'
    ) {
      return (item as Error & { code: string }).code;
    }
  }
  return null;
}

function messageOf(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === 'string' && error) return error;
  return 'The operation failed';
}

/**
 * Classify an execution failure. Exported so code outside the hook (an
 * imperative `session.execute`) can show the same messages.
 */
export function toOperationMutationError<TPrepared extends PreparedOperation = PreparedOperation>(
  error: unknown,
  prepared: TPrepared | null = null,
  completedSignatures: readonly string[] = []
): OperationMutationError<TPrepared> {
  if (error instanceof OperationMutationError) return error as OperationMutationError<TPrepared>;
  const failure = getTransactionFailureOutcome(error);
  const completed = error instanceof OperationExecutionError
    ? error.completedReceipts.map((receipt) => receipt.signature)
    : completedSignatures;
  const make = (kind: OperationMutationErrorKind) => new OperationMutationError<TPrepared>({
    kind,
    message: messageOf(error),
    cause: error,
    failure,
    prepared,
    completedSignatures: completed,
  });

  if (failure?.status === 'chain-failed') return make('chain-failed');
  if (failure?.status === 'submitted-unknown') return make('submitted-unknown');
  switch (walletSignerErrorCode(error)) {
    case 'wallet_returned_invalid_transaction':
      return make('wallet-modified');
    case 'sign_transaction_unsupported':
      return make('wallet-feature-unsupported');
    case 'chain_unsupported':
      return make('wallet-chain-unsupported');
    case 'transaction_version_unsupported':
      return make('wallet-version-unsupported');
    case 'user_rejected':
      return make('wallet-rejected');
  }
  if (isWalletRejection(error)) return make('wallet-rejected');
  return make('not-submitted');
}

/** A wallet that reports its signing activity (`@usearete/adapter-kit` adapters do). */
interface WalletActivitySource {
  onActivity(
    listener: (activity: { readonly stage: string; readonly signature?: string }) => void
  ): () => void;
}

function hasActivity(wallet: unknown): wallet is WalletActivitySource {
  return typeof (wallet as Partial<WalletActivitySource> | null | undefined)?.onActivity === 'function';
}

function isPreparedWithSigners<TPrepared extends PreparedOperation>(
  value: TPrepared | PreparedOperationWithSigners<TPrepared>
): value is PreparedOperationWithSigners<TPrepared> {
  return typeof value === 'object'
    && value !== null
    && 'operation' in value
    && !('kind' in value && 'plan' in value);
}

async function resolveExecutor(source: OperationExecutorSource): Promise<OperationExecutor | null> {
  if (!source) return null;
  if (typeof source === 'function') return (await source()) ?? null;
  return source;
}

interface OperationMutationState<TPrepared extends PreparedOperation> {
  readonly status: OperationMutationStatus;
  readonly prepared: TPrepared | null;
  readonly result: OperationReceiptFor<TPrepared> | undefined;
  readonly error: OperationMutationError<TPrepared> | null;
  readonly signatures: readonly string[];
  readonly transactionIndex: number | null;
}

export interface UseOperationMutationResult<TInput, TPrepared extends PreparedOperation> {
  /** Run the operation; rejects with an {@link OperationMutationError}. */
  run(input: TInput, options?: OperationRunOptions): Promise<OperationReceiptFor<TPrepared>>;
  /** Event-handler form of `run`: the failure is kept in `error`, never thrown. */
  mutate(input: TInput, options?: OperationRunOptions): void;
  readonly status: OperationMutationStatus;
  /** `preparing`, `signing`, `sending` or `confirming`. */
  readonly isBusy: boolean;
  /** The wallet is waiting for the user: show "check your wallet". */
  readonly isAwaitingWallet: boolean;
  /** The operation of the latest run, once prepared. */
  readonly prepared: TPrepared | null;
  /** The prepared operation's artifacts (derived addresses and the like). */
  readonly artifacts: TPrepared['artifacts'] | undefined;
  /** The receipt of the latest successful run. */
  readonly result: OperationReceiptFor<TPrepared> | undefined;
  readonly error: OperationMutationError<TPrepared> | null;
  /** Signatures of the transactions confirmed so far in the latest run. */
  readonly signatures: readonly string[];
  /** Which of `transactionCount` transactions is in progress (0-based). */
  readonly transactionIndex: number | null;
  readonly transactionCount: number | null;
  reset(): void;
}

const IDLE_STATE: OperationMutationState<never> = {
  status: 'idle',
  prepared: null,
  result: undefined,
  error: null,
  signatures: [],
  transactionIndex: null,
};

/**
 * Run a prepared instruction, transaction or flow with a wallet and report
 * where it is: idle → preparing → signing → sending → confirming → done (or
 * error). Works with anything that has `execute(prepared, options)`, a
 * connected client or a session, so it needs no stack:
 *
 * ```tsx
 * const createMultisig = useOperationMutation(
 *   async (input: CreateInput) => {
 *     const createKey = await generateKeyPairSigner();
 *     return {
 *       operation: await squads.flows.arete.createMultisig.prepare({ ...input, createKey: createKey.address }),
 *       signers: [createKey],
 *     };
 *   },
 *   { executor: session, wallet },
 * );
 * // Inside an AreteSessionProvider, `executor` can be left out.
 * // createMultisig.mutate(input); createMultisig.isAwaitingWallet → "Approve in your wallet"
 * ```
 *
 * Signers returned by `prepare`, given to the hook and given to the run are
 * all passed to `execute`. A run without a wallet fails with `not-ready`
 * before `prepare` is called; set `allowExecutorWallet` to sign with the
 * executor's own wallet instead. A second run while one is in progress rejects
 * with `busy` and leaves the first alone, so a double click never opens two
 * wallet prompts.
 */
export function useOperationMutation<TInput, TPrepared extends PreparedOperation>(
  prepare: OperationPrepareFn<TInput, TPrepared>,
  options: UseOperationMutationOptions<TPrepared>
): UseOperationMutationResult<TInput, TPrepared> {
  const [state, setState] = useState<OperationMutationState<TPrepared>>(IDLE_STATE);
  const sessionContext = useOptionalAreteSession();
  const prepareRef = useRef(prepare);
  const optionsRef = useRef(options);
  const sessionReadyRef = useRef(sessionContext?.ready);
  prepareRef.current = prepare;
  optionsRef.current = options;
  sessionReadyRef.current = sessionContext?.ready;
  const generationRef = useRef(0);
  const runningRef = useRef(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      generationRef.current += 1;
    };
  }, []);

  const run = useCallback(async (
    input: TInput,
    runOptions: OperationRunOptions = {}
  ): Promise<OperationReceiptFor<TPrepared>> => {
    if (runningRef.current) {
      throw new OperationMutationError<TPrepared>({
        kind: 'busy',
        message: 'This operation is already running',
        cause: undefined,
      });
    }
    runningRef.current = true;
    const generation = ++generationRef.current;
    const hookOptions = optionsRef.current;
    let lastStatus: OperationMutationStatus = 'idle';
    let transactionIndex: number | null = null;
    const update = (patch: Partial<OperationMutationState<TPrepared>>) => {
      if (!mountedRef.current || generation !== generationRef.current) return;
      if (patch.transactionIndex !== undefined) transactionIndex = patch.transactionIndex;
      setState((current) => ({ ...current, ...patch }));
      if (patch.status && patch.status !== lastStatus) {
        lastStatus = patch.status;
        try {
          hookOptions.onStatusChange?.(patch.status, { transactionIndex });
        } catch {
          // Observational only.
        }
      }
    };
    let prepared: TPrepared | null = null;
    const signatures: string[] = [];
    let stopActivity: (() => void) | undefined;

    const fail = async (error: OperationMutationError<TPrepared>) => {
      update({ status: 'error', error, prepared, signatures: [...signatures] });
      try {
        await hookOptions.onError?.(error);
      } catch {
        // The callback observes the failure; it does not replace it.
      }
      return error;
    };

    setState(IDLE_STATE);
    update({ status: 'preparing' });
    try {
      let executor: OperationExecutor | null;
      const source = hookOptions.executor !== undefined
        ? hookOptions.executor
        : sessionReadyRef.current ?? null;
      try {
        executor = await resolveExecutor(source);
      } catch (cause) {
        throw await fail(new OperationMutationError<TPrepared>({
          kind: 'not-ready', message: messageOf(cause), cause,
        }));
      }
      if (!executor) {
        throw await fail(new OperationMutationError<TPrepared>({
          kind: 'not-ready',
          message: 'Nothing to execute the operation with yet',
          cause: undefined,
        }));
      }

      // No wallet, no run: nothing is prepared, nothing is signed.
      const wallet = runOptions.wallet
        ?? hookOptions.wallet
        ?? (hookOptions.allowExecutorWallet
          ? (executor as { wallet?: WalletAdapter }).wallet
          : undefined);
      if (!wallet) {
        throw await fail(new OperationMutationError<TPrepared>({
          kind: 'not-ready',
          message: 'Connect a wallet to run this operation',
          cause: undefined,
        }));
      }

      let extraSigners: readonly unknown[] = [];
      try {
        const built = await prepareRef.current(input);
        if (isPreparedWithSigners(built)) {
          prepared = built.operation;
          extraSigners = built.signers ?? [];
        } else {
          prepared = built;
        }
      } catch (cause) {
        throw await fail(new OperationMutationError<TPrepared>({
          kind: 'prepare-failed', message: messageOf(cause), cause,
        }));
      }
      update({ prepared });

      // A wallet that reports its activity says when its prompt opens; until
      // then the transaction is still being built (blockhash, budgets).
      const reportsActivity = hasActivity(wallet);
      if (reportsActivity) {
        stopActivity = wallet.onActivity((activity) => {
          if (activity.stage === 'signing' || activity.stage === 'sending'
            || activity.stage === 'confirming') {
            update({ status: activity.stage });
          }
        });
      }
      const signers = [
        ...(hookOptions.signers ?? []),
        ...extraSigners,
        ...(runOptions.signers ?? []),
      ];
      const send = runOptions.send ?? hookOptions.send;

      let result: OperationReceiptFor<TPrepared>;
      try {
        result = await executor.execute(prepared, {
          wallet,
          ...(signers.length > 0 ? { signers } : {}),
          ...(send ? { send } : {}),
          onTransactionStart: (event) => {
            update({
              status: reportsActivity ? 'preparing' : 'signing',
              transactionIndex: event.transactionIndex,
            });
          },
          onTransactionSuccess: (event) => {
            signatures.push(event.receipt.signature);
            update({ signatures: [...signatures] });
          },
        });
      } catch (cause) {
        throw await fail(toOperationMutationError(cause, prepared, signatures));
      }

      update({
        status: 'done',
        result,
        signatures: [...result.signatures],
        transactionIndex: null,
      });
      try {
        await hookOptions.onSuccess?.(result, prepared);
      } catch {
        // The operation confirmed; a failing callback does not undo that.
      }
      return result;
    } finally {
      stopActivity?.();
      runningRef.current = false;
    }
  }, []);

  const mutate = useCallback((input: TInput, runOptions?: OperationRunOptions) => {
    void run(input, runOptions).catch(() => undefined);
  }, [run]);

  const reset = useCallback(() => {
    if (runningRef.current) return;
    generationRef.current += 1;
    setState(IDLE_STATE);
  }, []);

  const busy = state.status === 'preparing' || state.status === 'signing'
    || state.status === 'sending' || state.status === 'confirming';
  return {
    run,
    mutate,
    reset,
    status: state.status,
    isBusy: busy,
    isAwaitingWallet: state.status === 'signing',
    prepared: state.prepared,
    artifacts: state.prepared?.artifacts as TPrepared['artifacts'] | undefined,
    result: state.result,
    error: state.error,
    signatures: state.signatures,
    transactionIndex: state.transactionIndex,
    transactionCount: state.prepared ? state.prepared.plan.transactions.length : null,
  };
}
