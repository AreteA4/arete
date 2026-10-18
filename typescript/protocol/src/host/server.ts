/**
 * `arete-protocol serve-handlers`: a local handler host that Arete
 * cloud dev environments call to run a protocol's reviewed handler
 * bundle during development, over the handler host protocol:
 *
 *   GET    /handler-host/v1/inventory
 *     -> {build:{name,version}, releases:[{releaseHash,specHash,bundleHash}]},
 *        sorted by releaseHash. The host holds at most MAX_HOST_RELEASES.
 *   PUT    /handler-host/v1/releases/{releaseHash}
 *     <- {release, spec, stackSource, receipt, review, bundleBase64 | null}
 *     -> {releaseHash}. Repeating a PUT is safe.
 *   DELETE /handler-host/v1/releases/{releaseHash}
 *     -> 204, whether or not the release was loaded.
 *   POST   /handler-host/v1/prepare
 *     <- {releaseHash, step, invocationToken, input}, where input is a
 *        PrepareInputV1 whose step equals step
 *     -> {ok:true, operation} or {ok:false, error:{code,message}}
 *
 * Bodies are camelCase JSON and request objects reject unknown fields. Every
 * request carries `x-arete-handler-host-token`, compared in constant time;
 * without it the host answers 401. Refusals are `{"error":{"code","message"}}`.
 * Release bodies are capped at 64 MiB, prepare bodies at 8 MiB.
 *
 * On a PUT the host trusts nothing in the body: it validates every contract
 * and recomputes the spec, stack source, release and bundle hashes; the
 * release hash must equal the path, the receipt's outputs must match, and the
 * `arete.protocol-release-review/v1` signature must verify against a trusted
 * review key before the bundle is imported. Refusal codes: invalid_request,
 * contract_invalid, release_hash_mismatch, spec_hash_mismatch,
 * stack_source_hash_mismatch, receipt_mismatch, review_key_unknown,
 * review_invalid, bundle_missing, bundle_unexpected, bundle_hash_mismatch,
 * bundle_load_failed, bundle_export_missing, release_limit_reached.
 *
 * A prepare runs the export named by the step's `build: {kind: "handler",
 * export}` (see prepare.ts and ctx.ts for the ctx callback). Prepare error
 * codes: release_not_loaded, step_not_handler, handler_threw,
 * handler_result_invalid, ctx_error, timeout.
 *
 * Handlers run with the same ABI, the same `ctx` methods and the same result
 * validation as a production host; `ctx` calls go back to the runtime's
 * callback URL. Removing a release drops the host's reference to it, but Node
 * keeps imported modules cached until the process exits.
 */

import { createHash, timingSafeEqual } from "node:crypto";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { isIP, type AddressInfo } from "node:net";

import type { PrepareInputV1 } from "../abi.js";
import { ProtocolContractError } from "../contracts/errors.js";
import { parseContractJson } from "../contracts/json.js";
import { compareUtf8, decodeBase58 } from "../contracts/shape.js";
import { keyId } from "../contracts/release.js";
import { PROTOCOL_HANDLER_ABI_V1 } from "../contracts/types.js";
import { PACKAGE_NAME, PACKAGE_VERSION } from "../version.js";
import { HostError, exactFields, isPlainObject } from "./errors.js";
import { runPrepare } from "./prepare.js";
import { BundleLoader, verifyReleaseRequest, type LoadedRelease, type ReviewKeys } from "./releases.js";

export const HANDLER_HOST_READY_SCHEMA = "arete.handler-host-ready/v1";
export const HANDLER_HOST_TOKEN_HEADER = "x-arete-handler-host-token";
/** Prefix of every handler host route. */
export const HANDLER_HOST_ROUTE_PREFIX = "/handler-host/v1";
/** The runtime route a handler `ctx` call is posted to, under `ctxUrl`. */
export const HANDLER_HOST_CTX_PATH = `${HANDLER_HOST_ROUTE_PREFIX}/ctx`;
export const MAX_HOST_RELEASES = 4096;
export const DEFAULT_PREPARE_TIMEOUT_MS = 10_000;

