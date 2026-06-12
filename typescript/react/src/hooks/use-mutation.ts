import { useState, useCallback } from 'react';
import type {
  TypedInstruction,
  InstructionExecutorOptions,
  ExecutionResult,
} from '@usearete/sdk';
import { InstructionError } from '@usearete/sdk';

export type MutationStatus = 'idle' | 'pending' | 'success' | 'error';

export interface UseMutationOptions extends InstructionExecutorOptions {
  onSuccess?: (result: ExecutionResult) => void;
  onError?: (error: Error) => void;
}

/**
 * Result of {@link useInstructionMutation}.
 *
 * `TParams` is the merged params object accepted by the instruction (IDL args
 * plus any user-provided account addresses), inferred from the generated
 * handler. `TError` is the handler's typed program-error union.
 */
export interface UseMutationResult<TParams = Record<string, unknown>, _TError = unknown> {
  submit: (args: TParams, options?: Partial<UseMutationOptions>) => Promise<ExecutionResult>;
  status: MutationStatus;
  error: string | null;
  signature: string | null;
  isLoading: boolean;
  reset: () => void;
}

export function useInstructionMutation<TParams = Record<string, unknown>, TError = unknown>(
  execute: TypedInstruction<TParams, TError>
): UseMutationResult<TParams, TError> {
  const [status, setStatus] = useState<MutationStatus>('idle');
  const [error, setError] = useState<string | null>(null);
  const [signature, setSignature] = useState<string | null>(null);

  const submit = useCallback(async (
    args: TParams,
    options?: Partial<UseMutationOptions>
  ): Promise<ExecutionResult> => {
    setStatus('pending');
    setError(null);
    setSignature(null);

    try {
      const result = await execute(args, options as InstructionExecutorOptions);

      setStatus('success');
      setSignature(result.signature);

      if (options?.onSuccess) {
        options.onSuccess(result);
      }

      return result;
    } catch (err) {
      // The core executor already parses program errors against the handler's
      // IDL error definitions and throws an InstructionError.
      const displayError =
        err instanceof InstructionError && err.programError
          ? `${err.programError.name}: ${err.programError.message}`
          : err instanceof Error
            ? err.message
            : String(err);

      setStatus('error');
      setError(displayError);

      if (options?.onError && err instanceof Error) {
        options.onError(err);
      }

      throw err;
    }
  }, [execute]);

  const reset = useCallback(() => {
    setStatus('idle');
    setError(null);
    setSignature(null);
  }, []);

  return {
    submit,
    status,
    error,
    signature,
    isLoading: status === 'pending',
    reset,
  };
}
