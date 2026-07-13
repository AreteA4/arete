import type { ChainClient } from './chain';

export interface AccountLoader {
  getAccount(address: string): Promise<{ data: Uint8Array } | null>;
}

export function chainAccountLoader(chain: ChainClient): AccountLoader {
  return {
    async getAccount(address: string) {
      const account = await chain.account(address);
      return account ? { data: account.data } : null;
    },
  };
}
