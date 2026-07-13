import { useState, useCallback } from 'react';
import { InstructionError } from '@usearete/sdk';

export type MutationStatus = 'idle' | 'pending' | 'success' | 'error';

export type UseMutationOptions<TOptions extends object = Record<string, never>, TResult = unknown> = TOptions & {
  onSuccess?: (result: TResult) => void;
  onError?: (error: Error) => void;
};

/**
 * Result of {@link useInstructionMutation}.
 *
 * `TParams` is the merged params object accepted by the instruction (IDL args
 * plus any user-provided account addresses), inferred from the generated
 * handler. `TError` is the handler's typed program-error union.
 */
export interface UseMutationResult<
  TParams = Record<string, unknown>,
  TResult = unknown,
  TOptions extends object = Record<string, never>,
> {
  submit: (args: TParams, options?: Partial<UseMutationOptions<TOptions, TResult>>) => Promise<TResult>;
  status: MutationStatus;
  error: string | null;
  signatures: string[];
  signature: string | null;
  isLoading: boolean;
  reset: () => void;
}

export type MutationExecutor<
  TParams = Record<string, unknown>,
  TResult = unknown,
  TOptions extends object = Record<string, never>,
> = {
  (args: TParams, options?: TOptions): Promise<TResult>;
};

export function useInstructionMutation<
  TParams = Record<string, unknown>,
  TResult = unknown,
  TOptions extends object = Record<string, never>,
>(
  execute: MutationExecutor<TParams, TResult, TOptions>
): UseMutationResult<TParams, TResult, TOptions> {
  const [status, setStatus] = useState<MutationStatus>('idle');
  const [error, setError] = useState<string | null>(null);
  const [signatures, setSignatures] = useState<string[]>([]);
  const [signature, setSignature] = useState<string | null>(null);

  const submit: UseMutationResult<TParams, TResult, TOptions>['submit'] = useCallback(async (
    args: TParams,
    options?: Partial<UseMutationOptions<TOptions, TResult>>
  ): Promise<TResult> => {
    setStatus('pending');
    setError(null);
    setSignatures([]);
    setSignature(null);

    const { onSuccess, onError, ...executionOptions } = options ?? {};

    try {
      const result = await execute(args, executionOptions as TOptions);
      const resultSignatures =
        typeof result === 'object' && result !== null && 'signatures' in result
          && Array.isArray((result as { signatures: unknown }).signatures)
          ? (result as { signatures: unknown[] }).signatures.map(String)
          : typeof result === 'object' && result !== null && 'signature' in result
            ? [String((result as { signature: unknown }).signature)]
            : [];

      setStatus('success');
      setSignatures(resultSignatures);
      setSignature(resultSignatures.length === 1 ? resultSignatures[0]! : null);

      if (onSuccess) {
        onSuccess(result);
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

      if (onError && err instanceof Error) {
        onError(err);
      }

      throw err;
    }
  }, [execute]);

  const reset = useCallback(() => {
    setStatus('idle');
    setError(null);
    setSignatures([]);
    setSignature(null);
  }, []);

  return {
    submit,
    status,
    error,
    signatures,
    signature,
    isLoading: status === 'pending',
    reset,
  };
}
