jest.mock('react', () => ({
  useCallback: jest.fn((callback: unknown) => callback),
}));

import { reconcileProcessedSlot } from './reconciliation';

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
});
