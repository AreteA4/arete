/**
 * Execution-side contracts: provider snapshots and execution intents.
 * Mirrors `arete-hash/src/protocol/intent.rs`.
 */

import {
  CTX_METHODS,
  EXECUTION_INTENT_SCHEMA_V1,
  PROVIDER_SNAPSHOT_SCHEMA_V1,
  SIGNER_CLASSES,
  type ExecutionIntentV1,
  type ProviderSnapshotV1,
} from "./types.js";
import {
  MAX_SAFE,
  Obj,
  U64_MAX,
  asArray,
  asBool,
  asInteger,
  asStr,
  boundedStr,
  decimal,
  decodeBase64,
  err,
  identifierValue,
  index,
  oneOf,
  pubkey,
  rejectFloats,
  schemaTag,
  sha256Hex,
  sortedEntries,
  tagged,
  uniqueStrings,
} from "./shape.js";
import { hashIdValue } from "./spec.js";

export const MAX_CTX_CALLS = 64;
export const MAX_INTENT_TRANSACTIONS = 8;
const MAX_SNAPSHOT_READS = 64;
const MAX_INTENT_SIGNERS = 32;

// ---------------------------------------------------------------------------
// Provider snapshot
// ---------------------------------------------------------------------------

export function shapeAccountRead(value: unknown, path: string): void {
  const read = new Obj(value, path, ["address", "exists", "slot", "lamports", "owner", "dataBase64"]);
  pubkey(read.req("address"), read.pathOf("address"));
  const exists = asBool(read.req("exists"), read.pathOf("exists"));
  decimal(read.req("slot"), read.pathOf("slot"), U64_MAX);
  for (const key of ["lamports", "owner", "dataBase64"]) {
    const present = read.opt(key) !== undefined;
    if (present !== exists) {
      if (exists) err("missing-field", read.pathOf(key), `existing accounts carry '${key}'`);
      err("invalid-value", read.pathOf(key), `missing accounts carry no '${key}'`);
    }
  }
  if (exists) {
    decimal(read.req("lamports"), read.pathOf("lamports"), U64_MAX);
    pubkey(read.req("owner"), read.pathOf("owner"));
    const data = asStr(read.req("dataBase64"), read.pathOf("dataBase64"));
    if (decodeBase64(data) === undefined) {
      err("invalid-value", read.pathOf("dataBase64"), "expected canonical base64");
    }
  }
}

export function validateProviderSnapshot(value: unknown): ProviderSnapshotV1 {
  rejectFloats(value, "");
  const root = new Obj(value, "", ["schema", "contextSlot", "reads", "ctxCalls", "observedAt", "expiresAt"]);
  schemaTag(root.req("schema"), "/schema", PROVIDER_SNAPSHOT_SCHEMA_V1);
  decimal(root.req("contextSlot"), "/contextSlot", U64_MAX);
  const reads = root.req("reads");
  asArray(reads, "/reads", MAX_SNAPSHOT_READS);
  sortedEntries(reads, "/reads", (item, path) => {
    const entry = new Obj(item, path, ["name", "result"]);
    const name = identifierValue(entry.req("name"), entry.pathOf("name"));
    shapeAccountRead(entry.req("result"), entry.pathOf("result"));
    return name;
  });
  const calls = asArray(root.req("ctxCalls"), "/ctxCalls", MAX_CTX_CALLS);
  calls.forEach((item, i) => {
    const path = index("/ctxCalls", i);
    const call = new Obj(item, path, ["seq", "method", "args", "resultSha256"]);
    const seq = asInteger(call.req("seq"), call.pathOf("seq"), 0, MAX_CTX_CALLS);
    if (seq !== i) err("invalid-value", call.pathOf("seq"), `expected seq ${i}`);
    oneOf(call.req("method"), call.pathOf("method"), CTX_METHODS);
    call.req("args");
    sha256Hex(call.req("resultSha256"), call.pathOf("resultSha256"));
  });
  const observed = asInteger(root.req("observedAt"), "/observedAt", 0, MAX_SAFE);
  const expires = asInteger(root.req("expiresAt"), "/expiresAt", 0, MAX_SAFE);
  if (expires < observed) err("invalid-value", "/expiresAt", "expiresAt is before observedAt");
  return value as ProviderSnapshotV1;
}

// ---------------------------------------------------------------------------
// Execution intent
// ---------------------------------------------------------------------------

/** Lowercase canonical UUID text. */
export function uuid(value: unknown, path: string): string {
  const text = asStr(value, path);
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(text)) {
    err("invalid-value", path, "expected a lowercase UUID");
  }
  return text;
}

const INTENT_KEYS = [
  "schema",
  "executionId",
  "deployment",
  "step",
  "instanceId",
  "participantId",
  "wallet",
  "role",
  "callerAccount",
  "sessionKeyClass",
  "transactionFormat",
  "inputSha256",
  "resourceHashes",
  "providerSnapshotHash",
  "fees",
  "signers",
  "transactions",
  "blockhash",
  "lastValidBlockHeight",
  "expiresAt",
  "createdAt",
];

