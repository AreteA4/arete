import { managedU64 } from './managed-solana';
export type TransactionCommitment = 'processed' | 'confirmed' | 'finalized';
export type TransactionAuthScope = 'transaction:inspect' | 'transaction:send';

export interface TransactionRequestContext {
  commitment?: TransactionCommitment;
  minContextSlot?: bigint;
}

export interface LatestBlockhashResult {
  blockhash: string;
  contextSlot: bigint;
  lastValidBlockHeight: bigint;
}

export interface TransactionFeeResult {
  feeLamports: bigint | null;
  contextSlot: bigint;
}

export interface TransactionSimulationOptions extends TransactionRequestContext {
  accounts?: readonly string[];
  innerInstructions?: boolean;
  replaceRecentBlockhash?: boolean;
}

export interface TransactionSimulationResult {
  contextSlot: bigint;
  err: unknown | null;
  logs: readonly string[] | null;
  unitsConsumed?: bigint;
  /** Loaded account data size, when the relay reports it. */
  loadedAccountsDataSize?: bigint;
  accounts?: readonly unknown[] | null;
}

export interface TransactionSendOptions {
  skipPreflight?: boolean;
  preflightCommitment?: TransactionCommitment;
  minContextSlot?: bigint;
}

export interface TransactionSendResult {
  signature: string;
}

export interface TransactionSignatureStatus {
  signature: string;
  slot: bigint | null;
  confirmationStatus: TransactionCommitment | null;
  err: unknown | null;
}

export interface TransactionTransportErrorBody {
  code: string;
  message: string;
  retryable: boolean;
  request_id?: string;
  submission_state?: 'not_submitted' | 'unknown';
  signature?: string;
  details?: unknown;
}

export class TransactionTransportError extends Error {
  readonly code: string;
  readonly retryable: boolean;
  readonly requestId?: string;
  readonly submissionState?: 'not_submitted' | 'unknown';
  readonly signature?: string;
  readonly details?: unknown;
  readonly status: number;

  constructor(status: number, body: TransactionTransportErrorBody) {
    super(body.message);
    this.name = 'TransactionTransportError';
    this.status = status;
    this.code = body.code;
    this.retryable = body.retryable;
    this.requestId = body.request_id;
    this.submissionState = body.submission_state;
    this.signature = body.signature;
    this.details = body.details;
  }
}

export interface TransactionInspectOptions {
  commitment?: 'confirmed' | 'finalized';
  maxSupportedTransactionVersion?: number;
}
export interface TransactionAccountBalance { pubkey: string; preBalance: bigint; postBalance: bigint }
export interface TransactionExecutionMetadata {
  err?: unknown; fee?: string; preBalances?: string[]; postBalances?: string[];
  preTokenBalances?: unknown[]; postTokenBalances?: unknown[]; innerInstructions?: unknown[] | null;
  logMessages?: string[] | null; returnData?: unknown; computeUnitsConsumed?: string; costUnits?: string;
  [field: string]: unknown;
}
export interface ConfirmedTransaction {
  signature: string; slot: bigint; blockTime: bigint | null; err: unknown | null;
  accounts: TransactionAccountBalance[];
  transaction?: Record<string, unknown>; meta?: TransactionExecutionMetadata | null;
  version?: 'legacy' | number; metadataAvailable?: boolean;
}

export interface TransactionTransport {
  get(signature: string, options?: TransactionInspectOptions): Promise<ConfirmedTransaction | null>;
  getLatestBlockhash(options?: TransactionRequestContext): Promise<LatestBlockhashResult>;
  getFeeForMessage(message: string, options?: TransactionRequestContext): Promise<TransactionFeeResult>;
  simulateTransaction(
    transaction: string,
    options?: TransactionSimulationOptions
  ): Promise<TransactionSimulationResult>;
  sendTransaction(transaction: string, options?: TransactionSendOptions): Promise<TransactionSendResult>;
  getSignatureStatus(
    signature: string,
    options?: TransactionRequestContext & { searchTransactionHistory?: boolean }
  ): Promise<TransactionSignatureStatus | null>;
  getBlockHeight(options?: TransactionRequestContext): Promise<bigint>;
}

type AuthenticatedTransactionFetch = (
  input: string,
  init: RequestInit,
  scope: TransactionAuthScope,
  allowAuthReplay: boolean
) => Promise<Response>;

function decimal(value: bigint | undefined): string | undefined {
  return value === undefined ? undefined : value.toString(10);
}

function bigintField(value: unknown, field: string): bigint {
  if (typeof value !== 'string' || !/^\d+$/.test(value)) {
    throw new Error(`Invalid decimal u64 field '${field}' in transaction response`);
  }
  return BigInt(value);
}

function optionalBigint(value: unknown, field: string): bigint | undefined {
  return value === undefined || value === null ? undefined : bigintField(value, field);
}

function requestBody(value: Record<string, unknown>): string {
  return JSON.stringify(Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined)
  ));
}

async function parseError(response: Response): Promise<TransactionTransportError> {
  let body: Partial<TransactionTransportErrorBody> = {};
  try {
    body = await response.json() as Partial<TransactionTransportErrorBody>;
  } catch {
    // Public errors are deliberately synthesized without reflecting raw bodies.
  }
  return new TransactionTransportError(response.status, {
    code: typeof body.code === 'string' ? body.code : 'transaction_transport_error',
    message: typeof body.message === 'string' ? body.message : `Transaction request failed (${response.status})`,
    retryable: body.retryable === true,
    request_id: body.request_id ?? (body as { requestId?: string }).requestId,
    submission_state: body.submission_state
      ?? (body as { submissionState?: 'not_submitted' | 'unknown' }).submissionState,
    signature: body.signature,
    details: body.details,
  });
}

