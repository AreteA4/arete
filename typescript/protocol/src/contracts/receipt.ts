/**
 * Execution receipts and the receipt key set. Mirrors
 * `arete-hash/src/protocol/receipt.rs`.
 *
 * A receipt's body is every field except `signatures`. Each signature is an
 * Ed25519 signature by a receipt key over
 * `arete.protocol-execution-receipt/v1\n<keyId>\n<bodySha256>`, where
 * `bodySha256` is the lowercase hex SHA-256 of the body's Arete JCS bytes.
 */

import { sha256 } from "@noble/hashes/sha256";
import { canonicalizeJcs, type JsonValue } from "@usearete/hash";

import {
  PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1,
  PROTOCOL_RECEIPT_KEYS_SCHEMA_V1,
  TRUST_TIERS,
  type ProtocolExecutionReceiptV1,
  type ProtocolReceiptKeySetV1,
} from "./types.js";
import { uuid } from "./intent.js";
import { keyId } from "./release.js";
import {
  MAX_SAFE,
  Obj,
  U64_MAX,
  asArray,
  asInteger,
  asStr,
  decimal,
  decodeBase58,
  decodeBase64,
  err,
  identifierValue,
  index,
  isObject,
  oneOf,
  pubkey,
  rejectFloats,
  schemaTag,
  sha256Hex,
  tagged,
} from "./shape.js";
import { hashIdValue } from "./spec.js";

export const MAX_RECEIPT_SIGNATURES = 4;
export const MAX_RECEIPT_KEYS = 32;
const MAX_TRANSACTIONS = 8;
const encoder = new TextEncoder();

const RECEIPT_KEYS = [
  "schema",
  "signature",
  "slot",
  "deployment",
  "trustTier",
  "reviewHash",
  "step",
  "instanceId",
  "participantId",
  "transaction",
  "messageSha256",
  "intentHash",
  "attestationPubkey",
  "fees",
  "status",
  "issuedAt",
  "signatures",
];

function transactionSignature(value: unknown, path: string): string {
  const text = asStr(value, path);
  const bytes = text.length <= 88 ? decodeBase58(text) : undefined;
  if (bytes === undefined || bytes.length !== 64) {
    err("invalid-signature", path, "expected a base58 64-byte transaction signature");
  }
  return text;
}

function shortId(value: unknown, path: string): string {
  const text = asStr(value, path);
  if (!/^[A-Za-z0-9_-]{1,64}$/.test(text)) {
    err("invalid-identifier", path, "short ids match ^[A-Za-z0-9_-]{1,64}$");
  }
  return text;
}

function base64Signature(value: unknown, path: string): void {
  const bytes = decodeBase64(asStr(value, path));
  if (bytes === undefined || bytes.length !== 64) {
    err("invalid-signature", path, "expected a base64 64-byte Ed25519 signature");
  }
}

function sortedKeyIds(values: unknown[], path: string, read: (item: unknown, path: string) => string): void {
  let previous: string | undefined;
  values.forEach((item, i) => {
    const itemPath = index(path, i);
    const key = read(item, itemPath);
    if (previous !== undefined) {
      if (key === previous) err("duplicate-entry", itemPath, `'${key}' is listed twice`);
      if (key < previous) err("unsorted-entries", itemPath, "entries must be sorted");
    }
    previous = key;
  });
}

