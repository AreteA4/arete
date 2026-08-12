import { sha256 } from "@noble/hashes/sha256";

import { isCanonicalBase58_32 } from "./base58.js";
import { canonicalizeJcs, parseJsonBytesStrict } from "./canonical.js";
import { hashError } from "./error.js";
import { bytesToHex, hashCanonicalPayload } from "./hash.js";
import type {
  DecoderFixtureSetHash,
  IdlNormalizedHash,
  JsonValue,
} from "./types.js";

export const DECODER_FIXTURE_SCHEMA_V2 = "arete.decoder-fixtures/v2" as const;
export const DECODER_FIXTURE_PUBLIC_VALUE_DIGEST_PREFIX = "sha256:" as const;
export const DECODER_FIXTURE_MAX_CASES = 256;
export const DECODER_FIXTURE_MAX_ACCOUNT_BYTES = 1024 * 1024;
export const DECODER_FIXTURE_MAX_TOTAL_ACCOUNT_BYTES = 8 * 1024 * 1024;
export const DECODER_FIXTURE_ACCOUNT_DECODE_ERROR_CATEGORIES = [
  "owner_mismatch",
  "unknown_account_type",
  "account_type_mismatch",
  "ambiguous_account_type",
  "account_decode_failed",
] as const;

export type DecoderFixturePublicValueDigest = `sha256:${string}`;
export type DecoderFixtureAccountDecodeErrorCategory =
  (typeof DECODER_FIXTURE_ACCOUNT_DECODE_ERROR_CATEGORIES)[number];

export interface DecoderFixtureSetV2 {
  readonly schema: typeof DECODER_FIXTURE_SCHEMA_V2;
  readonly programId: string;
  readonly normalizedIdlHash: IdlNormalizedHash;
  readonly decoderEngineId: string;
  readonly decoderAbiVersion: string;
  readonly cases: readonly DecoderFixtureCaseV2[];
}

export interface DecoderFixtureCaseV2 {
  readonly id: string;
  readonly accountType: string;
  readonly owner: string;
  readonly accountDataHex: string;
  readonly expected: DecoderFixtureExpectedV2;
  readonly expectedPrivateDiagnostics?: DecoderFixturePrivateDiagnosticsV2;
}

export type DecoderFixtureExpectedV2 =
  | {
      readonly kind: "decoded";
      readonly publicValueDigest: DecoderFixturePublicValueDigest;
    }
  | {
      readonly kind: "error";
      readonly category: DecoderFixtureAccountDecodeErrorCategory;
    };

export interface DecoderFixturePrivateDiagnosticsV2 {
  readonly trailingBytes?: number;
  readonly candidateCount?: number;
}

export function parseDecoderFixtureSetV2(bytes: Uint8Array): DecoderFixtureSetV2 {
  return validateDecoderFixtureSetV2(parseJsonBytesStrict(bytes));
}

