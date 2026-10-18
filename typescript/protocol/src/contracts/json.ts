/**
 * Strict contract JSON parsing, mirroring `parse_contract_json` in
 * `arete-hash/src/protocol/mod.rs`.
 *
 * Rust rejects unsafe integer tokens lexically before parsing, then parses
 * with duplicate-key detection, and keeps number tokens with a fraction or
 * exponent as floats (which every contract validator then rejects). This
 * module reproduces that order: the `@usearete/hash` strict parser supplies
 * well-formedness, and a second pass keeps float tokens as `FloatToken`
 * markers so `1.0` is reported as a float exactly as Rust reports it.
 */

import { HashError, parseJsonBytesStrict } from "@usearete/hash";

import { contractError } from "./errors.js";
import { FloatToken } from "./shape.js";

const MAX_SAFE_INTEGER_DIGITS = "9007199254740991";
const decoder = new TextDecoder("utf-8", { fatal: true });

/**
 * Parse contract JSON bytes. The result is plain JSON, except that number
 * tokens written with a fraction or exponent become `FloatToken`s.
 */
export function parseContractJson(bytes: Uint8Array): unknown {
  const unsafe = findUnsafeIntegerToken(bytes);
  if (unsafe !== undefined) {
    throw contractError("unsafe-json-integer", "", `unsafe JSON integer '${unsafe}'`);
  }
  try {
    parseJsonBytesStrict(bytes);
  } catch (error) {
    if (error instanceof HashError) {
      const code =
        error.code === "duplicate-json-key"
          ? "duplicate-json-key"
          : error.code === "unsafe-json-integer"
            ? "unsafe-json-integer"
            : error.code === "non-finite-number"
              ? "float-not-allowed"
              : "invalid-json";
      throw contractError(code, "", error.message);
    }
    throw error;
  }
  return new MarkingParser(decoder.decode(bytes)).parse();
}

function isDigit(byte: number | undefined): boolean {
  return byte !== undefined && byte >= 0x30 && byte <= 0x39;
}

/** Port of `reject_unsafe_integer_tokens` in `arete-hash/src/canonical.rs`. */
function findUnsafeIntegerToken(bytes: Uint8Array): string | undefined {
  let i = 0;
  while (i < bytes.length) {
    const byte = bytes[i]!;
    if (byte === 0x22) {
      i += 1;
      while (i < bytes.length) {
        if (bytes[i] === 0x5c) i += 2;
        else if (bytes[i] === 0x22) {
          i += 1;
          break;
        } else i += 1;
      }
    } else if (byte === 0x2d || isDigit(byte)) {
      const scanned = scanNumberToken(bytes, i);
      if (scanned === undefined) {
        i += 1;
        continue;
      }
      const [end, digits] = scanned;
      if (digits !== undefined) {
        const magnitude = digits.replace(/^-+/, "");
        const unsafe =
          magnitude.length > MAX_SAFE_INTEGER_DIGITS.length ||
          (magnitude.length === MAX_SAFE_INTEGER_DIGITS.length && magnitude > MAX_SAFE_INTEGER_DIGITS);
        if (unsafe) return String.fromCharCode(...bytes.slice(i, end));
      }
      i = end;
    } else {
      i += 1;
    }
  }
  return undefined;
}

function scanNumberToken(bytes: Uint8Array, start: number): [number, string | undefined] | undefined {
  let i = start;
  if (bytes[i] === 0x2d) i += 1;
  if (bytes[i] === 0x30) {
    i += 1;
    if (isDigit(bytes[i])) return undefined;
  } else if (bytes[i] !== undefined && bytes[i]! >= 0x31 && bytes[i]! <= 0x39) {
    while (isDigit(bytes[i])) i += 1;
  } else {
    return undefined;
  }
  const digits = String.fromCharCode(...bytes.slice(start, i));
  let integer = true;
  if (bytes[i] === 0x2e) {
    integer = false;
    i += 1;
    const fractionStart = i;
    while (isDigit(bytes[i])) i += 1;
    if (i === fractionStart) return undefined;
  }
  if (bytes[i] === 0x65 || bytes[i] === 0x45) {
    integer = false;
    i += 1;
    if (bytes[i] === 0x2b || bytes[i] === 0x2d) i += 1;
    const exponentStart = i;
    while (isDigit(bytes[i])) i += 1;
    if (i === exponentStart) return undefined;
  }
  const next = bytes[i];
  const complete =
    next === undefined ||
    next === 0x2c ||
    next === 0x5d ||
    next === 0x7d ||
    next === 0x20 ||
    next === 0x09 ||
    next === 0x0a ||
    next === 0x0d;
  if (!complete) return undefined;
  return [i, integer ? digits : undefined];
}

/** A lenient recursive-descent parser for text the strict parser already accepted. */
class MarkingParser {
  private i = 0;

  constructor(private readonly text: string) {}

  parse(): unknown {
    this.ws();
    return this.value();
  }

  private ws(): void {
    while (/[\t\n\r ]/.test(this.text[this.i] ?? "")) this.i += 1;
  }

  private value(): unknown {
    const c = this.text[this.i];
    if (c === "{") {
      this.i += 1;
      const out = Object.create(null) as Record<string, unknown>;
      this.ws();
      if (this.text[this.i] === "}") {
        this.i += 1;
        return out;
      }
      for (;;) {
        this.ws();
        const key = this.string();
        this.ws();
        this.i += 1; // ':'
        this.ws();
        out[key] = this.value();
        this.ws();
        if (this.text[this.i++] === "}") return out;
      }
    }
    if (c === "[") {
      this.i += 1;
      const out: unknown[] = [];
      this.ws();
      if (this.text[this.i] === "]") {
        this.i += 1;
        return out;
      }
      for (;;) {
        this.ws();
        out.push(this.value());
        this.ws();
        if (this.text[this.i++] === "]") return out;
      }
    }
    if (c === '"') return this.string();
    if (this.text.startsWith("true", this.i)) {
      this.i += 4;
      return true;
    }
    if (this.text.startsWith("false", this.i)) {
      this.i += 5;
      return false;
    }
    if (this.text.startsWith("null", this.i)) {
      this.i += 4;
      return null;
    }
    const match = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(this.text.slice(this.i))!;
    const token = match[0];
    this.i += token.length;
    return /[.eE]/.test(token) ? new FloatToken(token) : Number(token);
  }

  private string(): string {
    const start = this.i;
    this.i += 1;
    while (this.text[this.i] !== '"') {
      this.i += this.text[this.i] === "\\" ? 2 : 1;
    }
    this.i += 1;
    return JSON.parse(this.text.slice(start, this.i)) as string;
  }
}
