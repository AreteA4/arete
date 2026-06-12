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

/**
 * Parses an error returned from a Solana transaction.
 * 
 * @param error - The error from the transaction
 * @param errorMetadata - Error definitions from the IDL
 * @returns Parsed program error or null if not a program error
 */
export function parseInstructionError(
  error: unknown,
  errorMetadata: ErrorMetadata[]
): ProgramError | null {
  if (!error) {
    return null;
  }
  
  const errorCode = extractErrorCode(error);
  
  if (errorCode === null) {
    return null;
  }
  
  const metadata = errorMetadata.find(e => e.code === errorCode);
  
  if (metadata) {
    return {
      code: metadata.code,
      name: metadata.name,
      message: metadata.msg,
    };
  }
  
  return {
    code: errorCode,
    name: `CustomError${errorCode}`,
    message: `Unknown error with code ${errorCode}`,
  };
}

function extractErrorCode(error: unknown): number | null {
  if (typeof error !== 'object' || error === null) {
    return null;
  }
  
  const errorObj = error as Record<string, unknown>;
  
  // Check for InstructionError format
  if (errorObj.InstructionError) {
    const instructionError = errorObj.InstructionError as [number, { Custom?: number }];
    if (instructionError[1]?.Custom !== undefined) {
      return instructionError[1].Custom;
    }
  }
  
  // Check for direct code
  if (typeof errorObj.code === 'number') {
    return errorObj.code;
  }
  
  return null;
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

  constructor(message: string, programError: ProgramError | null, cause: unknown) {
    super(message);
    this.name = 'InstructionError';
    this.programError = programError;
    this.cause = cause;
  }
}
