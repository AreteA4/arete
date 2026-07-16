/**
 * Parses and handles instruction errors.
 */

/**
 * Custom error from a Solana program.
 */
export interface ProgramError {
  /** Error code */
  code: number;
  /** Error name */
  name: string;
  /** Error message */
  message: string;
}

/**
 * Error metadata from IDL.
 */
export interface ErrorMetadata {
  code: number;
  name: string;
  msg: string;
}

export type TransactionFailureStatus =
  | 'not-submitted'
  | 'submitted-unknown'
  | 'chain-failed';

export type TransactionFailurePhase =
  | 'build'
  | 'wallet'
  | 'send'
  | 'confirmation'
  | 'chain';

export interface ConfirmedTransactionOutcome {
  readonly status: 'confirmed';
  readonly phase: 'confirmation';
  readonly signature: string;
  readonly slot?: number;
}

export interface NotSubmittedTransactionOutcome {
  readonly status: 'not-submitted';
  readonly phase: 'build' | 'wallet' | 'send';
  readonly cause: unknown;
}

export interface SubmittedUnknownTransactionOutcome {
  readonly status: 'submitted-unknown';
  readonly phase: 'send' | 'confirmation';
  readonly signature: string;
  readonly slot?: number;
  readonly cause: unknown;
}

export interface ChainFailedTransactionOutcome {
  readonly status: 'chain-failed';
  readonly phase: 'confirmation' | 'chain';
  readonly signature?: string;
  readonly slot?: number;
  readonly programError?: ProgramError;
  readonly cause: unknown;
}

export type TransactionFailureOutcome =
  | NotSubmittedTransactionOutcome
  | SubmittedUnknownTransactionOutcome
  | ChainFailedTransactionOutcome;

export type TransactionOutcome = ConfirmedTransactionOutcome | TransactionFailureOutcome;

/**
 * Structured transaction failure thrown by adapters and the core executor.
 */
export class TransactionExecutionError extends Error {
  readonly outcome: TransactionFailureOutcome;
  readonly cause: unknown;
  readonly signature?: string;
  readonly slot?: number;

  constructor(outcome: TransactionFailureOutcome, message?: string) {
    super(message ?? errorMessage(outcome.cause, defaultOutcomeMessage(outcome)));
    this.name = 'TransactionExecutionError';
    this.outcome = outcome;
    this.cause = outcome.cause;
    this.signature = 'signature' in outcome ? outcome.signature : undefined;
    this.slot = 'slot' in outcome ? outcome.slot : undefined;
  }
}

/**
 * Parses an error returned from a Solana transaction.
 * 
 * @param error - The error from the transaction
 * @param errorMetadata - Error definitions from the IDL
 * @returns Parsed program error or null if not a program error
 */
export function parseInstructionError(
  error: unknown,
  errorMetadata: readonly ErrorMetadata[]
): ProgramError | null {
  return parseInstructionErrorMatch(error, errorMetadata)?.programError ?? null;
}

interface ExtractedErrorCode {
  readonly code: number;
  readonly source: 'instruction-error' | 'program-error' | 'direct-code';
}

interface InstructionErrorMatch {
  readonly programError: ProgramError;
  readonly deterministic: boolean;
}

function extractErrorCodes(
  error: unknown,
  seen: Set<object>,
  results: ExtractedErrorCode[]
): void {
  if (typeof error !== 'object' || error === null) {
    return;
  }

  if (seen.has(error)) {
    return;
  }
  seen.add(error);

  if (error instanceof InstructionError && error.programError) {
    results.push({ code: error.programError.code, source: 'program-error' });
    return;
  }

  if (error instanceof TransactionExecutionError) {
    const outcomeCode = error.outcome.status === 'chain-failed'
      ? error.outcome.programError?.code
      : undefined;
    if (outcomeCode !== undefined) {
      results.push({ code: outcomeCode, source: 'program-error' });
      return;
    }
  }

  const errorObj = error as Record<string, unknown>;

  // Check for InstructionError format
  if (Array.isArray(errorObj.InstructionError)) {
    const instructionError = errorObj.InstructionError;
    const detail = instructionError[1];
    if (typeof detail === 'object' && detail !== null) {
      const custom = (detail as { Custom?: unknown }).Custom;
      if (typeof custom === 'number') {
        results.push({ code: custom, source: 'instruction-error' });
      }
    }
  }

  const programError = errorObj.programError;
  if (typeof programError === 'object' && programError !== null) {
    const code = (programError as { code?: unknown }).code;
    if (typeof code === 'number') {
      results.push({ code, source: 'program-error' });
    }
  }

  if (typeof errorObj.code === 'number') {
    results.push({ code: errorObj.code, source: 'direct-code' });
  }

  for (const key of ['cause', 'error', 'err', 'value', 'data', 'outcome', 'transactionError']) {
    extractErrorCodes(errorObj[key], seen, results);
  }
}

