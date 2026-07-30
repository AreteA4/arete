import { hashError } from "./error.js";
import { concatBytes, frameBytes, hashCanonicalPayload, u64be } from "./hash.js";
import {
  identityMetadata,
  type HashId,
  type JcsHashKind,
  type JsonValue,
  type RawBytesHashKind,
  type TupleField,
  type TupleHashKind,
} from "./types.js";

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
const maxSafeInteger = BigInt(Number.MAX_SAFE_INTEGER);

export function parseJsonBytesStrict(bytes: Uint8Array): JsonValue {
  let text: string;
  try {
    text = decoder.decode(bytes);
  } catch {
    return hashError("invalid-json", "JSON input is not valid UTF-8");
  }
  return new StrictJsonParser(text).parse();
}

export function canonicalizeJsonBytes(bytes: Uint8Array): Uint8Array {
  return encoder.encode(serializeCanonical(parseJsonBytesStrict(bytes), false));
}

export function canonicalizeJcs(value: JsonValue): Uint8Array {
  return encoder.encode(serializeCanonical(value));
}

export function hashRawBytes<K extends RawBytesHashKind>(
  kind: K,
  bytes: Uint8Array,
): HashId<K> {
  requireProfile(kind, "raw-bytes-v1");
  return hashCanonicalPayload(kind, "raw-bytes-v1", bytes);
}

export function hashJsonBytes<K extends JcsHashKind>(
  kind: K,
  bytes: Uint8Array,
): HashId<K> {
  requireProfile(kind, "arete-jcs-v1");
  return hashCanonicalPayload(kind, "arete-jcs-v1", canonicalizeJsonBytes(bytes));
}

export function hashJcs<K extends JcsHashKind>(kind: K, value: JsonValue): HashId<K> {
  requireProfile(kind, "arete-jcs-v1");
  return hashCanonicalPayload(kind, "arete-jcs-v1", canonicalizeJcs(value));
}

export function framedTuplePayload(fields: readonly TupleField[]): Uint8Array {
  const labels = new Set<string>();
  const output: Uint8Array[] = [u64be(fields.length)];
  for (const field of fields) {
    if (labels.has(field.label)) {
      return hashError("duplicate-tuple-label", `tuple labels must be unique: '${field.label}'`);
    }
    labels.add(field.label);
    output.push(frameBytes(encoder.encode(field.label)), frameBytes(field.value));
  }
  return concatBytes(...output);
}

export function hashFramedTuple<K extends TupleHashKind>(
  kind: K,
  fields: readonly TupleField[],
): HashId<K> {
  requireProfile(kind, "framed-tuple-v1");
  return hashCanonicalPayload(kind, "framed-tuple-v1", framedTuplePayload(fields));
}

function requireProfile(kind: Parameters<typeof identityMetadata>[0], actual: string): void {
  const expected = identityMetadata(kind).profile;
  if (expected !== actual) {
    hashError(
      "profile-mismatch",
      `hash kind '${kind}' requires profile '${expected}', not '${actual}'`,
    );
  }
}

