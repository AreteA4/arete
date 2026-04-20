import { useState, useCallback } from 'react';
import type { 
  InstructionExecutor,
  InstructionExecutorOptions, 
  ExecutionResult,
} from '@usearete/sdk';
import { parseInstructionError } from '@usearete/sdk';

export type MutationStatus = 'idle' | 'pending' | 'success' | 'error';

export interface UseMutationOptions extends InstructionExecutorOptions {
  onSuccess?: (result: ExecutionResult) => void;
  onError?: (error: Error) => void;
}

export interface UseMutationResult {
  submit: (args: Record<string, unknown>, options?: Partial<UseMutationOptions>) => Promise<ExecutionResult>;
  status: MutationStatus;
  error: string | null;
  signature: string | null;
  isLoading: boolean;
  reset: () => void;
}

export function useInstructionMutation(
  execute: InstructionExecutor
): UseMutationResult {
  const [status, setStatus] = useState<MutationStatus>('idle');
  const [error, setError] = useState<string | null>(null);
  const [signature, setSignature] = useState<string | null>(null);

  const submit = useCallback(async (
    args: Record<string, unknown>,
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
      const errorMessage = err instanceof Error ? err.message : String(err);
      
      const programError = parseInstructionError(err, []);
      const displayError = programError 
        ? `${programError.name}: ${programError.message}`
        : errorMessage;

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