export function validateDecoderFixtureSetV2(value: unknown): DecoderFixtureSetV2 {
  const fixture = expectObject(value, "decoder fixture set");
  expectKeys(
    fixture,
    [
      "schema",
      "programId",
      "normalizedIdlHash",
      "decoderEngineId",
      "decoderAbiVersion",
      "cases",
    ],
    "decoder fixture set",
  );
  if (fixture.schema !== DECODER_FIXTURE_SCHEMA_V2) {
    return hashError(
      "unknown-version",
      `unknown decoder fixture schema '${String(fixture.schema)}'`,
    );
  }
  const programId = expectPubkey(fixture.programId, "programId");
  const normalizedIdlHash = expectNormalizedIdlHash(fixture.normalizedIdlHash);
  const decoderEngineId = expectIdentifier(fixture.decoderEngineId, "decoderEngineId", 128);
  const decoderAbiVersion = expectIdentifier(fixture.decoderAbiVersion, "decoderAbiVersion", 64);
  if (
    !Array.isArray(fixture.cases) ||
    fixture.cases.length === 0 ||
    fixture.cases.length > DECODER_FIXTURE_MAX_CASES
  ) {
    return invalid(`cases must contain between 1 and ${DECODER_FIXTURE_MAX_CASES} entries`);
  }

  const ids = new Set<string>();
  let totalBytes = 0;
  const cases: DecoderFixtureCaseV2[] = fixture.cases.map(
    (caseValue, index): DecoderFixtureCaseV2 => {
      const item = expectObject(caseValue, `cases[${index}]`);
      expectKeys(
        item,
        [
          "id",
          "accountType",
          "owner",
          "accountDataHex",
          "expected",
          "expectedPrivateDiagnostics",
        ],
        `cases[${index}]`,
        ["expectedPrivateDiagnostics"],
      );
      const id = expectStableId(item.id, `cases[${index}].id`, 128);
      if (ids.has(id)) return invalid(`case id '${id}' is duplicated`);
      ids.add(id);
      const accountType = expectIdentifier(
        item.accountType,
        `case '${id}' accountType`,
        128,
      );
      const owner = expectPubkey(item.owner, `case '${id}' owner`);
      const accountDataHex = expectAccountDataHex(item.accountDataHex, id);
      const accountBytes = accountDataHex.length / 2;
      if (accountBytes > DECODER_FIXTURE_MAX_ACCOUNT_BYTES) {
        return invalid(
          `case '${id}' accountDataHex exceeds ${DECODER_FIXTURE_MAX_ACCOUNT_BYTES} bytes`,
        );
      }
      totalBytes += accountBytes;
      if (totalBytes > DECODER_FIXTURE_MAX_TOTAL_ACCOUNT_BYTES) {
        return invalid(
          `fixture accountDataHex exceeds ${DECODER_FIXTURE_MAX_TOTAL_ACCOUNT_BYTES} total bytes`,
        );
      }

      const expected = validateExpected(item.expected, id);
      const expectedPrivateDiagnostics =
        item.expectedPrivateDiagnostics === undefined
          ? undefined
          : validatePrivateDiagnostics(item.expectedPrivateDiagnostics, id);
      return {
        id,
        accountType,
        owner,
        accountDataHex,
        expected,
        ...(expectedPrivateDiagnostics === undefined ? {} : { expectedPrivateDiagnostics }),
      };
    },
  );

  cases.sort((left, right) => compareUtf8(left.id, right.id));
  return {
    schema: DECODER_FIXTURE_SCHEMA_V2,
    programId,
    normalizedIdlHash,
    decoderEngineId,
    decoderAbiVersion,
    cases,
  };
}

export function hashDecoderFixtureSetV2(value: unknown): DecoderFixtureSetHash {
  const projection = validateDecoderFixtureSetV2(value);
  return hashCanonicalPayload(
    "decoder-fixture-set",
    "arete-jcs-v1",
    canonicalizeJcs(projection as unknown as JsonValue),
  );
}

export function digestDecoderFixturePublicValueV2(
  value: JsonValue,
): DecoderFixturePublicValueDigest {
  return `${DECODER_FIXTURE_PUBLIC_VALUE_DIGEST_PREFIX}${bytesToHex(
    sha256(canonicalizeJcs(value)),
  )}`;
}

function validateExpected(value: unknown, id: string): DecoderFixtureExpectedV2 {
  const expected = expectObject(value, `case '${id}' expected`);
  if (expected.kind === "decoded") {
    expectKeys(expected, ["kind", "publicValueDigest"], `case '${id}' expected`);
    return {
      kind: "decoded",
      publicValueDigest: expectPublicValueDigest(expected.publicValueDigest),
    };
  }
  if (expected.kind === "error") {
    expectKeys(expected, ["kind", "category"], `case '${id}' expected`);
    return {
      kind: "error",
      category: expectAccountDecodeErrorCategory(expected.category, id),
    };
  }
  return invalid(`case '${id}' expected.kind must be 'decoded' or 'error'`);
}

function validatePrivateDiagnostics(
  value: unknown,
  id: string,
): DecoderFixturePrivateDiagnosticsV2 {
  const diagnostics = expectObject(value, `case '${id}' expectedPrivateDiagnostics`);
  expectKeys(
    diagnostics,
    ["trailingBytes", "candidateCount"],
    `case '${id}' expectedPrivateDiagnostics`,
    ["trailingBytes", "candidateCount"],
  );
  if (Object.keys(diagnostics).length === 0) {
    return invalid(`case '${id}' expectedPrivateDiagnostics must not be empty`);
  }
  const trailingBytes = diagnostics.trailingBytes === undefined
    ? undefined
    : expectUint32(diagnostics.trailingBytes, `case '${id}' trailingBytes`, true);
  const candidateCount = diagnostics.candidateCount === undefined
    ? undefined
    : expectUint32(diagnostics.candidateCount, `case '${id}' candidateCount`, false);
  return {
    ...(trailingBytes === undefined ? {} : { trailingBytes }),
    ...(candidateCount === undefined ? {} : { candidateCount }),
  };
}

