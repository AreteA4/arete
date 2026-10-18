/**
 * A small structural walker over JSON values.
 *
 * This mirrors `arete-hash/src/protocol/shape.rs` exactly. Each contract
 * validator checks a document with these helpers before it reads it as a
 * typed model: objects reject their lexicographically first unknown key
 * (raw UTF-8 byte order, as Rust sorts `String`s) before anything else, then
 * declared fields are checked in the same order as Rust, so both languages
 * report the same first error for a document with one defect.
 */

import { contractError, type ProtocolErrorCode } from "./errors.js";

export const MAX_IDENTIFIER_BYTES = 64;
export const MAX_DESCRIPTION_BYTES = 1024;
export const MAX_EXPRESSION_BYTES = 2048;
export const MAX_SAFE = 9_007_199_254_740_991;

const encoder = new TextEncoder();

export type JsonObject = Record<string, unknown>;

export function err(code: ProtocolErrorCode, path: string, message: string): never {
  throw contractError(code, path, message);
}

export function child(path: string, key: string): string {
  return `${path}/${key.replace(/~/g, "~0").replace(/\//g, "~1")}`;
}

export function index(path: string, position: number): string {
  return `${path}/${position}`;
}

export function byteLength(text: string): number {
  return encoder.encode(text).length;
}

/** Compare two strings by their UTF-8 bytes (Unicode code point order). */
export function compareUtf8(left: string, right: string): number {
  const a = Array.from(left, (c) => c.codePointAt(0)!);
  const b = Array.from(right, (c) => c.codePointAt(0)!);
  const length = Math.min(a.length, b.length);
  for (let i = 0; i < length; i += 1) {
    if (a[i]! !== b[i]!) return a[i]! < b[i]! ? -1 : 1;
  }
  return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
}

export function sortedKeys(object: JsonObject): string[] {
  return Object.keys(object).sort(compareUtf8);
}

export function isObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasOwn(object: JsonObject, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(object, key);
}

/** Rust `char::is_control`: the Unicode `Cc` category. */
export function isControl(codePoint: number): boolean {
  return codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f);
}

/** Rust `char::is_whitespace`: the Unicode `White_Space` property. */
export function isRustWhitespace(codePoint: number): boolean {
  return (
    (codePoint >= 0x09 && codePoint <= 0x0d) ||
    codePoint === 0x20 ||
    codePoint === 0x85 ||
    codePoint === 0xa0 ||
    codePoint === 0x1680 ||
    (codePoint >= 0x2000 && codePoint <= 0x200a) ||
    codePoint === 0x2028 ||
    codePoint === 0x2029 ||
    codePoint === 0x202f ||
    codePoint === 0x205f ||
    codePoint === 0x3000
  );
}

function hasControl(text: string): boolean {
  for (const character of text) {
    const codePoint = character.codePointAt(0)!;
    if (isControl(codePoint) && codePoint !== 0x0a) return true;
  }
  return false;
}

/**
 * Reject floats anywhere in a document. Contracts carry integer base units
 * as decimal strings and small counts as safe JSON integers only.
 *
 * Values that are not JSON at all (`undefined`, functions, bigints, class
 * instances) are reported as `invalid-type` at the same point.
 */
export function rejectFloats(value: unknown, path: string): void {
  if (value === null || typeof value === "string" || typeof value === "boolean") return;
  if (typeof value === "number") {
    if (!Number.isInteger(value)) {
      err("float-not-allowed", path, "floating-point numbers are not allowed");
    }
    return;
  }
  if (value instanceof FloatToken) {
    err("float-not-allowed", path, "floating-point numbers are not allowed");
  }
  if (Array.isArray(value)) {
    value.forEach((item, i) => rejectFloats(item, index(path, i)));
    return;
  }
  if (isObject(value)) {
    const prototype = Object.getPrototypeOf(value) as object | null;
    if (prototype !== Object.prototype && prototype !== null) {
      err("invalid-type", path, "expected a JSON value");
    }
    for (const key of sortedKeys(value)) rejectFloats(value[key], child(path, key));
    return;
  }
  err("invalid-type", path, "expected a JSON value");
}

