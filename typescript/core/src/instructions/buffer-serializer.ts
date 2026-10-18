/**
 * `serializeInstructionData`, the `Buffer`-returning form of
 * {@link encodeInstructionData} that the root `@usearete/sdk` entry has
 * always exported. Only the root entry imports this module, so the
 * `buffer` polyfill stays out of `@usearete/sdk/program`.
 */

import { Buffer } from 'buffer';

import { encodeInstructionData, type ArgSchema } from './serializer';

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
  const bytes = encodeInstructionData(discriminator, args, schema);
  return Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}