function serializeCanonical(value: JsonValue, rejectUnsafeObjectIntegers = true): string {
  if (value === null || typeof value === "boolean") return String(value);
  if (typeof value === "string") {
    assertUnicodeScalarString(value);
    return JSON.stringify(value);
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      return hashError("non-finite-number", "non-finite JSON numbers are not supported");
    }
    if (rejectUnsafeObjectIntegers && Number.isInteger(value) && !Number.isSafeInteger(value)) {
      return hashError("unsafe-json-integer", `unsafe JSON integer '${value}'`);
    }
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value
      .map((item) => serializeCanonical(item, rejectUnsafeObjectIntegers))
      .join(",")}]`;
  }
  if (typeof value !== "object") {
    return hashError("serialization", "value is not representable as JSON");
  }
  const prototype = Object.getPrototypeOf(value) as object | null;
  if (prototype !== Object.prototype && prototype !== null) {
    return hashError("serialization", "only JSON objects can be canonicalized");
  }
  const keys = Object.keys(value).sort();
  return `{${keys
    .map((key) => {
      assertUnicodeScalarString(key);
      const child = (value as Record<string, unknown>)[key];
      if (child === undefined || typeof child === "function" || typeof child === "symbol") {
        return hashError("serialization", `property '${key}' is not representable as JSON`);
      }
      return `${JSON.stringify(key)}:${serializeCanonical(
        child as JsonValue,
        rejectUnsafeObjectIntegers,
      )}`;
    })
    .join(",")}}`;
}

function assertUnicodeScalarString(value: string): void {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next < 0xdc00 || next > 0xdfff) {
        hashError("serialization", "lone UTF-16 surrogate is not valid JCS");
      }
      index += 1;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      hashError("serialization", "lone UTF-16 surrogate is not valid JCS");
    }
  }
}

class StrictJsonParser {
  private index = 0;

  constructor(private readonly text: string) {}

  parse(): JsonValue {
    this.skipWhitespace();
    const value = this.parseValue();
    this.skipWhitespace();
    if (this.index !== this.text.length) this.invalid("trailing characters");
    return value;
  }

  private parseValue(): JsonValue {
    const character = this.text[this.index];
    if (character === '"') return this.parseString();
    if (character === "{") return this.parseObject();
    if (character === "[") return this.parseArray();
    if (character === "t") return this.parseLiteral("true", true);
    if (character === "f") return this.parseLiteral("false", false);
    if (character === "n") return this.parseLiteral("null", null);
    if (character === "-" || (character !== undefined && character >= "0" && character <= "9")) {
      return this.parseNumber();
    }
    return this.invalid("expected a JSON value");
  }

  private parseObject(): JsonValue {
    this.index += 1;
    this.skipWhitespace();
    const value = Object.create(null) as Record<string, JsonValue>;
    const keys = new Set<string>();
    if (this.consume("}")) return value;
    while (true) {
      if (this.text[this.index] !== '"') this.invalid("object key must be a string");
      const key = this.parseString();
      if (keys.has(key)) {
        hashError("duplicate-json-key", `duplicate JSON object key '${key}'`);
      }
      keys.add(key);
      this.skipWhitespace();
      if (!this.consume(":")) this.invalid("expected ':' after object key");
      this.skipWhitespace();
      value[key] = this.parseValue();
      this.skipWhitespace();
      if (this.consume("}")) return value;
      if (!this.consume(",")) this.invalid("expected ',' or '}' in object");
      this.skipWhitespace();
    }
  }

  private parseArray(): JsonValue[] {
    this.index += 1;
    this.skipWhitespace();
    const value: JsonValue[] = [];
    if (this.consume("]")) return value;
    while (true) {
      value.push(this.parseValue());
      this.skipWhitespace();
      if (this.consume("]")) return value;
      if (!this.consume(",")) this.invalid("expected ',' or ']' in array");
      this.skipWhitespace();
    }
  }

  private parseString(): string {
    const start = this.index;
    this.index += 1;
    while (this.index < this.text.length) {
      const code = this.text.charCodeAt(this.index);
      if (code === 0x22) {
        this.index += 1;
        let value: string;
        try {
          value = JSON.parse(this.text.slice(start, this.index)) as string;
        } catch {
          return this.invalid("malformed JSON string");
        }
        try {
          assertUnicodeScalarString(value);
        } catch {
          return this.invalid("lone UTF-16 surrogate in JSON string");
        }
        return value;
      }
      if (code < 0x20) this.invalid("unescaped control character in JSON string");
      if (code === 0x5c) {
        this.index += 1;
        const escape = this.text[this.index];
        if (escape === "u") {
          const digits = this.text.slice(this.index + 1, this.index + 5);
          if (!/^[0-9a-fA-F]{4}$/.test(digits)) this.invalid("malformed Unicode escape");
          this.index += 5;
          continue;
        }
        if (escape === undefined || !'"\\/bfnrt'.includes(escape)) {
          this.invalid("malformed JSON escape");
        }
      }
      this.index += 1;
    }
    return this.invalid("unterminated JSON string");
  }

  private parseNumber(): number {
    const remaining = this.text.slice(this.index);
    const match = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(remaining);
    if (!match) return this.invalid("malformed JSON number");
    const token = match[0];
    const next = remaining[token.length];
    if (next !== undefined && !/[,\]}\s]/.test(next)) {
      return this.invalid("malformed JSON number");
    }
    if (!token.includes(".") && !/[eE]/.test(token)) {
      const magnitude = BigInt(token) < 0n ? -BigInt(token) : BigInt(token);
      if (magnitude > maxSafeInteger) {
        return hashError("unsafe-json-integer", `unsafe JSON integer '${token}'`);
      }
    }
    const value = Number(token);
    if (!Number.isFinite(value)) {
      return hashError("non-finite-number", "non-finite JSON numbers are not supported");
    }
    this.index += token.length;
    return value;
  }

  private parseLiteral<T extends JsonValue>(token: string, value: T): T {
    if (!this.text.startsWith(token, this.index)) this.invalid(`expected '${token}'`);
    this.index += token.length;
    return value;
  }

  private skipWhitespace(): void {
    while (/[\t\n\r ]/.test(this.text[this.index] ?? "")) this.index += 1;
  }

  private consume(character: string): boolean {
    if (this.text[this.index] !== character) return false;
    this.index += 1;
    return true;
  }

  private invalid(reason: string): never {
    return hashError("invalid-json", `invalid JSON at character ${this.index}: ${reason}`);
  }
}
