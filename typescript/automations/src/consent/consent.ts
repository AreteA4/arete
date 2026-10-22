/**
 * Consent: what a user signs to let automations run steps for their wallet.
 *
 * The proof is a human-readable, domain-bound message in the style of
 * Sign-In-With-Solana. It is built from the grant's fields, so the backend
 * rebuilds it from the fields it is given and checks the wallet's signature
 * over that, rather than parsing a message. Raising a limit or changing a
 * setting needs a new signature; revoking does not.
 *
 * The signature is an accountability and anti-bug record. It does not
 * protect against a compromised backend; the wallet provider's policy and
 * the protocol's on-chain rules do.
 */

import { canonicalizeJson } from '@usearete/sdk';

import { isNativeMint } from '../policy/authority';
import { isPubkey } from '../policy/encoding';
import type { PreparedSummary } from '../policy/summary';
import { type Duration, durationMs } from '../runtime/time';
import type { StoredConsentLimits } from '../store/store';

/** The asset key for SOL (wrapped SOL included), in lamports. */
export const SOL = 'SOL';

export const CONSENT_PROOF_VERSION = 'arete.automation-consent/v1';

type Amount = string | number | bigint;

/** Spending limits, per asset: `SOL` (lamports) or a mint (base units). */
export interface ConsentLimits {
  /** The most one step may move out of the wallet. An asset without a limit may not leave. */
  readonly perExecution?: Readonly<Record<string, Amount>>;
  /** The most all steps together may move out within a rolling window. */
  readonly perWindow?: Readonly<Record<string, { readonly amount: Amount; readonly window: Duration }>>;
}

/** What a user consents to. */
export interface ConsentFields {
  readonly wallet: string;
  readonly deploymentId: string;
  /** One instance, or omit for every instance of the deployment. */
  readonly instanceId?: string | null;
  readonly steps: readonly string[];
  readonly limits: ConsentLimits;
  /** The user's settings for the steps, for example `{ amount: '50000000', weekday: 'fri' }`. */
  readonly params?: Readonly<Record<string, unknown>>;
  /** When the grant ends (a `Date` or milliseconds since the epoch). */
  readonly expiresAt: Date | number;
}

export interface NormalizedConsent {
  readonly wallet: string;
  readonly deploymentId: string;
  readonly instanceId: string | null;
  readonly steps: string[];
  readonly limits: StoredConsentLimits;
  readonly params: Record<string, unknown>;
  readonly expiresAt: number;
}

export type ConsentErrorCode =
  | 'invalid_fields'
  | 'invalid_signature'
  | 'nonce_not_usable'
  | 'proof_stale'
  | 'expired'
  | 'no_domain';

export class ConsentError extends Error {
  readonly code: ConsentErrorCode;

  constructor(code: ConsentErrorCode, message: string) {
    super(message);
    this.name = 'ConsentError';
    this.code = code;
  }
}

function singleLine(name: string, value: unknown): string {
  if (typeof value !== 'string' || value.length === 0 || /[\r\n]/.test(value)) {
    throw new ConsentError('invalid_fields', `${name} must be a non-empty single-line string`);
  }
  return value;
}

function amountText(name: string, value: Amount): string {
  let amount: bigint | undefined;
  if (typeof value === 'bigint' && value >= 0n) amount = value;
  else if (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0) amount = BigInt(value);
  else if (typeof value === 'string' && /^\d+$/.test(value)) amount = BigInt(value);
  if (amount === undefined) throw new ConsentError('invalid_fields', `${name} must be a non-negative integer`);
  return amount.toString();
}

function asset(name: string, key: string): string {
  if (key !== SOL && !isPubkey(key)) {
    throw new ConsentError('invalid_fields', `${name}: '${key}' is neither SOL nor a mint address`);
  }
  return key;
}

function sortedRecord<T>(entries: Array<[string, T]>): Record<string, T> {
  return Object.fromEntries(entries.sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0)));
}

