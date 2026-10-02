import { decodeBase58 } from './instructions/pda';

export const MANAGED_SOLANA_CONTRACT_VERSION = 'managed-solana/v1';
export const MAX_MANAGED_BATCH_ADDRESSES = 100;
export const MAX_MANAGED_PAGE_SIZE = 100;
export interface ManagedReadOptions {
  commitment?: 'processed' | 'confirmed' | 'finalized';
  minContextSlot?: number | bigint;
}
export interface ReadContext { slot: bigint }
export interface Contextual<T> { context: ReadContext | null; value: T }
export interface DiscoveryProvenance { source: string; observedAt: string; watermark?: bigint }
export interface OwnerTokenAccountsRequest {
  owner: string; mint?: string; tokenProgram?: string; limit?: number; cursor?: string;
}
export interface OwnerTokenAccount {
  address: string; mint: string; tokenProgram: string; owner: string;
  amount: bigint; decimals: number; state: 'initialized' | 'frozen' | 'uninitialized';
  delegate?: string; delegatedAmount?: bigint; closeAuthority?: string;
}
export interface OwnerTokenAccountsPage {
  items: OwnerTokenAccount[]; nextCursor: string | null; discovery: DiscoveryProvenance;
}
export interface NativePositionQuery { owner?: string; pool?: string; limit?: number; cursor?: string }
export interface NativePositionPage { addresses: string[]; nextCursor: string | null; discovery: DiscoveryProvenance }

export function managedU64(value: unknown, name: string): bigint {
  if (typeof value !== 'string' || !/^\d+$/.test(value)) throw new TypeError(`${name} must be a decimal u64 string`);
  const parsed = BigInt(value);
  if (parsed > 18_446_744_073_709_551_615n) throw new RangeError(`${name} exceeds u64`);
  return parsed;
}
export function managedAddress(value: string): void {
  if (typeof value !== 'string' || value.length > 44 || decodeBase58(value).length !== 32) throw new TypeError('address must be a base58-encoded 32-byte value');
}
export function managedPage(limit = MAX_MANAGED_PAGE_SIZE, cursor?: string | null): void {
  if (!Number.isInteger(limit) || limit < 1 || limit > MAX_MANAGED_PAGE_SIZE) throw new RangeError('limit must be between 1 and 100');
  if (cursor != null && (typeof cursor !== 'string' || new TextEncoder().encode(cursor).length < 1 || new TextEncoder().encode(cursor).length > 2048)) throw new RangeError('cursor must contain 1..2048 bytes');
}
export function managedReadOptions(options: ManagedReadOptions = {}): Record<string, unknown> {
  if (options.commitment !== undefined && !['processed', 'confirmed', 'finalized'].includes(options.commitment)) throw new TypeError('Invalid commitment');
  const slot = options.minContextSlot;
  if (typeof slot === 'number' && (!Number.isSafeInteger(slot) || slot < 0)) throw new RangeError('minContextSlot must be a non-negative safe integer or bigint');
  if (slot !== undefined) managedU64(slot.toString(), 'minContextSlot');
  return { commitment: options.commitment, minContextSlot: slot?.toString() };
}
export function managedContext(value: unknown, options: ManagedReadOptions, required: boolean): ReadContext | null {
  if (value === null && !required) return null;
  if (!value || typeof value !== 'object') throw new TypeError('Missing actual read context');
  const slot = managedU64((value as Record<string, unknown>).slot, 'context.slot');
  if (options.minContextSlot !== undefined && slot < BigInt(options.minContextSlot)) throw new RangeError('Read context is below minContextSlot');
  return { slot };
}
export function managedDiscovery(value: Record<string, unknown>): DiscoveryProvenance {
  if (!value || typeof value.source !== 'string' || !value.source || typeof value.observedAt !== 'string' || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/i.test(value.observedAt) || !Number.isFinite(Date.parse(value.observedAt))) throw new TypeError('Invalid discovery provenance');
  return { source: value.source, observedAt: value.observedAt, ...(value.watermark == null ? {} : { watermark: managedU64(value.watermark, 'watermark') }) };
}

/** Authoritative chain deletion; ingestion owns recognition and entity mapping. */
export interface AccountTombstone { address: string; slot: bigint; writeVersion: bigint }
export function accountTombstone(value: Record<string, unknown>): AccountTombstone {
  managedAddress(value.address as string);
  if (Object.keys(value).some(key => !['address', 'slot', 'writeVersion'].includes(key))) throw new TypeError('Unknown tombstone field');
  return { address: value.address as string, slot: managedU64(value.slot, 'slot'), writeVersion: managedU64(value.writeVersion, 'writeVersion') };
}
