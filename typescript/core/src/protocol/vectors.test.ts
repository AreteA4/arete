import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import {
  canonicalizeJson,
  protocolMetadataProofMessage,
  PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS,
  receiptCanonicalBody,
  receiptSigningMessage,
  signWebhook,
  verifyReceipt,
  verifyReceiptOnChain,
  verifyWebhook,
  WEBHOOK_TOLERANCE_SECONDS,
} from '../index';

// The cross-language vectors every protocol client (TypeScript, Rust,
// Python) checks itself against.
function vector(name: string): any {
  return JSON.parse(
    readFileSync(new URL(`../../../../test-vectors/protocol-client/${name}`, import.meta.url), 'utf8'),
  );
}

describe('webhook-signature-v1 vectors', () => {
  const vectors = vector('webhook-signature-v1.json');

  it('records the tolerance', () => {
    expect(vectors.toleranceSeconds).toBe(WEBHOOK_TOLERANCE_SECONDS);
  });

  for (const testCase of vectors.cases) {
    it(testCase.name, () => {
      const { input, expected } = testCase;
      if (expected.signature !== undefined) {
        expect(signWebhook(input.secret, input.timestamp, input.body)).toBe(expected.signature);
        return;
      }
      const run = () => verifyWebhook(input.secret, input.body, input.headers, { now: input.now });
      if (expected.error !== undefined) {
        expect(run).toThrow(expect.objectContaining({ code: expected.error }));
      } else {
        expect(run()).toEqual({ id: expected.id, timestamp: expected.timestamp, payload: JSON.parse(input.body) });
      }
    });
  }
});

describe('metadata-proof-v1 vectors', () => {
  const vectors = vector('metadata-proof-v1.json');

  it('records the maximum TTL', () => {
    expect(vectors.maxTtlSeconds).toBe(PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS);
  });

  for (const testCase of vectors.cases) {
    it(testCase.name, () => {
      const { input, expected } = testCase;
      const run = () => protocolMetadataProofMessage(input);
      if (expected.error === undefined) {
        const proof = run();
        expect(canonicalizeJson(input.patch)).toBe(expected.canonicalPatch);
        expect(proof.patchSha256).toBe(expected.patchSha256);
        expect(proof.message).toBe(expected.message);
      } else {
        expect(run).toThrow(expected.error === 'expires-at' ? RangeError : TypeError);
      }
    });
  }
});

describe('execution-receipt-v1 vectors', () => {
  const vectors = vector('execution-receipt-v1.json');

  for (const testCase of vectors.verifyCases) {
    it(`verifyReceipt ${testCase.name}`, async () => {
      const { receipt, expected } = testCase;
      const result = await verifyReceipt(receipt, vectors.keySet);
      if (expected.valid) {
        expect(result).toEqual({ valid: true, keyId: expected.keyId });
      } else {
        expect(result).toMatchObject({ valid: false, error: expected.error });
      }
      if (expected.canonicalBody !== undefined) {
        expect(receiptCanonicalBody(receipt)).toBe(expected.canonicalBody);
        expect(new TextDecoder().decode(receiptSigningMessage(expected.keyId, expected.bodySha256))).toBe(
          expected.signingMessage,
        );
      }
    });
  }

  for (const testCase of vectors.onChainCases) {
    it(`verifyReceiptOnChain ${testCase.name}`, async () => {
      const { receipt, transaction, expected } = testCase;
      const result = await verifyReceiptOnChain(receipt, transaction);
      if (expected.valid) {
        expect(result).toEqual({ valid: true });
      } else {
        expect(result).toMatchObject({ valid: false, error: expected.error });
      }
    });
  }

  it('accepts raw wire bytes', async () => {
    const legacy = vectors.onChainCases.find((testCase: any) => testCase.name === 'legacy-wire');
    const bytes = Uint8Array.from(Buffer.from(legacy.transaction, 'base64'));
    expect(await verifyReceiptOnChain(legacy.receipt, bytes)).toEqual({ valid: true });
  });
});
