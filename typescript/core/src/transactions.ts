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

export interface TransactionTransport {
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