/**
 * Marks a JSON number token written with a fraction or exponent (`1.0`,
 * `1e3`). JavaScript parses those to integers, but Rust parses them as
 * floats and rejects them, so `parseContractJson` keeps the distinction.
 */
export class FloatToken {
  constructor(readonly token: string) {}
}

export class Obj {
  readonly path: string;
  private readonly map: JsonObject;

  /** Open an object, rejecting the lexicographically first key that is not in `allowed`. */
  constructor(value: unknown, path: string, allowed: readonly string[]) {
    if (!isObject(value)) err("invalid-type", path, "expected an object");
    const unknown = sortedKeys(value).find((key) => !allowed.includes(key));
    if (unknown !== undefined) {
      err("unknown-field", child(path, unknown), `unknown field '${unknown}'`);
    }
    this.path = path;
    this.map = value;
  }

  pathOf(key: string): string {
    return child(this.path, key);
  }

  /** A required, non-null field. */
  req(key: string): unknown {
    if (!hasOwn(this.map, key)) err("missing-field", this.pathOf(key), `missing field '${key}'`);
    const value = this.map[key];
    if (value === null) {
      err("null-not-allowed", this.pathOf(key), `field '${key}' must not be null`);
    }
    return value;
  }

  /** An optional field. Absent means "not set"; `null` is never accepted. */
  opt(key: string): unknown | undefined {
    if (!hasOwn(this.map, key)) return undefined;
    const value = this.map[key];
    if (value === null) {
      err(
        "null-not-allowed",
        this.pathOf(key),
        `optional field '${key}' must be omitted, not null`,
      );
    }
    return value;
  }

  /** A field that must be present and may be `null` (returned as `undefined`). */
  nullable(key: string): unknown | undefined {
    if (!hasOwn(this.map, key)) err("missing-field", this.pathOf(key), `missing field '${key}'`);
    const value = this.map[key];
    return value === null ? undefined : value;
  }

  has(key: string): boolean {
    return hasOwn(this.map, key);
  }
}

export function asStr(value: unknown, path: string): string {
  if (typeof value !== "string") err("invalid-type", path, "expected a string");
  return value;
}

export function asBool(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") err("invalid-type", path, "expected a boolean");
  return value;
}

export function asArray(value: unknown, path: string, max: number): unknown[] {
  if (!Array.isArray(value)) err("invalid-type", path, "expected an array");
  if (value.length > max) err("array-too-large", path, `at most ${max} entries are allowed`);
  return value;
}

export type KeyCheck = (key: string, path: string) => void;

/** A map whose keys are validated with `keyCheck`, returned in sorted order. */
export function asMap(
  value: unknown,
  path: string,
  max: number,
  keyCheck: KeyCheck,
): Array<[string, unknown, string]> {
  if (!isObject(value)) err("invalid-type", path, "expected an object");
  const keys = sortedKeys(value);
  if (keys.length > max) err("map-too-large", path, `at most ${max} entries are allowed`);
  return keys.map((key) => {
    const entryPath = child(path, key);
    keyCheck(key, entryPath);
    return [key, value[key], entryPath];
  });
}

export function asInteger(value: unknown, path: string, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isInteger(value)) {
    err("invalid-type", path, "expected an integer");
  }
  if (value < min || value > max) {
    err("integer-out-of-range", path, `expected an integer in ${min}..=${max}`);
  }
  return value;
}

export function boundedStr(value: unknown, path: string, maxBytes: number): string {
  const text = asStr(value, path);
  if (byteLength(text) > maxBytes) {
    err("string-too-long", path, `at most ${maxBytes} bytes are allowed`);
  }
  return text;
}