export function createTransactionTransport(
  baseUrl: string,
  authenticatedFetch: AuthenticatedTransactionFetch
): TransactionTransport {
  const root = `${baseUrl.replace(/\/$/, '')}/transactions/v1`;
  const post = async <T>(
    route: string,
    body: Record<string, unknown>,
    scope: TransactionAuthScope
  ): Promise<T> => {
    const response = await authenticatedFetch(`${root}/${route}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: requestBody(body),
    }, scope, true);
    if (!response.ok) throw await parseError(response);
    return response.json() as Promise<T>;
  };

  return {
    async get(signature, options = {}) {
      if (options.commitment !== undefined && !['confirmed', 'finalized'].includes(options.commitment)) throw new TypeError('get accepts confirmed or finalized');
      if (options.maxSupportedTransactionVersion !== undefined && (!Number.isInteger(options.maxSupportedTransactionVersion) || options.maxSupportedTransactionVersion < 0 || options.maxSupportedTransactionVersion > 255)) throw new RangeError('Invalid maximum transaction version');
      const body = await post<{ transaction: Record<string, unknown> | null }>('get', { signature, ...options }, 'transaction:inspect');
      if (body.transaction === null) return null;
      const tx = body.transaction;
      if (!tx || typeof tx.signature !== 'string' || !Array.isArray(tx.accounts)) throw new TypeError('Invalid transaction response');
      const accounts = tx.accounts.map((entry: Record<string, unknown>) => {
        if (typeof entry.pubkey !== 'string') throw new TypeError('Missing transaction account pubkey');
        return { pubkey: entry.pubkey, preBalance: managedU64(entry.preBalance, 'preBalance'), postBalance: managedU64(entry.postBalance, 'postBalance') };
      });
      let blockTime: bigint | null = null;
      if (tx.blockTime != null) {
        if (typeof tx.blockTime !== 'string' || !/^-?\d+$/.test(tx.blockTime)) throw new TypeError('Invalid blockTime');
        blockTime = BigInt(tx.blockTime);
        if (blockTime < -9223372036854775808n || blockTime > 9223372036854775807n) throw new RangeError('blockTime exceeds i64');
      }
      return { ...tx, signature: tx.signature, slot: managedU64(tx.slot, 'slot'), blockTime, err: tx.err ?? null, accounts } as ConfirmedTransaction;
    },
    async getLatestBlockhash(options = {}) {
      const value = await post<Record<string, unknown>>('latest-blockhash', {
        commitment: options.commitment,
        minContextSlot: decimal(options.minContextSlot),
      }, 'transaction:inspect');
      return {
        blockhash: String(value.blockhash),
        contextSlot: bigintField(value.contextSlot, 'contextSlot'),
        lastValidBlockHeight: bigintField(value.lastValidBlockHeight, 'lastValidBlockHeight'),
      };
    },
    async getFeeForMessage(message, options = {}) {
      const value = await post<Record<string, unknown>>('fee', {
        message,
        commitment: options.commitment,
        minContextSlot: decimal(options.minContextSlot),
      }, 'transaction:inspect');
      return {
        feeLamports: value.feeLamports === null ? null : bigintField(value.feeLamports, 'feeLamports'),
        contextSlot: bigintField(value.contextSlot, 'contextSlot'),
      };
    },
    async simulateTransaction(transaction, options = {}) {
      const value = await post<Record<string, unknown>>('simulate', {
        transaction,
        commitment: options.commitment,
        minContextSlot: decimal(options.minContextSlot),
        accounts: options.accounts ? { addresses: options.accounts } : undefined,
        innerInstructions: options.innerInstructions,
        replaceRecentBlockhash: options.replaceRecentBlockhash,
      }, 'transaction:inspect');
      return {
        contextSlot: bigintField(value.contextSlot, 'contextSlot'),
        err: value.err ?? null,
        logs: value.logs as readonly string[] | null ?? null,
        unitsConsumed: optionalBigint(value.unitsConsumed, 'unitsConsumed'),
        loadedAccountsDataSize: optionalBigint(
          value.loadedAccountsDataSize,
          'loadedAccountsDataSize'
        ),
        accounts: value.accounts as readonly unknown[] | null | undefined,
      };
    },
    sendTransaction(transaction, options = {}) {
      return post<TransactionSendResult>('send', {
        transaction,
        skipPreflight: options.skipPreflight,
        preflightCommitment: options.preflightCommitment,
        minContextSlot: decimal(options.minContextSlot),
      }, 'transaction:send');
    },
    async getSignatureStatus(signature, options = {}) {
      const value = await post<Record<string, unknown> | null>('signature-status', {
        signature,
        searchTransactionHistory: options.searchTransactionHistory,
      }, 'transaction:inspect');
      const status = value?.status as Record<string, unknown> | null | undefined;
      if (!status) return null;
      return {
        signature,
        slot: status.slot === null ? null : bigintField(status.slot, 'slot'),
        confirmationStatus: status.confirmationStatus as TransactionCommitment | null,
        err: status.err ?? null,
      };
    },
    async getBlockHeight(options = {}) {
      const value = await post<Record<string, unknown>>('block-height', {
        commitment: options.commitment,
        minContextSlot: decimal(options.minContextSlot),
      }, 'transaction:inspect');
      return bigintField(value.blockHeight, 'blockHeight');
    },
  };
}