function parseInstructionErrorMatch(
  error: unknown,
  errorMetadata: readonly ErrorMetadata[]
): InstructionErrorMatch | null {
  if (!error) {
    return null;
  }

  const candidates: ExtractedErrorCode[] = [];
  extractErrorCodes(error, new Set(), candidates);
  const selected = candidates.find((candidate) => candidate.source === 'instruction-error')
    ?? candidates.find((candidate) => candidate.source === 'program-error')
    ?? candidates.find((candidate) =>
      errorMetadata.some((metadata) => metadata.code === candidate.code)
    )
    ?? candidates[0];
  if (!selected) {
    return null;
  }

  const metadata = errorMetadata.find((entry) => entry.code === selected.code);
  const programError = metadata
    ? {
        code: metadata.code,
        name: metadata.name,
        message: metadata.msg,
      }
    : {
        code: selected.code,
        name: `CustomError${selected.code}`,
        message: `Unknown error with code ${selected.code}`,
      };

  return {
    programError,
    deterministic: selected.source !== 'direct-code' || metadata !== undefined,
  };
}

/**
 * Formats an error for display.
 * 
 * @param error - The program error
 * @returns Human-readable error message
 */
export function formatProgramError(error: ProgramError): string {
  return `${error.name} (${error.code}): ${error.message}`;
}

/**
 * Error thrown when an instruction fails to send and the underlying failure
 * could be parsed against the handler's IDL error definitions.
 */
export class InstructionError extends Error {
  /** Parsed program error, if the failure matched a known error code. */
  readonly programError: ProgramError | null;
  /** The original underlying error from the wallet adapter / RPC. */
  readonly cause: unknown;
  /** Structured chain-failure outcome, including signature and slot when known. */
  readonly outcome: ChainFailedTransactionOutcome;
  readonly signature?: string;
  readonly slot?: number;

  constructor(
    message: string,
    programError: ProgramError | null,
    cause: unknown,
    outcome?: ChainFailedTransactionOutcome
  ) {
    super(message);
    this.name = 'InstructionError';
    this.programError = programError;
    if (outcome) {
      this.outcome = outcome;
    } else {
      const context = getTransactionFailureOutcome(cause);
      this.outcome = {
        status: 'chain-failed',
        phase: 'chain',
        signature: context && 'signature' in context ? context.signature : undefined,
        slot: context && 'slot' in context ? context.slot : undefined,
        programError: programError ?? undefined,
        cause: context?.cause ?? cause,
      };
    }
    this.cause = this.outcome.cause;
    this.signature = this.outcome.signature;
    this.slot = this.outcome.slot;
  }
}

function defaultOutcomeMessage(outcome: TransactionFailureOutcome): string {
  switch (outcome.status) {
    case 'not-submitted':
      return `Transaction was not submitted during ${outcome.phase}`;
    case 'submitted-unknown':
      return `Transaction ${outcome.signature} was submitted but its status is unknown`;
    case 'chain-failed':
      return outcome.signature
        ? `Transaction ${outcome.signature} failed on chain`
        : 'Transaction failed on chain';
  }
}

function errorMessage(cause: unknown, fallback: string): string {
  return cause instanceof Error && cause.message ? cause.message : fallback;
}

function isFailureOutcome(value: unknown): value is TransactionFailureOutcome {
  if (!value || typeof value !== 'object') {
    return false;
  }
  const candidate = value as { status?: unknown; phase?: unknown; cause?: unknown };
  return (
    (candidate.status === 'not-submitted'
      || candidate.status === 'submitted-unknown'
      || candidate.status === 'chain-failed')
    && typeof candidate.phase === 'string'
    && 'cause' in candidate
  );
}

/** Find a structured transaction failure through nested error causes. */
export function getTransactionFailureOutcome(error: unknown): TransactionFailureOutcome | null {
  const seen = new Set<object>();
  let current = error;
  while (typeof current === 'object' && current !== null && !seen.has(current)) {
    seen.add(current);
    if (current instanceof TransactionExecutionError || current instanceof InstructionError) {
      return current.outcome;
    }
    const candidate = current as { outcome?: unknown; cause?: unknown };
    if (isFailureOutcome(candidate.outcome)) {
      return candidate.outcome;
    }
    current = candidate.cause;
  }
  return null;
}

