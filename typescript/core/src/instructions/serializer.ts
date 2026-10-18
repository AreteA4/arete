/**
 * Borsh-compatible instruction data serializer.
 *
 * This module handles serializing instruction arguments into the binary format
 * expected by Solana programs using Borsh serialization.
 *
 * It uses only `Uint8Array`, `DataView` and `TextEncoder`, so it runs in any
 * JavaScript runtime and bundles without a `buffer` polyfill (the
 * `@usearete/sdk/program` entry depends on that). The bytes, and the inputs
 * it rejects, match the `Buffer`-based encoder it replaced.
 */

import { decodeBase58 } from './pda';

/**
 * Instruction argument schema for serialization.
 */
export interface ArgSchema {
  /** Argument name */
  name: string;
  /** Argument type */
  type: ArgType;
}

/**
 * Supported argument types for Borsh serialization.
 *
 * Struct and enum schemas are fully inlined (field names and types travel
 * with the schema), so the serializer needs no runtime type registry.
 */
export type ArgType =
  | 'u8' | 'u16' | 'u32' | 'u64' | 'u128'
  | 'i8' | 'i16' | 'i32' | 'i64' | 'i128'
  | 'f32' | 'f64'
  | 'bool'
  | 'string'
  /** UTF-8 string with a u64 LE byte length (bincode-style; native programs' seeds). */
  | 'stringU64Len'
  | 'pubkey'
  | 'bytes'
  | { vec: ArgType }
  | { vecU64Len: ArgType }
  | { option: ArgType }
  | { array: readonly [ArgType, number] }
  | { tuple: readonly ArgType[] }
  | { hashMap: readonly [ArgType, ArgType] }
  | { struct: readonly ArgStructField[] }
  | { enum: readonly EnumVariant[] };

/** One field of a struct schema, in declaration (serialization) order. */
export interface ArgStructField {
  readonly name: string;
  readonly type: ArgType;
}

/**
 * One enum variant: a bare string for fieldless variants, or a named variant
 * carrying struct fields / tuple elements.
 *
 * Values: fieldless variants are passed as the variant name (or its index);
 * data-carrying variants as a single-key object, e.g. `{ transfer: { amount } }`
 * or `{ pair: [1, 2] }`.
 */
export type EnumVariant =
  | string
  | { readonly name: string; readonly fields: readonly ArgStructField[] }
  | { readonly name: string; readonly tuple: readonly ArgType[] };

/**
 * Serializes instruction arguments into bytes using Borsh encoding.
 *
 * @param discriminator - The 8-byte instruction discriminator
 * @param args - Arguments to serialize
 * @param schema - Schema defining argument types
 * @returns Serialized instruction data
 */
export function encodeInstructionData(
  discriminator: Uint8Array,
  args: Record<string, unknown>,
  schema: ArgSchema[]
): Uint8Array {
  const parts: Uint8Array[] = [Uint8Array.from(discriminator)];

  for (const field of schema) {
    const value = args[field.name];
    // Option fields treat undefined as None; everything else must be present
    // (silently encoding zeros for a missing arg corrupts instruction data).
    if (value === undefined && !(typeof field.type === 'object' && 'option' in field.type)) {
      throw new Error(
        `Missing required argument "${field.name}" (type ${JSON.stringify(field.type)})`
      );
    }
    parts.push(serializeValue(value, field.type));
  }

  return concatBytes(parts);
}

const UTF8 = new TextEncoder();
const U64_MASK = (BigInt(1) << BigInt(64)) - BigInt(1);
const I64_MIN = -(BigInt(1) << BigInt(63));
const I64_MAX = (BigInt(1) << BigInt(63)) - BigInt(1);

