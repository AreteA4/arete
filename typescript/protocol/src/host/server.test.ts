import { createServer, type IncomingMessage, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { ed25519 } from "@noble/curves/ed25519";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";

import { buildProtocol } from "../build/build.js";
import { reviewSigningMessage } from "../contracts/release.js";
import { encodeBase64 } from "../contracts/shape.js";
import { PROTOCOL_HANDLER_ABI_V1 } from "../contracts/types.js";
import type { PrepareInputV1 } from "../abi.js";
import { PACKAGE_NAME, PACKAGE_VERSION } from "../version.js";
import {
  HANDLER_HOST_TOKEN_HEADER,
  isLoopbackHost,
  parseListenAddress,
  parseReviewKey,
  startHandlerHost,
  type HandlerHost,
} from "./server.js";

const here = dirname(fileURLToPath(import.meta.url));
const TOKEN = "host-token-for-tests";
const REVIEW_KEY_ID = "test-review-1";
const REVIEW_SEED = new Uint8Array(32).fill(7);
const OTHER_SEED = new Uint8Array(32).fill(9);
const decoder = new TextDecoder();

const BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
function encodeBase58(bytes: Uint8Array): string {
  let value = 0n;
  for (const byte of bytes) value = (value << 8n) | BigInt(byte);
  let out = "";
  while (value > 0n) {
    out = BASE58[Number(value % 58n)]! + out;
    value /= 58n;
  }
  for (const byte of bytes) {
    if (byte !== 0) break;
    out = `1${out}`;
  }
  return out;
}

interface ReleaseBody {
  release: Record<string, any>;
  spec: Record<string, any>;
  stackSource: Record<string, any>;
  receipt: Record<string, any>;
  review: Record<string, any>;
  bundleBase64: string | null;
}

function signReview(releaseHash: string, seed = REVIEW_SEED, keyId = REVIEW_KEY_ID) {
  const signature = ed25519.sign(reviewSigningMessage(releaseHash, keyId, "reviewed"), seed);
  return {
    schema: "arete.protocol-release-review/v1",
    releaseHash,
    reviewKeyId: keyId,
    trustTier: "reviewed",
    signature: encodeBase64(signature),
  };
}

async function releaseBody(entry: string): Promise<{ hash: string; body: ReleaseBody }> {
  const result = await buildProtocol({ entry, protocolModule: resolve(here, "../index.ts") });
  const json = (name: string) => JSON.parse(decoder.decode(result.files.get(name)!)) as Record<string, any>;
  const receipt = json("build-receipt.json");
  const hash = receipt.outputs.releaseHash as string;
  const bundle = result.files.get("handlers.mjs");
  return {
    hash,
    body: {
      release: json("protocol-release.json"),
      spec: json("protocol-spec.json"),
      stackSource: json("protocol-stack-source.json"),
      receipt,
      review: signReview(hash),
      bundleBase64: bundle === undefined ? null : encodeBase64(bundle),
    },
  };
}

// ---------------------------------------------------------------------------
// The fake runtime ctx endpoint
// ---------------------------------------------------------------------------

interface CtxRequest {
  path: string;
  authorization: string | undefined;
  body: { seq: number; method: string; args: unknown };
}

let ctxServer: Server;
let ctxUrl: string;
const ctxRequests: CtxRequest[] = [];
let ctxReply: (request: CtxRequest) => { status?: number; body: unknown } = () => ({ body: { result: null } });

function readJson(request: IncomingMessage): Promise<any> {
  return new Promise((resolveBody) => {
    const chunks: Buffer[] = [];
    request.on("data", (chunk: Buffer) => chunks.push(chunk));
    request.on("end", () => resolveBody(JSON.parse(Buffer.concat(chunks).toString("utf8"))));
  });
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

let tokenRaise: { hash: string; body: ReleaseBody };
let probe: { hash: string; body: ReleaseBody };
let host: HandlerHost;

const WALLET = "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj";
const QUOTE_MINT = "So11111111111111111111111111111111111111112";

function createInput(): PrepareInputV1 {
  return {
    abi: PROTOCOL_HANDLER_ABI_V1,
    step: "create",
    args: {
      deadline: "1790000000",
      softCap: "1000",
      hardCap: "5000",
      walletCap: "500",
      quoteMint: QUOTE_MINT,
      members: ["SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf"],
    },
    resources: { launchDefaults: { tokenDecimals: 6 } },
    params: {},
    instance: null,
    participant: null,
    reads: {},
    wallet: WALLET,
    executionId: "00000000-0000-4000-8000-000000000001",
    ephemeral: {
      mint: "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
      presaleBase: "ComputeBudget111111111111111111111111111111",
      createKey: "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
    },
    programs: {
      meteoraPresale: "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
      squadsV4: "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf",
      system: "11111111111111111111111111111111",
      splToken: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    },
    fee: null,
    now: 1789000000,
    deployment: {
      id: "00000000-0000-4000-8000-0000000000d1",
      attestation: "Vote111111111111111111111111111111111111111",
      squadsProposer: null,
    },
  };
}

function probeInput(mode: string): PrepareInputV1 {
  return {
    ...createInput(),
    step: "open",
    args: { mode },
    ephemeral: {},
    programs: { system: "11111111111111111111111111111111" },
  };
}

async function call(
  method: string,
  path: string,
  body?: unknown,
  token: string | null = TOKEN,
  target: HandlerHost = host,
): Promise<{ status: number; body: any }> {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (token !== null) headers[HANDLER_HOST_TOKEN_HEADER] = token;
  const response = await fetch(`${target.url}${path}`, {
    method,
    headers,
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const text = await response.text();
  return { status: response.status, body: text === "" ? undefined : JSON.parse(text) };
}

const put = (hash: string, body: unknown, target?: HandlerHost) =>
  call("PUT", `/handler-host/v1/releases/${encodeURIComponent(hash)}`, body, TOKEN, target);

const prepare = (releaseHash: string, step: string, input: PrepareInputV1, target?: HandlerHost) =>
  call("POST", "/handler-host/v1/prepare", { releaseHash, step, invocationToken: "invocation-token-test", input }, TOKEN, target);

const reviewKeys = new Map([[REVIEW_KEY_ID, ed25519.getPublicKey(REVIEW_SEED)]]);

beforeAll(async () => {
  ctxServer = createServer(async (request, response) => {
    const entry: CtxRequest = { path: request.url ?? "", authorization: request.headers.authorization, body: await readJson(request) };
    ctxRequests.push(entry);
    const reply = ctxReply(entry);
    response.writeHead(reply.status ?? 200, { "content-type": "application/json" });
    response.end(JSON.stringify(reply.body));
  });
  await new Promise<void>((done) => ctxServer.listen(0, "127.0.0.1", done));
  ctxUrl = `http://127.0.0.1:${(ctxServer.address() as AddressInfo).port}/`;

  tokenRaise = await releaseBody(resolve(here, "../../fixtures/token-raise/protocol.ts"));
  probe = await releaseBody(resolve(here, "../../fixtures/handler-probe/protocol.ts"));
  host = await startHandlerHost({ token: TOKEN, reviewKeys, ctxUrl, port: 0 });
});

afterAll(async () => {
  await host?.close();
  await new Promise((done) => ctxServer?.close(done));
});

beforeEach(() => {
  ctxRequests.length = 0;
  ctxReply = () => ({ body: { result: null } });
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("handler host: auth and routing", () => {
  it("refuses requests without the right token", async () => {
    expect(await call("GET", "/handler-host/v1/inventory", undefined, null)).toMatchObject({
      status: 401,
      body: { error: { code: "unauthorized" } },
    });
    expect((await call("GET", "/handler-host/v1/inventory", undefined, "wrong")).status).toBe(401);
    expect((await call("GET", "/handler-host/v1/inventory", undefined, `${TOKEN}x`)).status).toBe(401);
    // The token is checked before routing.
    expect((await call("GET", "/nope", undefined, null)).status).toBe(401);
  });

  it("serves the inventory and refuses unknown routes and methods", async () => {
    const inventory = await call("GET", "/handler-host/v1/inventory");
    expect(inventory.status).toBe(200);
    expect(inventory.body.build).toEqual({ name: PACKAGE_NAME, version: PACKAGE_VERSION });
    expect(Array.isArray(inventory.body.releases)).toBe(true);
    expect(HANDLER_HOST_TOKEN_HEADER).toBe("x-arete-handler-host-token");
    expect(await call("GET", "/nope")).toMatchObject({ status: 404, body: { error: { code: "not_found" } } });
    expect(await call("POST", "/handler-host/v1/inventory", {})).toMatchObject({
      status: 405,
      body: { error: { code: "method_not_allowed" } },
    });
  });
});

describe("handler host: releases", () => {
  it("loads a reviewed release idempotently and removes it", async () => {
    expect(await put(tokenRaise.hash, tokenRaise.body)).toEqual({ status: 200, body: { releaseHash: tokenRaise.hash } });
    expect(await put(tokenRaise.hash, tokenRaise.body)).toEqual({ status: 200, body: { releaseHash: tokenRaise.hash } });
    expect(await put(probe.hash, probe.body)).toEqual({ status: 200, body: { releaseHash: probe.hash } });

    const releases = (await call("GET", "/handler-host/v1/inventory")).body.releases;
    const expected = [tokenRaise, probe]
      .map(({ hash, body }) => ({ releaseHash: hash, specHash: body.release.specHash, bundleHash: body.release.handlerBundleHash }))
      .sort((a, b) => (a.releaseHash < b.releaseHash ? -1 : 1));
    expect(releases).toEqual(expected);

    expect((await call("DELETE", `/handler-host/v1/releases/${tokenRaise.hash}`)).status).toBe(204);
    expect((await call("DELETE", `/handler-host/v1/releases/${tokenRaise.hash}`)).status).toBe(204);
    const after = (await call("GET", "/handler-host/v1/inventory")).body.releases;
    expect(after.map((release: { releaseHash: string }) => release.releaseHash)).toEqual([probe.hash]);
    expect((await prepare(tokenRaise.hash, "create", createInput())).body).toMatchObject({
      ok: false,
      error: { code: "release_not_loaded" },
    });
  });

  it("refuses a release whose hash does not match the path", async () => {
    expect(await put(probe.hash, tokenRaise.body)).toMatchObject({
      status: 400,
      body: { error: { code: "release_hash_mismatch" } },
    });
  });

  it("refuses a spec, stack source or receipt that does not match the release", async () => {
    const spec = { ...tokenRaise.body.spec, description: "changed" };
    expect((await put(tokenRaise.hash, { ...tokenRaise.body, spec })).body.error.code).toBe("spec_hash_mismatch");
    expect((await put(tokenRaise.hash, { ...tokenRaise.body, stackSource: probe.body.stackSource })).body.error.code).toBe(
      "stack_source_hash_mismatch",
    );
    expect((await put(tokenRaise.hash, { ...tokenRaise.body, receipt: probe.body.receipt })).body.error.code).toBe(
      "receipt_mismatch",
    );
  });

  it("refuses unknown review keys and bad signatures", async () => {
    const unknown = signReview(tokenRaise.hash, OTHER_SEED, "someone-else");
    expect(await put(tokenRaise.hash, { ...tokenRaise.body, review: unknown })).toMatchObject({
      status: 403,
      body: { error: { code: "review_key_unknown" } },
    });
    const forged = signReview(tokenRaise.hash, OTHER_SEED);
    expect((await put(tokenRaise.hash, { ...tokenRaise.body, review: forged })).body.error.code).toBe("review_invalid");
    const otherRelease = signReview(probe.hash);
    expect((await put(tokenRaise.hash, { ...tokenRaise.body, review: otherRelease })).body.error.code).toBe("review_invalid");
    const unreviewed = { ...tokenRaise.body.review, trustTier: "unreviewed" };
    expect((await put(tokenRaise.hash, { ...tokenRaise.body, review: unreviewed })).body.error.code).toBe("review_invalid");
  });

  it("refuses a bundle that does not match the release", async () => {
    expect(await put(tokenRaise.hash, { ...tokenRaise.body, bundleBase64: probe.body.bundleBase64 })).toMatchObject({
      status: 400,
      body: { error: { code: "bundle_hash_mismatch" } },
    });
    expect((await put(tokenRaise.hash, { ...tokenRaise.body, bundleBase64: null })).body.error.code).toBe("bundle_missing");
  });

  it("refuses malformed requests", async () => {
    expect((await put(tokenRaise.hash, { ...tokenRaise.body, extra: 1 })).body.error.code).toBe("invalid_request");
    const { review: _review, ...missing } = tokenRaise.body;
    expect((await put(tokenRaise.hash, missing)).body.error.code).toBe("invalid_request");
    const release = { ...tokenRaise.body.release, extra: true };
    expect(await put(tokenRaise.hash, { ...tokenRaise.body, release })).toMatchObject({
      status: 400,
      body: { error: { code: "contract_invalid" } },
    });
  });
});

describe("handler host: prepare", () => {
  beforeAll(async () => {
    expect((await put(tokenRaise.hash, tokenRaise.body)).status).toBe(200);
    expect((await put(probe.hash, probe.body)).status).toBe(200);
  });

  it("runs a handler with ctx callbacks and returns the validated operation", async () => {
    ctxReply = () => ({
      body: {
        result: {
          address: QUOTE_MINT,
          exists: true,
          slot: "300000000",
          lamports: "1461600",
          owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
          dataBase64: "AAAA",
        },
      },
    });
    const response = await prepare(tokenRaise.hash, "create", createInput());
    expect(response.status).toBe(200);
    expect(response.body.ok).toBe(true);
    expect(response.body.operation.abi).toBe(PROTOCOL_HANDLER_ABI_V1);
    expect(response.body.operation.transactions[0].instructions.map((ix: { programId: string }) => ix.programId)).toEqual([
      "11111111111111111111111111111111",
      "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
      "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
      "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf",
    ]);
    expect(ctxRequests).toEqual([
      {
        path: "/handler-host/v1/ctx",
        authorization: "Bearer invocation-token-test",
        body: { seq: 0, method: "solana.account", args: { pubkey: QUOTE_MINT } },
      },
    ]);
  });

  it("numbers ctx calls per invocation and decodes their results", async () => {
    const account = { address: WALLET, exists: true, slot: "7", lamports: "1", owner: WALLET, dataBase64: "AQID" };
    ctxReply = ({ body }) => {
      switch (body.method) {
        case "solana.account":
          return { body: { result: account } };
        case "solana.accounts":
          return { body: { result: [account, { address: "x", exists: false, slot: "7" }] } };
        case "solana.tokenAccountsByOwner":
          return { body: { result: [{ address: WALLET, mint: WALLET, owner: WALLET, amount: "5", slot: "7" }] } };
        default:
          return { body: { result: { n: 42 } } };
      }
    };
    for (let round = 0; round < 2; round += 1) {
      ctxRequests.length = 0;
      const response = await prepare(probe.hash, "open", probeInput("ctx"));
      expect(response.body.ok).toBe(true);
      // [2, 0, 0, 0, decoded length 3, 1 existing account, 1 token account, view n 42]
      expect(response.body.operation.transactions[0].instructions[0].dataBase64).toBe(encodeBase64(Uint8Array.from([2, 0, 0, 0, 3, 1, 1, 42])));
      expect(ctxRequests.map((request) => request.body)).toEqual([
        { seq: 0, method: "solana.account", args: { pubkey: WALLET, minContextSlot: 5 } },
        { seq: 1, method: "solana.accounts", args: { pubkeys: [WALLET, "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"] } },
        { seq: 2, method: "solana.tokenAccountsByOwner", args: { owner: WALLET, mints: ["TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"] } },
        { seq: 3, method: "stacks.view", args: { stack: "probeStack", view: "counter", key: "k1" } },
      ]);
      expect(ctxRequests.every((request) => request.authorization === "Bearer invocation-token-test")).toBe(true);
    }
  });

  it("refuses steps without a handler", async () => {
    expect((await prepare(tokenRaise.hash, "contribute", { ...createInput(), step: "contribute" })).body).toMatchObject({
      ok: false,
      error: { code: "step_not_handler" },
    });
    expect((await prepare(tokenRaise.hash, "nope", { ...createInput(), step: "nope" })).body.error.code).toBe("step_not_handler");
  });

  it("reports a handler that throws", async () => {
    expect((await prepare(probe.hash, "open", probeInput("throw"))).body).toEqual({
      ok: false,
      error: { code: "handler_threw", message: "Error: probe refused" },
    });
    // ctx returned null for the quote mint, so the token raise handler throws.
    expect((await prepare(tokenRaise.hash, "create", createInput())).body).toMatchObject({
      ok: false,
      error: { code: "handler_threw", message: "Error: quote mint does not exist" },
    });
  });

  it("fails the invocation on a ctx error, even when the handler swallows it", async () => {
    ctxReply = () => ({ status: 403, body: { error: { code: "read_not_allowed", message: "nope" } } });
    const direct = await prepare(tokenRaise.hash, "create", createInput());
    expect(direct.body).toMatchObject({ ok: false, error: { code: "ctx_error" } });
    expect(direct.body.error.message).toContain("read_not_allowed");
    const swallowed = await prepare(probe.hash, "open", probeInput("swallow"));
    expect(swallowed.body).toMatchObject({ ok: false, error: { code: "ctx_error" } });

    ctxReply = () => ({ body: { result: { not: "an account" } } });
    expect((await prepare(tokenRaise.hash, "create", createInput())).body.error.code).toBe("ctx_error");
  });

  it("validates the handler result", async () => {
    const invalid = await prepare(probe.hash, "open", probeInput("invalid"));
    expect(invalid.body).toMatchObject({ ok: false, error: { code: "handler_result_invalid" } });
    expect(invalid.body.error.message).toContain("/transactions");
    const huge = await prepare(probe.hash, "open", probeInput("huge"));
    expect(huge.body).toMatchObject({ ok: false, error: { code: "handler_result_invalid" } });
    expect(huge.body.error.message).toContain("string-too-long");
    expect((await prepare(probe.hash, "open", probeInput("ok"))).body.ok).toBe(true);
  });

  it("times out a handler that never settles", async () => {
    const quick = await startHandlerHost({ token: TOKEN, reviewKeys, ctxUrl, timeoutMs: 150 });
    try {
      expect((await put(probe.hash, probe.body, quick)).status).toBe(200);
      const started = Date.now();
      expect((await prepare(probe.hash, "open", probeInput("hang"), quick)).body).toMatchObject({
        ok: false,
        error: { code: "timeout" },
      });
      expect(Date.now() - started).toBeLessThan(5_000);
    } finally {
      await quick.close();
    }
  });

  it("refuses malformed prepare requests", async () => {
    expect((await call("POST", "/handler-host/v1/prepare", { releaseHash: probe.hash, step: "open" })).body.error.code).toBe(
      "invalid_request",
    );
    expect(
      (await call("POST", "/handler-host/v1/prepare", { releaseHash: probe.hash, step: "open", invocationToken: "t", input: probeInput("ok"), x: 1 }))
        .body.error.code,
    ).toBe("invalid_request");
    expect((await prepare(probe.hash, "open", { ...probeInput("ok"), step: "other" })).status).toBe(400);
  });
});

describe("handler host: options", () => {
  it("binds only to loopback", async () => {
    expect(parseListenAddress("127.0.0.1:0")).toEqual({ host: "127.0.0.1", port: 0 });
    expect(parseListenAddress("localhost:8080")).toEqual({ host: "127.0.0.1", port: 8080 });
    expect(parseListenAddress("[::1]:9000")).toEqual({ host: "::1", port: 9000 });
    expect(() => parseListenAddress("0.0.0.0:8080")).toThrow(/loopback/);
    expect(() => parseListenAddress("192.168.1.2:8080")).toThrow(/loopback/);
    expect(() => parseListenAddress("[::]:8080")).toThrow(/loopback/);
    expect(isLoopbackHost("127.0.0.2")).toBe(true);
    await expect(startHandlerHost({ token: TOKEN, reviewKeys, ctxUrl, host: "0.0.0.0" })).rejects.toThrow(/loopback/);
  });

  it("parses review keys", () => {
    const publicKey = ed25519.getPublicKey(REVIEW_SEED);
    const [id, key] = parseReviewKey(`${REVIEW_KEY_ID}=${encodeBase58(publicKey)}`);
    expect(id).toBe(REVIEW_KEY_ID);
    expect([...key]).toEqual([...publicKey]);
    expect(() => parseReviewKey("Bad_Id=11111111111111111111111111111111")).toThrow(/must match/);
    expect(() => parseReviewKey("ok=abc")).toThrow(/32-byte/);
    expect(() => parseReviewKey("nokey")).toThrow(/expects/);
  });
});