export function description(value: unknown, path: string): void {
  const text = boundedStr(value, path, MAX_DESCRIPTION_BYTES);
  if (hasControl(text)) err("invalid-value", path, "control characters are not allowed");
}

export function oneOf<T extends string>(value: unknown, path: string, allowed: readonly T[]): T {
  const text = asStr(value, path);
  if (!(allowed as readonly string[]).includes(text)) {
    err("invalid-value", path, `expected one of ${allowed.join(", ")}`);
  }
  return text as T;
}

export function schemaTag(value: unknown, path: string, expected: string): void {
  const text = asStr(value, path);
  if (text !== expected) err("invalid-schema-tag", path, `expected '${expected}'`);
}

const isLower = (c: number) => c >= 0x61 && c <= 0x7a;
const isUpper = (c: number) => c >= 0x41 && c <= 0x5a;
const isDigit = (c: number) => c >= 0x30 && c <= 0x39;
const isAlpha = (c: number) => isLower(c) || isUpper(c);
const isAlnum = (c: number) => isAlpha(c) || isDigit(c);
const isWord = (c: number) => isAlnum(c) || c === 0x5f;
const isSnake = (c: number) => isLower(c) || isDigit(c) || c === 0x5f;

function asciiMatches(
  text: string,
  first: (c: number) => boolean,
  rest: (c: number) => boolean,
): boolean {
  // Every accepted character is ASCII, so UTF-16 length equals byte length.
  if (text.length === 0 || text.length > MAX_IDENTIFIER_BYTES) return false;
  if (!first(text.charCodeAt(0))) return false;
  for (let i = 1; i < text.length; i += 1) {
    if (!rest(text.charCodeAt(i))) return false;
  }
  return true;
}

/** `^[A-Za-z][A-Za-z0-9_]{0,63}$`. */
export function identifier(text: string, path: string): void {
  if (!asciiMatches(text, isAlpha, isWord)) {
    err("invalid-identifier", path, `'${text}' must match ^[A-Za-z][A-Za-z0-9_]{0,63}$`);
  }
}

export function isIdentifier(text: string): boolean {
  return asciiMatches(text, isAlpha, isWord);
}

/** `^[a-z][A-Za-z0-9]{0,63}$`: the protocol name. */
export function protocolName(text: string, path: string): void {
  if (!asciiMatches(text, isLower, isAlnum)) {
    err("invalid-identifier", path, `'${text}' must match ^[a-z][A-Za-z0-9]{0,63}$`);
  }
}

/** `^[a-z][a-z0-9_]{0,63}$`: machine states and guard codes. */
export function snakeIdentifier(text: string, path: string): void {
  if (!asciiMatches(text, isLower, isSnake)) {
    err("invalid-identifier", path, `'${text}' must match ^[a-z][a-z0-9_]{0,63}$`);
  }
}

/** Dotted event names: one to four `[a-z][A-Za-z0-9_]*` segments, at most 64 bytes. */
export function eventName(text: string, path: string): void {
  const segments = text.split(".");
  const valid =
    text.length > 0 &&
    byteLength(text) <= MAX_IDENTIFIER_BYTES &&
    segments.length <= 4 &&
    segments.every(
      (segment) =>
        segment.length > 0 &&
        isLower(segment.charCodeAt(0)) &&
        Array.from(segment.slice(1)).every((c) => c.length === 1 && isWord(c.charCodeAt(0))),
    );
  if (!valid) err("invalid-identifier", path, `'${text}' must be dotted lower-camel segments`);
}

export const keyIdentifier: KeyCheck = (key, path) => identifier(key, path);
export const keySnake: KeyCheck = (key, path) => snakeIdentifier(key, path);
export const keyEvent: KeyCheck = (key, path) => eventName(key, path);

export function identifierValue(value: unknown, path: string): string {
  const text = asStr(value, path);
  identifier(text, path);
  return text;
}

export function snakeValue(value: unknown, path: string): string {
  const text = asStr(value, path);
  snakeIdentifier(text, path);
  return text;
}

