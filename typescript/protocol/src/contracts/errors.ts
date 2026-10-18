/**
 * Every stable error code a protocol contract validator can return.
 *
 * This list mirrors `PROTOCOL_ERROR_CODES` in `arete-hash/src/protocol/error.rs`
 * exactly, in the same order. `test-vectors/protocol-v1.json` records it as
 * `errorCodes`, and the tests assert the two are equal.
 */
export const PROTOCOL_ERROR_CODES = [
  // JSON well-formedness (shared with the hash profile).
  "invalid-json",
  "duplicate-json-key",
  "unsafe-json-integer",
  "float-not-allowed",
  // Shape.
  "invalid-type",
  "unknown-field",
  "missing-field",
  "null-not-allowed",
  "invalid-schema-tag",
  "invalid-value",
  "invalid-identifier",
  "invalid-decimal",
  "invalid-pubkey",
  "invalid-hash-id",
  "invalid-semver",
  "invalid-expression",
  "invalid-signature",
  "string-too-long",
  "array-too-large",
  "map-too-large",
  "integer-out-of-range",
  "duplicate-entry",
  "unsorted-entries",
  "invalid-schema",
  // References.
  "unknown-program",
  "unknown-read",
  "unknown-state",
  "unknown-step",
  "unknown-event",
  "unknown-role",
  "unknown-ephemeral",
  "unknown-bound-account",
  "unknown-arg",
  // Machines.
  "unreachable-state",
  "dead-end-state",
  "terminal-state-exit",
  "derived-state-without-rule",
  "derive-targets-recorded-state",
  "transition-targets-derived-state",
  "missing-participant-machine",
  // Steps.
  "duplicate-guard-code",
  "reserved-guard-code",
  "create-step-count",
  "invalid-authority",
  "missing-access",
  "forbidden-access",
  "missing-role-guard",
  "role-not-bound",
  "bind-outside-create",
  "create-step-transition",
  "invalid-fee-basis",
  "handler-abi-required",
  "handler-abi-unused",
  "remaining-account-signer",
  // Reactions and events.
  "reaction-action-mismatch",
  "event-not-emitted",
  "event-machine-mismatch",
] as const;

export type ProtocolErrorCode = (typeof PROTOCOL_ERROR_CODES)[number];

/**
 * A contract validation failure with a stable, language-neutral code.
 *
 * `path` is a JSON Pointer (RFC 6901) into the document that failed; the
 * empty string is the document root. Rust and TypeScript report the same
 * `code` and `path` for every vector in `test-vectors/protocol-v1.json`.
 */
export class ProtocolContractError extends Error {
  readonly code: ProtocolErrorCode;
  readonly path: string;

  /** `message` is the bare explanation, as in Rust; `toString()` adds the code and path. */
  constructor(code: ProtocolErrorCode, path: string, message: string) {
    super(message);
    this.name = "ProtocolContractError";
    this.code = code;
    this.path = path;
  }

  override toString(): string {
    return `${this.code} at ${this.path === "" ? "/" : this.path}: ${this.message}`;
  }
}

export function contractError(
  code: ProtocolErrorCode,
  path: string,
  message: string,
): ProtocolContractError {
  return new ProtocolContractError(code, path, message);
}
