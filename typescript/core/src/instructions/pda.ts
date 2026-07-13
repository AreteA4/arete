/**
 * PDA (Program Derived Address) derivation utilities.
 * 
 * Implements Solana's PDA derivation algorithm without depending on @solana/web3.js.
 */

import { Point } from '@noble/ed25519';

// Base58 alphabet (Bitcoin/Solana style)
const BASE58_ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';

/**
 * Decode base58 string to Uint8Array.
 */
export function decodeBase58(str: string): Uint8Array {
  if (str.length === 0) {
    return new Uint8Array(0);
  }

  // Big-endian byte accumulator (stored little-endian here, reversed at the
  // end). Must start empty: a leading `[0]` produces a spurious extra byte for
  // all-zero values such as the System Program ("111...1").
  const bytes: number[] = [];
  
  for (const char of str) {
    const value = BASE58_ALPHABET.indexOf(char);
    if (value === -1) {
      throw new Error('Invalid base58 character: ' + char);
    }
    
    let carry = value;
    for (let i = 0; i < bytes.length; i++) {
      carry += (bytes[i] ?? 0) * 58;
      bytes[i] = carry & 0xff;
      carry >>= 8;
    }
    while (carry > 0) {
      bytes.push(carry & 0xff);
      carry >>= 8;
    }
  }
  
  // Add leading zeros for each leading '1' in input
  for (const char of str) {
    if (char !== '1') break;
    bytes.push(0);
  }
  
  return new Uint8Array(bytes.reverse());
}

/**
 * Encode Uint8Array to base58 string.
 */
export function encodeBase58(bytes: Uint8Array): string {
  if (bytes.length === 0) {
    return '';
  }

  // Must start empty for the same reason as `decodeBase58`: a leading `[0]`
  // yields an extra '1' character when encoding all-zero inputs.
  const digits: number[] = [];
  
  for (const byte of bytes) {
    let carry = byte;
    for (let i = 0; i < digits.length; i++) {
      carry += (digits[i] ?? 0) << 8;
      digits[i] = carry % 58;
      carry = (carry / 58) | 0;
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = (carry / 58) | 0;
    }
  }
  
  // Add leading zeros for each leading 0 byte in input
  for (const byte of bytes) {
    if (byte !== 0) break;
    digits.push(0);
  }
  
  return digits.reverse().map(d => BASE58_ALPHABET[d]).join('');
}

// SHA-256 round constants (first 32 bits of the fractional parts of the cube
// roots of the first 64 primes).
const SHA256_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

/**
 * Dependency-free, synchronous SHA-256.
 *
 * Used so PDA derivation works identically in browsers, Node (both CJS and
 * ESM), and bundlers without relying on `require('crypto')` or async WebCrypto.
 */
