/**
 * Public contracts for Arete Protocols.
 *
 * Every contract is camelCase JSON that rejects unknown fields, floats and
 * `null` for optional fields, is bounded, and is hashed with the Arete JCS
 * profile under its own hash kind. Rust (`arete-hash::protocol`) and this
 * package run the shared vectors in `test-vectors/protocol-v1.json` and
 * must agree on every hash and every error code.
 */

import { HashError, hashJcs, hashRawBytes, type JcsHashKind, type ProtocolHandlerBundleHash } from "@usearete/hash";

import { ProtocolContractError, contractError } from "./errors.js";
import { validatePreparedOperation } from "./handler.js";
import { validateExecutionIntent, validateProviderSnapshot } from "./intent.js";
import { parseContractJson } from "./json.js";
import { validateProtocolExecutionReceipt, validateProtocolReceiptKeySet } from "./receipt.js";
import {
  validateProtocolBuildReceipt,
  validateProtocolRelease,
  validateProtocolReleaseReview,
  validateProtocolResource,
  validateProtocolStackSource,
} from "./release.js";
import { validateProtocolSpec } from "./spec.js";

export const PROTOCOL_CONTRACT_KINDS = [
  "protocol-spec",
  "protocol-stack-source",
  "protocol-release",
  "protocol-release-review",
  "protocol-build-receipt",
  "protocol-resource",
  "provider-snapshot",
  "execution-intent",
  "prepared-operation",
  "protocol-execution-receipt",
  "protocol-receipt-keys",
] as const;

/** A contract kind that the shared vectors exercise. */
export type ProtocolContractKind = (typeof PROTOCOL_CONTRACT_KINDS)[number];

const HASH_KINDS: Partial<Record<ProtocolContractKind, JcsHashKind>> = {
  "protocol-spec": "protocol-spec",
  "protocol-stack-source": "protocol-stack-source",
  "protocol-release": "protocol-release",
  "protocol-release-review": "protocol-release-review",
  "protocol-resource": "protocol-resource",
  "provider-snapshot": "provider-snapshot",
  "execution-intent": "execution-intent",
};

const VALIDATORS: Record<ProtocolContractKind, (value: unknown) => unknown> = {
  "protocol-spec": validateProtocolSpec,
  "protocol-stack-source": validateProtocolStackSource,
  "protocol-release": validateProtocolRelease,
  "protocol-release-review": validateProtocolReleaseReview,
  "protocol-build-receipt": validateProtocolBuildReceipt,
  "protocol-resource": validateProtocolResource,
  "provider-snapshot": validateProviderSnapshot,
  "execution-intent": validateExecutionIntent,
  "prepared-operation": validatePreparedOperation,
  "protocol-execution-receipt": validateProtocolExecutionReceipt,
  "protocol-receipt-keys": validateProtocolReceiptKeySet,
};

/**
 * Validate a value and return its hash identifier, or `null` for contracts
 * without a hash kind (the build receipt, the prepared operation, the
 * execution receipt and the receipt key set).
 * Throws `ProtocolContractError` on the first defect.
 */
export function validateContract(kind: ProtocolContractKind, value: unknown): string | null {
  const validator = VALIDATORS[kind];
  if (validator === undefined) throw new TypeError(`unknown protocol contract kind '${String(kind)}'`);
  validator(value);
  const hashKind = HASH_KINDS[kind];
  if (hashKind === undefined) return null;
  try {
    return hashJcs(hashKind, value as never);
  } catch (error) {
    if (error instanceof HashError) throw contractError("invalid-value", "", error.message);
    throw error;
  }
}

/** Parse contract JSON bytes strictly, then `validateContract`. */
export function parseContract(kind: ProtocolContractKind, bytes: Uint8Array): string | null {
  return validateContract(kind, parseContractJson(bytes));
}

/** The identity of a handler bundle: its exact ESM bytes. */
export function hashHandlerBundle(bytes: Uint8Array): ProtocolHandlerBundleHash {
  return hashRawBytes("protocol-handler-bundle", bytes);
}

export { ProtocolContractError };
