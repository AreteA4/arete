/**
 * Snapshot replay for handler tests (`@usearete/protocol/testing`).
 *
 * The platform records every `ctx` call a handler makes (method, canonical
 * arguments, result) as the provider snapshot. `createReplayContext` feeds a
 * recorded call log back to a handler and fails on the first call that
 * diverges in order, method or arguments, so a test proves the handler
 * makes exactly the recorded calls and builds from exactly those results.
 */

import { sha256 } from "@noble/hashes/sha256";
import { bytesToHex, canonicalizeJcs, type JsonValue } from "@usearete/hash";

import {
  invokeHandler,
  type AccountDecoder,
  type HostedPreparedOperationV1,
  type PrepareInputV1,
  type ProtocolContextV1,
  type ProtocolHandler,
  type TokenAccountV1,
} from "./abi.js";
import { validatePreparedOperation } from "./contracts/handler.js";
import { decodeOrRaw } from "./ctx-results.js";
import type { AccountReadResultV1, CtxMethod } from "./contracts/types.js";

/** One recorded `ctx` call. */
export interface RecordedCtxCallV1 {
  method: CtxMethod;
  /** Canonical arguments, as `canonicalCtxArgs` builds them. */
  args: JsonValue;
  /**
   * The recorded result: an `AccountReadResultV1` (or `null`) for
   * `solana.account`, a list of them for `solana.accounts`, a list of
   * `TokenAccountV1` for `solana.tokenAccountsByOwner`, and the view value
   * (or `null`) for `stacks.view`.
   */
  result: JsonValue;
  /** When present, the SHA-256 of the canonical result must match. */
  resultSha256?: string;
}

export class ReplayDivergenceError extends Error {
  readonly code: "ctx-replay-divergence" | "ctx-replay-unconsumed" | "ctx-replay-result-mismatch";
  readonly seq: number;

  constructor(code: ReplayDivergenceError["code"], seq: number, message: string) {
    super(message);
    this.name = "ReplayDivergenceError";
    this.code = code;
    this.seq = seq;
  }
}

/** The canonical arguments the platform records for each `ctx` method. */
export function canonicalCtxArgs(method: CtxMethod, args: readonly unknown[]): JsonValue {
  switch (method) {
    case "solana.account": {
      const [pubkey, , opts] = args as [string, unknown, { minContextSlot?: number } | undefined];
      return opts?.minContextSlot === undefined ? { pubkey } : { pubkey, minContextSlot: opts.minContextSlot };
    }
    case "solana.accounts":
      return { pubkeys: [...(args[0] as string[])] };
    case "solana.tokenAccountsByOwner":
      return { owner: args[0] as string, mints: [...(args[1] as string[])] };
    case "stacks.view":
      return { stack: args[0] as string, view: args[1] as string, key: args[2] as string };
  }
}

export function resultSha256(result: JsonValue): string {
  return bytesToHex(sha256(canonicalizeJcs(result)));
}

function canonicalText(value: JsonValue): string {
  return new TextDecoder().decode(canonicalizeJcs(value));
}

export interface ReplayContext extends ProtocolContextV1 {
  /** Calls replayed so far. */
  readonly consumed: number;
  /** Throws on any earlier replay failure or unconsumed recorded call. */
  assertConsumed(): void;
}

/** A `ctx` that replays `calls` in order and throws on any divergence. */
export function createReplayContext(calls: readonly RecordedCtxCallV1[]): ReplayContext {
  let seq = 0;
  let failure: unknown;

  const readNext = (method: CtxMethod, args: readonly unknown[]): JsonValue => {
    const actual = canonicalCtxArgs(method, args);
    const recorded = calls[seq];
    if (recorded === undefined) {
      throw new ReplayDivergenceError(
        "ctx-replay-divergence",
        seq,
        `call ${seq}: ${method}(${canonicalText(actual)}) was not recorded; the snapshot has ${calls.length} calls`,
      );
    }
    if (recorded.method !== method || canonicalText(recorded.args) !== canonicalText(actual)) {
      throw new ReplayDivergenceError(
        "ctx-replay-divergence",
        seq,
        `call ${seq}: expected ${recorded.method}(${canonicalText(recorded.args)}), got ${method}(${canonicalText(actual)})`,
      );
    }
    if (recorded.resultSha256 !== undefined && resultSha256(recorded.result) !== recorded.resultSha256) {
      throw new ReplayDivergenceError(
        "ctx-replay-result-mismatch",
        seq,
        `call ${seq}: the recorded result does not match its resultSha256`,
      );
    }
    seq += 1;
    return recorded.result;
  };

  const next = (method: CtxMethod, args: readonly unknown[]): JsonValue => {
    if (failure !== undefined) throw failure;
    try {
      return readNext(method, args);
    } catch (error) {
      failure = error;
      throw error;
    }
  };

  return {
    get consumed() {
      return seq;
    },
    assertConsumed() {
      if (failure !== undefined) throw failure;
      if (seq !== calls.length) {
        throw new ReplayDivergenceError(
          "ctx-replay-unconsumed",
          seq,
          `the handler made ${seq} of ${calls.length} recorded calls`,
        );
      }
    },
    solana: {
      async account<T>(pubkey: string, decoder?: AccountDecoder<T>, opts?: { minContextSlot?: number }) {
        const result = next("solana.account", [pubkey, decoder, opts]) as unknown as AccountReadResultV1 | null;
        return decodeOrRaw(result, decoder);
      },
      async accounts<T>(pubkeys: string[], decoder?: AccountDecoder<T>) {
        const results = next("solana.accounts", [pubkeys]) as unknown as (AccountReadResultV1 | null)[];
        return results.map((result) => decodeOrRaw(result, decoder));
      },
      async tokenAccountsByOwner(owner: string, mints: string[]) {
        return next("solana.tokenAccountsByOwner", [owner, mints]) as unknown as TokenAccountV1[];
      },
    },
    stacks: {
      async view<T>(stack: string, view: string, key: string) {
        return next("stacks.view", [stack, view, key]) as unknown as T | null;
      },
    },
  } as ReplayContext;
}

/**
 * Run a handler against a recorded call log. Fails if it diverges from the
 * log, leaves recorded calls unused, or returns an invalid operation.
 */
export async function runHandler<TArgs, TParams>(
  handler: ProtocolHandler<TArgs, TParams>,
  input: PrepareInputV1<TArgs, TParams>,
  recordedCalls: readonly RecordedCtxCallV1[],
): Promise<HostedPreparedOperationV1> {
  const ctx = createReplayContext(recordedCalls);
  const operation = await invokeHandler(handler, input, ctx);
  ctx.assertConsumed();
  return validatePreparedOperation(operation);
}
