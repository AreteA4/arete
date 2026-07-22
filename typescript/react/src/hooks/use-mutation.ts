import { useCallback, useRef, useState } from 'react';
import {
  InstructionError,
  OperationExecutionError,
  getTransactionFailureOutcome,
  unwrapOperationExecutionError,
  type OperationExecutionEvent,
  type OperationExecutionSuccessEvent,
  type OperationTransactionReceipt,
  type PreparedOperation,
  type TransactionFailureOutcome,
  type TransactionOutcome,
} from '@usearete/sdk';

export type MutationStatus = 'idle' | 'pending' | 'success' | 'error';

export type MutationPhase =
  | 'idle'
  | 'preparing'
  | 'awaiting-wallet'
  | 'submitted'
  | 'confirmed'
  | 'reconciling'
  | 'reconciled'
  | 'confirmed-unreconciled'
  | 'not-submitted'
  | 'submitted-unknown'
  | 'chain-failed';

export interface MutationLifecycleEvent<
  TResult = unknown,
  TPrepared extends PreparedOperation = PreparedOperation,
> {
  readonly phase: MutationPhase;
  readonly prepared: TPrepared | null;
  readonly result?: TResult;
  readonly failure?: TransactionFailureOutcome;
  readonly error?: Error;
  readonly transactionEvent?:
    | OperationExecutionEvent<TPrepared>
    | OperationExecutionSuccessEvent<TPrepared>;
}

export interface MutationReconciliationContext<
  TResult = unknown,
  TPrepared extends PreparedOperation = PreparedOperation,
> {
  readonly result: TResult;
  readonly prepared: TPrepared | null;
  readonly signatures: readonly string[];
  readonly completedReceipts: readonly OperationTransactionReceipt[];
  readonly signal: AbortSignal;
}

export interface MutationExecutionObserver<
  TPrepared extends PreparedOperation = PreparedOperation,
> {
  onPrepared(prepared: TPrepared): void;
  onAwaitingWallet(): void;
  onTransactionStart(event: OperationExecutionEvent<TPrepared>): void;
  onTransactionSuccess(event: OperationExecutionSuccessEvent<TPrepared>): void;
  onCallbackError(error: unknown): void;
}

export type MutationReconcileFn<
  TResult = unknown,
  TPrepared extends PreparedOperation = PreparedOperation,
> = (
  context: MutationReconciliationContext<TResult, TPrepared>
) => unknown | Promise<unknown>;

/**
 * Anything that can be refreshed after a mutation reconciles: a plain
 * callback, or any hook result carrying a `refresh` method (view hooks from
 * `arete.views.*` and read hooks from `arete.read.*`), so callers can write
 * `reconcile: { refresh: [board, round, quoteRead] }` without extracting
 * `.refresh` functions by hand.
 */
export type ReconciliationRefreshTarget =
  | (() => unknown | Promise<unknown>)
  | { readonly refresh: () => unknown | Promise<unknown> };

/**
 * Shorthand overrides for the default reconciliation injected into generated
 * program hooks. Only valid when the hook-level default is a marked default
 * reconciliation (see `createProcessedSlotReconciliation`).
 */
export interface MutationReconcileOverrides {
  readonly refresh?:
    | ReconciliationRefreshTarget
    | readonly ReconciliationRefreshTarget[];
  readonly timeoutMs?: number;
}

export type UseMutationOptions<
  TOptions extends object = Record<string, unknown>,
  TResult = unknown,
  TPrepared extends PreparedOperation = PreparedOperation,
> = TOptions & {
  onConfirmed?: (result: TResult) => void | Promise<void>;
  onSuccess?: (result: TResult) => void | Promise<void>;
  onError?: (error: Error) => void | Promise<void>;
  reconcile?:
    | false
    | MutationReconcileFn<TResult, TPrepared>
    | MutationReconcileOverrides;
};

interface MarkedDefaultReconciliation<
  TResult,
  TPrepared extends PreparedOperation,
> {
  (
    context: MutationReconciliationContext<TResult, TPrepared>
  ): unknown | Promise<unknown>;
  readonly areteDefaultReconciliation: true;
  readonly shouldReconcile: (
    context: MutationReconciliationContext<TResult, TPrepared>
  ) => boolean;
  readonly withOverrides: (
    overrides: MutationReconcileOverrides
  ) => MutationReconcileFn<TResult, TPrepared>;
}

