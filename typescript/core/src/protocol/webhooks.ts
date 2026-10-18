/**
 * Webhook signature verification for emitted protocol events.
 *
 * A spec's `emit` reactions publish named events. A webhook endpoint
 * subscribed to given event names receives a signed POST. Payloads carry ids
 * and states only; receivers read details with their own credentials.
 *
 * Signature scheme (`v1`):
 *
 * - `arete-webhook-id`: the delivery id, stable across retries;
 * - `arete-webhook-timestamp`: Unix seconds when the delivery was signed;
 * - `arete-webhook-signature`: one or more space-separated `v1=<hex>`
 *   entries, each a lowercase hex HMAC-SHA256 over
 *   `${timestamp}.${body}` keyed with the endpoint secret. Several entries
 *   appear while a secret is being rotated; any one matching is enough.
 */

import { hmac } from '@noble/hashes/hmac';
import { sha256 } from '@noble/hashes/sha256';

import type { ProtocolEventPayloadV1 } from './types';

export const WEBHOOK_ID_HEADER = 'arete-webhook-id';
export const WEBHOOK_TIMESTAMP_HEADER = 'arete-webhook-timestamp';
export const WEBHOOK_SIGNATURE_HEADER = 'arete-webhook-signature';
export const WEBHOOK_TOLERANCE_SECONDS = 300;

export const WEBHOOK_ERROR_CODES = [
  'missing-header',
  'invalid-timestamp',
  'timestamp-out-of-tolerance',
  'invalid-signature',
  'invalid-payload',
] as const;
export type WebhookErrorCode = (typeof WEBHOOK_ERROR_CODES)[number];

export class WebhookVerificationError extends Error {
  readonly code: WebhookErrorCode;

  constructor(code: WebhookErrorCode, message: string) {
    super(message);
    this.name = 'WebhookVerificationError';
    this.code = code;
  }
}

export type WebhookHeaders =
  | { get(name: string): string | null }
  | Record<string, string | readonly string[] | undefined>;

export interface VerifyWebhookOptions {
  /** Unix seconds; defaults to the current time. */
  now?: number;
  /** Allowed clock skew in seconds; defaults to 300. */
  toleranceSeconds?: number;
}

export interface VerifiedWebhook<TPayload = ProtocolEventPayloadV1> {
  id: string;
  timestamp: number;
  payload: TPayload;
}

const encoder = new TextEncoder();

function header(headers: WebhookHeaders, name: string): string | undefined {
  if (typeof (headers as { get?: unknown }).get === 'function') {
    return (headers as { get(name: string): string | null }).get(name) ?? undefined;
  }
  const record = headers as Record<string, string | readonly string[] | undefined>;
  const key = Object.keys(record).find((candidate) => candidate.toLowerCase() === name);
  const value = key === undefined ? undefined : record[key];
  return Array.isArray(value) ? value.join(' ') : (value as string | undefined);
}

function toBytes(value: string | Uint8Array): Uint8Array {
  return typeof value === 'string' ? encoder.encode(value) : value;
}

function hex(bytes: Uint8Array): string {
  let out = '';
  for (const byte of bytes) out += byte.toString(16).padStart(2, '0');
  return out;
}

/** Constant-time comparison of two equal-length strings. */
function timingSafeEqual(left: string, right: string): boolean {
  if (left.length !== right.length) return false;
  let diff = 0;
  for (let i = 0; i < left.length; i += 1) diff |= left.charCodeAt(i) ^ right.charCodeAt(i);
  return diff === 0;
}

/** The `v1` signature for a body: lowercase hex HMAC-SHA256 over `${timestamp}.${body}`. */
export function signWebhook(
  secret: string | Uint8Array,
  timestamp: number,
  body: string | Uint8Array
): string {
  const bodyBytes = toBytes(body);
  const prefix = encoder.encode(`${timestamp}.`);
  const message = new Uint8Array(prefix.length + bodyBytes.length);
  message.set(prefix);
  message.set(bodyBytes, prefix.length);
  return hex(hmac(sha256, toBytes(secret), message));
}

/**
 * Verify a webhook delivery and return its parsed payload. Pass the raw
 * request body exactly as received (string or bytes), not re-serialized
 * JSON. Throws `WebhookVerificationError` on any failure.
 */
export function verifyWebhook<TPayload = ProtocolEventPayloadV1>(
  secret: string | Uint8Array,
  body: string | Uint8Array,
  headers: WebhookHeaders,
  options: VerifyWebhookOptions = {}
): VerifiedWebhook<TPayload> {
  const id = header(headers, WEBHOOK_ID_HEADER);
  const timestampText = header(headers, WEBHOOK_TIMESTAMP_HEADER);
  const signatureText = header(headers, WEBHOOK_SIGNATURE_HEADER);
  if (!id) throw new WebhookVerificationError('missing-header', `missing ${WEBHOOK_ID_HEADER}`);
  if (!timestampText) {
    throw new WebhookVerificationError('missing-header', `missing ${WEBHOOK_TIMESTAMP_HEADER}`);
  }
  if (!signatureText) {
    throw new WebhookVerificationError('missing-header', `missing ${WEBHOOK_SIGNATURE_HEADER}`);
  }
  if (!/^(0|[1-9][0-9]{0,15})$/.test(timestampText)) {
    throw new WebhookVerificationError('invalid-timestamp', 'the timestamp is not Unix seconds');
  }
  const timestamp = Number(timestampText);
  const now = options.now ?? Math.floor(Date.now() / 1000);
  const tolerance = options.toleranceSeconds ?? WEBHOOK_TOLERANCE_SECONDS;
  if (Math.abs(now - timestamp) > tolerance) {
    throw new WebhookVerificationError(
      'timestamp-out-of-tolerance',
      'the delivery is outside the allowed clock skew'
    );
  }
  const expected = signWebhook(secret, timestamp, body);
  const candidates = signatureText
    .split(/\s+/)
    .filter((entry) => entry.startsWith('v1='))
    .map((entry) => entry.slice(3));
  let matched = false;
  for (const candidate of candidates) matched = timingSafeEqual(candidate, expected) || matched;
  if (!matched) throw new WebhookVerificationError('invalid-signature', 'no v1 signature matches');
  let payload: TPayload;
  try {
    payload = JSON.parse(typeof body === 'string' ? body : new TextDecoder().decode(body)) as TPayload;
  } catch {
    throw new WebhookVerificationError('invalid-payload', 'the body is not JSON');
  }
  return { id, timestamp, payload };
}
