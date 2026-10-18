import * as ed25519 from '@noble/ed25519';
import { sha256 } from '@noble/hashes/sha256';

import { decodeBase58, encodeBase58 } from '../instructions/pda';
import { canonicalizeJson } from './metadata-proof';

/**
 * Execution receipts: check a receipt offline, and check its claims against
 * the transaction itself. Every SDK runs the shared vectors in
 * `test-vectors/protocol-client/execution-receipt-v1.json`.
 *
 * - {@link verifyReceipt} checks the receipt-key signatures. A receipt's body
 *   is every field except `signatures`; each signature is Ed25519 over
 *   `arete.protocol-execution-receipt/v1\n<keyId>\n<bodySha256>`, where
 *   `bodySha256` is the lowercase hex SHA-256 of the body in RFC 8785 (JCS)
 *   form. A retired key verifies only receipts issued before `retiredAt`.
 * - {@link verifyReceiptOnChain} checks a transaction fetched from any RPC:
 *   its first signature and message hash, and the attested Memo v2
 *   (`arete:p2:<shortId>:<scope|->:<instance>:<participant|->:<step>:<n>/<count>`)
 *   that the attestation key signs.
 */

export const PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1 = 'arete.protocol-execution-receipt/v1';
export const PROTOCOL_RECEIPT_KEYS_SCHEMA_V1 = 'arete.protocol-receipt-keys/v1';
export const MEMO_V2_PREFIX = 'arete:p2:';
export const MEMO_PROGRAM_ID = 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr';
const LEGACY_MEMO_PROGRAM_ID = 'Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo';

/** A signed execution receipt, as `GET /api/protocol-receipts/:signature` returns it. */
export interface ProtocolExecutionReceipt {
  readonly schema: typeof PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1;
  /** The transaction's first signature, base58. */
  readonly signature: string;
  /** Decimal string. */
  readonly slot: string;
  readonly deployment: {
    readonly deploymentId: string;
    /** The deployment short id the memo carries. */
    readonly shortId: string;
    readonly revision: number;
    readonly releaseHash: string;
  };
  readonly trustTier: 'reviewed' | 'unreviewed';
  /** Present exactly when `trustTier` is `reviewed`. */
  readonly reviewHash?: string;
  readonly step: string;
  readonly instanceId: string;
  readonly participantId?: string;
  /** `index` counts from 0; the memo's `<n>/<count>` from 1. */
  readonly transaction: { readonly index: number; readonly count: number };
  /** Lowercase hex SHA-256 of the exact message bytes. */
  readonly messageSha256: string;
  readonly intentHash: string;
  readonly attestationPubkey: string;
  readonly fees?: {
    readonly asset: { readonly kind: 'native' } | { readonly kind: 'spl'; readonly mint: string };
    readonly platform: string;
    readonly creator: string;
  };
  readonly status: 'confirmed' | 'finalized';
  /** Unix seconds. */
  readonly issuedAt: number;
  readonly signatures: ReadonlyArray<{ readonly keyId: string; readonly signature: string }>;
}

/** The published receipt keys (`GET /api/protocol-receipts/keys`). */
export interface ProtocolReceiptKeySet {
  readonly schema: typeof PROTOCOL_RECEIPT_KEYS_SCHEMA_V1;
  readonly keys: ReadonlyArray<{
    readonly keyId: string;
    /** Base58 Ed25519 public key. */
    readonly publicKey: string;
    /** Unix seconds. Receipts issued at or after this time do not verify with this key. */
    readonly retiredAt?: number;
  }>;
}

export const RECEIPT_ERROR_CODES = [
  'invalid-receipt',
  'unknown-key',
  'key-retired',
  'bad-signature',
  'invalid-transaction',
  'transaction-failed',
  'slot-mismatch',
  'signature-mismatch',
  'message-hash-mismatch',
  'attestation-not-signer',
  'memo-missing',
  'memo-not-unique',
  'memo-invalid',
  'memo-mismatch',
  'attestation-signature-invalid',
] as const;
export type ReceiptErrorCode = (typeof RECEIPT_ERROR_CODES)[number];