function sha256Pure(data: Uint8Array): Uint8Array {
  const h = new Uint32Array([
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ]);

  const bitLen = data.length * 8;
  // Pad: append 0x80, then zeros, until length ≡ 56 (mod 64), then 8-byte length.
  const paddedLen = ((data.length + 8) >> 6) * 64 + 64;
  const msg = new Uint8Array(paddedLen);
  msg.set(data);
  msg[data.length] = 0x80;
  // 64-bit big-endian bit length (high 32 bits assumed 0 for our seed sizes).
  const dv = new DataView(msg.buffer);
  dv.setUint32(paddedLen - 4, bitLen >>> 0, false);
  dv.setUint32(paddedLen - 8, Math.floor(bitLen / 0x100000000), false);

  const w = new Uint32Array(64);
  for (let offset = 0; offset < paddedLen; offset += 64) {
    for (let i = 0; i < 16; i++) {
      w[i] = dv.getUint32(offset + i * 4, false);
    }
    for (let i = 16; i < 64; i++) {
      const w15 = w[i - 15]!;
      const w2 = w[i - 2]!;
      const s0 = ((w15 >>> 7) | (w15 << 25)) ^ ((w15 >>> 18) | (w15 << 14)) ^ (w15 >>> 3);
      const s1 = ((w2 >>> 17) | (w2 << 15)) ^ ((w2 >>> 19) | (w2 << 13)) ^ (w2 >>> 10);
      w[i] = (w[i - 16]! + s0 + w[i - 7]! + s1) >>> 0;
    }

    let a = h[0]!, b = h[1]!, c = h[2]!, d = h[3]!;
    let e = h[4]!, f = h[5]!, g = h[6]!, hh = h[7]!;

    for (let i = 0; i < 64; i++) {
      const S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
      const ch = (e & f) ^ (~e & g);
      const t1 = (hh + S1 + ch + SHA256_K[i]! + w[i]!) >>> 0;
      const S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const t2 = (S0 + maj) >>> 0;

      hh = g; g = f; f = e; e = (d + t1) >>> 0;
      d = c; c = b; b = a; a = (t1 + t2) >>> 0;
    }

    h[0] = (h[0]! + a) >>> 0;
    h[1] = (h[1]! + b) >>> 0;
    h[2] = (h[2]! + c) >>> 0;
    h[3] = (h[3]! + d) >>> 0;
    h[4] = (h[4]! + e) >>> 0;
    h[5] = (h[5]! + f) >>> 0;
    h[6] = (h[6]! + g) >>> 0;
    h[7] = (h[7]! + hh) >>> 0;
  }

  const out = new Uint8Array(32);
  const outView = new DataView(out.buffer);
  for (let i = 0; i < 8; i++) {
    outView.setUint32(i * 4, h[i]!, false);
  }
  return out;
}

/**
 * SHA-256 hash function (synchronous).
 */
function sha256Sync(data: Uint8Array): Uint8Array {
  return sha256Pure(data);
}

/**
 * SHA-256 hash function (async). Uses WebCrypto when available for speed,
 * otherwise falls back to the pure implementation.
 */
async function sha256Async(data: Uint8Array): Promise<Uint8Array> {
  if (typeof globalThis !== 'undefined' && globalThis.crypto && globalThis.crypto.subtle) {
    const copy = new Uint8Array(data);
    const hashBuffer = await globalThis.crypto.subtle.digest('SHA-256', copy);
    return new Uint8Array(hashBuffer);
  }
  return sha256Pure(data);
}

/**
 * Check if a 32-byte value is a valid point on the ed25519 curve.
 *
 * A valid PDA must be OFF the curve (it must NOT correspond to a real
 * ed25519 public key, so that no private key can ever sign for it).
 *
 * We determine on-curve status by attempting to decompress the candidate
 * as a compressed Edwards point. If decompression succeeds the value lies
 * on the curve; if it throws, the value is off-curve and is a valid PDA.
 * This matches the behaviour of `PublicKey.isOnCurve` in @solana/web3.js.
 */
function isOnCurve(publicKey: Uint8Array): boolean {
  if (publicKey.length !== 32) {
    return false;
  }
  try {
    Point.fromHex(publicKey);
    return true;
  } catch {
    return false;
  }
}

/**
 * PDA marker bytes appended to seeds before hashing.
 */
const PDA_MARKER = new TextEncoder().encode('ProgramDerivedAddress');

/**
 * Build the hash input buffer for PDA derivation.
 */
function buildPdaBuffer(
  seeds: Uint8Array[],
  programIdBytes: Uint8Array,
  bump: number
): Uint8Array {
  const totalLength = seeds.reduce((sum, s) => sum + s.length, 0) 
    + 1 // bump
    + 32 // programId
    + PDA_MARKER.length;
  
  const buffer = new Uint8Array(totalLength);
  let offset = 0;
  
  // Copy seeds
  for (const seed of seeds) {
    buffer.set(seed, offset);
    offset += seed.length;
  }
  
  // Add bump seed
  buffer[offset++] = bump;
  
  // Add program ID
  buffer.set(programIdBytes, offset);
  offset += 32;
  
  // Add PDA marker
  buffer.set(PDA_MARKER, offset);
  
  return buffer;
}

