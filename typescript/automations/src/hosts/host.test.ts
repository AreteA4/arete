import { signWebhook } from '@usearete/sdk';
import { describe, expect, it } from 'vitest';

import { toWorker } from '../cloudflare';
import { serve } from '../node';
import type { AutomationOutcome } from '../runtime/automations';
import { defineAutomations } from '../runtime/define';
import { memoryStore } from '../store/memory';
import { DEPLOYMENT, fakeProtocol, feeOnlySimulator, PROGRAM, policy } from '../test-support/fake-protocol';
import { ALICE, storedGrant, walletsSigner } from '../test-support/harness';
import { toVercel } from '../vercel';
import { checkCluster, type RuntimeConfig } from './host';

const SECRET = 'whsec_host';
const CRON = 'cron-secret';

const definition = defineAutomations(
  { name: 'vault', deployment: { deploymentId: DEPLOYMENT }, events: { 'vault.rebalanceDue': { machine: 'instance' } } } as never,
  (automations) => {
    automations.on('vault.rebalanceDue', (event: { instanceId: string }, ctx) => [
      ctx.step('rebalance', { wallet: ALICE, instance: event.instanceId }),
    ]);
    automations.schedule('hourly', { every: 'hour' });
    automations.on('hourly', (_trigger, ctx) => [ctx.step('deposit', { wallet: ALICE, instance: 'inst-1', args: { amount: '1' } })]);
  },
  { policy: { programs: [PROGRAM], maxSolPerTransaction: '0.01' } },
);

/** A runtime factory over the fake protocol, sharing one store across invocations as a database would. */
function runtime(options: { cluster?: 'dev' | 'mainnet' } = {}) {
  const fake = fakeProtocol();
  const store = memoryStore();
  const outcomes: AutomationOutcome[] = [];
  let built = 0;
  let disposed = 0;
  void store.consent.put(storedGrant(ALICE, Date.now()));
  const factory = (): RuntimeConfig => {
    built += 1;
    return {
      client: fake.client,
      signer: walletsSigner(),
      policy,
      simulator: feeOnlySimulator,
      store,
      cluster: options.cluster ?? 'dev',
      trackInterval: 1,
      onOutcome: (outcome) => void outcomes.push(outcome),
      dispose: () => void (disposed += 1),
    };
  };
  return { factory, fake, outcomes, counts: () => ({ built, disposed }) };
}

function delivery(path = 'https://automations.example/arete/webhook', secret = SECRET) {
  const body = JSON.stringify({
    id: 'em-1',
    event: 'vault.rebalanceDue',
    deploymentId: DEPLOYMENT,
    instanceId: 'inst-1',
    participantId: null,
    state: 'open',
    occurredAt: '2026-10-22T00:00:00Z',
  });
  const timestamp = Math.floor(Date.now() / 1000);
  return new Request(path, {
    method: 'POST',
    body,
    headers: {
      'content-type': 'application/json',
      'arete-webhook-id': 'dlv-1',
      'arete-webhook-timestamp': String(timestamp),
      'arete-webhook-signature': `v1=${signWebhook(secret, timestamp, body)}`,
    },
  });
}

/** A Worker's ExecutionContext that collects the background work. */
function context() {
  const pending: Promise<unknown>[] = [];
  return { ctx: { waitUntil: (promise: Promise<unknown>) => void pending.push(promise) }, settled: () => Promise.all(pending) };
}

const env = { ARETE_WEBHOOK_SECRET: SECRET, CRON_SECRET: CRON };

describe('the Workers adapter', () => {
  it('accepts a webhook, runs its jobs after the response and disposes the runtime', async () => {
    const r = runtime();
    const worker = toWorker(definition, r.factory);
    const { ctx, settled } = context();
    const response = await worker.fetch(delivery(), env, ctx);
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ accepted: true, duplicate: false });
    await settled();
    expect(r.outcomes).toMatchObject([{ state: 'completed', step: 'rebalance', wallet: ALICE }]);
    expect(r.counts()).toEqual({ built: 1, disposed: 1 });

    // A retried delivery is a duplicate; a fresh runtime sees the same store.
    const again = context();
    expect(await (await worker.fetch(delivery(), env, again.ctx)).json()).toEqual({ accepted: false, duplicate: true });
    await again.settled();
    expect(r.counts()).toEqual({ built: 2, disposed: 2 });
  });

  it('refuses a forged delivery without building a runtime', async () => {
    const r = runtime();
    const worker = toWorker(definition, r.factory);
    const { ctx } = context();
    const response = await worker.fetch(delivery(undefined, 'whsec_other'), env, ctx);
    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: 'invalid-signature' });
    expect(r.counts().built).toBe(0);
    expect((await worker.fetch(delivery(), { CRON_SECRET: CRON }, ctx)).status).toBe(503);
  });

  it('ticks schedules from the cron trigger', async () => {
    const r = runtime();
    const worker = toWorker(definition, r.factory);
    const { ctx, settled } = context();
    await worker.scheduled({}, env, ctx);
    await settled();
    expect(r.outcomes).toMatchObject([{ state: 'completed', step: 'deposit' }]);
    expect(r.counts()).toEqual({ built: 1, disposed: 1 });
  });

  it('refuses a dev client when ARETE_CLUSTER asks for mainnet', async () => {
    const r = runtime({ cluster: 'dev' });
    const worker = toWorker(definition, r.factory);
    const { ctx } = context();
    const response = await worker.fetch(new Request('https://automations.example/arete/health'), { ...env, ARETE_CLUSTER: 'mainnet' }, ctx);
    expect(response.status).toBe(500);
    expect((await response.json()).message).toMatch(/ARETE_CLUSTER is mainnet, but the runtime is configured for dev/);
    expect(r.counts()).toEqual({ built: 1, disposed: 1 });
  });
});

