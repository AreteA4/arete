import { sha256 } from "@noble/hashes/sha256";

import { hashError, HashError } from "./error.js";
import {
  CANONICALIZATION_PROFILES,
  HASH_ALGORITHM,
  HASH_PROTOCOL_LABEL,
  HASH_PROTOCOL_VERSION,
  IDENTITY_REGISTRY,
  type CanonicalizationProfile,
  type HashId,
  type HashKind,
  type ParsedHashId,
} from "./types.js";

const encoder = new TextEncoder();
const lowercaseDigestPattern = /^[0-9a-f]{64}$/;

export function parseCanonicalizationProfile(value: string): CanonicalizationProfile {
  if ((CANONICALIZATION_PROFILES as readonly string[]).includes(value)) {
    return value as CanonicalizationProfile;
  }
  return hashError("invalid-hash-id", "unknown canonicalization profile");
}

export function parseHashKind(value: string): HashKind {
  const kind = IDENTITY_REGISTRY.find((metadata) => metadata.kind === value)?.kind;
  if (!kind) return hashError("unknown-kind", `unknown hash kind '${value}'`);
  return kind;
}

export function parseHashId<K extends HashKind = HashKind>(
  value: string,
  expectedKind?: K,
): ParsedHashId<K> {
  const parts = value.split(":");
  if (parts[0] !== "arete") {
    return hashError("invalid-hash-id", "hash protocol must be 'arete'");
  }
  if (parts[1] !== "h1") {
    return hashError("unknown-version", `unknown hash protocol version '${parts[1] ?? ""}'`);
  }
  if (parts.length < 3) return hashError("invalid-hash-id", "missing hash kind");
  const kind = parseHashKind(parts[2] ?? "");
  if (expectedKind !== undefined && kind !== expectedKind) {
    return hashError(
      "unexpected-kind",
      `expected hash kind '${expectedKind}', got '${kind}'`,
    );
  }
  if (parts.length < 4) return hashError("invalid-hash-id", "missing hash algorithm");
  if (parts[3] !== HASH_ALGORITHM) {
    return hashError("unknown-algorithm", `unknown hash algorithm '${parts[3] ?? ""}'`);
  }
  if (parts.length < 5) return hashError("invalid-hash-id", "missing digest");
  if (parts.length > 5) return hashError("invalid-hash-id", "too many hash components");
  const digestHex = parts[4] ?? "";
  if (!lowercaseDigestPattern.test(digestHex)) {
    return hashError(
      "invalid-hash-id",
      "digest must contain 64 lowercase hexadecimal digits",
    );
  }
  return {
    id: value as HashId<K>,
    kind: kind as K,
    digest: hexToBytes(digestHex),
    digestHex,
  };
}

export function isHashId<K extends HashKind>(value: unknown, kind?: K): value is HashId<K> {
  if (typeof value !== "string") return false;
  try {
    parseHashId(value, kind);
    return true;
  } catch (error) {
    if (error instanceof HashError) return false;
    throw error;
  }
}

export function framedPreimage(
  kind: HashKind,
  profile: CanonicalizationProfile,
  payload: Uint8Array,
): Uint8Array {
  return concatBytes(
    frameBytes(encoder.encode(HASH_PROTOCOL_LABEL)),
    u32be(HASH_PROTOCOL_VERSION),
    frameBytes(encoder.encode(kind)),
    frameBytes(encoder.encode(profile)),
    frameBytes(payload),
  );
}

export function createHashId<K extends HashKind>(kind: K, digest: Uint8Array): HashId<K> {
  if (digest.length !== 32) {
    return hashError("invalid-hash-id", "SHA-256 digest must contain 32 bytes");
  }
  return `arete:h1:${kind}:sha256:${bytesToHex(digest)}` as HashId<K>;
}

export function hashCanonicalPayload<K extends HashKind>(
  kind: K,
  profile: CanonicalizationProfile,
  payload: Uint8Array,
): HashId<K> {
  return createHashId(kind, sha256(framedPreimage(kind, profile, payload)));
}

export function frameBytes(bytes: Uint8Array): Uint8Array {
  return concatBytes(u64be(bytes.length), bytes);
}

export function u64be(value: number): Uint8Array {
  if (!Number.isSafeInteger(value) || value < 0) {
    return hashError("serialization", "framed byte length must be a safe non-negative integer");
  }
  const output = new Uint8Array(8);
  new DataView(output.buffer).setBigUint64(0, BigInt(value), false);
  return output;
}

function u32be(value: number): Uint8Array {
  const output = new Uint8Array(4);
  new DataView(output.buffer).setUint32(0, value, false);
  return output;
}

export function concatBytes(...parts: readonly Uint8Array[]): Uint8Array {
  const length = parts.reduce((total, part) => total + part.length, 0);
  const output = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.length;
  }
  return output;
}

export function bytesToHex(bytes: Uint8Array): string {
  let output = "";
  for (const byte of bytes) output += byte.toString(16).padStart(2, "0");
  return output;
}

export function hexToBytes(value: string): Uint8Array {
  if (value.length % 2 !== 0 || !/^[0-9a-f]*$/.test(value)) {
    return hashError("serialization", "hex input must contain lowercase byte pairs");
  }
  const output = new Uint8Array(value.length / 2);
  for (let index = 0; index < output.length; index += 1) {
    output[index] = Number.parseInt(value.slice(index * 2, index * 2 + 2), 16);
  }
  return output;
}