/**
 * Validate seeds before PDA derivation.
 */
function validateSeeds(seeds: Uint8Array[]): void {
  if (seeds.length > 16) {
    throw new Error('Maximum of 16 seeds allowed');
  }
  for (let i = 0; i < seeds.length; i++) {
    const seed = seeds[i];
    if (seed && seed.length > 32) {
      throw new Error('Seed ' + i + ' exceeds maximum length of 32 bytes');
    }
  }
}

/**
 * Derives a Program-Derived Address (PDA) from seeds and program ID.
 * 
 * Algorithm:
 * 1. For bump = 255 down to 0:
 *    a. Concatenate: seeds + [bump] + programId + "ProgramDerivedAddress"
 *    b. SHA-256 hash the concatenation
 *    c. If result is off the ed25519 curve, return it
 * 2. If no valid PDA found after 256 attempts, throw error
 * 
 * @param seeds - Array of seed buffers (max 32 bytes each, max 16 seeds)
 * @param programId - The program ID (base58 string)
 * @returns Tuple of [derivedAddress (base58), bumpSeed]
 */
export async function findProgramAddress(
  seeds: Uint8Array[],
  programId: string
): Promise<[string, number]> {
  validateSeeds(seeds);

  const programIdBytes = decodeBase58(programId);
  if (programIdBytes.length !== 32) {
    throw new Error('Program ID must be 32 bytes');
  }

  // Try bump seeds from 255 down to 0
  for (let bump = 255; bump >= 0; bump--) {
    const buffer = buildPdaBuffer(seeds, programIdBytes, bump);
    const hash = await sha256Async(buffer);
    
    if (!isOnCurve(hash)) {
      return [encodeBase58(hash), bump];
    }
  }

  throw new Error('Unable to find a valid PDA');
}

/**
 * Synchronous version of findProgramAddress.
 * Uses synchronous SHA-256 (Node.js crypto module).
 */
export function findProgramAddressSync(
  seeds: Uint8Array[],
  programId: string
): [string, number] {
  validateSeeds(seeds);

  const programIdBytes = decodeBase58(programId);
  if (programIdBytes.length !== 32) {
    throw new Error('Program ID must be 32 bytes');
  }

  // Try bump seeds from 255 down to 0
  for (let bump = 255; bump >= 0; bump--) {
    const buffer = buildPdaBuffer(seeds, programIdBytes, bump);
    const hash = sha256Sync(buffer);
    
    if (!isOnCurve(hash)) {
      return [encodeBase58(hash), bump];
    }
  }

  throw new Error('Unable to find a valid PDA');
}

/**
 * Creates a seed buffer from various input types.
 * 
 * @param value - The value to convert to a seed
 * @returns Uint8Array suitable for PDA derivation
 */
export function createSeed(value: string | Uint8Array | bigint | number): Uint8Array {
  if (value instanceof Uint8Array) {
    return value;
  }
  
  if (typeof value === 'string') {
    return new TextEncoder().encode(value);
  }
  
  if (typeof value === 'bigint') {
    // Convert bigint to 8-byte buffer (u64 little-endian)
    const buffer = new Uint8Array(8);
    let n = value;
    for (let i = 0; i < 8; i++) {
      buffer[i] = Number(n & BigInt(0xff));
      n >>= BigInt(8);
    }
    return buffer;
  }
  
  if (typeof value === 'number') {
    // Assume u64
    return createSeed(BigInt(value));
  }
  
  throw new Error('Cannot create seed from value');
}

/**
 * Creates a public key seed from a base58-encoded address.
 * 
 * @param address - Base58-encoded public key
 * @returns 32-byte Uint8Array
 */
export function createPublicKeySeed(address: string): Uint8Array {
  const decoded = decodeBase58(address);
  if (decoded.length !== 32) {
    throw new Error('Invalid public key length: expected 32, got ' + decoded.length);
  }
  return decoded;
}

// Legacy export for backwards compatibility
export { findProgramAddress as derivePda };
