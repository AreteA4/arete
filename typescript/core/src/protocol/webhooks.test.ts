import { describe, expect, it } from 'vitest';

import { signWebhook, verifyWebhook } from '../index';

const secret = 'whsec_test';
const body = JSON.stringify({
  id: 'evt_1',
  event: 'raise.succeeded',
  deploymentId: 'dep_1',
  instanceId: 'inst_1',
  participantId: null,
  state: 'succeeded',
  occurredAt: '2026-10-13T00:00:00Z',
});
const now = 1_790_000_000;

function headers(signature: string, timestamp = now) {
  return {
    'Arete-Webhook-Id': 'dlv_1',
    'arete-webhook-timestamp': String(timestamp),
    'arete-webhook-signature': signature,
  };
}

describe('verifyWebhook', () => {
  it('accepts a valid v1 signature (HMAC-SHA256 over `${timestamp}.${body}`)', () => {
    const signature = signWebhook(secret, now, body);
    expect(signature).toMatch(/^[0-9a-f]{64}$/);
    const verified = verifyWebhook(secret, body, headers(`v1=${signature}`), { now });
    expect(verified).toMatchObject({
      id: 'dlv_1',
      timestamp: now,
      payload: { event: 'raise.succeeded', participantId: null },
    });
  });

  it('accepts Headers objects, byte bodies and rotated secrets', () => {
    const signature = signWebhook(secret, now, body);
    const h = new Headers(headers(`v1=${'0'.repeat(64)} v1=${signature}`));
    expect(verifyWebhook(secret, new TextEncoder().encode(body), h, { now }).id).toBe('dlv_1');
  });

  it('rejects tampering, stale timestamps and missing headers', () => {
    const signature = signWebhook(secret, now, body);
    expect(() => verifyWebhook(secret, `${body} `, headers(`v1=${signature}`), { now })).toThrow(
      expect.objectContaining({ code: 'invalid-signature' }),
    );
    expect(() => verifyWebhook('other', body, headers(`v1=${signature}`), { now })).toThrow(
      expect.objectContaining({ code: 'invalid-signature' }),
    );
    expect(() => verifyWebhook(secret, body, headers(`v1=${signature}`), { now: now + 301 })).toThrow(
      expect.objectContaining({ code: 'timestamp-out-of-tolerance' }),
    );
    expect(verifyWebhook(secret, body, headers(`v1=${signature}`), { now: now + 300 }).id).toBe('dlv_1');
    expect(() => verifyWebhook(secret, body, { 'arete-webhook-id': 'x' }, { now })).toThrow(
      expect.objectContaining({ code: 'missing-header' }),
    );
    expect(() => verifyWebhook(secret, body, headers(`v1=${signature}`, -1 as never), { now })).toThrow(
      expect.objectContaining({ code: 'invalid-timestamp' }),
    );
  });

  it('signs byte-identically to the reference HMAC-SHA256', async () => {
    const { createHmac } = await import('node:crypto');
    const expected = createHmac('sha256', secret).update(`${now}.${body}`).digest('hex');
    expect(signWebhook(secret, now, body)).toBe(expected);
  });
});
