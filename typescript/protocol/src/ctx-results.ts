/**
 * Turning `ctx` wire results into what a handler sees. Shared by the replay
 * context (`@usearete/protocol/testing`) and the local handler host so both
 * hand handlers exactly the same values.
 */

import { decodeAccount, type AccountDecoder, type RawAccountV1 } from "./abi.js";
import { decodeBase64 } from "./contracts/shape.js";
import type { AccountReadResultV1 } from "./contracts/types.js";

/** An `AccountReadResultV1` as a handler sees it: `null` when the account does not exist. */
export function toRawAccount(result: AccountReadResultV1 | null): RawAccountV1 | null {
  if (result === null || !result.exists) return null;
  return {
    address: result.address,
    lamports: result.lamports ?? "0",
    owner: result.owner ?? "",
    data: decodeBase64(result.dataBase64 ?? "") ?? new Uint8Array(),
    slot: result.slot,
  };
}

/** The raw account, or the decoder's output when the handler passed one. */
export function decodeOrRaw<T>(result: AccountReadResultV1 | null, decoder?: AccountDecoder<T>): T | null {
  const raw = toRawAccount(result);
  if (raw === null) return null;
  return decoder === undefined ? (raw as unknown as T) : decodeAccount(decoder, raw);
}