/** Validate and normalize consent fields. Throws {@link ConsentError}. */
export function normalizeConsent(fields: ConsentFields): NormalizedConsent {
  const wallet = singleLine('wallet', fields.wallet);
  if (!isPubkey(wallet)) throw new ConsentError('invalid_fields', 'wallet must be a public key');
  const deploymentId = singleLine('deploymentId', fields.deploymentId);
  const instanceId =
    fields.instanceId === undefined || fields.instanceId === null ? null : singleLine('instanceId', fields.instanceId);
  if (!Array.isArray(fields.steps) || fields.steps.length === 0) {
    throw new ConsentError('invalid_fields', 'steps must list at least one step');
  }
  const steps = [...new Set(fields.steps.map((step, index) => singleLine(`steps[${index}]`, step)))].sort();
  if (steps.some((step) => /[,\s]/.test(step))) {
    throw new ConsentError('invalid_fields', 'step names cannot contain commas or spaces');
  }
  const perExecution = sortedRecord(
    Object.entries(fields.limits?.perExecution ?? {}).map(([key, value]) => [
      asset('limits.perExecution', key),
      amountText(`limits.perExecution.${key}`, value),
    ]),
  );
  const perWindow = sortedRecord(
    Object.entries(fields.limits?.perWindow ?? {}).map(([key, value]) => {
      const windowMs = durationMs(value.window);
      if (windowMs <= 0 || !Number.isSafeInteger(windowMs)) {
        throw new ConsentError('invalid_fields', `limits.perWindow.${key}.window must be a positive duration`);
      }
      return [asset('limits.perWindow', key), { amount: amountText(`limits.perWindow.${key}.amount`, value.amount), windowMs }];
    }),
  );
  // A deep snapshot: the grant stores exactly what the message states, even
  // if the caller changes its objects while the grant is verified.
  let params: Record<string, unknown>;
  try {
    params = JSON.parse(canonicalizeJson(fields.params ?? {})) as Record<string, unknown>;
  } catch (error) {
    throw new ConsentError('invalid_fields', `params must be JSON: ${(error as Error).message}`);
  }
  const expiresAt = fields.expiresAt instanceof Date ? fields.expiresAt.getTime() : fields.expiresAt;
  if (!Number.isSafeInteger(expiresAt) || expiresAt <= 0) {
    throw new ConsentError('invalid_fields', 'expiresAt must be a date');
  }
  return { wallet, deploymentId, instanceId, steps, limits: { perExecution, perWindow }, params, expiresAt };
}

/** A window as the message shows it: the largest whole unit. */
export function windowText(windowMs: number): string {
  for (const [unit, size] of [
    ['d', 86_400_000],
    ['h', 3_600_000],
    ['m', 60_000],
    ['s', 1000],
  ] as const) {
    if (windowMs % size === 0) return `${windowMs / size}${unit}`;
  }
  return `${windowMs}ms`;
}

function amountLine(key: string, amount: string): string {
  return key === SOL ? `${amount} lamports of SOL` : `${amount} base units of ${key}`;
}

/**
 * The message a wallet signs to grant consent. Lines are joined by `\n`:
 *
 * ```text
 * <domain> asks to run protocol steps for your wallet without asking each time:
 * <wallet>
 *
 * Deployment: <deployment id>
 * Instance: <instance id, or "any">
 * Steps: <steps, sorted, comma-separated>
 * Per run: <amount> lamports of SOL | <amount> base units of <mint>   (one line per asset, sorted)
 * Per <window>: <amount> ...                                          (one line per asset, sorted)
 * Setting <key as a JSON string>: <RFC 8785 JSON of the value>       (one line per key, sorted)
 * Expires At: <ISO 8601 UTC>
 *
 * Nothing else may leave your wallet through these steps. You can revoke this at any time without signing.
 *
 * Version: arete.automation-consent/v1
 * Nonce: <nonce>
 * Issued At: <ISO 8601 UTC>
 * ```
 *
 * Without per-run limits the message says that nothing may leave the wallet.
 */
