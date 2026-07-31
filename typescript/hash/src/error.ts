export const HASH_ERROR_CODES = [
  "invalid-hash-id",
  "unknown-version",
  "unknown-kind",
  "unexpected-kind",
  "unknown-algorithm",
  "profile-mismatch",
  "invalid-json",
  "duplicate-json-key",
  "unsafe-json-integer",
  "non-finite-number",
  "duplicate-tuple-label",
  "invalid-artifact-path",
  "duplicate-artifact-path",
  "symlink-artifact",
  "invalid-self-hash-projection",
  "invalid-projection",
  "missing-program-id",
  "invalid-program-id-location",
  "conflicting-program-ids",
  "invalid-idl",
  "serialization",
] as const;

export type HashErrorCode = (typeof HASH_ERROR_CODES)[number];

export class HashError extends Error {
  readonly code: HashErrorCode;

  constructor(code: HashErrorCode, message: string) {
    super(message);
    this.name = "HashError";
    this.code = code;
  }
}

export function hashError(code: HashErrorCode, message: string): never {
  throw new HashError(code, message);
}