function expectAccountDecodeErrorCategory(
  value: unknown,
  id: string,
): DecoderFixtureAccountDecodeErrorCategory {
  if (
    typeof value !== "string" ||
    !(DECODER_FIXTURE_ACCOUNT_DECODE_ERROR_CATEGORIES as readonly string[]).includes(value)
  ) {
    return invalid(`case '${id}' error category is not a public AccountDecodeErrorCategory`);
  }
  return value as DecoderFixtureAccountDecodeErrorCategory;
}

function expectNormalizedIdlHash(value: unknown): IdlNormalizedHash {
  if (
    typeof value !== "string" ||
    !/^arete:h1:idl-normalized:sha256:[0-9a-f]{64}$/.test(value)
  ) {
    return invalid("normalizedIdlHash must be an idl-normalized typed hash");
  }
  return value as IdlNormalizedHash;
}

function expectPublicValueDigest(value: unknown): DecoderFixturePublicValueDigest {
  if (typeof value !== "string" || !/^sha256:[0-9a-f]{64}$/.test(value)) {
    return invalid("publicValueDigest must use the sha256:<lowercase-hex> format");
  }
  return value as DecoderFixturePublicValueDigest;
}

function expectAccountDataHex(value: unknown, id: string): string {
  if (
    typeof value !== "string" ||
    value.length % 2 !== 0 ||
    !/^[0-9a-f]*$/.test(value)
  ) {
    return invalid(`case '${id}' accountDataHex must contain lowercase hexadecimal byte pairs`);
  }
  return value;
}

function expectPubkey(value: unknown, field: string): string {
  if (!isCanonicalBase58_32(value)) {
    return invalid(`${field} must be a base58 Solana public key`);
  }
  return value;
}

function expectIdentifier(value: unknown, field: string, maxLength: number): string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.trim() !== value ||
    byteLength(value) > maxLength
  ) {
    return invalid(`${field} must be a nonempty, trimmed string of at most ${maxLength} bytes`);
  }
  return value;
}

function expectStableId(value: unknown, field: string, maxLength: number): string {
  if (
    typeof value !== "string" ||
    byteLength(value) > maxLength ||
    !/^[a-z0-9][a-z0-9._-]*$/.test(value)
  ) {
    return invalid(`${field} must be a lowercase stable identifier of at most ${maxLength} bytes`);
  }
  return value;
}

function expectUint32(value: unknown, field: string, allowZero: boolean): number {
  if (
    !Number.isInteger(value) ||
    (value as number) < (allowZero ? 0 : 1) ||
    (value as number) > 0xffff_ffff
  ) {
    return invalid(`${field} must be ${allowZero ? "a" : "a nonzero"} uint32`);
  }
  return value as number;
}

function expectObject(value: unknown, field: string): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return invalid(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function expectKeys(
  value: Record<string, unknown>,
  keys: readonly string[],
  field: string,
  optional: readonly string[] = [],
): void {
  const allowed = new Set(keys);
  const optionalKeys = new Set(optional);
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) invalid(`${field} contains unknown field '${key}'`);
  }
  for (const key of keys) {
    if (!optionalKeys.has(key) && !Object.hasOwn(value, key)) {
      invalid(`${field} is missing '${key}'`);
    }
  }
}

function compareUtf8(left: string, right: string): number {
  const leftBytes = new TextEncoder().encode(left);
  const rightBytes = new TextEncoder().encode(right);
  const length = Math.min(leftBytes.length, rightBytes.length);
  for (let index = 0; index < length; index += 1) {
    if (leftBytes[index] !== rightBytes[index]) return leftBytes[index]! - rightBytes[index]!;
  }
  return leftBytes.length - rightBytes.length;
}

function byteLength(value: string): number {
  return new TextEncoder().encode(value).length;
}

function invalid<T>(reason: string): T {
  return hashError("invalid-projection", `invalid decoder fixture set projection: ${reason}`);
}
