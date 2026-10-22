import { encodeBase58 } from './encoding';

/** One top-level instruction: indexes into the message's account keys (static keys, then lookup-table writable entries, then read-only ones). */
export interface CompiledInstruction {
  readonly programIndex: number;
  readonly accounts: readonly number[];
  readonly data: Uint8Array;
}

/** One address lookup table a v0 message loads from. */
export interface Lookup {
  readonly table: string;
  readonly writable: readonly number[];
  readonly readonly: readonly number[];
}

/** A decoded wire transaction. */
export interface DecodedTransaction {
  /** `null` for legacy messages, else the message version. */
  readonly version: 0 | 1 | null;
  /** Signature slots, in key order; all zero when not signed yet. */
  readonly signatures: readonly Uint8Array[];
  /** Where the first signature slot starts in the wire bytes. */
  readonly signatureOffset: number;
  /** The byte range every signer signs, `[start, end)`. */
  readonly message: { readonly start: number; readonly end: number };
  readonly requiredSignatures: number;
  readonly staticKeys: readonly string[];
  /** Indexes of the static keys the message marks writable. */
  readonly writableStatic: readonly number[];
  /** The program of each top-level instruction, in order. */
  readonly instructionPrograms: readonly string[];
  readonly instructions: readonly CompiledInstruction[];
  readonly lookups: readonly Lookup[];
}

export class TransactionDecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'TransactionDecodeError';
  }
}

/** The version byte of a v1 transaction, which leads its wire bytes. */
const V1_PREFIX = 0x81;
/** Configuration mask bits a v1 message may set; each adds one 4-byte value. */
const V1_KNOWN_CONFIG_BITS = 0b11111;

class Reader {
  at: number;

  constructor(
    readonly bytes: Uint8Array,
    at = 0,
  ) {
    this.at = at;
  }

  compactU16(): number {
    let value = 0;
    for (let index = 0; index < 3; index++) {
      const byte = this.bytes[this.at];
      if (byte === undefined) throw new TransactionDecodeError('truncated transaction');
      this.at += 1;
      value |= (byte & 0x7f) << (7 * index);
      if ((byte & 0x80) === 0) return value;
    }
    throw new TransactionDecodeError('invalid compact-u16 length');
  }

  take(length: number): Uint8Array {
    if (this.at + length > this.bytes.length) {
      throw new TransactionDecodeError('truncated transaction');
    }
    const slice = this.bytes.subarray(this.at, this.at + length);
    this.at += length;
    return slice;
  }

  byte(): number {
    return this.take(1)[0]!;
  }
}

function writableIndexes(
  keyCount: number,
  required: number,
  readonlySigned: number,
  readonlyUnsigned: number,
): number[] {
  if (required > keyCount || readonlySigned > required || readonlyUnsigned > keyCount - required) {
    throw new TransactionDecodeError('invalid message header');
  }
  const out: number[] = [];
  for (let index = 0; index < keyCount; index++) {
    const writable =
      index < required ? index < required - readonlySigned : index < keyCount - readonlyUnsigned;
    if (writable) out.push(index);
  }
  return out;
}

/** Decode a legacy, v0 or v1 wire transaction. */
export function decodeTransaction(bytes: Uint8Array): DecodedTransaction {
  if (bytes[0] === V1_PREFIX) return decodeV1(bytes);
  const reader = new Reader(bytes);
  const signatureCount = reader.compactU16();
  const signatureOffset = reader.at;
  const signatures: Uint8Array[] = [];
  for (let index = 0; index < signatureCount; index++) {
    signatures.push(reader.take(64));
  }
  const messageStart = reader.at;
  const first = bytes[reader.at];
  if (first === undefined) throw new TransactionDecodeError('truncated message');
  let version: 0 | null = null;
  if ((first & 0x80) !== 0) {
    reader.at += 1;
    if ((first & 0x7f) !== 0) throw new TransactionDecodeError('unsupported transaction version');
    version = 0;
  }
  const header = reader.take(3);
  const [required, readonlySigned, readonlyUnsigned] = [header[0]!, header[1]!, header[2]!];
  const keyCount = reader.compactU16();
  const staticKeys: string[] = [];
  for (let index = 0; index < keyCount; index++) {
    staticKeys.push(encodeBase58(reader.take(32)));
  }
  const writableStatic = writableIndexes(keyCount, required, readonlySigned, readonlyUnsigned);
  reader.take(32); // recent blockhash
  const instructionCount = reader.compactU16();
  const instructionPrograms: string[] = [];
  const instructions: CompiledInstruction[] = [];
  for (let index = 0; index < instructionCount; index++) {
    const programIndex = reader.byte();
    const accounts = reader.take(reader.compactU16());
    const data = reader.take(reader.compactU16());
    instructions.push({ programIndex, accounts: [...accounts], data: data.slice() });
    const program = staticKeys[programIndex];
    if (program === undefined) {
      throw new TransactionDecodeError("an instruction's program is not a static key");
    }
    instructionPrograms.push(program);
  }
  const lookups: Lookup[] = [];
  if (version !== null) {
    const count = reader.compactU16();
    for (let index = 0; index < count; index++) {
      const table = encodeBase58(reader.take(32));
      const writable = [...reader.take(reader.compactU16())];
      const readonly = [...reader.take(reader.compactU16())];
      lookups.push({ table, writable, readonly });
    }
  }
  return {
    version,
    signatures,
    signatureOffset,
    message: { start: messageStart, end: bytes.length },
    requiredSignatures: required,
    staticKeys,
    writableStatic,
    instructionPrograms,
    instructions,
    lookups,
  };
}