const MAX_RELEASE_BODY_BYTES = 64 * 1024 * 1024;
const MAX_PREPARE_BODY_BYTES = 8 * 1024 * 1024;
const INVENTORY_PATH = `${HANDLER_HOST_ROUTE_PREFIX}/inventory`;
const PREPARE_PATH = `${HANDLER_HOST_ROUTE_PREFIX}/prepare`;
const RELEASE_PATH = /^\/handler-host\/v1\/releases\/([^/]+)$/;

export type HostLogLevel = "debug" | "info" | "warn" | "error";
export type HostLogger = (level: HostLogLevel, message: string, fields?: Record<string, unknown>) => void;

export interface HandlerHostOptions {
  /** The shared secret every request must carry in `x-arete-handler-host-token`. */
  token: string;
  /** Trusted review keys: key id → 32-byte Ed25519 public key. */
  reviewKeys: ReviewKeys;
  /** The runtime's base URL; ctx calls go to `{ctxUrl}/handler-host/v1/ctx`. */
  ctxUrl: string;
  /** Loopback address to bind (default `127.0.0.1`). */
  host?: string;
  /** Port to bind (default `0`, any free port). */
  port?: number;
  /** Per-invocation timeout (default 10 s). */
  timeoutMs?: number;
  log?: HostLogger;
  /** Overrides the `fetch` used for ctx callbacks. */
  fetch?: typeof fetch;
}

export interface HandlerHost {
  /** `http://<host>:<port>` */
  readonly url: string;
  readonly server: Server;
  /** Stop accepting requests, let in-flight ones finish (up to `graceMs`, default 5 s), then drop every release. */
  close(graceMs?: number): Promise<void>;
}

/** True for `localhost`, `127.0.0.0/8` and `::1`. */
export function isLoopbackHost(host: string): boolean {
  const bare = host.startsWith("[") && host.endsWith("]") ? host.slice(1, -1) : host;
  if (bare === "localhost") return true;
  if (isIP(bare) === 4) return bare.startsWith("127.");
  if (isIP(bare) === 6) return bare === "::1" || bare === "0:0:0:0:0:0:0:1";
  return false;
}

/** Parse `host:port` (or `[v6]:port`), refusing anything that is not loopback. */
export function parseListenAddress(text: string): { host: string; port: number } {
  const match = /^(\[[^\]]+\]|[^:]+):(\d{1,5})$/.exec(text);
  if (match === null) throw new Error(`--listen expects host:port, got '${text}'`);
  const host = match[1]!;
  const port = Number(match[2]);
  if (port > 65535) throw new Error(`--listen port ${port} is out of range`);
  if (!isLoopbackHost(host)) throw new Error(`--listen must be a loopback address, got '${host}'`);
  const bare = host.startsWith("[") ? host.slice(1, -1) : host;
  return { host: bare === "localhost" ? "127.0.0.1" : bare, port };
}

/** Parse a `--review-key <keyId>=<base58 pubkey>` value. */
export function parseReviewKey(text: string): [string, Uint8Array] {
  const at = text.indexOf("=");
  if (at <= 0) throw new Error(`--review-key expects <keyId>=<base58 pubkey>, got '${text}'`);
  const id = text.slice(0, at);
  try {
    keyId(id, "");
  } catch {
    throw new Error(`--review-key id '${id}' must match ^[a-z0-9][a-z0-9-]{0,63}$`);
  }
  const key = decodeBase58(text.slice(at + 1));
  if (key === undefined || key.length !== 32) throw new Error(`--review-key '${id}' is not a base58 32-byte public key`);
  return [id, key];
}

/** The callback URL for a runtime base URL. */
export function ctxCallbackUrl(ctxUrl: string): string {
  const url = new URL(ctxUrl);
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error(`--ctx-url must be http(s), got '${ctxUrl}'`);
  return `${url.href.replace(/\/+$/, "")}${HANDLER_HOST_CTX_PATH}`;
}

function digest(text: string): Buffer {
  return createHash("sha256").update(text, "utf8").digest();
}

