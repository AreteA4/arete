import { base58, base64 } from '@scure/base';

export function encodeBase58(bytes: Uint8Array): string {
  return base58.encode(bytes);
}

/** The bytes of a base58 string, or `null` when it is not base58. */
export function decodeBase58(text: string): Uint8Array | null {
  try {
    return base58.decode(text);
  } catch {
    return null;
  }
}

export function encodeBase64(bytes: Uint8Array): string {
  return base64.encode(bytes);
}

/** The bytes of a padded base64 string, or `null` when it is not base64. */
export function decodeBase64(text: string): Uint8Array | null {
  try {
    return base64.decode(text);
  } catch {
    return null;
  }
}

/** Whether `value` is a base58 public key (32 bytes). */
export function isPubkey(value: string): boolean {
  return decodeBase58(value)?.length === 32;
}

/** A JSON object's field; `undefined` when `value` is not an object. */
export function field(value: unknown, key: string): unknown {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    return undefined;
  }
  return (value as Record<string, unknown>)[key];
}

/** A JSON value as a non-negative integer, as serde's `as_u64` reads it. */
export function asU64(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 ? value : undefined;
}

export function asString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined;
}

export function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