export type ReceiptVerification =
  | { readonly valid: true; readonly keyId: string }
  | { readonly valid: false; readonly error: ReceiptErrorCode; readonly message: string };

export type ReceiptOnChainVerification =
  | { readonly valid: true }
  | { readonly valid: false; readonly error: ReceiptErrorCode; readonly message: string };

/**
 * A transaction from any RPC: base64 wire bytes, the raw bytes, or the result
 * of `getTransaction` with `encoding: "base64"`.
 */
export type ReceiptTransactionInput =
  | string
  | Uint8Array
  | {
      readonly slot?: number | null;
      readonly transaction: unknown;
      readonly meta?: { readonly err?: unknown } | null;
    };

const encoder = new TextEncoder();

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function hex(bytes: Uint8Array): string {
  let out = '';
  for (const byte of bytes) out += byte.toString(16).padStart(2, '0');
  return out;
}

const BASE64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function decodeBase64(text: string): Uint8Array | undefined {
  if (text.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(text)) return undefined;
  const padding = text.endsWith('==') ? 2 : text.endsWith('=') ? 1 : 0;
  const out = new Uint8Array((text.length / 4) * 3 - padding);
  let at = 0;
  for (let i = 0; i < text.length; i += 4) {
    const n =
      (BASE64.indexOf(text[i]!) << 18) |
      (BASE64.indexOf(text[i + 1]!) << 12) |
      ((text[i + 2] === '=' ? 0 : BASE64.indexOf(text[i + 2]!)) << 6) |
      (text[i + 3] === '=' ? 0 : BASE64.indexOf(text[i + 3]!));
    for (const shift of [16, 8, 0]) {
      if (at < out.length) out[at++] = (n >> shift) & 0xff;
    }
  }
  return out;
}

function tryBase58(text: string): Uint8Array | undefined {
  try {
    return decodeBase58(text);
  } catch {
    return undefined;
  }
}

/** The receipt body (`signatures` removed) in RFC 8785 (JCS) form. */
export function receiptCanonicalBody(receipt: unknown): string {
  if (!isRecord(receipt)) throw new TypeError('a receipt is a JSON object');
  const { signatures: _signatures, ...body } = receipt;
  return canonicalizeJson(body);
}

/** The exact bytes a receipt key signs. */
export function receiptSigningMessage(keyId: string, bodySha256: string): Uint8Array {
  return encoder.encode(`${PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1}\n${keyId}\n${bodySha256}`);
}

async function ed25519Verify(signature: Uint8Array, message: Uint8Array, publicKey: Uint8Array): Promise<boolean> {
  try {
    return await ed25519.verifyAsync(signature, message, publicKey);
  } catch {
    return false;
  }
}

/**
 * Verify a receipt's signatures against the published key set, offline. The
 * receipt is valid when any signature verifies with a key that was not
 * retired when the receipt was issued. Pass the receipt exactly as served.
 */
export async function verifyReceipt(receipt: unknown, keys: ProtocolReceiptKeySet): Promise<ReceiptVerification> {
  const fail = (error: ReceiptErrorCode, message: string): ReceiptVerification => ({ valid: false, error, message });
  if (!isRecord(receipt) || receipt.schema !== PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1) {
    return fail('invalid-receipt', 'not an arete.protocol-execution-receipt/v1');
  }
  const issuedAt = receipt.issuedAt;
  if (typeof issuedAt !== 'number' || !Number.isSafeInteger(issuedAt)) {
    return fail('invalid-receipt', 'issuedAt must be unix seconds');
  }
  const signatures = Array.isArray(receipt.signatures)
    ? receipt.signatures.filter(
        (entry): entry is { keyId: string; signature: string } =>
          isRecord(entry) && typeof entry.keyId === 'string' && typeof entry.signature === 'string'
      )
    : [];
  if (signatures.length === 0) return fail('invalid-receipt', 'the receipt carries no signatures');
  let body: string;
  try {
    body = receiptCanonicalBody(receipt);
  } catch (error) {
    return fail('invalid-receipt', (error as Error).message);
  }
  const bodySha256 = hex(sha256(encoder.encode(body)));
  let bad = false;
  let retired = false;
  for (const entry of signatures) {
    const key = keys.keys.find((candidate) => candidate.keyId === entry.keyId);
    if (key === undefined) continue;
    if (key.retiredAt !== undefined && issuedAt >= key.retiredAt) {
      retired = true;
      continue;
    }
    const signature = decodeBase64(entry.signature);
    const publicKey = tryBase58(key.publicKey);
    const message = receiptSigningMessage(entry.keyId, bodySha256);
    if (
      signature?.length === 64 &&
      publicKey?.length === 32 &&
      (await ed25519Verify(signature, message, publicKey))
    ) {
      return { valid: true, keyId: entry.keyId };
    }
    bad = true;
  }
  if (bad) return fail('bad-signature', 'no receipt signature verifies');
  if (retired) return fail('key-retired', 'the receipt was signed by a key retired before it was issued');
  return fail('unknown-key', 'no receipt signature names a published key');
}

