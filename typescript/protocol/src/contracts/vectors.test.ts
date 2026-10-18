import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { ed25519 } from "@noble/curves/ed25519";
import { canonicalizeJcs, type JsonValue } from "@usearete/hash";
import { describe, expect, it } from "vitest";

import {
  PROTOCOL_ERROR_CODES,
  ProtocolContractError,
  decodeBase58,
  encodeBase64,
  decodeBase64,
  hashHandlerBundle,
  parseContract,
  parseContractJson,
  receiptBodyBytes,
  receiptBodySha256,
  receiptSigningMessage,
  reviewSigningMessage,
  validateContract,
  verifyReleaseReview,
  type ProtocolContractKind,
} from "../index.js";
import { FloatToken } from "./shape.js";

const here = dirname(fileURLToPath(import.meta.url));
const vectorsPath = resolve(here, "../../../../test-vectors/protocol-v1.json");
const encoder = new TextEncoder();

interface Vectors {
  schema: string;
  documents: Record<string, { kind: ProtocolContractKind; document: unknown }>;
  validVectors: Array<{ name: string; kind: ProtocolContractKind; document: string; hash: string | null }>;
  invalidVectors: Array<{
    name: string;
    kind: ProtocolContractKind;
    document: string;
    patch: Array<{ op: string; path: string; value?: unknown }>;
    code: string;
    path: string;
  }>;
  bundleVectors: Array<{ name: string; bytesUtf8: string; hash: string }>;
  reviewVectors: Array<{
    name: string;
    review: string;
    publicKeyBase58: string;
    reviewKeyId: string;
    signingMessageUtf8: string;
  }>;
  rawJsonVectors: Array<{ name: string; kind: ProtocolContractKind; json: string; code: string; path: string }>;
  errorCodes: string[];
}

// Parse the vector file with the contract parser so float tokens such as
// `1.0` in patches stay floats, exactly as Rust sees them.
const vectors = parseContractJson(readFileSync(vectorsPath)) as Vectors;

function clone<T>(value: T): T {
  if (Array.isArray(value)) return value.map(clone) as T;
  if (value instanceof FloatToken) return value;
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) out[key] = clone(item);
    return out as T;
  }
  return value;
}

function pointer(path: string): string[] {
  return path
    .split("/")
    .slice(1)
    .map((part) => part.replace(/~1/g, "/").replace(/~0/g, "~"));
}

/** The same patch semantics as the Rust generator's `apply_patch`. */
function applyPatch(document: unknown, patch: Vectors["invalidVectors"][number]["patch"]): unknown {
  for (const op of patch) {
    const parts = pointer(op.path);
    const last = parts.pop()!;
    let target = document as any;
    for (const part of parts) target = Array.isArray(target) ? target[Number(part)] : target[part];
    if (op.op === "remove") {
      if (Array.isArray(target)) target.splice(Number(last), 1);
      else delete target[last];
    } else if (Array.isArray(target)) {
      if (op.op === "add" && last === "-") target.push(clone(op.value));
      else if (op.op === "add") target.splice(Number(last), 0, clone(op.value));
      else target[Number(last)] = clone(op.value);
    } else {
      target[last] = clone(op.value);
    }
  }
  return document;
}

function documentOf(name: string) {
  const entry = vectors.documents[name];
  if (!entry) throw new Error(`missing document ${name}`);
  return { kind: entry.kind, document: clone(entry.document) };
}

