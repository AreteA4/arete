import type {
  ChainClient,
  ContextSlotOptions,
  NativeBalanceInfo,
  TokenBalanceInfo,
  TokenBalanceInput,
} from '@usearete/sdk';
import {
  useAsyncRead,
  type UseAsyncReadOptions,
  type UseAsyncReadResult,
} from './hooks/use-async-read';

export interface BalanceHookOptions<T> extends ContextSlotOptions, UseAsyncReadOptions<T> {}

const chainIds = new WeakMap<object, number>();
let nextChainId = 1;

function chainKey(chain: ChainClient | null | undefined): number | null {
  if (!chain) {
    return null;
  }
  const existing = chainIds.get(chain);
  if (existing !== undefined) {
    return existing;
  }
  const id = nextChainId++;
  chainIds.set(chain, id);
  return id;
}

function abortError(): Error {
  const error = new Error('Chain read was aborted');
  error.name = 'AbortError';
  return error;
}

export function useNativeBalance(
  chain: ChainClient | null | undefined,
  address: string | null | undefined,
  options: BalanceHookOptions<NativeBalanceInfo> = {}
): UseAsyncReadResult<NativeBalanceInfo> {
  const enabled = (options.enabled ?? true) && Boolean(chain && address);
  const minContextSlot = options.minContextSlot;
  return useAsyncRead(
    ['native-balance', chainKey(chain), address, minContextSlot?.toString()],
    async ({ signal }) => {
      if (!chain || !address) {
        throw new Error('Chain client and address are required for a native balance read');
      }
      if (signal.aborted) throw abortError();
      const result = await chain.nativeBalance(address, { minContextSlot });
      if (signal.aborted) throw abortError();
      return result;
    },
    { enabled, initialData: options.initialData }
  );
}

export function useTokenBalance(
  chain: ChainClient | null | undefined,
  input: TokenBalanceInput | null | undefined,
  options: BalanceHookOptions<TokenBalanceInfo> = {}
): UseAsyncReadResult<TokenBalanceInfo> {
  const enabled = (options.enabled ?? true) && Boolean(chain && input);
  const minContextSlot = options.minContextSlot;
  return useAsyncRead(
    [
      'token-balance',
      chainKey(chain),
      input?.owner,
      input?.mint,
      input?.tokenProgram,
      minContextSlot?.toString(),
    ],
    async ({ signal }) => {
      if (!chain || !input) {
        throw new Error('Chain client and token input are required for a token balance read');
      }
      if (signal.aborted) throw abortError();
      const result = await chain.balance(input, { minContextSlot });
      if (signal.aborted) throw abortError();
      return result;
    },
    { enabled, initialData: options.initialData }
  );
}