// ---------------------------------------------------------------------------
// On-chain check
// ---------------------------------------------------------------------------

interface Instruction {
  program: number;
  accounts: number[];
  data: Uint8Array;
}

interface WireTransaction {
  signatures: Uint8Array[];
  message: Uint8Array;
  requiredSigners: number;
  staticKeys: Uint8Array[];
  instructions: Instruction[];
}

class Reader {
  at = 0;
  constructor(readonly bytes: Uint8Array) {}

  take(length: number): Uint8Array {
    if (this.at + length > this.bytes.length) throw new RangeError('truncated transaction');
    const slice = this.bytes.subarray(this.at, this.at + length);
    this.at += length;
    return slice;
  }

  byte(): number {
    return this.take(1)[0]!;
  }

  compact(): number {
    let value = 0;
    for (let index = 0; index < 3; index += 1) {
      const byte = this.byte();
      value |= (byte & 0x7f) << (7 * index);
      if ((byte & 0x80) === 0) return value;
    }
    throw new RangeError('invalid compact-u16');
  }
}

const V1_KNOWN_CONFIG_BITS = 0b11111;

function popcount(value: number): number {
  let count = 0;
  for (let bits = value; bits !== 0; bits >>>= 1) count += bits & 1;
  return count;
}

/** Decode a legacy, v0 or V1 wire transaction; throws on malformed bytes. */
function decodeWire(bytes: Uint8Array): WireTransaction {
  const reader = new Reader(bytes);
  if (bytes[0] === 0x81) {
    reader.at = 1;
    const header = reader.take(3);
    const required = header[0]!;
    const maskBytes = reader.take(4);
    const mask = (maskBytes[0]! | (maskBytes[1]! << 8) | (maskBytes[2]! << 16) | (maskBytes[3]! << 24)) >>> 0;
    if ((mask & ~V1_KNOWN_CONFIG_BITS) !== 0) throw new RangeError('unsupported V1 configuration');
    reader.take(32);
    const instructionCount = reader.byte();
    const keyCount = reader.byte();
    const staticKeys = Array.from({ length: keyCount }, () => reader.take(32));
    reader.take(popcount(mask) * 4);
    const headers = reader.take(instructionCount * 4);
    const instructions: Instruction[] = [];
    for (let i = 0; i < instructionCount; i += 1) {
      const program = headers[i * 4]!;
      const accountCount = headers[i * 4 + 1]!;
      const dataLength = headers[i * 4 + 2]! | (headers[i * 4 + 3]! << 8);
      const accounts = Array.from(reader.take(accountCount));
      instructions.push({ program, accounts, data: reader.take(dataLength) });
    }
    const message = bytes.subarray(0, reader.at);
    const signatures = Array.from({ length: required }, () => reader.take(64));
    if (reader.at !== bytes.length || required > keyCount) throw new RangeError('malformed V1 transaction');
    return { signatures, message, requiredSigners: required, staticKeys, instructions };
  }
  const signatureCount = reader.compact();
  const signatures = Array.from({ length: signatureCount }, () => reader.take(64));
  const messageStart = reader.at;
  const first = bytes[reader.at];
  if (first === undefined) throw new RangeError('truncated message');
  const versioned = (first & 0x80) !== 0;
  if (versioned) {
    if (first !== 0x80) throw new RangeError('unsupported transaction version');
    reader.at += 1;
  }
  const header = reader.take(3);
  const required = header[0]!;
  const keyCount = reader.compact();
  const staticKeys = Array.from({ length: keyCount }, () => reader.take(32));
  reader.take(32);
  const instructionCount = reader.compact();
  const instructions: Instruction[] = [];
  for (let i = 0; i < instructionCount; i += 1) {
    const program = reader.byte();
    const accounts = Array.from(reader.take(reader.compact()));
    const data = reader.take(reader.compact());
    instructions.push({ program, accounts, data });
  }
  if (versioned) {
    const lookups = reader.compact();
    for (let i = 0; i < lookups; i += 1) {
      reader.take(32);
      reader.take(reader.compact());
      reader.take(reader.compact());
    }
  }
  if (reader.at !== bytes.length || signatureCount !== required || required > keyCount) {
    throw new RangeError('malformed transaction');
  }
  return { signatures, message: bytes.subarray(messageStart), requiredSigners: required, staticKeys, instructions };
}

