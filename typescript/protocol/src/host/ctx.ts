/**
 * The `arete.protocol-ctx/v1` surface for one handler invocation, served by
 * calling back to the runtime that asked for the prepare:
 *
 *   POST {ctxUrl}/handler-host/v1/ctx
 *   Authorization: Bearer <invocationToken>
 *   {"seq": 0, "method": "solana.account", "args": {"pubkey": "…"}}
 *
 * `seq` starts at 0 for each invocation.
 * The runtime answers `{"result": …}` or `{"error": {"code", "message"}}`.
 * Arguments are the canonical ones the replay harness records, and results
 * reach the handler exactly as in replay.
 */

import type { AccountDecoder, ProtocolContextV1, TokenAccountV1 } from "../abi.js";
import { MAX_CTX_CALLS } from "../contracts/intent.js";
import type { AccountReadResultV1, CtxMethod } from "../contracts/types.js";
import { decodeOrRaw } from "../ctx-results.js";
import { canonicalCtxArgs } from "../testing.js";
import { isPlainObject } from "./errors.js";

/** A failed `ctx` call. Once one fails, the invocation fails whatever the handler does next. */
export class CtxCallError extends Error {
  readonly code = "ctx_error";
  readonly seq: number;

  constructor(seq: number, message: string) {
    super(message);
    this.name = "CtxCallError";
    this.seq = seq;
  }
}

export interface RemoteContextOptions {
  /** The full callback URL (`…/handler-host/v1/ctx`). */
  callbackUrl: string;
  invocationToken: string;
  signal: AbortSignal;
  fetch?: typeof fetch;
}

export interface RemoteContext {
  ctx: ProtocolContextV1;
  /** The first failed call, if any. */
  failure(): CtxCallError | undefined;
  /** Calls made so far. */
  calls(): number;
}

function isAccountResult(value: unknown): value is AccountReadResultV1 | null {
  return (
    value === null ||
    (isPlainObject(value) &&
      typeof value.address === "string" &&
      typeof value.exists === "boolean" &&
      typeof value.slot === "string")
  );
}

export function createRemoteContext(options: RemoteContextOptions): RemoteContext {
  const fetchImpl = options.fetch ?? fetch;
  let seq = 0;
  let failed: CtxCallError | undefined;

  const fail = (callSeq: number, message: string): never => {
    failed ??= new CtxCallError(callSeq, message);
    throw failed;
  };

  const call = async (method: CtxMethod, rawArgs: readonly unknown[]): Promise<{ seq: number; result: unknown }> => {
    if (failed !== undefined) throw failed;
    const callSeq = seq;
    if (options.signal.aborted) fail(callSeq, `call ${callSeq}: the invocation has ended`);
    if (callSeq >= MAX_CTX_CALLS) fail(callSeq, `call ${callSeq}: at most ${MAX_CTX_CALLS} ctx calls are allowed`);
    seq += 1;
    const body = JSON.stringify({ seq: callSeq, method, args: canonicalCtxArgs(method, rawArgs) });
    let text: string;
    let status: number;
    try {
      const response = await fetchImpl(options.callbackUrl, {
        method: "POST",
        headers: { authorization: `Bearer ${options.invocationToken}`, "content-type": "application/json" },
        body,
        signal: options.signal,
      });
      status = response.status;
      text = await response.text();
    } catch (error) {
      return fail(callSeq, `call ${callSeq} ${method}: ${error instanceof Error ? error.message : String(error)}`);
    }
    if (failed !== undefined) throw failed;
    let reply: unknown;
    try {
      reply = JSON.parse(text);
    } catch {
      return fail(callSeq, `call ${callSeq} ${method}: the runtime answered ${status} with a non-JSON body`);
    }
    if (isPlainObject(reply) && isPlainObject(reply.error) && Object.keys(reply).length === 1) {
      const code = typeof reply.error.code === "string" ? reply.error.code : "unknown";
      const message = typeof reply.error.message === "string" ? reply.error.message : "";
      return fail(callSeq, `call ${callSeq} ${method}: ${code}: ${message}`);
    }
    if (status < 200 || status >= 300 || !isPlainObject(reply) || Object.keys(reply).length !== 1 || !("result" in reply)) {
      return fail(callSeq, `call ${callSeq} ${method}: the runtime answered ${status} without a result`);
    }
    return { seq: callSeq, result: reply.result };
  };

  const malformed = (callSeq: number, method: CtxMethod): never =>
    fail(callSeq, `call ${callSeq} ${method}: the runtime returned a malformed result`);

  const ctx: ProtocolContextV1 = {
    solana: {
      async account<T>(pubkey: string, decoder?: AccountDecoder<T>, opts?: { minContextSlot?: number }) {
        const { seq: callSeq, result } = await call("solana.account", [pubkey, decoder, opts]);
        if (!isAccountResult(result)) return malformed(callSeq, "solana.account");
        return decodeOrRaw(result, decoder);
      },
      async accounts<T>(pubkeys: string[], decoder?: AccountDecoder<T>) {
        const { seq: callSeq, result: results } = await call("solana.accounts", [pubkeys]);
        if (!Array.isArray(results) || results.length !== pubkeys.length || !results.every(isAccountResult)) {
          return malformed(callSeq, "solana.accounts");
        }
        return (results as (AccountReadResultV1 | null)[]).map((result) => decodeOrRaw(result, decoder));
      },
      async tokenAccountsByOwner(owner: string, mints: string[]) {
        const { seq: callSeq, result: results } = await call("solana.tokenAccountsByOwner", [owner, mints]);
        if (!Array.isArray(results)) return malformed(callSeq, "solana.tokenAccountsByOwner");
        return results as TokenAccountV1[];
      },
    },
    stacks: {
      async view<T>(stack: string, view: string, key: string) {
        return (await call("stacks.view", [stack, view, key])).result as T | null;
      },
    },
  };

  return { ctx, failure: () => failed, calls: () => seq };
}
