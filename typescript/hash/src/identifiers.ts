import { hashError, HashError } from "./error.js";

declare const programReadBindingIdBrand: unique symbol;
declare const decoderBindingIdBrand: unique symbol;
declare const decoderEngineIdBrand: unique symbol;

const urlSafeIdentifierSuffix = /^[A-Za-z0-9_-]{32}$/;

export type ProgramReadBindingId = string & {
  readonly [programReadBindingIdBrand]: true;
};

// Internal tooling types: deliberately not re-exported by the package root.
export type DecoderBindingId = string & {
  readonly [decoderBindingIdBrand]: true;
};
export type DecoderEngineId = string & {
  readonly [decoderEngineIdBrand]: true;
};

export function parseProgramReadBindingId(value: string): ProgramReadBindingId {
  return parsePrefixedIdentifier(value, "prb_", "program read binding") as ProgramReadBindingId;
}

export function isProgramReadBindingId(value: unknown): value is ProgramReadBindingId {
  if (typeof value !== "string") return false;
  try {
    parseProgramReadBindingId(value);
    return true;
  } catch (error) {
    if (error instanceof HashError) return false;
    throw error;
  }
}

export function parseDecoderBindingId(value: string): DecoderBindingId {
  return parsePrefixedIdentifier(value, "dec_", "decoder binding") as DecoderBindingId;
}

export function parseDecoderEngineId(value: string): DecoderEngineId {
  if (value.length === 0 || new TextEncoder().encode(value).length > 128) {
    return hashError(
      "invalid-projection",
      "invalid decoder engine projection: identifier must contain between 1 and 128 bytes",
    );
  }
  return value as DecoderEngineId;
}

function parsePrefixedIdentifier(value: string, prefix: string, projection: string): string {
  if (!value.startsWith(prefix)) {
    return hashError(
      "invalid-projection",
      `invalid ${projection} projection: identifier must begin with '${prefix}'`,
    );
  }
  if (!urlSafeIdentifierSuffix.test(value.slice(prefix.length))) {
    return hashError(
      "invalid-projection",
      `invalid ${projection} projection: identifier suffix must contain exactly 32 URL-safe characters`,
    );
  }
  return value;
}
