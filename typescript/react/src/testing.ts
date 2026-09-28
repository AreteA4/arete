/**
 * `@usearete/react/testing`: supported helpers for testing React components
 * built on `@usearete/react`. It re-exports every `@usearete/sdk/testing`
 * helper and adds React-specific fixtures.
 */
import type {
  OperationTransactionReceipt,
  PreparedOperation,
  TransactionFailureOutcome,
  TransactionOutcome,
} from '@usearete/sdk';
import {
  FIXTURE_SIGNATURE,
  FIXTURE_SLOT,
  createTransactionOutcomeFixtures,
  transactionFailureError,
  type WebSocketHarness,
} from '@usearete/sdk/testing';
import type { MutationPhase, UseMutationResult } from './hooks';
import type { AreteConfig } from './types';

export * from '@usearete/sdk/testing';

/** Every value `useMutation().phase` can take, in lifecycle order. */
export const MUTATION_PHASES = [
  'idle',
  'preparing',
  'awaiting-wallet',
  'submitted',
  'confirmed',
  'reconciling',
  'reconciled',
  'confirmed-unreconciled',
  'not-submitted',
  'submitted-unknown',
  'chain-failed',
] as const satisfies readonly MutationPhase[];

const LANDED_PHASES = new Set<MutationPhase>([
  'submitted',
  'confirmed',
  'reconciling',
  'reconciled',
  'confirmed-unreconciled',
]);

function failureFor(phase: MutationPhase): TransactionFailureOutcome | null {
  const outcomes = createTransactionOutcomeFixtures();
  switch (phase) {
    case 'not-submitted':
      return outcomes['not-submitted:wallet'];
    case 'submitted-unknown':
      return outcomes['submitted-unknown:send'];
    case 'chain-failed':
      return outcomes['chain-failed:chain'];
    default:
      return null;
  }
}

function statusFor(phase: MutationPhase): UseMutationResult['status'] {
  if (phase === 'idle') return 'idle';
  if (phase === 'reconciled' || phase === 'confirmed-unreconciled') return 'success';
  if (failureFor(phase)) return 'error';
  return 'pending';
}

function displayErrorFor(failure: TransactionFailureOutcome | null): string | null {
  if (!failure) return null;
  if (failure.status === 'chain-failed' && failure.programError) {
    return `${failure.programError.name}: ${failure.programError.message}`;
  }
  return failure.cause instanceof Error ? failure.cause.message : 'Transaction failed';
}

/**
 * A complete, internally consistent `useMutation()` result for one phase, as
 * the hook reports it: status, receipts, signatures, outcome, failure, display
 * error and the derived `is*` flags all agree. Use it to render and test UI
 * for every phase without a wallet or a chain. `overrides` replace any field.
 *
 * `submit`/`mutateAsync` resolve with the fixture result on landed phases and
 * reject with the phase's failure otherwise; `mutate`, `reset` and
 * `retryReconciliation` do nothing.
 */
export function createMutationResultFixture<
  TParams = Record<string, unknown>,
  TResult = unknown,
  TOptions extends object = Record<string, unknown>,
  TPrepared extends PreparedOperation = PreparedOperation,
>(
  phase: MutationPhase,
  overrides: Partial<UseMutationResult<TParams, TResult, TOptions, TPrepared>> = {},
): UseMutationResult<TParams, TResult, TOptions, TPrepared> {
  const failure = failureFor(phase);
  const landed = LANDED_PHASES.has(phase);
  const receipt: OperationTransactionReceipt = {
    transactionIndex: 0,
    transactionName: 'fixture',
    signature: FIXTURE_SIGNATURE,
    slot: FIXTURE_SLOT,
  };
  const completedReceipts = landed ? [receipt] : [];
  const signatures = landed || (failure && 'signature' in failure && failure.signature)
    ? [FIXTURE_SIGNATURE]
    : [];
  const outcome: TransactionOutcome | null = failure
    ?? (landed && phase !== 'submitted'
      ? { status: 'confirmed', phase: 'confirmation', signature: FIXTURE_SIGNATURE, slot: FIXTURE_SLOT }
      : null);
  const result = (landed && phase !== 'submitted'
    ? { signature: FIXTURE_SIGNATURE, slot: FIXTURE_SLOT, transaction: receipt }
    : undefined) as TResult | undefined;
  const displayError = displayErrorFor(failure);
  const reconciliationError = phase === 'confirmed-unreconciled'
    ? new Error('Fixture: the stream did not reach the confirmed slot in time')
    : null;
  const status = statusFor(phase);

  const settle = async (): Promise<TResult> => {
    if (failure) throw transactionFailureError(failure);
    return result as TResult;
  };

  return {
    mutate: () => undefined,
    mutateAsync: settle,
    submit: settle,
    status,
    phase,
    latestEvent: phase === 'idle'
      ? null
      : {
          phase,
          prepared: null,
          ...(result !== undefined ? { result } : {}),
          ...(failure ? { failure } : {}),
        },
    error: displayError,
    displayError,
    failure,
    outcome,
    prepared: null,
    data: result,
    result,
    signatures,
    signature: signatures.length === 1 ? signatures[0]! : null,
    completedReceipts,
    callbackError: null,
    callbackErrors: [],
    reconciliationError,
    isLoading: status === 'pending',
    isConfirmed: outcome?.status === 'confirmed',
    isSubmittedUnknown: failure?.status === 'submitted-unknown',
    isPreparing: phase === 'preparing',
    isAwaitingWallet: phase === 'awaiting-wallet',
    isReconciling: phase === 'reconciling',
    canRetryReconciliation: phase === 'confirmed-unreconciled',
    retryReconciliation: async () => undefined,
    reset: () => undefined,
    ...overrides,
  };
}

/**
 * `<AreteProvider>` props that connect every client to a
 * {@link WebSocketHarness} instead of the network: no reconnects, no
 * batching delay, so frames a test sends are visible on the next render.
 *
 * ```tsx
 * const ws = createWebSocketHarness();
 * render(<AreteProvider {...createAreteTestConfig(ws, { wallet })}><App /></AreteProvider>);
 * const subscription = await ws.waitForSubscription('OreRound/latest');
 * ws.serve(subscription, [{ key: '1', data: round }]);
 * ```
 */
export function createAreteTestConfig(
  harness: Pick<WebSocketHarness, 'websocketFactory'>,
  overrides: AreteConfig = {},
): AreteConfig {
  return {
    autoReconnect: false,
    flushIntervalMs: 0,
    ...overrides,
    auth: { ...overrides.auth, websocketFactory: harness.websocketFactory },
  };
}
