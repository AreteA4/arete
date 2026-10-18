/**
 * One `POST /handler-host/v1/prepare`: run a loaded release's handler for a
 * step with a callback `ctx`, under a timeout, and validate its result
 * exactly as the platform does (size bound, strict JSON, then shape).
 */

import { invokeHandler, type PrepareInputV1, type ProtocolHandler } from "../abi.js";
import { ProtocolContractError } from "../contracts/errors.js";
import { parsePreparedOperation } from "../contracts/handler.js";
import type { PreparedOperationV1 } from "../contracts/types.js";
import { createRemoteContext } from "./ctx.js";
import type { LoadedRelease } from "./releases.js";

export const PREPARE_ERROR_CODES = [
  "release_not_loaded",
  "step_not_handler",
  "handler_threw",
  "handler_result_invalid",
  "ctx_error",
  "timeout",
] as const;

export type PrepareErrorCode = (typeof PREPARE_ERROR_CODES)[number];

export type PrepareResponse =
  | { ok: true; operation: PreparedOperationV1 }
  | { ok: false; error: { code: PrepareErrorCode; message: string } };

export interface PrepareOptions {
  callbackUrl: string;
  timeoutMs: number;
  fetch?: typeof fetch;
}

export interface PrepareRequest {
  release: LoadedRelease | undefined;
  step: string;
  invocationToken: string;
  input: PrepareInputV1;
}

const encoder = new TextEncoder();

function failure(code: PrepareErrorCode, message: string): PrepareResponse {
  return { ok: false, error: { code, message } };
}

function describe(error: unknown): string {
  if (error instanceof Error) return error.message === "" ? error.name : `${error.name}: ${error.message}`;
  return String(error);
}

export async function runPrepare(request: PrepareRequest, options: PrepareOptions): Promise<PrepareResponse> {
  const { release, step } = request;
  if (release === undefined) return failure("release_not_loaded", "the release is not loaded on this host");
  const definition = Object.prototype.hasOwnProperty.call(release.spec.steps, step) ? release.spec.steps[step] : undefined;
  if (definition === undefined || definition.build.kind !== "handler") {
    return failure("step_not_handler", `step '${step}' has no handler in this release`);
  }
  const handler = release.exports?.[definition.build.export] as ProtocolHandler | undefined;
  if (handler === undefined) return failure("step_not_handler", `the bundle has no export '${definition.build.export}'`);

  const controller = new AbortController();
  const remote = createRemoteContext({
    callbackUrl: options.callbackUrl,
    invocationToken: request.invocationToken,
    signal: controller.signal,
    ...(options.fetch ? { fetch: options.fetch } : {}),
  });

  let timer: ReturnType<typeof setTimeout> | undefined;
  const timedOut = new Promise<{ timeout: true }>((resolve) => {
    timer = setTimeout(() => resolve({ timeout: true }), options.timeoutMs);
  });
  const run = (async () => {
    try {
      return { value: (await invokeHandler(handler, request.input, remote.ctx)) as unknown };
    } catch (error) {
      return { thrown: error };
    }
  })();
  const outcome = await Promise.race([run, timedOut]);
  clearTimeout(timer);
  // Ends any ctx call still in flight; later calls fail.
  controller.abort();

  if ("timeout" in outcome) {
    return failure("timeout", `the handler did not finish within ${options.timeoutMs} ms`);
  }
  // A failed ctx call fails the invocation even if the handler caught it.
  const ctxFailure = remote.failure();
  if (ctxFailure !== undefined) return failure("ctx_error", ctxFailure.message);
  if ("thrown" in outcome) return failure("handler_threw", describe(outcome.thrown));

  let bytes: Uint8Array;
  try {
    const text = JSON.stringify(outcome.value);
    if (text === undefined) return failure("handler_result_invalid", "the handler returned no value");
    bytes = encoder.encode(text);
  } catch (error) {
    return failure("handler_result_invalid", `the handler result is not JSON: ${describe(error)}`);
  }
  try {
    return { ok: true, operation: parsePreparedOperation(bytes) };
  } catch (error) {
    if (error instanceof ProtocolContractError) return failure("handler_result_invalid", error.toString());
    throw error;
  }
}