/** The memo form of a lowercase UUID: base58 of its 16 bytes. */
export function receiptMemoId(uuid: string): string | undefined {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(uuid)) return undefined;
  const hexText = uuid.replace(/-/g, '');
  const bytes = new Uint8Array(16);
  for (let i = 0; i < 16; i += 1) bytes[i] = parseInt(hexText.slice(i * 2, i * 2 + 2), 16);
  return encodeBase58(bytes);
}

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i += 1) if (left[i] !== right[i]) return false;
  return true;
}

class Failure extends Error {
  constructor(
    readonly code: ReceiptErrorCode,
    message: string
  ) {
    super(message);
  }
}

function transactionBytes(input: ReceiptTransactionInput): { bytes: Uint8Array; slot?: number } {
  const fromBase64 = (text: string) => {
    const bytes = decodeBase64(text);
    if (bytes === undefined) throw new Failure('invalid-transaction', 'the transaction is not base64');
    return bytes;
  };
  if (input instanceof Uint8Array) return { bytes: input };
  if (typeof input === 'string') return { bytes: fromBase64(input) };
  if (!isRecord(input)) {
    throw new Failure('invalid-transaction', 'expected base64 wire bytes or a getTransaction result');
  }
  const error = isRecord(input.meta) ? input.meta.err : undefined;
  if (error !== undefined && error !== null) {
    throw new Failure('transaction-failed', `the transaction failed: ${JSON.stringify(error)}`);
  }
  let slot: number | undefined;
  if (input.slot !== undefined && input.slot !== null) {
    if (typeof input.slot !== 'number' || !Number.isSafeInteger(input.slot)) {
      throw new Failure('invalid-transaction', 'slot must be an integer');
    }
    slot = input.slot;
  }
  const pair = input.transaction;
  if (!Array.isArray(pair) || pair[1] !== 'base64' || typeof pair[0] !== 'string') {
    throw new Failure(
      'invalid-transaction',
      'fetch the transaction with encoding "base64" so its message bytes are exact'
    );
  }
  return { bytes: fromBase64(pair[0]), slot };
}

function parseMemo(text: string) {
  if (!text.startsWith(MEMO_V2_PREFIX)) return undefined;
  const parts = text.slice(MEMO_V2_PREFIX.length).split(':');
  if (parts.length !== 6) return undefined;
  const [shortId, , instance, participant, step, position] = parts as [string, string, string, string, string, string];
  return { shortId, instance, participant, step, position };
}

/**
 * Check a receipt's claims against the transaction itself, fetched from any
 * RPC. With a `getTransaction` result, its `slot` and `meta.err` are checked
 * too. This does not check the receipt's own signature: use
 * {@link verifyReceipt} for that.
 */