export function validateProtocolExecutionReceipt(value: unknown): ProtocolExecutionReceiptV1 {
  rejectFloats(value, "");
  const root = new Obj(value, "", RECEIPT_KEYS);
  schemaTag(root.req("schema"), "/schema", PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1);
  transactionSignature(root.req("signature"), "/signature");
  decimal(root.req("slot"), "/slot", U64_MAX);
  const deployment = new Obj(root.req("deployment"), "/deployment", [
    "deploymentId",
    "shortId",
    "revision",
    "releaseHash",
  ]);
  uuid(deployment.req("deploymentId"), "/deployment/deploymentId");
  shortId(deployment.req("shortId"), "/deployment/shortId");
  asInteger(deployment.req("revision"), "/deployment/revision", 1, 4_294_967_295);
  hashIdValue(deployment.req("releaseHash"), "/deployment/releaseHash", "protocol-release");
  const tier = oneOf(root.req("trustTier"), "/trustTier", TRUST_TIERS);
  const review = root.opt("reviewHash");
  if (tier === "reviewed") {
    if (review === undefined) err("missing-field", "/reviewHash", "a reviewed receipt carries 'reviewHash'");
    hashIdValue(review, "/reviewHash", "protocol-release-review");
  } else if (review !== undefined) {
    err("invalid-value", "/reviewHash", "an unreviewed receipt carries no 'reviewHash'");
  }
  identifierValue(root.req("step"), "/step");
  uuid(root.req("instanceId"), "/instanceId");
  const participant = root.opt("participantId");
  if (participant !== undefined) uuid(participant, "/participantId");
  const transaction = new Obj(root.req("transaction"), "/transaction", ["index", "count"]);
  const position = asInteger(transaction.req("index"), "/transaction/index", 0, MAX_TRANSACTIONS - 1);
  const count = asInteger(transaction.req("count"), "/transaction/count", 1, MAX_TRANSACTIONS);
  if (position >= count) err("invalid-value", "/transaction/index", "index must be below count");
  sha256Hex(root.req("messageSha256"), "/messageSha256");
  hashIdValue(root.req("intentHash"), "/intentHash", "execution-intent");
  pubkey(root.req("attestationPubkey"), "/attestationPubkey");
  const feesValue = root.opt("fees");
  if (feesValue !== undefined) {
    const fees = new Obj(feesValue, "/fees", ["asset", "platform", "creator"]);
    const [kind, asset] = tagged(fees.req("asset"), "/fees/asset", [
      ["native", []],
      ["spl", ["mint"]],
    ]);
    if (kind === "spl") pubkey(asset.req("mint"), "/fees/asset/mint");
    decimal(fees.req("platform"), "/fees/platform", U64_MAX);
    decimal(fees.req("creator"), "/fees/creator", U64_MAX);
  }
  oneOf(root.req("status"), "/status", ["confirmed", "finalized"]);
  asInteger(root.req("issuedAt"), "/issuedAt", 0, MAX_SAFE);
  const signatures = asArray(root.req("signatures"), "/signatures", MAX_RECEIPT_SIGNATURES);
  if (signatures.length === 0) err("invalid-value", "/signatures", "a receipt carries at least one signature");
  sortedKeyIds(signatures, "/signatures", (item, path) => {
    const signature = new Obj(item, path, ["keyId", "signature"]);
    const key = keyId(signature.req("keyId"), signature.pathOf("keyId"));
    base64Signature(signature.req("signature"), signature.pathOf("signature"));
    return key;
  });
  return value as ProtocolExecutionReceiptV1;
}

export function validateProtocolReceiptKeySet(value: unknown): ProtocolReceiptKeySetV1 {
  rejectFloats(value, "");
  const root = new Obj(value, "", ["schema", "keys"]);
  schemaTag(root.req("schema"), "/schema", PROTOCOL_RECEIPT_KEYS_SCHEMA_V1);
  const keys = asArray(root.req("keys"), "/keys", MAX_RECEIPT_KEYS);
  sortedKeyIds(keys, "/keys", (item, path) => {
    const key = new Obj(item, path, ["keyId", "publicKey", "retiredAt"]);
    const id = keyId(key.req("keyId"), key.pathOf("keyId"));
    pubkey(key.req("publicKey"), key.pathOf("publicKey"));
    const retired = key.opt("retiredAt");
    if (retired !== undefined) asInteger(retired, key.pathOf("retiredAt"), 0, MAX_SAFE);
    return id;
  });
  return value as ProtocolReceiptKeySetV1;
}

/** The receipt body (`signatures` removed) in Arete JCS form. */
export function receiptBodyBytes(receipt: unknown): Uint8Array {
  if (!isObject(receipt)) throw new TypeError("a receipt is a JSON object");
  const { signatures: _signatures, ...body } = receipt;
  return canonicalizeJcs(body as JsonValue);
}

/** Lowercase hex SHA-256 of `receiptBodyBytes`. */
export function receiptBodySha256(receipt: unknown): string {
  let out = "";
  for (const byte of sha256(receiptBodyBytes(receipt))) out += byte.toString(16).padStart(2, "0");
  return out;
}

/** The exact bytes a receipt key signs. */
export function receiptSigningMessage(keyIdText: string, bodySha256: string): Uint8Array {
  return encoder.encode(`${PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1}\n${keyIdText}\n${bodySha256}`);
}