/**
 * An expression source string. Contracts only bound its size and reject
 * control characters; the platform's expression checker parses and
 * type-checks it.
 */
export function expression(value: unknown, path: string): string {
  const text = asStr(value, path);
  if (Array.from(text).every((c) => isRustWhitespace(c.codePointAt(0)!))) {
    err("invalid-expression", path, "expressions must not be empty");
  }
  if (byteLength(text) > MAX_EXPRESSION_BYTES) {
    err("string-too-long", path, `expressions are at most ${MAX_EXPRESSION_BYTES} bytes`);
  }
  if (hasControl(text)) {
    err("invalid-expression", path, "control characters are not allowed in expressions");
  }
  return text;
}

/** `^(0|[1-9][0-9]*)$` with an upper bound. */
export function decimal(value: unknown, path: string, max: bigint): bigint {
  const text = asStr(value, path);
  const wellFormed =
    text.length > 0 &&
    text.length <= 39 &&
    /^[0-9]+$/.test(text) &&
    (text === "0" || !text.startsWith("0"));
  if (wellFormed) {
    const parsed = BigInt(text);
    if (parsed <= U128_MAX && parsed <= max) return parsed;
  }
  return err("invalid-decimal", path, `expected a canonical decimal string at most ${max}`);
}

export const U64_MAX = (1n << 64n) - 1n;
export const U128_MAX = (1n << 128n) - 1n;

const BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/** Decode base58 the way the Rust `bs58` crate does; `undefined` on a bad character. */
export function decodeBase58(text: string): Uint8Array | undefined {
  let numeric = 0n;
  for (const character of text) {
    const digit = BASE58_ALPHABET.indexOf(character);
    if (digit < 0) return undefined;
    numeric = numeric * 58n + BigInt(digit);
  }
  const suffix: number[] = [];
  while (numeric > 0n) {
    suffix.push(Number(numeric & 0xffn));
    numeric >>= 8n;
  }
  suffix.reverse();
  let leadingZeroes = 0;
  while (leadingZeroes < text.length && text[leadingZeroes] === "1") leadingZeroes += 1;
  const out = new Uint8Array(leadingZeroes + suffix.length);
  out.set(suffix, leadingZeroes);
  return out;
}

export function isPubkey(text: string): boolean {
  const decoded = decodeBase58(text);
  return decoded !== undefined && decoded.length === 32 && byteLength(text) <= 44;
}

export function pubkey(value: unknown, path: string): string {
  const text = asStr(value, path);
  if (!isPubkey(text)) err("invalid-pubkey", path, "expected a base58 32-byte public key");
  return text;
}

/** A lowercase hex SHA-256 digest. */
export function sha256Hex(value: unknown, path: string): string {
  const text = asStr(value, path);
  if (!/^[0-9a-f]{64}$/.test(text)) {
    err("invalid-value", path, "expected a lowercase hex SHA-256 digest");
  }
  return text;
}

/** `MAJOR.MINOR.PATCH` with an optional pre-release; no build metadata. */
export function semver(value: unknown, path: string): string {
  const text = asStr(value, path);
  const dash = text.indexOf("-");
  const core = dash < 0 ? text : text.slice(0, dash);
  const pre = dash < 0 ? undefined : text.slice(dash + 1);
  const numeric = (part: string) =>
    part.length > 0 && /^[0-9]+$/.test(part) && (part === "0" || !part.startsWith("0"));
  const parts = core.split(".");
  const preOk =
    pre === undefined ||
    (pre.length > 0 &&
      pre.split(".").every((id) => id.length > 0 && /^[A-Za-z0-9-]+$/.test(id)));
  if (!(byteLength(text) <= 64 && parts.length === 3 && parts.every(numeric) && preOk)) {
    err("invalid-semver", path, "expected MAJOR.MINOR.PATCH[-pre]");
  }
  return text;
}

