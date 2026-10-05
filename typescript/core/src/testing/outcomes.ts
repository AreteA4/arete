import {
  TransactionExecutionError,
  encodeBase58,
  type ChainFailedTransactionOutcome,
  type ConfirmedTransactionOutcome,
  type NotSubmittedTransactionOutcome,
  type SubmittedUnknownTransactionOutcome,
  type TransactionFailureOutcome,
  type TransactionOutcome,
} from '../index';

/** A deterministic, well-formed transaction signature for fixtures. */
export const FIXTURE_SIGNATURE = encodeBase58(new Uint8Array(64).fill(9));

/** The slot fixture outcomes land in. */
export const FIXTURE_SLOT = 4242;

/** Every status/phase pair the outcome model defines. */
export type TransactionOutcomeFixtureName =
  | 'confirmed'
  | 'not-submitted:build'
  | 'not-submitted:wallet'
  | 'not-submitted:send'
  | 'submitted-unknown:send'
  | 'submitted-unknown:confirmation'
  | 'chain-failed:confirmation'
  | 'chain-failed:chain';

function fixtures() {
  const confirmed: ConfirmedTransactionOutcome = {
    status: 'confirmed',
    phase: 'confirmation',
    signature: FIXTURE_SIGNATURE,
    slot: FIXTURE_SLOT,
  };
  const notSubmitted = (phase: NotSubmittedTransactionOutcome['phase'], message: string) => ({
    status: 'not-submitted',
    phase,
    cause: new Error(message),
  }) satisfies NotSubmittedTransactionOutcome;
  const submittedUnknown = (
    phase: SubmittedUnknownTransactionOutcome['phase'],
    message: string,
  ) => ({
    status: 'submitted-unknown',
    phase,
    signature: FIXTURE_SIGNATURE,
    cause: new Error(message),
  }) satisfies SubmittedUnknownTransactionOutcome;
  const chainFailed = (phase: ChainFailedTransactionOutcome['phase']) => ({
    status: 'chain-failed',
    phase,
    signature: FIXTURE_SIGNATURE,
    slot: FIXTURE_SLOT,
    programError: { code: 6000, name: 'FixtureError', message: 'Fixture program error' },
    cause: { InstructionError: [0, { Custom: 6000 }] },
  }) satisfies ChainFailedTransactionOutcome;

  return {
    confirmed,
    'not-submitted:build': notSubmitted('build', 'Fixture: the transaction could not be built'),
    'not-submitted:wallet': notSubmitted('wallet', 'Fixture: the wallet rejected the request'),
    'not-submitted:send': notSubmitted('send', 'Fixture: the relay refused the transaction'),
    'submitted-unknown:send': submittedUnknown(
      'send',
      'Fixture: the relay did not acknowledge the submission',
    ),
    'submitted-unknown:confirmation': submittedUnknown(
      'confirmation',
      'Fixture: confirmation timed out',
    ),
    'chain-failed:confirmation': chainFailed('confirmation'),
    'chain-failed:chain': chainFailed('chain'),
  } satisfies Record<TransactionOutcomeFixtureName, TransactionOutcome>;
}

/**
 * One fresh outcome per status/phase pair of the transaction outcome model:
 * `confirmed`, `not-submitted`, `submitted-unknown` and `chain-failed`.
 */
export function createTransactionOutcomeFixtures(): ReturnType<typeof fixtures> {
  return fixtures();
}

/** One outcome by name, with optional field overrides. */
export function transactionOutcomeFixture<TName extends TransactionOutcomeFixtureName>(
  name: TName,
  overrides: Partial<ReturnType<typeof fixtures>[TName]> = {},
): ReturnType<typeof fixtures>[TName] {
  return { ...fixtures()[name], ...overrides };
}

/**
 * The structured error an adapter throws for a failure outcome, as
 * `useMutation`, `execute` and `getTransactionFailureOutcome` classify it.
 */
export function transactionFailureError(
  outcome: TransactionFailureOutcome,
): TransactionExecutionError {
  return new TransactionExecutionError(outcome);
}
