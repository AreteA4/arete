jest.mock('react', () => ({
  useCallback: jest.fn((callback: unknown) => callback),
}));

import { createProcessedSlotReconciliation, reconcileProcessedSlot } from './reconciliation';

describe('reconcileProcessedSlot', () => {
  it('waits for stored stream progress before refreshing dependent reads', async () => {
    const order: string[] = [];
    const client = {
      waitForProcessedSlot: jest.fn(async () => {
        order.push('processed');
        return 101n;
      }),
    };
    const refresh = jest.fn(async () => {
      order.push('refresh');
    });

    await expect(reconcileProcessedSlot(client, 100, { refresh })).resolves.toEqual({
      status: 'reconciled',
      confirmedSlot: 100n,
      processedSlot: 101n,
    });
    expect(order).toEqual(['processed', 'refresh']);
  });

  it('accepts hook results carrying a refresh method as refresh targets', async () => {
    const client = {
      waitForProcessedSlot: jest.fn(async () => 101n),
    };
    const viewRefresh = jest.fn();
    const readRefresh = jest.fn(async () => 'data');

    await expect(reconcileProcessedSlot(client, 100, {
      refresh: [{ refresh: viewRefresh }, { refresh: readRefresh }],
    })).resolves.toEqual({
      status: 'reconciled',
      confirmedSlot: 100n,
      processedSlot: 101n,
    });
    expect(viewRefresh).toHaveBeenCalled();
    expect(readRefresh).toHaveBeenCalled();
  });

  it('returns confirmed-unreconciled instead of throwing on timeout', async () => {
    const timeout = new Error('stream timeout');
    const client = {
      waitForProcessedSlot: jest.fn(async () => {
        throw timeout;
      }),
    };

    await expect(reconcileProcessedSlot(client, 100n, { timeoutMs: 1 })).resolves.toEqual({
      status: 'confirmed-unreconciled',
      confirmedSlot: 100n,
      error: timeout,
    });
  });

  it('returns confirmed-unreconciled when a refresh request rejects', async () => {
    const refreshError = new Error('refresh send failed');
    const client = {
      waitForProcessedSlot: jest.fn(async () => 101n),
    };

    await expect(reconcileProcessedSlot(client, 100n, {
      refresh: { refresh: jest.fn(async () => { throw refreshError; }) },
    })).resolves.toEqual({
      status: 'confirmed-unreconciled',
      confirmedSlot: 100n,
      error: refreshError,
    });
  });

  it('returns confirmed-unreconciled when refreshed data does not arrive', async () => {
    const client = {
      waitForProcessedSlot: jest.fn(async () => 101n),
    };

    await expect(reconcileProcessedSlot(client, 100n, {
      timeoutMs: 1,
      refresh: jest.fn(() => new Promise<void>(() => undefined)),
    })).resolves.toEqual({
      status: 'confirmed-unreconciled',
      confirmedSlot: 100n,
      error: expect.objectContaining({
        message: 'Timed out waiting for refreshed Arete data after 1ms',
      }),
    });
  });
});

describe('createProcessedSlotReconciliation', () => {
  const context = (completedReceipts: Array<Record<string, unknown>>) => ({
    result: undefined,
    prepared: null,
    signatures: [],
    completedReceipts: completedReceipts as never,
    signal: new AbortController().signal,
  });

  it('skips reconciliation when no receipt reports a slot', async () => {
    const client = { waitForProcessedSlot: jest.fn() };
    const reconcile = createProcessedSlotReconciliation(client);

    await expect(reconcile(context([]))).resolves.toBeUndefined();
    await expect(reconcile(context([
      { transactionIndex: 0, transactionName: 'deploy', signature: 'sig' },
    ]))).resolves.toBeUndefined();
    expect(client.waitForProcessedSlot).not.toHaveBeenCalled();
  });

  it('waits on the highest receipt slot', async () => {
    const client = { waitForProcessedSlot: jest.fn(async () => 55n) };
    const reconcile = createProcessedSlotReconciliation(client);

    await expect(reconcile(context([
      { transactionIndex: 0, transactionName: 'one', signature: 'sig-1', slot: 58 },
      { transactionIndex: 1, transactionName: 'two', signature: 'sig-2' },
      { transactionIndex: 2, transactionName: 'three', signature: 'sig-3', slot: 54 },
      { transactionIndex: 3, transactionName: 'four', signature: 'sig-4' },
    ]))).resolves.toEqual({ status: 'reconciled', confirmedSlot: 58n, processedSlot: 55n });
    expect(client.waitForProcessedSlot).toHaveBeenCalledWith(58n, expect.objectContaining({
      timeoutMs: 30_000,
    }));
  });

  it('marks the default and applies withOverrides merges', async () => {
    const client = { waitForProcessedSlot: jest.fn(async () => 11n) };
    const reconcile = createProcessedSlotReconciliation(client, { timeoutMs: 5_000 });
    expect(reconcile.areteDefaultReconciliation).toBe(true);

    const refresh = jest.fn();
    const overridden = reconcile.withOverrides({ refresh });
    expect(overridden.areteDefaultReconciliation).toBe(true);

    await overridden(context([
      { transactionIndex: 0, transactionName: 'deploy', signature: 'sig', slot: 10 },
    ]));
    expect(client.waitForProcessedSlot).toHaveBeenCalledWith(10n, expect.objectContaining({
      timeoutMs: 5_000,
    }));
    expect(refresh).toHaveBeenCalled();
  });

  it('forwards the reconciliation abort signal', async () => {
    const client = { waitForProcessedSlot: jest.fn(async () => 11n) };
    const reconcile = createProcessedSlotReconciliation(client);
    const signal = new AbortController().signal;

    await reconcile({ ...context([
      { transactionIndex: 0, transactionName: 'deploy', signature: 'sig', slot: 10 },
    ]), signal });
    expect(client.waitForProcessedSlot).toHaveBeenCalledWith(10n, expect.objectContaining({
      signal,
    }));
  });

  it('returns confirmed-unreconciled when the client is not connected', async () => {
    const reconcile = createProcessedSlotReconciliation(null);

    await expect(reconcile(context([
      { transactionIndex: 0, transactionName: 'deploy', signature: 'sig', slot: 10 },
    ]))).resolves.toEqual({
      status: 'confirmed-unreconciled',
      confirmedSlot: 10n,
      error: expect.objectContaining({ message: 'Arete client is not connected' }),
    });
  });
});