/** A list of unique strings, each validated by `check`. */
export function uniqueStrings(
  value: unknown,
  path: string,
  max: number,
  check: (value: unknown, path: string) => string,
): string[] {
  const values = asArray(value, path, max);
  const seen = new Set<string>();
  return values.map((item, i) => {
    const itemPath = index(path, i);
    const text = check(item, itemPath);
    if (seen.has(text)) err("duplicate-entry", itemPath, `'${text}' is listed more than once`);
    seen.add(text);
    return text;
  });
}

/** Read a `kind` tag, then open the object with the keys that kind allows. */
export function tagged(
  value: unknown,
  path: string,
  kinds: ReadonlyArray<readonly [string, readonly string[]]>,
): [string, Obj] {
  if (!isObject(value)) err("invalid-type", path, "expected an object");
  const kindPath = child(path, "kind");
  if (!Object.prototype.hasOwnProperty.call(value, "kind")) {
    err("missing-field", kindPath, "missing field 'kind'");
  }
  if (value.kind === null) err("null-not-allowed", kindPath, "field 'kind' must not be null");
  const kind = asStr(value.kind, kindPath);
  const match = kinds.find(([name]) => name === kind);
  if (!match) {
    err("invalid-value", kindPath, `expected one of ${kinds.map(([name]) => name).join(", ")}`);
  }
  return [match[0], new Obj(value, path, ["kind", ...match[1]])];
}

/**
 * Validate an array whose entries carry a key that must be strictly
 * increasing (sorted by raw bytes and unique).
 */
export function sortedEntries(
  value: unknown,
  path: string,
  entry: (value: unknown, path: string) => string,
  max = 64,
): void {
  const values = asArray(value, path, max);
  let previous: string | undefined;
  values.forEach((item, i) => {
    const itemPath = index(path, i);
    const key = entry(item, itemPath);
    if (previous !== undefined) {
      if (key === previous) err("duplicate-entry", itemPath, `'${key}' is listed twice`);
      if (compareUtf8(key, previous) < 0) err("unsorted-entries", itemPath, "entries must be sorted");
    }
    previous = key;
  });
}

const BASE64_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** Strict standard base64 with padding; `undefined` for any non-canonical encoding. */
export function decodeBase64(text: string): Uint8Array | undefined {
  if (text.length % 4 !== 0) return undefined;
  const out: number[] = [];
  const chunks = text.length / 4;
  for (let chunk = 0; chunk < chunks; chunk += 1) {
    const last = chunk === chunks - 1;
    const values = [0, 0, 0, 0];
    let padding = 0;
    for (let i = 0; i < 4; i += 1) {
      const character = text[chunk * 4 + i]!;
      if (character === "=") {
        if (!last || i < 2) return undefined;
        padding += 1;
        values[i] = 0;
      } else {
        if (padding > 0) return undefined;
        const position = BASE64_ALPHABET.indexOf(character);
        if (position < 0) return undefined;
        values[i] = position;
      }
    }
    const n = (values[0]! << 18) | (values[1]! << 12) | (values[2]! << 6) | values[3]!;
    out.push((n >> 16) & 0xff);
    if (padding < 2) out.push((n >> 8) & 0xff);
    if (padding < 1) out.push(n & 0xff);
    if ((padding === 1 && (values[2]! & 0b11) !== 0) || (padding === 2 && (values[1]! & 0b1111) !== 0)) {
      return undefined;
    }
  }
  return Uint8Array.from(out);
}

export function encodeBase64(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i]!;
    const b = bytes[i + 1];
    const c = bytes[i + 2];
    const n = (a << 16) | ((b ?? 0) << 8) | (c ?? 0);
    out += BASE64_ALPHABET[(n >> 18) & 63];
    out += BASE64_ALPHABET[(n >> 12) & 63];
    out += b === undefined ? "=" : BASE64_ALPHABET[(n >> 6) & 63];
    out += c === undefined ? "=" : BASE64_ALPHABET[n & 63];
  }
  return out;
}