function concatBytes(parts: readonly Uint8Array[]): Uint8Array {
  let length = 0;
  for (const part of parts) length += part.length;
  const out = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

/** Compare two byte strings the way `Buffer.compare` does. */
function compareBytes(left: Uint8Array, right: Uint8Array): number {
  const length = Math.min(left.length, right.length);
  for (let i = 0; i < length; i += 1) {
    if (left[i] !== right[i]) return left[i]! < right[i]! ? -1 : 1;
  }
  return left.length === right.length ? 0 : left.length < right.length ? -1 : 1;
}

/** One byte, wrapped like `Buffer.from([value])` (`value & 0xff`). */
function byte(value: unknown): Uint8Array {
  return Uint8Array.of(value as number);
}

/**
 * A fixed-width integer, little-endian. Out-of-range values throw a
 * `RangeError`, as `Buffer#writeUInt16LE` and friends do; NaN encodes as 0.
 */
function fixedInt(value: unknown, size: 2 | 4, signed: boolean): Uint8Array {
  const n = +(value as number);
  const bits = size * 8;
  const min = signed ? -(2 ** (bits - 1)) : 0;
  const max = signed ? 2 ** (bits - 1) - 1 : 2 ** bits - 1;
  if (n > max || n < min) {
    throw new RangeError(`The value ${String(value)} is out of range for a ${size}-byte ${signed ? 'signed' : 'unsigned'} integer`);
  }
  const out = new Uint8Array(size);
  const view = new DataView(out.buffer);
  if (size === 2) {
    if (signed) view.setInt16(0, n, true);
    else view.setUint16(0, n, true);
  } else if (signed) {
    view.setInt32(0, n, true);
  } else {
    view.setUint32(0, n, true);
  }
  return out;
}

function writeBigUint64(view: DataView, offset: number, value: bigint): void {
  if (value < BigInt(0) || value > U64_MASK) {
    throw new RangeError(`The value ${value} is out of range for an unsigned 64-bit integer`);
  }
  view.setBigUint64(offset, value, true);
}

function writeBigInt64(view: DataView, offset: number, value: bigint): void {
  if (value < I64_MIN || value > I64_MAX) {
    throw new RangeError(`The value ${value} is out of range for a signed 64-bit integer`);
  }
  view.setBigInt64(offset, value, true);
}

/**
 * The bytes of a `string` argument. A string is UTF-8 encoded; byte-like
 * values pass through as `Buffer.from(value, 'utf-8')` passed them, and
 * anything else is rejected as it was, rather than coerced to text.
 */
function stringBytes(value: unknown): Uint8Array {
  if (typeof value === 'string') return UTF8.encode(value);
  if (value instanceof String) return UTF8.encode(value.valueOf());
  if (value instanceof ArrayBuffer) return new Uint8Array(value.slice(0));
  if (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { length?: unknown }).length === 'number'
  ) {
    return Uint8Array.from(value as ArrayLike<number>);
  }
  throw new TypeError(
    `Cannot serialize string from value of type ${value === null ? 'null' : typeof value}`
  );
}

function lengthPrefix(length: number, bytes: 4 | 8 = 4): Uint8Array {
  const out = new Uint8Array(bytes);
  const view = new DataView(out.buffer);
  if (bytes === 8) writeBigUint64(view, 0, BigInt(length));
  else view.setUint32(0, length, true);
  return out;
}

function serializeValue(value: unknown, type: ArgType): Uint8Array {
  if (typeof type === 'string') {
    return serializePrimitive(value, type);
  }

  if ('vec' in type) {
    return serializeVec(value as unknown[], type.vec);
  }

  if ('vecU64Len' in type) {
    return serializeVec(value as unknown[], type.vecU64Len, 8);
  }

  if ('option' in type) {
    return serializeOption(value, type.option);
  }

  if ('array' in type) {
    return serializeArray(value as unknown[], type.array[0], type.array[1]);
  }

  if ('tuple' in type) {
    return serializeTuple(value, type.tuple);
  }

  if ('hashMap' in type) {
    return serializeHashMap(value, type.hashMap[0], type.hashMap[1]);
  }

  if ('struct' in type) {
    return serializeStruct(value, type.struct);
  }

  if ('enum' in type) {
    return serializeEnum(value, type.enum);
  }

  throw new Error(`Unknown type: ${JSON.stringify(type)}`);
}

function serializeStruct(value: unknown, fields: readonly ArgStructField[]): Uint8Array {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`Struct value must be a plain object, got ${typeof value}`);
  }
  const obj = value as Record<string, unknown>;
  const parts: Uint8Array[] = [];
  for (const field of fields) {
    const fieldValue = obj[field.name];
    if (
      fieldValue === undefined &&
      !(typeof field.type === 'object' && 'option' in field.type)
    ) {
      throw new Error(`Missing required struct field "${field.name}"`);
    }
    parts.push(serializeValue(fieldValue, field.type));
  }
  return concatBytes(parts);
}

