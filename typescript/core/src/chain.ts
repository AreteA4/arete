export interface ChainClock {
  slot: number;
  epoch?: number;
  leaderScheduleEpoch?: number;
  unixTimestamp: number;
}

export interface MintAccountInfo {
  address: string;
  ownerProgram: string;
  decimals: number | null;
  supply: string | null;
  mintAuthority: string | null;
  freezeAuthority: string | null;
}

export interface TokenAccountInfo {
  address: string;
  ownerProgram: string;
  mint: string | null;
  owner: string | null;
  amount: string | null;
  uiAmountString: string | null;
}

export interface TokenBalanceInfo {
  exists: boolean;
  address: string | null;
  owner: string;
  mint: string;
  tokenProgram?: string;
  amount: string;
  decimals?: number | null;
  uiAmountString?: string | null;
  contextSlot: bigint;
}

export interface NativeBalanceInfo {
  lamports: bigint;
  contextSlot: bigint;
}

export interface ContextSlotOptions {
  minContextSlot?: number | bigint;
}

export interface TokenBalanceInput {
  owner: string;
  mint: string;
  tokenProgram?: string;
}

export interface RawAccountInfo {
  address: string;
  ownerProgram: string;
  lamports: bigint;
  executable: boolean;
  data: Uint8Array;
}

export interface ChainClient {
  exists(address: string): Promise<boolean>;
  lamports(address: string): Promise<number>;
  nativeBalance(address: string, options?: ContextSlotOptions): Promise<NativeBalanceInfo>;
  minimumBalanceForRentExemption(space: number): Promise<number>;
  clock(): Promise<ChainClock>;
  account(address: string): Promise<RawAccountInfo | null>;
  accounts(addresses: readonly string[]): Promise<(RawAccountInfo | null)[]>;
  mint(address: string): Promise<MintAccountInfo | null>;
  tokenAccount(address: string): Promise<TokenAccountInfo | null>;
  balance(input: TokenBalanceInput, options?: ContextSlotOptions): Promise<TokenBalanceInfo>;
}

type FetchLike = typeof fetch;

function joinUrl(baseUrl: string, path: string): string {
  return `${baseUrl.replace(/\/$/, '')}${path.startsWith('/') ? path : `/${path}`}`;
}

function decodeBase64(encoded: string): Uint8Array {
  if (typeof atob === 'function') {
    const binary = atob(encoded);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    return bytes;
  }
  const bufferCtor = (globalThis as { Buffer?: { from(input: string, encoding: string): Uint8Array } }).Buffer;
  if (bufferCtor) {
    return new Uint8Array(bufferCtor.from(encoded, 'base64'));
  }
  throw new Error('No base64 decoder available in this environment');
}

const MAX_BATCH_ADDRESSES = 100;

interface RawAccountBody {
  address: string;
  ownerProgram: string;
  /** Decimal string, not a number: a balance above 2^53 would be rounded by `JSON.parse`. */
  lamports: string;
  executable: boolean;
  data: string;
}

function toRawAccount(body: RawAccountBody | null): RawAccountInfo | null {
  if (!body) {
    return null;
  }
  return {
    address: body.address,
    ownerProgram: body.ownerProgram,
    lamports: decimalU64(body.lamports, 'lamports'),
    executable: body.executable,
    data: decodeBase64(body.data),
  };
}

function decimalU64(value: string, field: string): bigint {
  if (!/^\d+$/.test(value)) {
    throw new TypeError(`${field} must be a decimal u64 string`);
  }
  const parsed = BigInt(value);
  if (parsed > 18_446_744_073_709_551_615n) {
    throw new RangeError(`${field} exceeds u64`);
  }
  return parsed;
}

function serializeContextSlot(value: number | bigint): string {
  if (typeof value === 'number') {
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new RangeError('minContextSlot must be a non-negative safe integer or bigint');
    }
    return String(value);
  }
  if (value < 0n || value > 18_446_744_073_709_551_615n) {
    throw new RangeError('minContextSlot must fit in u64');
  }
  return value.toString();
}

function withContextSlot<T extends object>(input: T, options?: ContextSlotOptions): T & {
  minContextSlot?: string;
} {
  if (options?.minContextSlot === undefined) {
    return input;
  }
  return {
    ...input,
    minContextSlot: serializeContextSlot(options.minContextSlot),
  };
}

export function deriveHttpEndpoint(wsUrl: string): string {
  try {
    const parsed = new URL(wsUrl);
    if (parsed.protocol === 'ws:') {
      parsed.protocol = 'http:';
    } else if (parsed.protocol === 'wss:') {
      parsed.protocol = 'https:';
    }
    return parsed.toString().replace(/\/$/, '');
  } catch {
    return wsUrl.replace(/^wss?:/i, (protocol) => protocol.toLowerCase() === 'wss:' ? 'https:' : 'http:');
  }
}

