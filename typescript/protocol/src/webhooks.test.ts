import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { signWebhook, verifyWebhook } from "./index.js";

const secret = "whsec_test";
const body = JSON.stringify({
  id: "evt_1",
  event: "raise.succeeded",
  deploymentId: "dep_1",
  instanceId: "inst_1",
  participantId: null,
  state: "succeeded",
  occurredAt: "2026-10-13T00:00:00Z",
});
const now = 1_790_000_000;

function headers(signature: string, timestamp = now) {
  return {
    "Arete-Webhook-Id": "dlv_1",
    "arete-webhook-timestamp": String(timestamp),
    "arete-webhook-signature": signature,
  };
}

describe("verifyWebhook", () => {
  it("accepts a valid v1 signature (HMAC-SHA256 over `${timestamp}.${body}`)", () => {
    const signature = signWebhook(secret, now, body);
    expect(signature).toMatch(/^[0-9a-f]{64}$/);
    const verified = verifyWebhook(secret, body, headers(`v1=${signature}`), { now });
    expect(verified).toMatchObject({ id: "dlv_1", timestamp: now, payload: { event: "raise.succeeded", participantId: null } });
  });

  it("matches the hosted signer's fixed vector", () => {
    // Computed by the hosted delivery signer for this secret, timestamp and body.
    expect(signWebhook(secret, now, body)).toBe("33cb922725c0a7e5885d14ca4d55bfd3b792b3fb33509f35cf346090c099b88c");
  });

  it("passes the shared cross-language vectors", () => {
    const vectors = JSON.parse(
      readFileSync(new URL("../../../test-vectors/protocol-client/webhook-signature-v1.json", import.meta.url), "utf8"),
    );
    expect(vectors.cases[0].input).toMatchObject({ secret, timestamp: now, body });
    for (const { name, input, expected } of vectors.cases) {
      if (expected.signature !== undefined) {
        expect(signWebhook(input.secret, input.timestamp, input.body), name).toBe(expected.signature);
        continue;
      }
      const run = () => verifyWebhook(input.secret, input.body, input.headers, { now: input.now });
      if (expected.error !== undefined) {
        expect(run, name).toThrow(expect.objectContaining({ code: expected.error }));
      } else {
        expect(run(), name).toEqual({ id: expected.id, timestamp: expected.timestamp, payload: JSON.parse(input.body) });
      }
    }
  });

  it("accepts Headers objects, byte bodies and rotated secrets", () => {
    const signature = signWebhook(secret, now, body);
    const h = new Headers(headers(`v1=${"0".repeat(64)} v1=${signature}`));
    expect(verifyWebhook(secret, new TextEncoder().encode(body), h, { now }).id).toBe("dlv_1");
  });

  it("rejects tampering, stale timestamps and missing headers", () => {
    const signature = signWebhook(secret, now, body);
    expect(() => verifyWebhook(secret, `${body} `, headers(`v1=${signature}`), { now })).toThrow(
      expect.objectContaining({ code: "invalid-signature" }),
    );
    expect(() => verifyWebhook("other", body, headers(`v1=${signature}`), { now })).toThrow(
      expect.objectContaining({ code: "invalid-signature" }),
    );
    expect(() => verifyWebhook(secret, body, headers(`v1=${signature}`), { now: now + 301 })).toThrow(
      expect.objectContaining({ code: "timestamp-out-of-tolerance" }),
    );
    expect(verifyWebhook(secret, body, headers(`v1=${signature}`), { now: now + 300 }).id).toBe("dlv_1");
    expect(() => verifyWebhook(secret, body, { "arete-webhook-id": "x" }, { now })).toThrow(
      expect.objectContaining({ code: "missing-header" }),
    );
    expect(() => verifyWebhook(secret, body, headers(`v1=${signature}`, -1 as never), { now })).toThrow(
      expect.objectContaining({ code: "invalid-timestamp" }),
    );
  });
});