function serializeTuple(value: unknown, elementTypes: readonly ArgType[]): Uint8Array {
  if (!Array.isArray(value)) {
    throw new Error(`Tuple value must be an array, got ${typeof value}`);
  }
  if (value.length !== elementTypes.length) {
    throw new Error(
      `Tuple length mismatch: expected ${elementTypes.length}, got ${value.length}`
    );
  }

  return concatBytes(
    elementTypes.map((elementType, index) => serializeValue(value[index], elementType))
  );
}

function serializeHashMap(value: unknown, keyType: ArgType, valueType: ArgType): Uint8Array {
  if (keyType !== 'string') {
    throw new Error(
      `Instruction hashMap keys must use the 'string' schema, got ${JSON.stringify(keyType)}`
    );
  }
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`HashMap value must be a plain object, got ${typeof value}`);
  }

  const entries = Object.entries(value as Record<string, unknown>).sort(([left], [right]) =>
    // Rust/Borsh sorts String keys by their UTF-8 bytes before serializing.
    compareBytes(UTF8.encode(left), UTF8.encode(right))
  );

  const parts: Uint8Array[] = [lengthPrefix(entries.length)];
  for (const [key, entryValue] of entries) {
    parts.push(serializePrimitive(key, 'string'));
    parts.push(serializeValue(entryValue, valueType));
  }
  return concatBytes(parts);
}

function variantName(variant: EnumVariant): string {
  return typeof variant === 'string' ? variant : variant.name;
}

function serializeEnum(value: unknown, variants: readonly EnumVariant[]): Uint8Array {
  // Numeric value: a bare variant index (fieldless variants only).
  if (typeof value === 'number') {
    if (!Number.isInteger(value) || value < 0 || value >= variants.length) {
      throw new Error(`Enum variant index ${value} out of range (0..${variants.length - 1})`);
    }
    const variant = variants[value]!;
    if (typeof variant !== 'string') {
      throw new Error(
        `Enum variant "${variantName(variant)}" carries data; pass { ${variantName(variant)}: ... } instead of an index`
      );
    }
    return byte(value);
  }

  // String value: a fieldless variant by name.
  if (typeof value === 'string') {
    const index = variants.findIndex((v) => variantName(v) === value);
    if (index === -1) {
      throw new Error(
        `Unknown enum variant "${value}". Expected one of: ${variants.map(variantName).join(', ')}`
      );
    }
    const variant = variants[index]!;
    if (typeof variant !== 'string') {
      throw new Error(
        `Enum variant "${value}" carries data; pass { ${value}: ... } instead of a bare name`
      );
    }
    return byte(index);
  }

  // Object value: { variantName: payload } for data-carrying variants.
  if (typeof value === 'object' && value !== null && !Array.isArray(value)) {
    const keys = Object.keys(value);
    if (keys.length !== 1) {
      throw new Error(
        `Enum value must be a single-key object ({ variantName: payload }), got keys [${keys.join(', ')}]`
      );
    }
    const key = keys[0]!;
    const payload = (value as Record<string, unknown>)[key];
    const index = variants.findIndex((v) => variantName(v) === key);
    if (index === -1) {
      throw new Error(
        `Unknown enum variant "${key}". Expected one of: ${variants.map(variantName).join(', ')}`
      );
    }
    const variant = variants[index]!;
    if (typeof variant === 'string') {
      throw new Error(`Enum variant "${key}" is fieldless; pass '${key}' instead of an object`);
    }
    const prefix = byte(index);
    if ('fields' in variant) {
      return concatBytes([prefix, serializeStruct(payload, variant.fields)]);
    }
    const tuple = payload as unknown[];
    if (!Array.isArray(tuple) || tuple.length !== variant.tuple.length) {
      throw new Error(
        `Enum variant "${key}" expects a tuple of length ${variant.tuple.length}`
      );
    }
    const elements = variant.tuple.map((elementType, i) => serializeValue(tuple[i], elementType));
    return concatBytes([prefix, ...elements]);
  }

  throw new Error(`Cannot serialize enum from value of type ${typeof value}`);
}

