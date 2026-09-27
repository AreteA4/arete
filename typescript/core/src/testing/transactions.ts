import {
  encodeBase58,
  type TransactionFeeResult,
  type TransactionSignatureStatus,
  type TransactionSimulationResult,
  type TransactionTransport,
} from '../index';

/** Every relay route, as recorded in {@link FakeTransactionTransport.calls}. */
export type FakeTransactionCall =
  | 'latest-blockhash'
  | 'fee'
  | 'simulate'
  | 'send'
  | 'status'
  | 'height';

export interface FakeTransactionTransportOptions {
  /** Blockhash returned by `getLatestBlockhash` (default: 32 zero bytes). */
  readonly blockhash?: string;
  readonly lastValidBlockHeight?: bigint;
  /** Height returned by `getBlockHeight` (default `10n`). */
  readonly blockHeight?: bigint;
  /** Fee returned by `getFeeForMessage` (default `5000n`). */
  readonly feeLamports?: bigint | null;
  /** Fields merged over the default successful simulation. */
  readonly simulation?: Partial<TransactionSimulationResult>;
  /**
   * Signature `sendTransaction` answers with. Defaults to the first signature
   * carried by the submitted wire transaction, as a real relay echoes it.
   */
  readonly signature?: string | ((transaction: string) => string);
  /** Fail the submission with this value, after recording the dispatch. */
  readonly sendError?: unknown;
  /** Record the dispatch, then never answer: a stalled relay. */
  readonly stallSend?: boolean;
  /**
   * Status answered by `getSignatureStatus`. Defaults to confirmed at slot
   * `42n`; `null` means the relay does not know the signature (yet).
   */
  readonly status?: Partial<Omit<TransactionSignatureStatus, 'signature'>> | null;
  /** Fail every status poll with this value. */
  readonly statusError?: unknown;
  /** Answer the submission, then never answer a status poll. */
  readonly stallStatus?: boolean;
}

/**
 * A recording {@link TransactionTransport}. `calls` is how a test asserts
 * "never sent": inspection records `fee` and `simulate`, and only a submission
 * records `send`.
 */
export interface FakeTransactionTransport extends TransactionTransport {
  readonly calls: FakeTransactionCall[];
  /** Wire transactions passed to `simulateTransaction`, in order. */
  readonly simulated: string[];
  /** Wire transactions passed to `sendTransaction`, in order. */
  readonly sent: string[];
  /** Messages passed to `getFeeForMessage`, in order. */
  readonly feeMessages: string[];
  /** Forget every recorded call. */
  reset(): void;
}

const ZERO_HASH = '11111111111111111111111111111111';

/** A promise that never settles, for stalled-backend probes. */
function pending<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

function base64Bytes(value: string): Uint8Array | null {
  try {
    const binary = atob(value);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
    return bytes;
  } catch {
    return null;
  }
}

/**
 * The first signature a base64 wire transaction carries (compact-u16 count,
 * then 64-byte signatures), or `undefined` when the bytes are not one.
 */
export function firstSignatureOfWireTransaction(transaction: string): string | undefined {
  const bytes = base64Bytes(transaction);
  if (!bytes || bytes.length < 65) return undefined;
  let count = 0;
  let offset = 0;
  let shift = 0;
  while (offset < 3) {
    const byte = bytes[offset]!;
    offset += 1;
    count |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) break;
    shift += 7;
  }
  if (count < 1 || bytes.length < offset + 64) return undefined;
  return encodeBase58(bytes.slice(offset, offset + 64));
}

/**
 * Create a recording, scriptable {@link TransactionTransport} for tests: pass
 * it as `ConnectOptions.transactions`, `useArete(stack, { transactions })`, or
 * an adapter's `transport`. Nothing reaches a network.
 */
export function createFakeTransactionTransport(
  options: FakeTransactionTransportOptions = {},
): FakeTransactionTransport {
  const calls: FakeTransactionCall[] = [];
  const simulated: string[] = [];
  const sent: string[] = [];
  const feeMessages: string[] = [];
  const fallbackSignature = encodeBase58(new Uint8Array(64).fill(7));

  return {
    calls,
    simulated,
    sent,
    feeMessages,
    reset() {
      calls.length = 0;
      simulated.length = 0;
      sent.length = 0;
      feeMessages.length = 0;
    },
    async getLatestBlockhash() {
      calls.push('latest-blockhash');
      return {
        blockhash: options.blockhash ?? ZERO_HASH,
        contextSlot: 1n,
        lastValidBlockHeight: options.lastValidBlockHeight ?? 999n,
      };
    },
    async getFeeForMessage(message: string): Promise<TransactionFeeResult> {
      calls.push('fee');
      feeMessages.push(message);
      return {
        feeLamports: options.feeLamports === undefined ? 5_000n : options.feeLamports,
        contextSlot: 400n,
      };
    },
    async simulateTransaction(transaction: string): Promise<TransactionSimulationResult> {
      calls.push('simulate');
      simulated.push(transaction);
      return {
        contextSlot: 401n,
        err: null,
        logs: ['Program log: simulated'],
        unitsConsumed: 1_200n,
        loadedAccountsDataSize: 4_096n,
        ...options.simulation,
      };
    },
    async sendTransaction(transaction: string) {
      calls.push('send');
      sent.push(transaction);
      if (options.stallSend) return pending();
      if (options.sendError !== undefined) throw options.sendError;
      const signature = typeof options.signature === 'function'
        ? options.signature(transaction)
        : options.signature
          ?? firstSignatureOfWireTransaction(transaction)
          ?? fallbackSignature;
      return { signature };
    },
    async getSignatureStatus(signature: string) {
      calls.push('status');
      if (options.stallStatus) return pending();
      if (options.statusError !== undefined) throw options.statusError;
      if (options.status === null) return null;
      return {
        signature,
        slot: 42n,
        confirmationStatus: 'confirmed',
        err: null,
        ...options.status,
      };
    },
    async getBlockHeight() {
      calls.push('height');
      return options.blockHeight ?? 10n;
    },
  };
}
