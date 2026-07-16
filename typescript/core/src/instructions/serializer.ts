/**
 * Borsh-compatible instruction data serializer.
 * 
 * This module handles serializing instruction arguments into the binary format
 * expected by Solana programs using Borsh serialization.
 */

import { Buffer } from 'buffer';
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
  | 'pubkey'
  | 'bytes'
  | { vec: ArgType }
  | { option: ArgType }
  | { array: readonly [ArgType, number] }
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
 * Serializes instruction arguments into a Buffer using Borsh encoding.
 * 
 * @param discriminator - The 8-byte instruction discriminator
 * @param args - Arguments to serialize
 * @param schema - Schema defining argument types
 * @returns Serialized instruction data
 */
export function serializeInstructionData(
  discriminator: Uint8Array,
  args: Record<string, unknown>,
  schema: ArgSchema[]
): Buffer {
  const buffers: Buffer[] = [Buffer.from(discriminator)];

  for (const field of schema) {
    const value = args[field.name];
    // Option fields treat undefined as None; everything else must be present
    // (silently encoding zeros for a missing arg corrupts instruction data).
    if (value === undefined && !(typeof field.type === 'object' && 'option' in field.type)) {
      throw new Error(
        `Missing required argument "${field.name}" (type ${JSON.stringify(field.type)})`
      );
    }
    const serialized = serializeValue(value, field.type);
    buffers.push(serialized);
  }

  return Buffer.concat(buffers);
}

function serializeValue(value: unknown, type: ArgType): Buffer {
  if (typeof type === 'string') {
    return serializePrimitive(value, type);
  }
  
  if ('vec' in type) {
    return serializeVec(value as unknown[], type.vec);
  }
  
  if ('option' in type) {
    return serializeOption(value, type.option);
  }
  
  if ('array' in type) {
    return serializeArray(value as unknown[], type.array[0], type.array[1]);
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

function serializeStruct(value: unknown, fields: readonly ArgStructField[]): Buffer {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`Struct value must be a plain object, got ${typeof value}`);
  }
  const obj = value as Record<string, unknown>;
  const buffers: Buffer[] = [];
  for (const field of fields) {
    const fieldValue = obj[field.name];
    if (
      fieldValue === undefined &&
      !(typeof field.type === 'object' && 'option' in field.type)
    ) {
      throw new Error(`Missing required struct field "${field.name}"`);
    }
    buffers.push(serializeValue(fieldValue, field.type));
  }
  return Buffer.concat(buffers);
}

function serializeHashMap(value: unknown, keyType: ArgType, valueType: ArgType): Buffer {
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
    Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'))
  );
  const len = Buffer.alloc(4);
  len.writeUInt32LE(entries.length, 0);

  const entryBuffers: Buffer[] = [];
  for (const [key, entryValue] of entries) {
    entryBuffers.push(serializePrimitive(key, 'string'));
    entryBuffers.push(serializeValue(entryValue, valueType));
  }
  return Buffer.concat([len, ...entryBuffers]);
}

function variantName(variant: EnumVariant): string {
  return typeof variant === 'string' ? variant : variant.name;
}

function serializeEnum(value: unknown, variants: readonly EnumVariant[]): Buffer {
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
    return Buffer.from([value]);
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
    return Buffer.from([index]);
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
    const prefix = Buffer.from([index]);
    if ('fields' in variant) {
      return Buffer.concat([prefix, serializeStruct(payload, variant.fields)]);
    }
    const tuple = payload as unknown[];
    if (!Array.isArray(tuple) || tuple.length !== variant.tuple.length) {
      throw new Error(
        `Enum variant "${key}" expects a tuple of length ${variant.tuple.length}`
      );
    }
    const elements = variant.tuple.map((elementType, i) => serializeValue(tuple[i], elementType));
    return Buffer.concat([prefix, ...elements]);
  }

  throw new Error(`Cannot serialize enum from value of type ${typeof value}`);
}