/** A v1 transaction: the version byte and message, then one signature per required signer. */
function decodeV1(bytes: Uint8Array): DecodedTransaction {
  const reader = new Reader(bytes, 1);
  const header = reader.take(3);
  const [required, readonlySigned, readonlyUnsigned] = [header[0]!, header[1]!, header[2]!];
  const maskBytes = reader.take(4);
  const mask =
    (maskBytes[0]! | (maskBytes[1]! << 8) | (maskBytes[2]! << 16) | (maskBytes[3]! << 24)) >>> 0;
  if ((mask & ~V1_KNOWN_CONFIG_BITS) >>> 0 !== 0) {
    throw new TransactionDecodeError('unsupported v1 transaction configuration');
  }
  reader.take(32); // lifetime specifier
  const instructionCount = reader.byte();
  const keyCount = reader.byte();
  const staticKeys: string[] = [];
  for (let index = 0; index < keyCount; index++) {
    staticKeys.push(encodeBase58(reader.take(32)));
  }
  reader.take(popcount(mask) * 4);
  const headers = reader.take(instructionCount * 4).slice();
  const instructionPrograms: string[] = [];
  const instructions: CompiledInstruction[] = [];
  for (let offset = 0; offset < headers.length; offset += 4) {
    const programIndex = headers[offset]!;
    const accountCount = headers[offset + 1]!;
    const dataLength = headers[offset + 2]! | (headers[offset + 3]! << 8);
    const accounts = reader.take(accountCount);
    const data = reader.take(dataLength);
    instructions.push({ programIndex, accounts: [...accounts], data: data.slice() });
    const program = staticKeys[programIndex];
    if (program === undefined) {
      throw new TransactionDecodeError("an instruction's program is not an address");
    }
    instructionPrograms.push(program);
  }
  const messageEnd = reader.at;
  const signatures: Uint8Array[] = [];
  for (let index = 0; index < required; index++) {
    signatures.push(reader.take(64));
  }
  if (reader.at !== bytes.length) {
    throw new TransactionDecodeError('trailing bytes after the transaction');
  }
  return {
    version: 1,
    signatures,
    signatureOffset: messageEnd,
    message: { start: 0, end: messageEnd },
    requiredSignatures: required,
    staticKeys,
    writableStatic: writableIndexes(keyCount, required, readonlySigned, readonlyUnsigned),
    instructionPrograms,
    instructions,
    lookups: [],
  };
}

function popcount(value: number): number {
  let count = 0;
  for (let bits = value >>> 0; bits !== 0; bits >>>= 1) count += bits & 1;
  return count;
}

/** The required signers and whether each has signed. */
export function transactionSigners(decoded: DecodedTransaction): Array<[string, boolean]> {
  return decoded.staticKeys
    .slice(0, decoded.requiredSignatures)
    .map((key, index) => [key, decoded.signatures[index]?.some((byte) => byte !== 0) ?? false]);
}

/** The distinct programs invoked, in first-use order. */
export function transactionPrograms(decoded: DecodedTransaction): string[] {
  return [...new Set(decoded.instructionPrograms)];
}

export function writableStaticKeys(decoded: DecodedTransaction): string[] {
  return decoded.writableStatic.map((index) => decoded.staticKeys[index]!);
}
