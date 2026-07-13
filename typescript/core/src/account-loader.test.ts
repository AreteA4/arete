import { describe, expect, it, vi } from 'vitest';

import { chainAccountLoader } from './account-loader';

describe('chainAccountLoader', () => {
  it('adapts chain.account() to the AccountLoader interface', async () => {
    const data = Uint8Array.from([1, 2, 3]);
    const chain = {
      account: vi.fn(async () => ({
        address: 'addr-1',
        ownerProgram: 'owner-program',
        lamports: 5,
        executable: false,
        data,
      })),
    };

    const loader = chainAccountLoader(chain as never);
    await expect(loader.getAccount('addr-1')).resolves.toEqual({ data });
    expect(chain.account).toHaveBeenCalledWith('addr-1');
  });

  it('returns null when the chain read misses', async () => {
    const chain = {
      account: vi.fn(async () => null),
    };

    const loader = chainAccountLoader(chain as never);
    await expect(loader.getAccount('missing')).resolves.toBeNull();
    expect(chain.account).toHaveBeenCalledWith('missing');
  });
});