describe('the Vercel adapter', () => {
  it('reuses one runtime, guards the tick route with CRON_SECRET and reports health', async () => {
    const r = runtime();
    const pending: Promise<unknown>[] = [];
    const { GET, POST } = toVercel(definition, r.factory, { env, waitUntil: (promise) => void pending.push(promise) });

    expect((await GET(new Request('https://app.vercel.app/api/arete/tick'))).status).toBe(401);
    const tick = await GET(new Request('https://app.vercel.app/api/arete/tick', { headers: { authorization: `Bearer ${CRON}` } }));
    expect(await tick.json()).toMatchObject({ scheduled: ['hourly'], ran: 2 });

    expect((await POST(delivery('https://app.vercel.app/api/arete/webhook'))).status).toBe(200);
    await Promise.all(pending);
    expect(r.outcomes.map((outcome) => outcome.step)).toEqual(['deposit', 'rebalance']);

    const health = await GET(new Request('https://app.vercel.app/api/arete/health/'));
    expect(await health.json()).toEqual({ ok: true, protocol: 'vault', deploymentId: DEPLOYMENT, cluster: 'dev' });
    expect((await GET(new Request('https://app.vercel.app/api/arete/other'))).status).toBe(404);
    expect(r.counts()).toEqual({ built: 1, disposed: 0 });
  });
});

describe('the Node server', () => {
  it('serves webhooks and runs its own workers and schedules', async () => {
    const r = runtime();
    const running = await serve(definition, r.factory, { env, port: 0, hostname: '127.0.0.1', handleSignals: false, tickInterval: '1h' });
    try {
      const request = delivery();
      const response = await fetch(`http://127.0.0.1:${running.port}/arete/webhook`, {
        method: 'POST',
        headers: (() => {
          const headers: Record<string, string> = {};
          request.headers.forEach((value, key) => void (headers[key] = value));
          return headers;
        })(),
        body: await request.text(),
      });
      expect(await response.json()).toEqual({ accepted: true, duplicate: false });
      for (let i = 0; i < 200 && r.outcomes.length < 2; i += 1) await new Promise((resolve) => setTimeout(resolve, 10));
      expect(r.outcomes.map((outcome) => outcome.step).sort()).toEqual(['deposit', 'rebalance']);
      const health = await fetch(`http://127.0.0.1:${running.port}/arete/health`);
      expect(await health.json()).toMatchObject({ ok: true, cluster: 'dev' });
    } finally {
      await running.close();
    }
    expect(r.counts()).toEqual({ built: 1, disposed: 1 });
  });

  it('releases what it built when it cannot start', async () => {
    const first = runtime();
    const running = await serve(definition, first.factory, { env, port: 0, hostname: '127.0.0.1', handleSignals: false, tickInterval: '1h' });
    try {
      const second = runtime();
      await expect(
        serve(definition, second.factory, { env, port: running.port, hostname: '127.0.0.1', handleSignals: false, tickInterval: '1h' }),
      ).rejects.toThrow(/EADDRINUSE/);
      expect(second.counts()).toEqual({ built: 1, disposed: 1 });

      const mainnet = runtime();
      await expect(serve(definition, mainnet.factory, { env: { ...env, ARETE_CLUSTER: 'mainnet' }, port: 0, handleSignals: false })).rejects.toThrow(
        /ARETE_CLUSTER is mainnet/,
      );
      expect(mainnet.counts()).toEqual({ built: 1, disposed: 1 });
    } finally {
      await running.close();
    }
  });
});

describe('checkCluster', () => {
  const client = (fields: Record<string, unknown>) => ({ ...fakeProtocol().client, ...fields }) as never;

  it('follows the client\'s environment switch', () => {
    expect(checkCluster({ ARETE_CLUSTER: 'dev' }, { client: client({ environment: 'dev' }) })).toBe('dev');
    expect(checkCluster({ ARETE_CLUSTER: 'mainnet' }, { client: client({ environment: 'production' }) })).toBe('mainnet');
    expect(checkCluster({}, { client: client({ environment: 'production' }) })).toBe('mainnet');
  });

  it('says the variable and the switch disagree', () => {
    expect(() => checkCluster({ ARETE_CLUSTER: 'mainnet' }, { client: client({ environment: 'dev' }) })).toThrow(
      /ARETE_CLUSTER is mainnet, but the protocol client runs in dev mode.*Unset ARETE_ENVIRONMENT/,
    );
    expect(() => checkCluster({ ARETE_CLUSTER: 'dev' }, { client: client({ environment: 'production' }) })).toThrow(
      /ARETE_CLUSTER is dev, but the protocol client runs in production.*ARETE_ENVIRONMENT=dev/,
    );
    // Neither message asks to restore the published client: the file is the same.
    expect(() => checkCluster({ ARETE_CLUSTER: 'mainnet' }, { client: client({ environment: 'dev' }) })).not.toThrow(/install/);
  });

  it('still names a client an older a4 generated for the dev environment', () => {
    const legacy = client({
      environment: 'production',
      definition: { deployment: { deploymentId: 'dep-dev', environment: 'dev' } },
    });
    expect(() => checkCluster({ ARETE_CLUSTER: 'mainnet' }, { client: legacy })).toThrow(/a4 install --locked/);
  });
});
