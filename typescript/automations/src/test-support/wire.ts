// Builds wire transactions and intents for tests. Not part of the package.

import { bytesToHex } from '@noble/hashes/utils';
import { sha256 } from '@noble/hashes/sha2';
import type { ExecutionIntentV1, ProtocolTransactionFormat } from '@usearete/sdk';

import { decodeTransaction } from '../policy/decode';
import { decodeBase58, encodeBase64 } from '../policy/encoding';

export interface WireInstruction {
  readonly program: number;
  readonly accounts: readonly number[];
  readonly data: readonly number[];
}

export interface WireMessage {
  /** (required signatures, read-only signed, read-only unsigned) */
  readonly header: readonly [number, number, number];
  /** Base58 keys. */
  readonly keys: readonly string[];
  readonly instructions: readonly WireInstruction[];
}

function compact(value: number, out: number[]): void {
  let rest = value;
  for (;;) {
    const byte = rest & 0x7f;
    rest >>= 7;
    if (rest === 0) {
      out.push(byte);
      return;
    }
    out.push(byte | 0x80);
  }
}

function key(text: string): number[] {
  const bytes = decodeBase58(text);
  if (bytes?.length !== 32) throw new Error(`not a key: ${text}`);
  return [...bytes];
}

/** A v0 wire transaction (no lookups) with empty signature slots. */
export function v0Transaction(message: WireMessage): Uint8Array {
  const body: number[] = [0x80, ...message.header];
  compact(message.keys.length, body);
  for (const k of message.keys) body.push(...key(k));
  body.push(...new Array(32).fill(7));
  compact(message.instructions.length, body);
  for (const ix of message.instructions) {
    body.push(ix.program);
    compact(ix.accounts.length, body);
    body.push(...ix.accounts);
    compact(ix.data.length, body);
    body.push(...ix.data);
  }
  compact(0, body);
  const out: number[] = [];
  compact(message.header[0], out);
  out.push(...new Array(64 * message.header[0]).fill(0), ...body);
  return Uint8Array.from(out);
}

/** A v1 wire transaction (no config values) with empty signature slots. */
export function v1Transaction(message: WireMessage): Uint8Array {
  const out: number[] = [0x81, ...message.header, 0, 0, 0, 0, ...new Array(32).fill(7)];
  out.push(message.instructions.length, message.keys.length);
  for (const k of message.keys) out.push(...key(k));
  for (const ix of message.instructions) {
    out.push(ix.program, ix.accounts.length, ix.data.length & 0xff, ix.data.length >> 8);
  }
  for (const ix of message.instructions) out.push(...ix.accounts, ...ix.data);
  out.push(...new Array(64 * message.header[0]).fill(0));
  return Uint8Array.from(out);
}

export function wire(message: WireMessage, format: ProtocolTransactionFormat): Uint8Array {
  return format === 'v1' ? v1Transaction(message) : v0Transaction(message);
}

export function messageHash(bytes: Uint8Array): string {
  const decoded = decodeTransaction(bytes);
  return bytesToHex(sha256(bytes.subarray(decoded.message.start, decoded.message.end)));
}

/** An intent committing to `transactions` for `wallet`. */
export function intentFor(
  wallet: string,
  format: ProtocolTransactionFormat,
  transactions: readonly Uint8Array[],
  overrides: Partial<ExecutionIntentV1> = {},
): ExecutionIntentV1 {
  return {
    schema: 'arete.execution-intent/v1',
    executionId: 'exec-1',
    deployment: { deploymentId: 'dep-1', revision: 1, releaseHash: 'sha256:00' },
    step: 'deposit',
    instanceId: 'inst-1',
    participantId: null,
    wallet,
    role: null,
    callerAccount: 'acct-1',
    sessionKeyClass: 'secret',
    transactionFormat: format,
    inputSha256: '00',
    resourceHashes: [],
    providerSnapshotHash: '00',
    fees: null,
    signers: [{ class: 'wallet', pubkey: wallet, name: null }],
    transactions: transactions.map((bytes, index) => ({
      index,
      messageSha256: messageHash(bytes),
      signers: [wallet],
    })),
    blockhash: '11111111111111111111111111111111',
    lastValidBlockHeight: '100',
    expiresAt: 0,
    createdAt: 0,
    ...overrides,
  };
}

export function prepared(transactions: readonly Uint8Array[]) {
  return transactions.map((bytes, index) => ({ index, transactionBase64: encodeBase64(bytes) }));
}
