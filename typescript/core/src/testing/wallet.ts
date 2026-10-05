import {
  TransactionExecutionError,
  encodeBase58,
  type BuiltInstruction,
  type SendOptions,
  type SendResult,
  type TransactionFailureOutcome,
  type TransactionInspectionOptions,
  type TransactionInspectionResult,
  type TransactionVersion,
  type WalletAdapter,
  type WalletExecutionContext,
} from '../index';

/** A deterministic, well-formed wallet address for fixtures. */
export const FIXTURE_WALLET_ADDRESS = encodeBase58(new Uint8Array(32).fill(1));

export interface WalletFixtureCall<TOptions> {
  readonly instructions: readonly BuiltInstruction[];
  readonly options: TOptions | undefined;
  readonly context: WalletExecutionContext | undefined;
}

/** What one `signAndSend` call does. */
export type WalletFixtureResponse =
  | SendResult
  | TransactionFailureOutcome
  | Error
  | 'stall';

export interface WalletFixtureOptions {
  /** Wallet address (default {@link FIXTURE_WALLET_ADDRESS}). */
  readonly publicKey?: string;
  /** Additional signer addresses the wallet can satisfy. */
  readonly signerAddresses?: readonly string[];
  /** Advertised versions; omitted means unknown, as for a real adapter. */
  readonly supportedTransactionVersions?: readonly TransactionVersion[];
  /**
   * Responses for successive sends; the last one repeats. A failure outcome
   * is thrown as a `TransactionExecutionError`, `'stall'` never settles.
   * Default: confirmed, with a distinct signature and slot per call.
   */
  readonly responses?: readonly WalletFixtureResponse[];
  /**
   * Result of `inspectTransaction`, or `false` for a wallet without the
   * inspection capability. Default: a successful simulation.
   */
  readonly inspection?: TransactionInspectionResult | false;
}

export interface WalletFixture extends WalletAdapter {
  /** Every `signAndSend` call, in order. */
  readonly sent: WalletFixtureCall<SendOptions>[];
  /** Every `inspectTransaction` call, in order. */
  readonly inspected: WalletFixtureCall<TransactionInspectionOptions>[];
  /** Forget every recorded call. */
  reset(): void;
}

function isFailureOutcome(value: WalletFixtureResponse): value is TransactionFailureOutcome {
  return typeof value === 'object'
    && value !== null
    && !(value instanceof Error)
    && 'status' in value
    && (value.status === 'not-submitted'
      || value.status === 'submitted-unknown'
      || value.status === 'chain-failed');
}

/**
 * A recording {@link WalletAdapter} for tests. It holds no keys and builds no
 * transactions: `signAndSend` answers from `responses`, so a test can drive a
 * mutation through every outcome without a chain.
 */
export function createWalletFixture(options: WalletFixtureOptions = {}): WalletFixture {
  const publicKey = options.publicKey ?? FIXTURE_WALLET_ADDRESS;
  const sent: WalletFixtureCall<SendOptions>[] = [];
  const inspected: WalletFixtureCall<TransactionInspectionOptions>[] = [];

  const respond = async (index: number): Promise<SendResult> => {
    const responses = options.responses ?? [];
    const response = responses.length === 0
      ? undefined
      : responses[Math.min(index, responses.length - 1)]!;
    if (response === undefined) {
      return {
        signature: encodeBase58(new Uint8Array(64).fill((index % 250) + 2)),
        slot: 100 + index,
      };
    }
    if (response === 'stall') return new Promise<SendResult>(() => {});
    if (response instanceof Error) throw response;
    if (isFailureOutcome(response)) throw new TransactionExecutionError(response);
    return response;
  };

  const wallet: WalletFixture = {
    publicKey,
    signerAddresses: [...new Set([publicKey, ...(options.signerAddresses ?? [])])],
    ...(options.supportedTransactionVersions
      ? { supportedTransactionVersions: options.supportedTransactionVersions }
      : {}),
    sent,
    inspected,
    reset() {
      sent.length = 0;
      inspected.length = 0;
    },
    async signAndSend(instructions, sendOptions, context) {
      const index = sent.length;
      sent.push({ instructions, options: sendOptions, context });
      return respond(index);
    },
  };

  if (options.inspection !== false) {
    const inspection = options.inspection ?? {
      feeLamports: 5_000,
      logs: ['Program log: inspected'],
      computeUnitsConsumed: 1_200,
      contextSlot: 401,
    };
    wallet.inspectTransaction = async (instructions, inspectOptions, context) => {
      inspected.push({ instructions, options: inspectOptions, context });
      return inspection;
    };
  }

  return wallet;
}
