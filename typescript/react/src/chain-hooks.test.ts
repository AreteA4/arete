jest.mock('./hooks/use-async-read', () => ({
  useAsyncRead: jest.fn((_key: unknown, read: unknown, options: unknown) => ({
    read,
    options,
  })),
}));

import { useAsyncRead } from './hooks/use-async-read';
import { useNativeBalance, useTokenBalance } from './chain-hooks';

const mockUseAsyncRead = useAsyncRead as jest.Mock;

describe('exact balance hooks', () => {
  beforeEach(() => {
    mockUseAsyncRead.mockClear();
  });

  it('returns exact native balances and forwards minContextSlot', async () => {
    const value = { lamports: 9_007_199_254_740_993n, contextSlot: 88n };
    const chain = {
      nativeBalance: jest.fn(async () => value),
    };
    const result = useNativeBalance(chain as never, 'owner', { minContextSlot: 77n });
    const read = (result as unknown as {
      read: (context: { signal: AbortSignal }) => Promise<typeof value>;
    }).read;

    await expect(read({ signal: new AbortController().signal })).resolves.toBe(value);
    expect(chain.nativeBalance).toHaveBeenCalledWith('owner', { minContextSlot: 77n });
  });

  it('preserves raw token amount strings and contextual slots', async () => {
    const input = { owner: 'owner', mint: 'mint', tokenProgram: 'token-program' };
    const value = {
      exists: true,
      address: 'ata',
      owner: 'owner',
      mint: 'mint',
      amount: '9007199254740993',
      contextSlot: 99n,
    };
    const chain = { balance: jest.fn(async () => value) };
    const result = useTokenBalance(chain as never, input, { minContextSlot: 90 });
    const read = (result as unknown as {
      read: (context: { signal: AbortSignal }) => Promise<typeof value>;
    }).read;

    await expect(read({ signal: new AbortController().signal })).resolves.toEqual(value);
    expect(chain.balance).toHaveBeenCalledWith(input, { minContextSlot: 90 });
    expect(value.amount).toBe('9007199254740993');
  });
});