function isMarkedDefaultReconciliation<
  TResult,
  TPrepared extends PreparedOperation,
>(value: unknown): value is MarkedDefaultReconciliation<TResult, TPrepared> {
  return (
    typeof value === 'function'
    && (value as { areteDefaultReconciliation?: unknown }).areteDefaultReconciliation === true
    && typeof (value as { shouldReconcile?: unknown }).shouldReconcile === 'function'
    && typeof (value as { withOverrides?: unknown }).withOverrides === 'function'
  );
}

/**
 * Result of {@link useInstructionMutation}.
 *
 * `phase` is the primary, discriminated status: it walks
 * 'preparing' → 'awaiting-wallet' → 'submitted' → 'confirmed' →
 * 'reconciling' → 'reconciled' on the happy path, and lands on
 * 'confirmed-unreconciled', 'not-submitted', 'submitted-unknown', or
 * 'chain-failed' otherwise. UIs should branch on `phase` for busy labels and
 * on {@link UseMutationResult.displayError} /
 * {@link UseMutationResult.reconciliationError} for messages.
 *
 * `failure` and `outcome` describe chain execution only; callback and
 * reconciliation failures never replace a confirmed outcome.
 */
export interface UseMutationResult<
  TParams = Record<string, unknown>,
  TResult = unknown,
  TOptions extends object = Record<string, unknown>,
  TPrepared extends PreparedOperation = PreparedOperation,
> {
  /** Event-handler form. Errors are recorded in mutation state and do not escape. */
  mutate: (
    args: TParams,
    options?: Partial<UseMutationOptions<TOptions, TResult, TPrepared>>
  ) => void;
  /** Rejecting form for imperative composition. Equivalent to submit(). */
  mutateAsync: (
    args: TParams,
    options?: Partial<UseMutationOptions<TOptions, TResult, TPrepared>>
  ) => Promise<TResult>;
  submit: (
    args: TParams,
    options?: Partial<UseMutationOptions<TOptions, TResult, TPrepared>>
  ) => Promise<TResult>;
  status: MutationStatus;
  phase: MutationPhase;
  latestEvent: MutationLifecycleEvent<TResult, TPrepared> | null;
  error: string | null;
  displayError: string | null;
  failure: TransactionFailureOutcome | null;
  outcome: TransactionOutcome | null;
  prepared: TPrepared | null;
  data: TResult | undefined;
  result: TResult | undefined;
  signatures: string[];
  signature: string | null;
  completedReceipts: OperationTransactionReceipt[];
  callbackError: Error | null;
  callbackErrors: Error[];
  reconciliationError: Error | null;
  isLoading: boolean;
  isConfirmed: boolean;
  isSubmittedUnknown: boolean;
  isPreparing: boolean;
  isAwaitingWallet: boolean;
  isReconciling: boolean;
  canRetryReconciliation: boolean;
  /** Retry only post-confirmation stream reconciliation; never resubmits. */
  retryReconciliation: () => Promise<void>;
  reset: () => void;
}

export type MutationExecutor<
  TParams = Record<string, unknown>,
  TResult = unknown,
  TOptions extends object = Record<string, unknown>,
  TPrepared extends PreparedOperation = PreparedOperation,
> = {
  (
    args: TParams,
    options?: TOptions,
    observer?: MutationExecutionObserver<TPrepared>
  ): Promise<TResult>;
};

interface MutationState<TResult, TPrepared extends PreparedOperation> {
  status: MutationStatus;
  phase: MutationPhase;
  latestEvent: MutationLifecycleEvent<TResult, TPrepared> | null;
  displayError: string | null;
  failure: TransactionFailureOutcome | null;
  outcome: TransactionOutcome | null;
  prepared: TPrepared | null;
  result: TResult | undefined;
  signatures: string[];
  completedReceipts: OperationTransactionReceipt[];
  callbackErrors: Error[];
  reconciliationError: Error | null;
}

interface ReconciliationRetryState<
  TResult,
  TPrepared extends PreparedOperation,
> {
  invocation: number;
  reconcile: MutationReconcileFn<TResult, TPrepared>;
  context: Omit<MutationReconciliationContext<TResult, TPrepared>, 'signal'>;
  onSuccess?: (result: TResult) => void | Promise<void>;
}

function initialMutationState<
  TResult,
  TPrepared extends PreparedOperation,
