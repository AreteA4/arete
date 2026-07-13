import { describe, expect, it } from 'vitest';

import { SortedStorageDecorator } from './sorted-decorator';
import { MemoryAdapter } from './memory-adapter';

describe('SortedStorageDecorator', () => {
  it('sorts bigint values numerically', () => {
    const storage = new SortedStorageDecorator(new MemoryAdapter());
    storage.setViewConfig?.('TokenPosition/list', {
      sort: {
        field: ['totalDeposit'],
        order: 'desc',
      },
    });

    storage.set('TokenPosition/list', 'small', { totalDeposit: 1n });
    storage.set('TokenPosition/list', 'largest', { totalDeposit: 9n });
    storage.set('TokenPosition/list', 'middle', { totalDeposit: 5n });

    expect(storage.getAll<{ totalDeposit: bigint }>('TokenPosition/list')).toEqual([
      { totalDeposit: 9n },
      { totalDeposit: 5n },
      { totalDeposit: 1n },
    ]);
  });
});
