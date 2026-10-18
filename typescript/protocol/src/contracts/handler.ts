/**
 * The `arete.protocol-handler/v1` wire ABI: the input a protocol handler
 * receives for a step and the operation it returns. Mirrors
 * `arete-hash/src/protocol/handler.rs`.
 *
 * A handler returns portable instructions only. It never returns signers,
 * fees, memos or compiled messages; trusted code adds those after
 * validating the operation against the step's declarations.
 */

import { PROTOCOL_HANDLER_ABI_V1, type PreparedOperationV1 } from "./types.js";
import {
  Obj,
  asArray,
  asBool,
  asStr,
  decodeBase64,
  err,
  identifierValue,
  index,
  pubkey,
  rejectFloats,
  schemaTag,
} from "./shape.js";
import { parseContractJson } from "./json.js";

export const MAX_OPERATION_TRANSACTIONS = 8;
export const MAX_TRANSACTION_INSTRUCTIONS = 64;
export const MAX_INSTRUCTION_ACCOUNTS = 256;
export const MAX_INSTRUCTION_DATA_BYTES = 10 * 1024;
export const MAX_HANDLER_RESULT_BYTES = 2 * 1024 * 1024;

/** Validate a handler result's bytes: size bound first, then strict JSON, then shape. */
export function parsePreparedOperation(bytes: Uint8Array): PreparedOperationV1 {
  if (bytes.length > MAX_HANDLER_RESULT_BYTES) {
    err("string-too-long", "", `handler results are at most ${MAX_HANDLER_RESULT_BYTES} bytes`);
  }
  return validatePreparedOperation(parseContractJson(bytes));
}

/**
 * Validate a handler result's shape and size bounds. Program, account and
 * signer policy is checked afterwards against the step by trusted code
 * (`wirePreparedOperation` gives authors the same feedback early).
 */
export function validatePreparedOperation(value: unknown): PreparedOperationV1 {
  rejectFloats(value, "");
  const root = new Obj(value, "", ["abi", "transactions"]);
  schemaTag(root.req("abi"), "/abi", PROTOCOL_HANDLER_ABI_V1);
  const transactions = asArray(root.req("transactions"), "/transactions", MAX_OPERATION_TRANSACTIONS);
  if (transactions.length === 0) {
    err("invalid-value", "/transactions", "an operation has at least one transaction");
  }
  transactions.forEach((item, i) => {
    const path = index("/transactions", i);
    const transaction = new Obj(item, path, ["name", "instructions"]);
    identifierValue(transaction.req("name"), transaction.pathOf("name"));
    const instructionsPath = transaction.pathOf("instructions");
    const instructions = asArray(transaction.req("instructions"), instructionsPath, MAX_TRANSACTION_INSTRUCTIONS);
    if (instructions.length === 0) {
      err("invalid-value", instructionsPath, "a transaction has at least one instruction");
    }
    instructions.forEach((entry, j) => {
      const instructionPath = index(instructionsPath, j);
      const instruction = new Obj(entry, instructionPath, ["programId", "keys", "dataBase64"]);
      pubkey(instruction.req("programId"), instruction.pathOf("programId"));
      const keysPath = instruction.pathOf("keys");
      asArray(instruction.req("keys"), keysPath, MAX_INSTRUCTION_ACCOUNTS).forEach((meta, k) => {
        const key = new Obj(meta, index(keysPath, k), ["pubkey", "isSigner", "isWritable"]);
        pubkey(key.req("pubkey"), key.pathOf("pubkey"));
        asBool(key.req("isSigner"), key.pathOf("isSigner"));
        asBool(key.req("isWritable"), key.pathOf("isWritable"));
      });
      const dataPath = instruction.pathOf("dataBase64");
      const data = asStr(instruction.req("dataBase64"), dataPath);
      const bytes = decodeBase64(data);
      if (bytes === undefined) err("invalid-value", dataPath, "expected canonical base64");
      if (bytes.length > MAX_INSTRUCTION_DATA_BYTES) {
        err("string-too-long", dataPath, "instruction data is too large");
      }
    });
  });
  return value as PreparedOperationV1;
}
