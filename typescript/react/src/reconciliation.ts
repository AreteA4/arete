import { useCallback } from 'react';
import type { PreparedOperation, WaitForProcessedSlotOptions } from '@usearete/sdk';
import type {
  MutationReconciliationContext,
  MutationReconcileOverrides,
  ReconciliationRefreshTarget,
} from './hooks/use-mutation';

export const DEFAULT_RECONCILIATION_TIMEOUT_MS = 30_000;

export interface ProcessedSlotClient {
  waitForProcessedSlot(
    slot: number | bigint,
    options?: WaitForProcessedSlotOptions
  ): Promise<bigint>;
}

export type ReconciliationRefresh = ReconciliationRefreshTarget;

export interface ProcessedSlotReconciliationOptions extends WaitForProcessedSlotOptions {
  refresh?: ReconciliationRefresh | readonly ReconciliationRefresh[];
}

export type ProcessedSlotReconciliationResult =
  | {
      readonly status: 'reconciled';
      readonly confirmedSlot: bigint;
      readonly processedSlot: bigint;
    }
  | {
      readonly status: 'confirmed-unreconciled';
      readonly confirmedSlot: bigint;
      readonly error: Error;
    };

function toSlot(slot: number | bigint): bigint {
  if (typeof slot === 'number') {
    if (!Number.isSafeInteger(slot) || slot < 0) {
      throw new RangeError('slot must be a non-negative safe integer or bigint');
    }
    return BigInt(slot);
  }
  if (slot < 0n) {
    throw new RangeError('slot must be non-negative');
  }
  return slot;
}

function reconciliationError(value: unknown): Error {
  if (value instanceof Error) {
    return value;
  }
  const error = new Error(typeof value === 'string' ? value : String(value));
  (error as Error & { cause?: unknown }).cause = value;
  return error;
}

async function refreshTargets(
  targets: readonly ReconciliationRefresh[],
  timeoutMs: number,
  signal?: AbortSignal,
): Promise<void> {
  if (targets.length === 0) return;

  let timeout: ReturnType<typeof setTimeout> | undefined;
  let onAbort: (() => void) | undefined;
  const interrupted = new Promise<never>((_resolve, reject) => {
    timeout = setTimeout(() => {
      reject(new Error(`Timed out waiting for refreshed Arete data after ${timeoutMs}ms`));
    }, timeoutMs);
    if (signal) {
      onAbort = () => reject(reconciliationError(signal.reason ?? 'Reconciliation was cancelled'));
      if (signal.aborted) {
        onAbort();
      } else {
        signal.addEventListener('abort', onAbort, { once: true });
      }
    }
  });

  try {
    await Promise.race([
      Promise.all(targets.map((target) =>
        typeof target === 'function' ? target() : target.refresh()
      )),
      interrupted,
    ]);
  } finally {
    if (timeout !== undefined) clearTimeout(timeout);
    if (signal && onAbort) signal.removeEventListener('abort', onAbort);
  }
}

export async function reconcileProcessedSlot(
  client: ProcessedSlotClient | null | undefined,
  slot: number | bigint,
  options: ProcessedSlotReconciliationOptions = {}
): Promise<ProcessedSlotReconciliationResult> {
  const confirmedSlot = toSlot(slot);
  if (!client) {
    return {
      status: 'confirmed-unreconciled',
      confirmedSlot,
      error: new Error('Arete client is not connected'),
    };
  }

  try {
    const processedSlot = await client.waitForProcessedSlot(confirmedSlot, {
      timeoutMs: options.timeoutMs ?? DEFAULT_RECONCILIATION_TIMEOUT_MS,
      signal: options.signal,
    });
    const refreshes = Array.isArray(options.refresh)
      ? options.refresh
      : options.refresh
        ? [options.refresh]
        : [];
    await refreshTargets(
      refreshes,
      options.timeoutMs ?? DEFAULT_RECONCILIATION_TIMEOUT_MS,
      options.signal,
    );
    return { status: 'reconciled', confirmedSlot, processedSlot };
  } catch (value) {
    return {
      status: 'confirmed-unreconciled',
      confirmedSlot,
      error: reconciliationError(value),
    };
  }
}

export function useReconcileProcessedSlot(
  client: ProcessedSlotClient | null | undefined
): (
  slot: number | bigint,
  options?: ProcessedSlotReconciliationOptions
) => Promise<ProcessedSlotReconciliationResult> {
  return useCallback(
    (slot, options) => reconcileProcessedSlot(client, slot, options),
    [client]
  );
}

/**
 * The default reconciliation injected into generated program mutation hooks.
 * Marked so `useInstructionMutation` can resolve `reconcile: { ... }` override
 * objects against it via {@link DefaultReconciliationFn.withOverrides}.
 */
export interface DefaultReconciliationFn {
  (
    context: MutationReconciliationContext<unknown, PreparedOperation>
  ): Promise<ProcessedSlotReconciliationResult | undefined>;
  readonly areteDefaultReconciliation: true;
  readonly shouldReconcile: (
    context: MutationReconciliationContext<unknown, PreparedOperation>
  ) => boolean;
  readonly withOverrides: (
    overrides: MutationReconcileOverrides
  ) => DefaultReconciliationFn;
}

/**
 * Build the default reconciliation for generated program hooks: wait until the
 * stream has processed the highest slot among completed transactions, then run
 * any requested refreshes. Mutations without a reported slot (e.g. raw
 * instruction sends) skip reconciliation silently.
 */
export function createProcessedSlotReconciliation(
  client: ProcessedSlotClient | null | undefined,
  baseOverrides: MutationReconcileOverrides = {}
): DefaultReconciliationFn {
  const build = (overrides: MutationReconcileOverrides): DefaultReconciliationFn => {
    const receiptSlot = (
      context: MutationReconciliationContext<unknown, PreparedOperation>
    ): number | undefined => context.completedReceipts.reduce<number | undefined>(
      (highest, receipt) => receipt.slot === undefined
        ? highest
        : highest === undefined
          ? receipt.slot
          : Math.max(highest, receipt.slot),
      undefined
    );
    const reconcile = async (
      context: MutationReconciliationContext<unknown, PreparedOperation>
    ): Promise<ProcessedSlotReconciliationResult | undefined> => {
      const slot = receiptSlot(context);
      if (slot === undefined) {
        return undefined;
      }
      return reconcileProcessedSlot(client, slot, {
        signal: context.signal,
        timeoutMs: overrides.timeoutMs,
        refresh: overrides.refresh,
      });
    };
    return Object.assign(reconcile, {
      areteDefaultReconciliation: true as const,
      shouldReconcile: (context: MutationReconciliationContext<unknown, PreparedOperation>) =>
        receiptSlot(context) !== undefined,
      withOverrides: (next: MutationReconcileOverrides) =>
        build({ ...overrides, ...next }),
    });
  };
  return build(baseOverrides);
}
