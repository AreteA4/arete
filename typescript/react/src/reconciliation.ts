import { useCallback } from 'react';
import type { WaitForProcessedSlotOptions } from '@usearete/sdk';

export const DEFAULT_RECONCILIATION_TIMEOUT_MS = 30_000;

export interface ProcessedSlotClient {
  waitForProcessedSlot(
    slot: number | bigint,
    options?: WaitForProcessedSlotOptions
  ): Promise<bigint>;
}

export type ReconciliationRefresh = () => unknown | Promise<unknown>;

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
    await Promise.all(refreshes.map((refresh) => refresh()));
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