export function consentMessage(domain: string, consent: NormalizedConsent, nonce: string, issuedAt: number): string {
  if (!/^[A-Za-z0-9.-]+(:\d+)?$/.test(domain)) {
    throw new ConsentError('no_domain', `'${domain}' is not a host name`);
  }
  if (!/^[1-9A-HJ-NP-Za-km-z]{16,64}$/.test(nonce)) {
    throw new ConsentError('invalid_fields', 'the nonce must be 16 to 64 base58 characters');
  }
  const limits = consent.limits;
  const perRun = Object.entries(limits.perExecution);
  const lines = [
    `${domain} asks to run protocol steps for your wallet without asking each time:`,
    consent.wallet,
    '',
    `Deployment: ${consent.deploymentId}`,
    `Instance: ${consent.instanceId ?? 'any'}`,
    `Steps: ${consent.steps.join(', ')}`,
    ...(perRun.length === 0
      ? ['Per run: nothing may leave your wallet']
      : perRun.map(([key, amount]) => `Per run: ${amountLine(key, amount)}`)),
    ...Object.entries(limits.perWindow).map(
      ([key, { amount, windowMs }]) => `Per ${windowText(windowMs)}: ${amountLine(key, amount)}`,
    ),
    ...Object.keys(consent.params)
      .sort()
      // Keys are JSON strings, so no key can end its line and forge another.
      .map((key) => `Setting ${JSON.stringify(key)}: ${canonicalizeJson(consent.params[key])}`),
    `Expires At: ${new Date(consent.expiresAt).toISOString()}`,
    '',
    'Nothing else may leave your wallet through these steps. You can revoke this at any time without signing.',
    '',
    `Version: ${CONSENT_PROOF_VERSION}`,
    `Nonce: ${nonce}`,
    `Issued At: ${new Date(issuedAt).toISOString()}`,
  ];
  return lines.join('\n');
}

function parse(value: string | null | undefined): bigint | null {
  return typeof value === 'string' && /^-?\d+$/.test(value) ? BigInt(value) : null;
}

/**
 * What a prepared step moves out of the wallet, per asset, summed over its
 * transactions: SOL as the policy measures it (wrapped SOL and token
 * accounts closed to someone else included), and every mint that leaves.
 * The summary must be complete; the policy refuses one that is not.
 */
export function stepOutflows(summary: PreparedSummary): Map<string, bigint> {
  const out = new Map<string, bigint>();
  for (const { amounts } of transactionOutflows(summary)) {
    for (const [asset, amount] of amounts) out.set(asset, (out.get(asset) ?? 0n) + amount);
  }
  return out;
}

/** {@link stepOutflows}, per transaction, so a partly landed intent can be accounted exactly. */
export function transactionOutflows(summary: PreparedSummary): Array<{ index: number; amounts: Map<string, bigint> }> {
  return summary.transactions.map((transaction) => {
    const out = new Map<string, bigint>();
    const add = (key: string, amount: bigint) => {
      if (amount > 0n) out.set(key, (out.get(key) ?? 0n) + amount);
    };
    const effect = transaction.signerEffect;
    const tokens = effect.tokens ?? [];
    const lamports = parse(effect.solLamports) ?? 0n;
    const wrapped = tokens
      .filter((token) => isNativeMint(token.mint))
      .reduce((sum, token) => sum + (parse(token.amount) ?? 0n), 0n);
    const closed = parse(effect.closedAccountLamportsOut) ?? 0n;
    add(SOL, closed - lamports - wrapped);
    for (const token of tokens) {
      if (!isNativeMint(token.mint)) add(token.mint, -(parse(token.amount) ?? 0n));
    }
    return { index: transaction.index, amounts: out };
  });
}
