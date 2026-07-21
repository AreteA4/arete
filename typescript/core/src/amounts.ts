import type { ChainClient } from './chain';
import type { SchemaResult } from './types';

/**
 * A token amount expressed either in UI units (human decimal string/number)
 * or raw base units. A bare bigint is treated as raw.
 */
export type AmountInput =
  | bigint
  | { ui: string | number }
  | { raw: bigint | string | number };

export interface AmountResolutionInput {
  mint: string;
  amount: AmountInput;
  decimals?: number;
}

/** Convert a UI amount ("1.5") to raw base units using string math (no float precision loss). */
export function parseUiAmountToRaw(value: string | number, decimals: number): bigint {
  const trimmed = String(value).trim();
  if (!/^\d+(?:\.\d+)?$/.test(trimmed)) {
    throw new Error(`Invalid UI amount: ${value}`);
  }

  const [wholePart, fractionPart = ''] = trimmed.split('.');
  if (fractionPart.length > decimals) {
    const excess = fractionPart.slice(decimals);
    if (/[1-9]/.test(excess)) {
      throw new Error(`UI amount ${value} has more fractional digits than the mint's ${decimals} decimals`);
    }
  }
  const fraction = fractionPart.padEnd(decimals, '0');
  const whole = BigInt(wholePart || '0') * 10n ** BigInt(decimals);
  const fractional = BigInt(fraction.slice(0, decimals) || '0');
  return whole + fractional;
}

/** Format raw base units as a UI decimal string (inverse of {@link parseUiAmountToRaw}). */
export function formatRawToUi(raw: bigint | string | number, decimals: number): string {
  const value = BigInt(raw);
  const negative = value < 0n;
  const magnitude = negative ? -value : value;
  const scale = 10n ** BigInt(decimals);
  const whole = magnitude / scale;
  const fraction = magnitude % scale;
  const sign = negative ? '-' : '';
  if (decimals === 0 || fraction === 0n) {
    return `${sign}${whole.toString()}`;
  }
  const fractionText = fraction.toString().padStart(decimals, '0').replace(/0+$/, '');
  return `${sign}${whole.toString()}.${fractionText}`;
}

/** Resolve an {@link AmountInput} to raw base units with known decimals. */
export function toRawAmount(amount: AmountInput, decimals: number): bigint {
  if (typeof amount === 'bigint') {
    return amount;
  }
  if ('raw' in amount) {
    return BigInt(amount.raw);
  }
  return parseUiAmountToRaw(amount.ui, decimals);
}

/** Convert an amount to raw units without throwing on invalid user input. */
export function safeToRawAmount(
  amount: AmountInput,
  decimals: number
): SchemaResult<bigint> {
  try {
    return { success: true, data: toRawAmount(amount, decimals) };
  } catch (error) {
    return { success: false, error };
  }
}

/** Fetch a mint's decimals via the chain read endpoint, throwing when unavailable. */
export async function getMintDecimals(chain: ChainClient, mint: string): Promise<number> {
  const account = await chain.mint(mint);
  if (!account || account.decimals == null) {
    throw new Error(`Mint ${mint} is missing decimals on the configured read endpoint.`);
  }
  return account.decimals;
}

/**
 * Resolve an {@link AmountInput} to raw base units, fetching the mint's
 * decimals only when they are unknown and actually needed (a bare bigint or
 * `{raw}` input with explicit `decimals` never touches the network).
 */
export async function resolveAmount(
  chain: ChainClient,
  input: AmountResolutionInput
): Promise<{ raw: bigint; decimals: number }> {
  const needsDecimalsForParse =
    typeof input.amount !== 'bigint' && 'ui' in input.amount;

  if (!needsDecimalsForParse) {
    const raw = typeof input.amount === 'bigint'
      ? input.amount
      : BigInt((input.amount as { raw: bigint | string | number }).raw);
    const decimals = input.decimals ?? await getMintDecimals(chain, input.mint);
    return { raw, decimals };
  }

  const decimals = input.decimals ?? await getMintDecimals(chain, input.mint);
  return { raw: toRawAmount(input.amount, decimals), decimals };
}

/**
 * Resolve an {@link AmountInput} to raw base units without forcing a decimals
 * fetch when the input is already expressed in raw units.
 */
export async function resolveAmountToRaw(
  chain: ChainClient,
  input: AmountResolutionInput
): Promise<bigint> {
  if (typeof input.amount === 'bigint') {
    return input.amount;
  }
  if ('raw' in input.amount) {
    return BigInt(input.amount.raw);
  }

  const decimals = input.decimals ?? await getMintDecimals(chain, input.mint);
  return toRawAmount(input.amount, decimals);
}

/** Resolve a named set of {@link AmountInput} values to raw base units. */
export async function resolveAmountsToRaw<TInputs extends Record<string, AmountResolutionInput>>(
  chain: ChainClient,
  inputs: TInputs
): Promise<{ [K in keyof TInputs]: bigint }> {
  const entries = await Promise.all(
    Object.entries(inputs).map(async ([name, input]) => {
      const raw = await resolveAmountToRaw(chain, input);
      return [name, raw] as const;
    })
  );

  return Object.fromEntries(entries) as { [K in keyof TInputs]: bigint };
}
