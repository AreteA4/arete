/**
 * Typed serialization of PDA seed values.
 *
 * Shared by the standalone PDA DSL (`pda-dsl.ts`) and instruction account
 * resolution (`account-resolver.ts`). When a seed carries a declared type
 * (from the IDL or the `pdas!` registry), encoding is exact: pubkeys are
 * base58-decoded to 32 bytes, integers are little-endian at the declared
 * width. Without a type, legacy heuristics apply for backward compatibility.
 */

import { decodeBase58 } from './pda';

/**
 * Canonical seed type names: 'pubkey', 'string', or 'u8'..'u128'/'i8'..'i128'.
 */
export type CanonicalSeedType = 'pubkey' | 'string' | `u${number}` | `i${number}`;

/**
 * Normalizes the type-name variants that IDLs and codegen produce
 * ("Pubkey", "publicKey", "solana_pubkey::Pubkey", "String", ...) to a
 * canonical seed type. Returns undefined for types that cannot be a seed.
 */
export function normalizeSeedType(argType?: string): CanonicalSeedType | undefined {
  if (!argType) return undefined;
  // Strip any path qualifier (e.g. solana_pubkey::Pubkey).
  const parts = argType.split('::');
  const t = (parts[parts.length - 1] ?? argType).trim();

  if (/^[ui](8|16|32|64|128)$/.test(t)) {
    return t as CanonicalSeedType;
  }
  switch (t) {
    case 'pubkey':
    case 'Pubkey':
    case 'publicKey':
    case 'PublicKey':
      return 'pubkey';
    case 'string':
    case 'String':
    case 'str':
      return 'string';
    default:
      return undefined;
  }
}

/**
 * Serializes a PDA seed value.
 *
 * With a recognized `argType`, encoding is strict and width-exact; an
 * incompatible value throws rather than deriving a wrong address. Without
 * one, the legacy heuristics apply: raw bytes pass through, 43/44-character
 * strings are tried as base58, other strings are utf-8, numbers are 8-byte
 * little-endian u64.
 */
export function serializeSeedValue(value: unknown, argType?: string): Uint8Array {
  if (value instanceof Uint8Array) {
    return value;
  }

  const t = normalizeSeedType(argType);

  if (t === 'pubkey') {
    if (typeof value !== 'string') {
      throw new Error(`Pubkey seed requires a base58 string, got ${typeof value}`);
    }
    const decoded = decodeBase58(value);
    if (decoded.length !== 32) {
      throw new Error(
        `Pubkey seed '${value}' decoded to ${decoded.length} bytes, expected 32`
      );
    }
    return decoded;
  }

  if (t === 'string') {
    if (typeof value !== 'string') {
      throw new Error(`String seed requires a string value, got ${typeof value}`);
    }
    return new TextEncoder().encode(value);
  }

  if (t !== undefined) {
    // Numeric type at a declared width.
    if (typeof value !== 'bigint' && typeof value !== 'number') {
      throw new Error(`Numeric seed of type ${t} requires a number/bigint, got ${typeof value}`);
    }
    const bits = parseInt(t.slice(1), 10);
    return serializeNumber(value, bits / 8, t.startsWith('i'));
  }

  // --- Untyped: legacy heuristics. ---
  if (typeof value === 'string') {
    if (value.length === 43 || value.length === 44) {
      try {
        return decodeBase58(value);
      } catch {
        return new TextEncoder().encode(value);
      }
    }
    return new TextEncoder().encode(value);
  }

  if (typeof value === 'bigint' || typeof value === 'number') {
    return serializeNumber(value, 8, true);
  }

  throw new Error(`Cannot serialize value for PDA seed: ${typeof value}`);
}

/**
 * Little-endian two's-complement encoding at a fixed byte width, with an
 * overflow check so out-of-range values fail instead of silently truncating.
 */
function serializeNumber(value: bigint | number, size: number, signed: boolean): Uint8Array {
  const buffer = new Uint8Array(size);
  let n = typeof value === 'bigint' ? value : BigInt(value);
  const original = n;
  for (let i = 0; i < size; i++) {
    buffer[i] = Number(n & BigInt(0xff));
    n >>= BigInt(8);
  }
  const fits = signed ? n === BigInt(0) || n === BigInt(-1) : n === BigInt(0);
  if (!fits) {
    throw new Error(`Seed value ${original} does not fit in ${size * 8} bits`);
  }
  return buffer;
}