>(): MutationState<TResult, TPrepared> {
  return {
    status: 'idle',
    phase: 'idle',
    latestEvent: null,
    displayError: null,
    failure: null,
    outcome: null,
    prepared: null,
    result: undefined,
    signatures: [],
    completedReceipts: [],
    callbackErrors: [],
    reconciliationError: null,
  };
}

function normalizeThrown(value: unknown): Error {
  if (value instanceof Error) {
    return value;
  }
  let message: string;
  try {
    message = typeof value === 'string' ? value : JSON.stringify(value);
  } catch {
    message = String(value);
  }
  const error = new Error(message || String(value));
  (error as Error & { cause?: unknown }).cause = value;
  return error;
}

function displayErrorFor(error: Error, failure: TransactionFailureOutcome | null): string {
  const unwrapped = unwrapOperationExecutionError(error);
  if (unwrapped instanceof InstructionError && unwrapped.programError) {
    return `${unwrapped.programError.name}: ${unwrapped.programError.message}`;
  }
  if (failure?.status === 'chain-failed' && failure.programError) {
    return `${failure.programError.name}: ${failure.programError.message}`;
  }
  if (
    failure
    && failure.cause instanceof Error
    && failure.cause.message
  ) {
    return failure.cause.message;
  }
  return unwrapped instanceof Error ? unwrapped.message : error.message;
}

function isReceipt(value: unknown): value is OperationTransactionReceipt {
  if (!value || typeof value !== 'object') {
    return false;
  }
  const receipt = value as Partial<OperationTransactionReceipt>;
  return (
    typeof receipt.transactionIndex === 'number'
    && typeof receipt.transactionName === 'string'
    && typeof receipt.signature === 'string'
  );
}

function receiptsFrom(value: unknown): OperationTransactionReceipt[] {
  if (!value || typeof value !== 'object') {
    return [];
  }
  const candidate = value as {
    transaction?: unknown;
    transactions?: unknown;
    completedReceipts?: unknown;
  };
  if (Array.isArray(candidate.completedReceipts)) {
    return candidate.completedReceipts.filter(isReceipt);
  }
  if (Array.isArray(candidate.transactions)) {
    return candidate.transactions.filter(isReceipt);
  }
  return isReceipt(candidate.transaction) ? [candidate.transaction] : [];
}

function signaturesFrom(value: unknown): string[] {
  if (!value || typeof value !== 'object') {
    return [];
  }
  const candidate = value as { signatures?: unknown; signature?: unknown };
  if (Array.isArray(candidate.signatures)) {
    return candidate.signatures.filter((signature): signature is string =>
      typeof signature === 'string'
    );
  }
  return typeof candidate.signature === 'string' ? [candidate.signature] : [];
}

function callbackErrorsFrom(value: unknown): Error[] {
  if (!value || typeof value !== 'object') {
    return [];
  }
  const callbackErrors = (value as { callbackErrors?: unknown }).callbackErrors;
  return Array.isArray(callbackErrors) ? callbackErrors.map(normalizeThrown) : [];
}

function uniqueErrors(errors: readonly Error[]): Error[] {
  return [...new Set(errors)];
}

function uniqueStrings(values: readonly string[]): string[] {
  return [...new Set(values)];
}

function confirmedOutcome(
  signatures: readonly string[],
  receipts: readonly OperationTransactionReceipt[]
): TransactionOutcome | null {
  const signature = signatures[signatures.length - 1];
  if (!signature) {
    return null;
  }
  const receipt = [...receipts].reverse().find((entry) => entry.signature === signature);
  return {
    status: 'confirmed',
    phase: 'confirmation',
    signature,
    slot: receipt?.slot,
  };
}

function unreconciledError(value: unknown): Error | null {
  if (!value || typeof value !== 'object') {
    return null;
  }
  const result = value as { status?: unknown; error?: unknown };
  if (result.status !== 'confirmed-unreconciled') {
    return null;
  }
  return normalizeThrown(result.error ?? new Error('Confirmed transaction was not reconciled'));
}

export function useInstructionMutation<
  TParams = Record<string, unknown>,
  TResult = unknown,
  TOptions extends object = Record<string, unknown>,
  TPrepared extends PreparedOperation = PreparedOperation,