function serializePrimitive(value: unknown, type: string): Uint8Array {
  switch (type) {
    case 'u8':
    case 'i8':
      return byte(value);
    case 'u16':
      return fixedInt(value, 2, false);
    case 'u32':
      return fixedInt(value, 4, false);
    case 'i16':
      return fixedInt(value, 2, true);
    case 'i32':
      return fixedInt(value, 4, true);
    case 'u64': {
      const out = new Uint8Array(8);
      writeBigUint64(new DataView(out.buffer), 0, BigInt(value as string | number | bigint));
      return out;
    }
    case 'u128': {
      // u128 is 16 bytes, little-endian: the masked low limb, then the high
      // limb, which must itself fit in 64 unsigned bits.
      const out = new Uint8Array(16);
      const view = new DataView(out.buffer);
      const big = BigInt(value as string | number | bigint);
      writeBigUint64(view, 0, big & U64_MASK);
      writeBigUint64(view, 8, big >> BigInt(64));
      return out;
    }
    case 'i64': {
      const out = new Uint8Array(8);
      writeBigInt64(new DataView(out.buffer), 0, BigInt(value as string | number | bigint));
      return out;
    }
    case 'i128': {
      const out = new Uint8Array(16);
      const view = new DataView(out.buffer);
      const big = BigInt(value as string | number | bigint);
      // The masked low limb is always non-negative, so it must be written
      // unsigned; the arithmetic-shifted high limb carries the sign.
      writeBigUint64(view, 0, big & U64_MASK);
      writeBigInt64(view, 8, big >> BigInt(64));
      return out;
    }
    case 'f32': {
      const out = new Uint8Array(4);
      new DataView(out.buffer).setFloat32(0, +(value as number), true);
      return out;
    }
    case 'f64': {
      const out = new Uint8Array(8);
      new DataView(out.buffer).setFloat64(0, +(value as number), true);
      return out;
    }
    case 'bool':
      return byte((value as boolean) ? 1 : 0);
    case 'bytes': {
      // Borsh bytes: u32 LE length prefix + raw bytes.
      const raw =
        value instanceof Uint8Array
          ? value
          : Array.isArray(value)
            ? Uint8Array.from(value as number[])
            : null;
      if (raw === null) {
        throw new Error(`Cannot serialize bytes from value of type ${typeof value}`);
      }
      return concatBytes([lengthPrefix(raw.length), raw]);
    }
    case 'string': {
      const strBytes = stringBytes(value);
      return concatBytes([lengthPrefix(strBytes.length), strBytes]);
    }
    case 'stringU64Len': {
      const strBytes = stringBytes(value);
      return concatBytes([lengthPrefix(strBytes.length, 8), strBytes]);
    }
    case 'pubkey': {
      // Public key is 32 bytes. Accept base58 strings or raw 32-byte buffers.
      if (value instanceof Uint8Array) {
        if (value.length !== 32) {
          throw new Error(`Invalid pubkey byte length: expected 32, got ${value.length}`);
        }
        return Uint8Array.from(value);
      }
      if (typeof value === 'string') {
        const decoded = decodeBase58(value);
        if (decoded.length !== 32) {
          throw new Error(`Invalid pubkey: '${value}' decoded to ${decoded.length} bytes, expected 32`);
        }
        return Uint8Array.from(decoded);
      }
      throw new Error(`Cannot serialize pubkey from value of type ${typeof value}`);
    }
    default:
      throw new Error(`Unknown primitive type: ${type}`);
  }
}

// `lengthBytes`: 4 for Borsh, 8 for bincode-style sequences.
function serializeVec(values: unknown[], elementType: ArgType, lengthBytes: 4 | 8 = 4): Uint8Array {
  return concatBytes([
    lengthPrefix(values.length, lengthBytes),
    ...values.map(v => serializeValue(v, elementType)),
  ]);
}

function serializeOption(value: unknown, innerType: ArgType): Uint8Array {
  if (value === null || value === undefined) {
    return byte(0); // None
  }

  return concatBytes([byte(1), serializeValue(value, innerType)]); // Some
}

function serializeArray(
  values: unknown[],
  elementType: ArgType,
  length: number
): Uint8Array {
  if (values.length !== length) {
    throw new Error(
      `Array length mismatch: expected ${length}, got ${values.length}`
    );
  }

  return concatBytes(values.map(v => serializeValue(v, elementType)));
}
