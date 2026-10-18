import { sha256 } from '@noble/hashes/sha256';

import { encodeBase58 } from '../instructions/pda';

/**
 * Serialize a JSON value in the RFC 8785 JSON Canonicalization Scheme (JCS)
 * form: object members sorted by the UTF-16 code units of their names, no
 * whitespace, numbers in ECMAScript `JSON.stringify` form and strings with
 * the minimal JSON escapes.
 *
 * Throws a `TypeError` for anything that is not JSON: non-finite numbers,
 * bigints (convert them to decimal strings first), functions, symbols,
 * `undefined` outside an object member, strings with lone surrogates,
 * non-plain objects, and objects or arrays with `toJSON` hooks.
 * Object members whose value is `undefined` are omitted, as
 * `JSON.stringify` omits them.
 */
export function canonicalizeJson(value: unknown): string {
  return canonicalize(value, '$');
}

function hasLoneSurrogate(value: string): boolean {
  for (let i = 0; i < value.length; i += 1) {
    const unit = value.charCodeAt(i);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(i + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return true;
      i += 1;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      return true;
    }
  }
  return false;
}

function canonicalString(value: string, path: string): string {
  if (hasLoneSurrogate(value)) {
    throw new TypeError(`${path} contains a lone surrogate and is not valid I-JSON`);
  }
  return JSON.stringify(value);
}

function canonicalize(value: unknown, path: string): string {
  if (value === null) return 'null';
  switch (typeof value) {
    case 'boolean':
      return value ? 'true' : 'false';
    case 'number':
      if (!Number.isFinite(value)) throw new TypeError(`${path} is not a finite number`);
      return JSON.stringify(value);
    case 'string':
      return canonicalString(value, path);
    case 'bigint':
      throw new TypeError(`${path} is a bigint; convert it to a decimal string first`);
    case 'object': {
      if (typeof (value as { toJSON?: unknown }).toJSON === 'function') {
        throw new TypeError(`${path} has a toJSON hook and is not a plain JSON value`);
      }
      if (Array.isArray(value)) {
        return `[${value.map((item, index) => canonicalize(item, `${path}[${index}]`)).join(',')}]`;
      }
      const prototype = Object.getPrototypeOf(value);
      if (prototype !== Object.prototype && prototype !== null) {
        throw new TypeError(`${path} is not a plain JSON object`);
      }
      const record = value as Record<string, unknown>;
      const members = Object.keys(record)
        .filter((key) => record[key] !== undefined)
        .sort()
        .map((key) => `${canonicalString(key, path)}:${canonicalize(record[key], `${path}.${key}`)}`);
      return `{${members.join(',')}}`;
    }
    default:
      throw new TypeError(`${path} is not a JSON value (${typeof value})`);
  }
}

function hex(bytes: Uint8Array): string {
  let out = '';
  for (const byte of bytes) out += byte.toString(16).padStart(2, '0');
  return out;
}

/** Lowercase hex SHA-256 of the JCS form of a JSON value. */
export function canonicalJsonSha256(value: unknown): string {
  return hex(sha256(new TextEncoder().encode(canonicalizeJson(value))));
}

/** First line of a metadata wallet proof message. */
export const PROTOCOL_METADATA_PROOF_DOMAIN = 'arete.protocol-metadata/v1';
/** How far ahead a metadata proof may expire, in seconds. */
export const PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS = 300;

export interface ProtocolMetadataProofMessageInput {
  readonly deploymentId: string;
  readonly instanceId: string;
  /** The wallet that signs. */
  readonly wallet: string;
  /** The JSON merge patch being authorised (`null` removes a field). */
  readonly patch: Record<string, unknown>;
  /** Unix seconds; in the future and at most 300 seconds ahead. */
  readonly expiresAt: number;
  /** Unix seconds used to check `expiresAt`; defaults to the current time. */
  readonly now?: number;
}

export interface ProtocolMetadataProofMessage {
  /** The UTF-8 message, lines joined by `\n`. */
  readonly message: string;
  /** `message` as bytes, for `wallet.signMessage`. */
  readonly bytes: Uint8Array;
  /** Lowercase hex SHA-256 of the patch's JCS form. */
  readonly patchSha256: string;
  readonly expiresAt: number;
}

/** The `proof` sent with a metadata update a participant wallet authorises. */
export interface ProtocolMetadataWalletProof {
  readonly wallet: string;
  /** Base58 Ed25519 signature over the proof message. */
  readonly signature: string;
  /** Unix seconds. */
  readonly expiresAt: number;
}

/**
 * Build the message a wallet signs to authorise a metadata patch:
 *
 * ```text
 * arete.protocol-metadata/v1
 * <deployment id>
 * <instance id>
 * <wallet>
 * <lowercase hex SHA-256 of the patch's RFC 8785 canonical JSON>
 * <expiresAt, unix seconds>
 * ```
 */
export function protocolMetadataProofMessage(
  input: ProtocolMetadataProofMessageInput
): ProtocolMetadataProofMessage {
  const { deploymentId, instanceId, wallet, patch, expiresAt } = input;
  for (const [name, value] of [
    ['deploymentId', deploymentId],
    ['instanceId', instanceId],
    ['wallet', wallet],
  ] as const) {
    if (typeof value !== 'string' || value.length === 0 || /[\r\n]/.test(value)) {
      throw new TypeError(`${name} must be a non-empty single-line string`);
    }
  }
  if (!Number.isSafeInteger(expiresAt) || expiresAt <= 0) {
    throw new TypeError('expiresAt must be unix seconds');
  }
  const now = input.now ?? Math.floor(Date.now() / 1000);
  if (expiresAt <= now || expiresAt - now > PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS) {
    throw new RangeError(
      `expiresAt must be in the future and at most ${PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS} seconds ahead`
    );
  }
  const patchSha256 = canonicalJsonSha256(patch);
  const message = [
    PROTOCOL_METADATA_PROOF_DOMAIN,
    deploymentId,
    instanceId,
    wallet,
    patchSha256,
    String(expiresAt),
  ].join('\n');
  return { message, bytes: new TextEncoder().encode(message), patchSha256, expiresAt };
}

/** Turn a wallet's 64-byte Ed25519 signature into the metadata update `proof`. */
export function protocolMetadataProof(input: {
  readonly wallet: string;
  readonly signature: Uint8Array;
  readonly expiresAt: number;
}): ProtocolMetadataWalletProof {
  if (!(input.signature instanceof Uint8Array) || input.signature.length !== 64) {
    throw new TypeError('signature must be the 64-byte Ed25519 signature');
  }
  return {
    wallet: input.wallet,
    signature: encodeBase58(input.signature),
    expiresAt: input.expiresAt,
  };
}