>(
  execute: MutationExecutor<TParams, TResult, TOptions, TPrepared>,
  defaults?: Partial<UseMutationOptions<TOptions, TResult, TPrepared>>
): UseMutationResult<TParams, TResult, TOptions, TPrepared> {
  const [state, setState] = useState<MutationState<TResult, TPrepared>>(
    initialMutationState<TResult, TPrepared>
  );
  const invocationRef = useRef(0);
  const reconciliationAbortRef = useRef<AbortController | null>(null);
  const reconciliationRetryRef = useRef<ReconciliationRetryState<TResult, TPrepared> | null>(null);
  const defaultsRef = useRef(defaults);

  defaultsRef.current = defaults;

  const submit: UseMutationResult<TParams, TResult, TOptions, TPrepared>['submit'] = useCallback(async (
    args,
    options
  ): Promise<TResult> => {
    const invocation = ++invocationRef.current;
    reconciliationAbortRef.current?.abort();
    reconciliationRetryRef.current = null;
    const reconciliationAbort = new AbortController();
    reconciliationAbortRef.current = reconciliationAbort;

    const updateCurrent = (
      update: (
        current: MutationState<TResult, TPrepared>
      ) => MutationState<TResult, TPrepared>
    ) => {
      if (invocation === invocationRef.current) {
        setState(update);
      }
    };
    const setPhase = (
      phase: MutationPhase,
      patch: Partial<MutationState<TResult, TPrepared>> = {},
      transactionEvent?: MutationLifecycleEvent<TResult, TPrepared>['transactionEvent']
    ) => {
      updateCurrent((current) => ({
        ...current,
        ...patch,
        phase,
        latestEvent: {
          phase,
          prepared: patch.prepared === undefined ? current.prepared : patch.prepared,
          ...(patch.result === undefined ? {} : { result: patch.result }),
          ...(patch.failure ? { failure: patch.failure } : {}),
          ...(transactionEvent ? { transactionEvent } : {}),
        },
      }));
    };

    setState({
      ...initialMutationState<TResult, TPrepared>(),
      status: 'pending',
      phase: 'preparing',
      latestEvent: { phase: 'preparing', prepared: null },
    });

    const mergedOptions = {
      ...(defaultsRef.current ?? {}),
      ...(options ?? {}),
    } as Partial<UseMutationOptions<TOptions, TResult, TPrepared>>;
    const onConfirmed = mergedOptions.onConfirmed;
    const onSuccess = mergedOptions.onSuccess;
    const onError = mergedOptions.onError;
    const reconcileOption = mergedOptions.reconcile;
    const executionOptions = { ...mergedOptions } as Record<string, unknown>;
    delete executionOptions.onConfirmed;
    delete executionOptions.onSuccess;
    delete executionOptions.onError;
    delete executionOptions.reconcile;

    let prepared: TPrepared | null = null;
    const observedReceipts: OperationTransactionReceipt[] = [];
    const observedCallbackErrors: Error[] = [];
    const observer: MutationExecutionObserver<TPrepared> = {
      onPrepared(nextPrepared) {
        prepared = nextPrepared;
        setPhase('awaiting-wallet', { prepared: nextPrepared });
      },
      onAwaitingWallet() {
        setPhase('awaiting-wallet');
      },
      onTransactionStart(event) {
        setPhase('awaiting-wallet', {}, event);
      },
      onTransactionSuccess(event) {
        observedReceipts.push(event.receipt);
        const receipts = [...observedReceipts];
        const signatures = uniqueStrings(receipts.map((receipt) => receipt.signature));
        setPhase('submitted', {
          completedReceipts: receipts,
          signatures,
        }, event);
      },
      onCallbackError(value) {
        const error = normalizeThrown(value);
        observedCallbackErrors.push(error);
        updateCurrent((current) => ({
          ...current,
          callbackErrors: uniqueErrors([...current.callbackErrors, error]),
        }));
      },
    };

    try {
      let reconcile: MutationReconcileFn<TResult, TPrepared> | undefined;
      if (reconcileOption === false || reconcileOption === undefined) {
        reconcile = undefined;
      } else if (typeof reconcileOption === 'function') {
        reconcile = reconcileOption;
      } else {
        const hookDefault = defaultsRef.current?.reconcile;
        if (isMarkedDefaultReconciliation<TResult, TPrepared>(hookDefault)) {
          reconcile = hookDefault.withOverrides(reconcileOption);
        } else {
          throw new Error(
            'reconcile option objects require a generated program hook with default reconciliation; pass a reconcile function instead'
          );
        }
      }

      const result = await execute(args, executionOptions as TOptions, observer);
      const completedReceipts = receiptsFrom(result).length > 0
        ? receiptsFrom(result)
        : [...observedReceipts];
      const signatures = uniqueStrings([
        ...signaturesFrom(result),
        ...completedReceipts.map((receipt) => receipt.signature),
      ]);
      const outcome = confirmedOutcome(signatures, completedReceipts);
      const callbackErrors = uniqueErrors([
        ...observedCallbackErrors,
        ...callbackErrorsFrom(result),
      ]);
      const reconciliationContext: MutationReconciliationContext<TResult, TPrepared> = {
        result,
        prepared,
        signatures,
        completedReceipts,
        signal: reconciliationAbort.signal,
      };
      const shouldReconcile = Boolean(
        reconcile
        && (
          !isMarkedDefaultReconciliation<TResult, TPrepared>(reconcile)
          || reconcile.shouldReconcile(reconciliationContext)
        )
      );
      if (invocation === invocationRef.current && reconcile && shouldReconcile) {
        reconciliationRetryRef.current = {
          invocation,
          reconcile,
          context: {
            result,
            prepared,
            signatures,
            completedReceipts,
          },
          onSuccess,
        };
      }

      setPhase('confirmed', {
        status: 'pending',
        result,
        outcome,
        prepared,
        signatures,
        completedReceipts,
        callbackErrors,
      });

      if (onConfirmed) {
        try {
          await onConfirmed(result);
        } catch (value) {
          const callbackError = normalizeThrown(value);
          observedCallbackErrors.push(callbackError);
          updateCurrent((current) => ({
            ...current,
            callbackErrors: uniqueErrors([...current.callbackErrors, callbackError]),
          }));
        }
      }

      if (reconciliationAbort.signal.aborted || invocation !== invocationRef.current) {
        return result;
      }

      let reconciled = !shouldReconcile;
      if (reconcile && shouldReconcile) {
        setPhase('reconciling', { status: 'pending' });
        try {
          const reconciliationResult = await reconcile(reconciliationContext);
          const error = unreconciledError(reconciliationResult);
          if (error) {
            setPhase('confirmed-unreconciled', {
              status: 'pending',
              reconciliationError: error,
            });
          } else {
            reconciled = true;
            if (reconciliationRetryRef.current?.invocation === invocation) {
              reconciliationRetryRef.current = null;
            }
            setPhase('reconciled', { status: 'pending' });
          }
        } catch (value) {
          const reconciliationError = normalizeThrown(value);
          setPhase('confirmed-unreconciled', {
            status: 'pending',
            reconciliationError,
          });
        }
      }

      if (
        reconciled
        && onSuccess
        && !reconciliationAbort.signal.aborted
        && invocation === invocationRef.current
      ) {
        try {
          await onSuccess(result);
        } catch (value) {
          const callbackError = normalizeThrown(value);
          observedCallbackErrors.push(callbackError);
          updateCurrent((current) => ({
            ...current,
            callbackErrors: uniqueErrors([...current.callbackErrors, callbackError]),
          }));
        }
      }

      updateCurrent((current) => ({ ...current, status: 'success' }));

      return result;
    } catch (value) {
      const error = normalizeThrown(value);
      const failure = getTransactionFailureOutcome(error) ?? {
        status: 'not-submitted',
        phase: prepared ? 'send' : 'build',
        cause: value,
      };
      const completedReceipts = value instanceof OperationExecutionError
        ? [...value.completedReceipts]
        : receiptsFrom(value);
      const signatures = uniqueStrings([
        ...completedReceipts.map((receipt) => receipt.signature),
        ...(failure && 'signature' in failure && failure.signature ? [failure.signature] : []),
      ]);
      const callbackErrors = uniqueErrors([
        ...observedCallbackErrors,
        ...callbackErrorsFrom(value),
      ]);
      const phase: MutationPhase = failure?.status ?? 'not-submitted';
      const displayError = displayErrorFor(error, failure);

      setPhase(phase, {
        status: 'pending',
        displayError,
        failure,
        outcome: failure,
        prepared: value instanceof OperationExecutionError
          ? value.operation as TPrepared
          : prepared,
        signatures,
        completedReceipts,
        callbackErrors,
      });

      if (onError) {
        try {
          await onError(error);
        } catch (callbackValue) {
          const callbackError = normalizeThrown(callbackValue);
          updateCurrent((current) => ({
            ...current,
            callbackErrors: uniqueErrors([...current.callbackErrors, callbackError]),
          }));
        }
      }

      updateCurrent((current) => ({ ...current, status: 'error' }));

      throw error;
    } finally {
      if (reconciliationAbortRef.current === reconciliationAbort) {
        reconciliationAbortRef.current = null;
      }
    }
  }, [execute]);

  const retryReconciliation = useCallback(async (): Promise<void> => {
    const pending = reconciliationRetryRef.current;
    if (!pending) {
      throw new Error('No failed reconciliation is available to retry');
    }

    const invocation = ++invocationRef.current;
    reconciliationAbortRef.current?.abort();
    const reconciliationAbort = new AbortController();
    reconciliationAbortRef.current = reconciliationAbort;
    setState((current) => ({
      ...current,
      status: 'pending',
      phase: 'reconciling',
      reconciliationError: null,
      latestEvent: {
        phase: 'reconciling',
        prepared: pending.context.prepared,
        result: pending.context.result,
      },
    }));

    try {
      const result = await pending.reconcile({
        ...pending.context,
        signal: reconciliationAbort.signal,
      });
      const error = unreconciledError(result);
      if (error) throw error;
      if (reconciliationAbort.signal.aborted || invocation !== invocationRef.current) return;

      reconciliationRetryRef.current = null;
      setState((current) => ({
        ...current,
        status: 'pending',
        phase: 'reconciled',
        reconciliationError: null,
        latestEvent: {
          phase: 'reconciled',
          prepared: pending.context.prepared,
          result: pending.context.result,
        },
      }));

      if (pending.onSuccess) {
        try {
          await pending.onSuccess(pending.context.result);
        } catch (value) {
          const callbackError = normalizeThrown(value);
          if (invocation === invocationRef.current) {
            setState((current) => ({
              ...current,
              callbackErrors: uniqueErrors([...current.callbackErrors, callbackError]),
            }));
          }
        }
      }
      if (invocation === invocationRef.current) {
        setState((current) => ({ ...current, status: 'success' }));
      }
    } catch (value) {
      const error = normalizeThrown(value);
      if (!reconciliationAbort.signal.aborted && invocation === invocationRef.current) {
        setState((current) => ({
          ...current,
          status: 'success',
          phase: 'confirmed-unreconciled',
          reconciliationError: error,
          latestEvent: {
            phase: 'confirmed-unreconciled',
            prepared: pending.context.prepared,
            result: pending.context.result,
            error,
          },
        }));
      }
      throw error;
    } finally {
      if (reconciliationAbortRef.current === reconciliationAbort) {
        reconciliationAbortRef.current = null;
      }
    }
  }, []);

  const reset = useCallback(() => {
    invocationRef.current += 1;
    reconciliationAbortRef.current?.abort();
    reconciliationAbortRef.current = null;
    reconciliationRetryRef.current = null;
    setState(initialMutationState<TResult, TPrepared>());
  }, []);

  const mutate = useCallback((
    args: TParams,
    options?: Partial<UseMutationOptions<TOptions, TResult, TPrepared>>,
  ): void => {
    void submit(args, options).catch(() => undefined);
  }, [submit]);

  const signature = state.signatures.length === 1 ? state.signatures[0]! : null;
  return {
    mutate,
    mutateAsync: submit,
    submit,
    status: state.status,
    phase: state.phase,
    latestEvent: state.latestEvent,
    error: state.displayError,
    displayError: state.displayError,
    failure: state.failure,
    outcome: state.outcome,
    prepared: state.prepared,
    data: state.result,
    result: state.result,
    signatures: state.signatures,
    signature,
    completedReceipts: state.completedReceipts,
    callbackError: state.callbackErrors[state.callbackErrors.length - 1] ?? null,
    callbackErrors: state.callbackErrors,
    reconciliationError: state.reconciliationError,
    isLoading: state.status === 'pending',
    isConfirmed: state.outcome?.status === 'confirmed',
    isSubmittedUnknown: state.failure?.status === 'submitted-unknown',
    isPreparing: state.phase === 'preparing',
    isAwaitingWallet: state.phase === 'awaiting-wallet',
    isReconciling: state.phase === 'reconciling',
    canRetryReconciliation: state.phase === 'confirmed-unreconciled'
      && reconciliationRetryRef.current !== null,
    retryReconciliation,
    reset,
  };
}
