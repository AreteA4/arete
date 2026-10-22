import type { ProtocolTransactionFormat } from '@usearete/sdk';

/** What the API prepares by default, then the fallback. */
export const DEFAULT_TRANSACTION_FORMATS: readonly ProtocolTransactionFormat[] = ['v1', 'v0'];

/**
 * The first format in `preferred` (the deployment's order) that the signer
 * supports, or `null` when they share none.
 */
export function negotiateTransactionFormat(
  signerFormats: readonly ProtocolTransactionFormat[],
  preferred: readonly ProtocolTransactionFormat[] = DEFAULT_TRANSACTION_FORMATS,
): ProtocolTransactionFormat | null {
  return preferred.find((format) => signerFormats.includes(format)) ?? null;
}