describe("protocol-v1 vectors", () => {
  it("uses the vector schema", () => {
    expect(vectors.schema).toBe("arete.protocol-test-vectors/v1");
  });

  it("exports exactly the Rust error codes", () => {
    expect([...PROTOCOL_ERROR_CODES]).toEqual(vectors.errorCodes);
  });

  for (const vector of vectors.validVectors) {
    it(`valid ${vector.name}`, () => {
      const { kind, document } = documentOf(vector.document);
      expect(kind).toBe(vector.kind);
      expect(validateContract(kind, document)).toBe(vector.hash);
    });
  }

  for (const vector of vectors.invalidVectors) {
    it(`invalid ${vector.name}`, () => {
      const { kind, document } = documentOf(vector.document);
      const patched = applyPatch(document, vector.patch);
      let caught: unknown;
      try {
        validateContract(kind, patched);
      } catch (error) {
        caught = error;
      }
      expect(caught).toBeInstanceOf(ProtocolContractError);
      const error = caught as ProtocolContractError;
      expect({ code: error.code, path: error.path }).toEqual({ code: vector.code, path: vector.path });
    });
  }

  for (const vector of vectors.rawJsonVectors) {
    it(`raw JSON ${vector.name}`, () => {
      let caught: unknown;
      try {
        parseContract(vector.kind, encoder.encode(vector.json));
      } catch (error) {
        caught = error;
      }
      expect(caught).toBeInstanceOf(ProtocolContractError);
      const error = caught as ProtocolContractError;
      expect({ code: error.code, path: error.path }).toEqual({ code: vector.code, path: vector.path });
    });
  }

  for (const vector of vectors.bundleVectors) {
    it(`bundle ${vector.name}`, () => {
      expect(hashHandlerBundle(encoder.encode(vector.bytesUtf8))).toBe(vector.hash);
    });
  }

  for (const vector of vectors.reviewVectors) {
    it(`review ${vector.name}`, () => {
      const { document } = documentOf(vector.review);
      const review = document as { releaseHash: string; reviewKeyId: string; signature: string };
      const publicKey = decodeBase58(vector.publicKeyBase58)!;
      expect(publicKey.length).toBe(32);
      expect(review.reviewKeyId).toBe(vector.reviewKeyId);
      expect(new TextDecoder().decode(reviewSigningMessage(review.releaseHash, review.reviewKeyId, "reviewed"))).toBe(
        vector.signingMessageUtf8,
      );
      expect(verifyReleaseReview(review, publicKey)).toBe(true);

      const tamperedSignature = decodeBase64(review.signature)!;
      tamperedSignature[0]! ^= 1;
      expect(verifyReleaseReview({ ...review, signature: encodeBase64(tamperedSignature) }, publicKey)).toBe(false);

      const otherRelease = `${review.releaseHash.slice(0, -1)}${review.releaseHash.endsWith("0") ? "1" : "0"}`;
      expect(verifyReleaseReview({ ...review, releaseHash: otherRelease }, publicKey)).toBe(false);
      expect(verifyReleaseReview({ ...review, reviewKeyId: "test-review-2" }, publicKey)).toBe(false);

      const otherKey = publicKey.slice();
      otherKey[31]! ^= 1;
      expect(verifyReleaseReview(review, otherKey)).toBe(false);
    });
  }
});

describe("execution receipts", () => {
  const keys = documentOf("receiptKeys").document as { keys: Array<{ keyId: string; publicKey: string }> };
  for (const name of ["contributeReceipt", "unreviewedReceipt"]) {
    it(`${name} verifies over the canonical body`, () => {
      const receipt = documentOf(name).document as {
        signatures: Array<{ keyId: string; signature: string }>;
        slot: string;
      };
      const { signatures, ...body } = receipt;
      expect(receiptBodyBytes(receipt)).toEqual(canonicalizeJcs(body as JsonValue));
      const [signature] = signatures;
      const key = keys.keys.find((entry) => entry.keyId === signature!.keyId)!;
      const message = receiptSigningMessage(signature!.keyId, receiptBodySha256(receipt));
      const bytes = decodeBase64(signature!.signature)!;
      expect(ed25519.verify(bytes, message, decodeBase58(key.publicKey)!)).toBe(true);
      const tampered = receiptSigningMessage(signature!.keyId, receiptBodySha256({ ...receipt, slot: "1" }));
      expect(ed25519.verify(bytes, tampered, decodeBase58(key.publicKey)!)).toBe(false);
    });
  }
});

describe("contract parsing", () => {
  it("reports a float written as 1.0 like Rust", () => {
    const json = '{"schema":"arete.protocol-resource/v1","name":"a","schemaSha256":"3333333333333333333333333333333333333333333333333333333333333333","value":1.0}';
    expect(() => parseContract("protocol-resource", encoder.encode(json))).toThrow(
      expect.objectContaining({ code: "float-not-allowed", path: "/value" }),
    );
  });

  it("rejects unsafe integers before duplicate keys, as Rust does", () => {
    const json = '{"a":1,"a":2,"b":9007199254740993}';
    expect(() => parseContract("protocol-resource", encoder.encode(json))).toThrow(
      expect.objectContaining({ code: "unsafe-json-integer", path: "" }),
    );
  });

  it("maps malformed JSON to invalid-json", () => {
    expect(() => parseContract("protocol-resource", encoder.encode("{"))).toThrow(
      expect.objectContaining({ code: "invalid-json" }),
    );
  });
});