export async function verifyReceiptOnChain(
  receipt: ProtocolExecutionReceipt,
  transaction: ReceiptTransactionInput
): Promise<ReceiptOnChainVerification> {
  try {
    await checkOnChain(receipt, transaction);
    return { valid: true };
  } catch (error) {
    if (error instanceof Failure) return { valid: false, error: error.code, message: error.message };
    throw error;
  }
}

async function checkOnChain(receipt: ProtocolExecutionReceipt, input: ReceiptTransactionInput): Promise<void> {
  if (!isRecord(receipt) || !isRecord(receipt.deployment) || !isRecord(receipt.transaction)) {
    throw new Failure('invalid-receipt', 'not an execution receipt');
  }
  const { bytes, slot } = transactionBytes(input);
  if (slot !== undefined && String(slot) !== receipt.slot) {
    throw new Failure('slot-mismatch', `the transaction landed in slot ${slot}, not ${receipt.slot}`);
  }
  let tx: WireTransaction;
  try {
    tx = decodeWire(bytes);
  } catch {
    throw new Failure('invalid-transaction', 'not a legacy, v0 or V1 wire transaction');
  }
  const first = tx.signatures[0];
  if (first === undefined || encodeBase58(first) !== receipt.signature) {
    throw new Failure('signature-mismatch', "the transaction's signature is not the receipt's");
  }
  if (hex(sha256(tx.message)) !== receipt.messageSha256) {
    throw new Failure('message-hash-mismatch', 'the message hash differs from the receipt\'s');
  }
  const attestation = tryBase58(receipt.attestationPubkey);
  if (attestation?.length !== 32) throw new Failure('invalid-receipt', 'attestationPubkey is not a public key');
  const signerIndex = tx.staticKeys
    .slice(0, tx.requiredSigners)
    .findIndex((key) => equalBytes(key, attestation));
  if (signerIndex < 0) {
    throw new Failure('attestation-not-signer', 'the attestation key does not sign the transaction');
  }
  const memoProgram = decodeBase58(MEMO_PROGRAM_ID);
  const legacyMemoProgram = decodeBase58(LEGACY_MEMO_PROGRAM_ID);
  const programOf = (ix: Instruction) => tx.staticKeys[ix.program];
  const listing = tx.instructions.filter((ix) => {
    const program = programOf(ix);
    const memo = program !== undefined && (equalBytes(program, memoProgram) || equalBytes(program, legacyMemoProgram));
    return memo && ix.accounts.some((account) => {
      const key = tx.staticKeys[account];
      return key !== undefined && equalBytes(key, attestation);
    });
  });
  if (listing.length === 0) throw new Failure('memo-missing', 'no memo instruction lists the attestation key');
  if (listing.length > 1) {
    throw new Failure('memo-not-unique', 'more than one memo instruction lists the attestation key');
  }
  const memo = listing[0]!;
  const program = programOf(memo);
  let text: string | undefined;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(memo.data);
  } catch {
    text = undefined;
  }
  const fields = text === undefined ? undefined : parseMemo(text);
  if (program === undefined || !equalBytes(program, memoProgram) || memo.accounts.length !== 1 || fields === undefined) {
    throw new Failure('memo-invalid', "the attestation key's instruction is not an attested Memo v2");
  }
  const participant = receipt.participantId === undefined ? '-' : receiptMemoId(receipt.participantId);
  const matches =
    fields.shortId === receipt.deployment.shortId &&
    fields.instance === receiptMemoId(receipt.instanceId) &&
    fields.participant === participant &&
    fields.step === receipt.step &&
    fields.position === `${receipt.transaction.index + 1}/${receipt.transaction.count}`;
  if (!matches) {
    throw new Failure(
      'memo-mismatch',
      'the attested memo names a different deployment, instance, participant, step or position'
    );
  }
  const signature = tx.signatures[signerIndex];
  if (signature === undefined || !(await ed25519Verify(signature, tx.message, attestation))) {
    throw new Failure(
      'attestation-signature-invalid',
      "the attestation key's signature does not verify over the message"
    );
  }
}