export function validateExecutionIntent(value: unknown): ExecutionIntentV1 {
  rejectFloats(value, "");
  const root = new Obj(value, "", INTENT_KEYS);
  schemaTag(root.req("schema"), "/schema", EXECUTION_INTENT_SCHEMA_V1);
  uuid(root.req("executionId"), "/executionId");
  const deployment = new Obj(root.req("deployment"), "/deployment", ["deploymentId", "revision", "releaseHash"]);
  uuid(deployment.req("deploymentId"), "/deployment/deploymentId");
  asInteger(deployment.req("revision"), "/deployment/revision", 1, 4_294_967_295);
  hashIdValue(deployment.req("releaseHash"), "/deployment/releaseHash", "protocol-release");
  identifierValue(root.req("step"), "/step");
  const instanceId = root.nullable("instanceId");
  if (instanceId !== undefined) uuid(instanceId, "/instanceId");
  const participantId = root.nullable("participantId");
  if (participantId !== undefined) uuid(participantId, "/participantId");
  pubkey(root.req("wallet"), "/wallet");
  const role = root.nullable("role");
  if (role !== undefined) identifierValue(role, "/role");
  const caller = boundedStr(root.req("callerAccount"), "/callerAccount", 64);
  if (caller.length === 0) err("invalid-value", "/callerAccount", "callerAccount must not be empty");
  oneOf(root.req("sessionKeyClass"), "/sessionKeyClass", ["publishable", "secret"]);
  oneOf(root.req("transactionFormat"), "/transactionFormat", ["v1", "v0"]);
  sha256Hex(root.req("inputSha256"), "/inputSha256");
  sortedEntries(root.req("resourceHashes"), "/resourceHashes", (item, path) =>
    hashIdValue(item, path, "protocol-resource"),
  );
  hashIdValue(root.req("providerSnapshotHash"), "/providerSnapshotHash", "provider-snapshot");
  const fees = root.nullable("fees");
  if (fees !== undefined) shapeFees(fees, "/fees");
  const signers = asArray(root.req("signers"), "/signers", MAX_INTENT_SIGNERS);
  signers.forEach((item, i) => {
    const path = index("/signers", i);
    const signer = new Obj(item, path, ["class", "pubkey", "name"]);
    oneOf(signer.req("class"), signer.pathOf("class"), SIGNER_CLASSES);
    pubkey(signer.req("pubkey"), signer.pathOf("pubkey"));
    const name = signer.nullable("name");
    if (name !== undefined) identifierValue(name, signer.pathOf("name"));
  });
  const transactions = asArray(root.req("transactions"), "/transactions", MAX_INTENT_TRANSACTIONS);
  if (transactions.length === 0) {
    err("invalid-value", "/transactions", "an intent has at least one transaction");
  }
  transactions.forEach((item, i) => {
    const path = index("/transactions", i);
    const transaction = new Obj(item, path, ["index", "messageSha256", "signers"]);
    const position = asInteger(transaction.req("index"), transaction.pathOf("index"), 0, 7);
    if (position !== i) err("invalid-value", transaction.pathOf("index"), `expected index ${i}`);
    sha256Hex(transaction.req("messageSha256"), transaction.pathOf("messageSha256"));
    const signerPath = transaction.pathOf("signers");
    const list = uniqueStrings(transaction.req("signers"), signerPath, MAX_INTENT_SIGNERS, pubkey);
    if (list.length === 0) err("invalid-value", signerPath, "a transaction has at least one signer");
  });
  pubkey(root.req("blockhash"), "/blockhash");
  decimal(root.req("lastValidBlockHeight"), "/lastValidBlockHeight", U64_MAX);
  const expires = asInteger(root.req("expiresAt"), "/expiresAt", 0, MAX_SAFE);
  const created = asInteger(root.req("createdAt"), "/createdAt", 0, MAX_SAFE);
  if (expires < created) err("invalid-value", "/expiresAt", "expiresAt is before createdAt");
  return value as ExecutionIntentV1;
}

function shapeFees(value: unknown, path: string): void {
  const fees = new Obj(value, path, [
    "asset",
    "gross",
    "total",
    "creator",
    "platform",
    "placement",
    "platformDestination",
    "creatorDestination",
    "feeAdjustment",
  ]);
  const [kind, asset] = tagged(fees.req("asset"), fees.pathOf("asset"), [
    ["native", []],
    ["spl", ["mint"]],
  ]);
  if (kind === "spl") pubkey(asset.req("mint"), asset.pathOf("mint"));
  const gross = decimal(fees.req("gross"), fees.pathOf("gross"), U64_MAX);
  const total = decimal(fees.req("total"), fees.pathOf("total"), U64_MAX);
  const creator = decimal(fees.req("creator"), fees.pathOf("creator"), U64_MAX);
  const platform = decimal(fees.req("platform"), fees.pathOf("platform"), U64_MAX);
  if (total > gross) err("invalid-value", fees.pathOf("total"), "the fee exceeds the gross amount");
  if (creator + platform !== total) {
    err("invalid-value", fees.pathOf("platform"), "creator + platform must equal total");
  }
  oneOf(fees.req("placement"), fees.pathOf("placement"), ["before", "after"]);
  pubkey(fees.req("platformDestination"), fees.pathOf("platformDestination"));
  const destination = fees.nullable("creatorDestination");
  if (destination !== undefined) {
    pubkey(destination, fees.pathOf("creatorDestination"));
  } else if (creator !== 0n) {
    err("invalid-value", fees.pathOf("creatorDestination"), "a non-zero creator leg needs a destination");
  }
  const adjustmentValue = fees.opt("feeAdjustment");
  if (adjustmentValue !== undefined) {
    const adjustment = new Obj(adjustmentValue, fees.pathOf("feeAdjustment"), ["kind", "id"]);
    oneOf(adjustment.req("kind"), adjustment.pathOf("kind"), ["cap", "override"]);
    const id = asStr(adjustment.req("id"), adjustment.pathOf("id"));
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(id)) {
      err("invalid-identifier", adjustment.pathOf("id"), "adjustment ids match ^[A-Za-z0-9_-]{1,64}$");
    }
  }
}