function send(response: ServerResponse, status: number, body?: unknown): void {
  if (body === undefined) {
    response.writeHead(status);
    response.end();
    return;
  }
  const bytes = Buffer.from(JSON.stringify(body), "utf8");
  response.writeHead(status, { "content-type": "application/json", "content-length": bytes.length });
  response.end(bytes);
}

function readBody(request: IncomingMessage, limit: number): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let size = 0;
    let rejected = false;
    request.on("data", (chunk: Buffer) => {
      if (rejected) return;
      size += chunk.length;
      if (size > limit) {
        rejected = true;
        reject(new HostError(413, "payload_too_large", `request bodies are at most ${limit} bytes`));
        return;
      }
      chunks.push(chunk);
    });
    request.on("end", () => {
      if (!rejected) resolve(Buffer.concat(chunks));
    });
    request.on("error", (error) => {
      if (!rejected) reject(error);
    });
  });
}

function parseStrict(bytes: Buffer): unknown {
  try {
    return parseContractJson(new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength));
  } catch (error) {
    if (error instanceof ProtocolContractError) throw new HostError(400, "invalid_request", `invalid JSON: ${error.toString()}`);
    throw error;
  }
}

function parseLoose(bytes: Buffer): unknown {
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch {
    throw new HostError(400, "invalid_request", "the body is not valid JSON");
  }
}

function nonEmptyString(value: unknown, field: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new HostError(400, "invalid_request", `${field} must be a non-empty string`);
  }
  return value;
}