function serializePrimitive(value: unknown, type: string): Buffer {
  switch (type) {
    case 'u8':
      return Buffer.from([value as number]);
    case 'u16':
      const u16 = Buffer.alloc(2);
      u16.writeUInt16LE(value as number, 0);
      return u16;
    case 'u32':
      const u32 = Buffer.alloc(4);
      u32.writeUInt32LE(value as number, 0);
      return u32;
    case 'u64':
      const u64 = Buffer.alloc(8);
      u64.writeBigUInt64LE(BigInt(value as string | number | bigint), 0);
      return u64;
    case 'u128':
      // u128 is 16 bytes, little-endian
      const u128 = Buffer.alloc(16);
      const bigU128 = BigInt(value as string | number | bigint);
      u128.writeBigUInt64LE(bigU128 & BigInt('0xFFFFFFFFFFFFFFFF'), 0);
      u128.writeBigUInt64LE(bigU128 >> BigInt(64), 8);
      return u128;
    case 'i8':
      return Buffer.from([value as number]);
    case 'i16':
      const i16 = Buffer.alloc(2);
      i16.writeInt16LE(value as number, 0);
      return i16;
    case 'i32':
      const i32 = Buffer.alloc(4);
      i32.writeInt32LE(value as number, 0);
      return i32;
    case 'i64':
      const i64 = Buffer.alloc(8);
      i64.writeBigInt64LE(BigInt(value as string | number | bigint), 0);
      return i64;
    case 'i128':
      const i128 = Buffer.alloc(16);
      const bigI128 = BigInt(value as string | number | bigint);
      // The masked low limb is always non-negative, so it must be written
      // unsigned; the arithmetic-shifted high limb carries the sign.
      i128.writeBigUInt64LE(bigI128 & BigInt('0xFFFFFFFFFFFFFFFF'), 0);
      i128.writeBigInt64LE(bigI128 >> BigInt(64), 8);
      return i128;
    case 'f32': {
      const f32 = Buffer.alloc(4);
      f32.writeFloatLE(value as number, 0);
      return f32;
    }
    case 'f64': {
      const f64 = Buffer.alloc(8);
      f64.writeDoubleLE(value as number, 0);
      return f64;
    }
    case 'bool':
      return Buffer.from([value as boolean ? 1 : 0]);
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
      const lenPrefix = Buffer.alloc(4);
      lenPrefix.writeUInt32LE(raw.length, 0);
      return Buffer.concat([lenPrefix, Buffer.from(raw)]);
    }
    case 'string':
      const str = value as string;
      const strBytes = Buffer.from(str, 'utf-8');
      const strLen = Buffer.alloc(4);
      strLen.writeUInt32LE(strBytes.length, 0);
      return Buffer.concat([strLen, strBytes]);
    case 'pubkey': {
      // Public key is 32 bytes. Accept base58 strings or raw 32-byte buffers.
      if (value instanceof Uint8Array) {
        if (value.length !== 32) {
          throw new Error(`Invalid pubkey byte length: expected 32, got ${value.length}`);
        }
        return Buffer.from(value);
      }
      if (typeof value === 'string') {
        const decoded = decodeBase58(value);
        if (decoded.length !== 32) {
          throw new Error(`Invalid pubkey: '${value}' decoded to ${decoded.length} bytes, expected 32`);
        }
        return Buffer.from(decoded);
      }
      throw new Error(`Cannot serialize pubkey from value of type ${typeof value}`);
    }
    default:
      throw new Error(`Unknown primitive type: ${type}`);
  }
}

function serializeVec(values: unknown[], elementType: ArgType): Buffer {
  const len = Buffer.alloc(4);
  len.writeUInt32LE(values.length, 0);
  
  const elementBuffers = values.map(v => serializeValue(v, elementType));
  return Buffer.concat([len, ...elementBuffers]);
}

function serializeOption(value: unknown, innerType: ArgType): Buffer {
  if (value === null || value === undefined) {
    return Buffer.from([0]); // None
  }
  
  const inner = serializeValue(value, innerType);
  return Buffer.concat([Buffer.from([1]), inner]); // Some
}

function serializeArray(
  values: unknown[],
  elementType: ArgType,
  length: number
): Buffer {
  if (values.length !== length) {
    throw new Error(
      `Array length mismatch: expected ${length}, got ${values.length}`
    );
  }
  
  const elementBuffers = values.map(v => serializeValue(v, elementType));
  return Buffer.concat(elementBuffers);
}