function extractTransactionContext(error: unknown): { signature?: string; slot?: number } {
  const seen = new Set<object>();
  let current = error;
  let signature: string | undefined;
  let slot: number | undefined;
  while (typeof current === 'object' && current !== null && !seen.has(current)) {
    seen.add(current);
    const candidate = current as {
      signature?: unknown;
      slot?: unknown;
      cause?: unknown;
      outcome?: unknown;
    };
    if (!signature && typeof candidate.signature === 'string') {
      signature = candidate.signature;
    }
    if (slot === undefined && typeof candidate.slot === 'number') {
      slot = candidate.slot;
    }
    if (isFailureOutcome(candidate.outcome)) {
      if (!signature && 'signature' in candidate.outcome) {
        signature = candidate.outcome.signature;
      }
      if (slot === undefined && 'slot' in candidate.outcome) {
        slot = candidate.outcome.slot;
      }
    }
    current = candidate.cause;
  }
  return { signature, slot };
}

function isWalletRejection(error: unknown): boolean {
  const seen = new Set<object>();
  let current = error;
  while (typeof current === 'object' && current !== null && !seen.has(current)) {
    seen.add(current);
    const candidate = current as {
      code?: unknown;
      name?: unknown;
      message?: unknown;
      cause?: unknown;
      error?: unknown;
    };
    if (
      candidate.code === 4001
      || candidate.code === '4001'
      || candidate.code === 'ACTION_REJECTED'
    ) {
      return true;
    }
    const name = String(candidate.name ?? '');
    if (/^(UserRejectedRequestError|UserRejectError|WalletRequestRejectedError)$/.test(name)) {
      return true;
    }
    const message = String(candidate.message ?? '').trim();
    if (
      /^(?:the )?user (?:rejected|declined|denied)(?: the)? (?:request|transaction|signature request|wallet request)[.!]?$/i.test(message)
      || /^(?:request|transaction) (?:was )?(?:rejected|declined|denied) by (?:the )?user[.!]?$/i.test(message)
    ) {
      return true;
    }
    current = candidate.cause ?? candidate.error;
  }
  return false;
}

/**
 * Normalize an adapter/host exception without retrying or resubmitting.
 */
export function normalizeTransactionError(
  cause: unknown,
  errorMetadata: readonly ErrorMetadata[] = [],
  fallbackPhase: 'wallet' | 'send' = 'send'
): InstructionError | TransactionExecutionError {
  if (cause instanceof InstructionError) {
    return cause;
  }

  const existingOutcome = getTransactionFailureOutcome(cause);
  const instructionMatch = parseInstructionErrorMatch(cause, errorMetadata);
  const asInstructionError = (programError: ProgramError): InstructionError => {
    const context = extractTransactionContext(cause);
    const originalCause = existingOutcome?.cause ?? cause;
    const outcome: ChainFailedTransactionOutcome = {
      status: 'chain-failed',
      phase: existingOutcome?.status === 'chain-failed'
        ? existingOutcome.phase
        : 'chain',
      signature: context.signature,
      slot: context.slot,
      programError,
      cause: originalCause,
    };
    return new InstructionError(
      formatProgramError(programError),
      programError,
      originalCause,
      outcome
    );
  };

  if (instructionMatch?.deterministic) {
    return asInstructionError(instructionMatch.programError);
  }
  if (existingOutcome) {
    return cause instanceof TransactionExecutionError
      ? cause
      : new TransactionExecutionError(existingOutcome);
  }
  if (!existingOutcome && isWalletRejection(cause)) {
    return new TransactionExecutionError({
      status: 'not-submitted',
      phase: 'wallet',
      cause,
    });
  }
  if (instructionMatch) {
    return asInstructionError(instructionMatch.programError);
  }

  if (cause instanceof TransactionExecutionError) {
    return cause;
  }
  if (existingOutcome) {
    return new TransactionExecutionError(existingOutcome);
  }

  const context = extractTransactionContext(cause);
  if (context.signature) {
    return new TransactionExecutionError({
      status: 'submitted-unknown',
      phase: 'confirmation',
      signature: context.signature,
      slot: context.slot,
      cause,
    });
  }
  return new TransactionExecutionError({
    status: 'not-submitted',
    phase: fallbackPhase,
    cause,
  });
}