/** Start a handler host. Resolves once it is listening. */
export async function startHandlerHost(options: HandlerHostOptions): Promise<HandlerHost> {
  if (options.token.length === 0) throw new Error("the handler host token must not be empty");
  const host = options.host ?? "127.0.0.1";
  if (!isLoopbackHost(host)) throw new Error(`the handler host binds only to loopback, not '${host}'`);
  const callbackUrl = ctxCallbackUrl(options.ctxUrl);
  const timeoutMs = options.timeoutMs ?? DEFAULT_PREPARE_TIMEOUT_MS;
  const log: HostLogger = options.log ?? (() => {});
  const expectedToken = digest(options.token);
  const releases = new Map<string, LoadedRelease>();
  const loader = new BundleLoader();
  // Serialises registrations so concurrent PUTs of one release load it once.
  let registering: Promise<unknown> = Promise.resolve();

  const authorized = (request: IncomingMessage): boolean => {
    const header = request.headers[HANDLER_HOST_TOKEN_HEADER];
    if (typeof header !== "string") return false;
    return timingSafeEqual(digest(header), expectedToken);
  };

  const inventory = () => ({
    build: { name: PACKAGE_NAME, version: PACKAGE_VERSION },
    releases: [...releases.values()]
      .map((release) => ({ releaseHash: release.releaseHash, specHash: release.specHash, bundleHash: release.bundleHash }))
      .sort((a, b) => compareUtf8(a.releaseHash, b.releaseHash)),
  });

  const putRelease = async (releaseHash: string, request: IncomingMessage) => {
    const body = parseStrict(await readBody(request, MAX_RELEASE_BODY_BYTES));
    const verified = verifyReleaseRequest(releaseHash, body, options.reviewKeys);
    const work = registering.then(async () => {
      if (releases.has(verified.releaseHash)) return { loaded: false };
      if (releases.size >= MAX_HOST_RELEASES) {
        throw new HostError(409, "release_limit_reached", `this host holds at most ${MAX_HOST_RELEASES} releases`);
      }
      releases.set(verified.releaseHash, await loader.load(verified));
      return { loaded: true };
    });
    registering = work.catch(() => undefined);
    const { loaded } = await work;
    log("info", loaded ? "release loaded" : "release already loaded", {
      releaseHash: verified.releaseHash,
      bundleHash: verified.bundleHash,
    });
    return { releaseHash: verified.releaseHash };
  };

  const prepare = async (request: IncomingMessage) => {
    const body = exactFields(
      parseLoose(await readBody(request, MAX_PREPARE_BODY_BYTES)),
      "the request",
      ["releaseHash", "step", "invocationToken", "input"],
    );
    const releaseHash = nonEmptyString(body.releaseHash, "releaseHash");
    const step = nonEmptyString(body.step, "step");
    const invocationToken = nonEmptyString(body.invocationToken, "invocationToken");
    const input = body.input;
    if (!isPlainObject(input)) throw new HostError(400, "invalid_request", "input must be a PrepareInputV1 object");
    if (input.abi !== PROTOCOL_HANDLER_ABI_V1) {
      throw new HostError(400, "invalid_request", `input.abi must be '${PROTOCOL_HANDLER_ABI_V1}'`);
    }
    if (input.step !== step) throw new HostError(400, "invalid_request", "input.step must equal step");
    const started = Date.now();
    const result = await runPrepare(
      { release: releases.get(releaseHash), step, invocationToken, input: input as unknown as PrepareInputV1 },
      { callbackUrl, timeoutMs, ...(options.fetch ? { fetch: options.fetch } : {}) },
    );
    log(result.ok ? "info" : "warn", "prepare", {
      releaseHash,
      step,
      ok: result.ok,
      ...(result.ok ? {} : { code: result.error.code, error: result.error.message }),
      durationMs: Date.now() - started,
    });
    return result;
  };

  const handle = async (request: IncomingMessage, response: ServerResponse): Promise<number> => {
    if (!authorized(request)) {
      send(response, 401, { error: { code: "unauthorized", message: `missing or wrong ${HANDLER_HOST_TOKEN_HEADER}` } });
      return 401;
    }
    const path = (request.url ?? "/").split("?")[0]!;
    const method = request.method ?? "GET";
    if (path === INVENTORY_PATH) {
      if (method !== "GET") throw new HostError(405, "method_not_allowed", `${method} is not allowed on ${path}`);
      send(response, 200, inventory());
      return 200;
    }
    if (path === PREPARE_PATH) {
      if (method !== "POST") throw new HostError(405, "method_not_allowed", `${method} is not allowed on ${path}`);
      send(response, 200, await prepare(request));
      return 200;
    }
    const match = RELEASE_PATH.exec(path);
    if (match !== null) {
      let releaseHash: string;
      try {
        releaseHash = decodeURIComponent(match[1]!);
      } catch {
        throw new HostError(400, "invalid_request", "the release hash is not valid percent-encoding");
      }
      if (method === "PUT") {
        send(response, 200, await putRelease(releaseHash, request));
        return 200;
      }
      if (method === "DELETE") {
        if (releases.delete(releaseHash)) log("info", "release removed", { releaseHash });
        send(response, 204);
        return 204;
      }
      throw new HostError(405, "method_not_allowed", `${method} is not allowed on ${path}`);
    }
    throw new HostError(404, "not_found", `no route for ${path}`);
  };

  const server = createServer((request, response) => {
    const started = Date.now();
    handle(request, response).then(
      (status) => {
        log("debug", "request", { method: request.method, path: request.url, status, durationMs: Date.now() - started });
      },
      (error: unknown) => {
        const failure =
          error instanceof HostError
            ? error
            : new HostError(500, "internal", error instanceof Error ? error.message : String(error));
        if (failure.status >= 500) log("error", "request failed", { path: request.url, error: failure.message });
        else log("warn", "request refused", { method: request.method, path: request.url, code: failure.code, error: failure.message });
        if (!response.headersSent) send(response, failure.status, { error: { code: failure.code, message: failure.message } });
        // Drop any unread body (for example after a 413).
        request.resume();
      },
    );
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(options.port ?? 0, host, () => {
      server.off("error", reject);
      resolve();
    });
  });
  const address = server.address() as AddressInfo;
  const url = `http://${address.family === "IPv6" || address.address.includes(":") ? `[${address.address}]` : address.address}:${address.port}`;

  return {
    url,
    server,
    close: (graceMs = 5_000) =>
      new Promise<void>((resolve) => {
        const connections = server as Server & { closeAllConnections?: () => void; closeIdleConnections?: () => void };
        const force = setTimeout(() => connections.closeAllConnections?.(), graceMs);
        server.close(() => {
          clearTimeout(force);
          loader.close();
          releases.clear();
          resolve();
        });
        // In-flight requests finish; idle keep-alive sockets close now.
        connections.closeIdleConnections?.();
      }),
  };
}