export function createChainClient(httpBaseUrl: string, fetchImpl: FetchLike): ChainClient {
  return {
    async exists(address: string): Promise<boolean> {
      const path = `/chain/exists/${encodeURIComponent(address)}`;
      const response = await fetchImpl(joinUrl(httpBaseUrl, path));
      const body = await parseReadResponse<{ exists: boolean }>(response, path);
      return body.exists;
    },

    async lamports(address: string): Promise<number> {
      const path = `/chain/lamports/${encodeURIComponent(address)}`;
      const response = await fetchImpl(joinUrl(httpBaseUrl, path));
      const body = await parseReadResponse<{ lamports: number }>(response, path);
      return body.lamports;
    },

    async nativeBalance(
      address: string,
      options?: ContextSlotOptions
    ): Promise<NativeBalanceInfo> {
      const path = '/chain/native-balance';
      const response = await fetchImpl(joinUrl(httpBaseUrl, path), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(withContextSlot({ address }, options)),
      });
      const body = await parseReadResponse<{ lamports: string; contextSlot: string }>(
        response,
        path
      );
      return {
        lamports: decimalU64(body.lamports, 'lamports'),
        contextSlot: decimalU64(body.contextSlot, 'contextSlot'),
      };
    },

    async minimumBalanceForRentExemption(space: number): Promise<number> {
      const path = `/chain/rent-exemption/${encodeURIComponent(String(space))}`;
      const response = await fetchImpl(
        joinUrl(httpBaseUrl, path)
      );
      const body = await parseReadResponse<{ lamports: number }>(response, path);
      return body.lamports;
    },

    async clock(): Promise<ChainClock> {
      const path = '/chain/clock';
      const response = await fetchImpl(joinUrl(httpBaseUrl, path));
      return parseReadResponse<ChainClock>(response, path);
    },

    async account(address: string): Promise<RawAccountInfo | null> {
      const path = `/chain/accounts/${encodeURIComponent(address)}`;
      const response = await fetchImpl(joinUrl(httpBaseUrl, path));
      return toRawAccount(await parseReadResponse<RawAccountBody | null>(response, path));
    },

    async accounts(addresses: readonly string[]): Promise<(RawAccountInfo | null)[]> {
      // Copied before anything else: `readonly string[]` accepts a mutable array, so a caller can
      // splice it while the request is in flight and the cardinality check below would then compare
      // the response against a list that is no longer what was asked for. The Python client copies
      // for the same reason.
      const requested = [...addresses];
      if (requested.length > MAX_BATCH_ADDRESSES) {
        throw new RangeError(
          `addresses exceeds the ${MAX_BATCH_ADDRESSES}-address limit for one batch`
        );
      }
      if (requested.length === 0) {
        return [];
      }
      const path = '/chain/accounts';
      const response = await fetchImpl(joinUrl(httpBaseUrl, path), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ addresses: requested }),
      });
      const body = await parseReadResponse<{ items: (RawAccountBody | null)[] }>(response, path);
      // A different count would shift every later account onto the wrong address.
      if (body.items.length !== requested.length) {
        throw new TypeError(
          `Invalid chain response for '${path}': expected ${requested.length} items, got ${body.items.length}`
        );
      }
      // Positionally aligned with `requested`; absent accounts arrive as null.
      return body.items.map(toRawAccount);
    },

    async mint(address: string): Promise<MintAccountInfo | null> {
      const path = `/chain/mints/${encodeURIComponent(address)}`;
      const response = await fetchImpl(joinUrl(httpBaseUrl, path));
      return parseReadResponse<MintAccountInfo | null>(response, path);
    },

    async tokenAccount(address: string): Promise<TokenAccountInfo | null> {
      const path = `/chain/token-accounts/${encodeURIComponent(address)}`;
      const response = await fetchImpl(joinUrl(httpBaseUrl, path));
      return parseReadResponse<TokenAccountInfo | null>(response, path);
    },

    async balance(
      input: TokenBalanceInput,
      options?: ContextSlotOptions
    ): Promise<TokenBalanceInfo> {
      const path = '/chain/balances';
      const response = await fetchImpl(joinUrl(httpBaseUrl, path), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(withContextSlot(input, options)),
      });
      const body = await parseReadResponse<
        Omit<TokenBalanceInfo, 'contextSlot'> & { contextSlot: string }
      >(response, path);
      return {
        ...body,
        contextSlot: decimalU64(body.contextSlot, 'contextSlot'),
      };
    },
  };
}
import { parseReadResponse } from './read';
