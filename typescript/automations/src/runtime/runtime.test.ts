import { signWebhook } from '@usearete/sdk';
import { describe, expect, it } from 'vitest';

import type { ProtocolSigner } from '../signer/signer';
import { SignerRefusal } from '../signer/signer';
import { runStoreConformance } from '../store/conformance';
import { memoryStore } from '../store/memory';
import type { AutomationStore } from '../store/store';
import { DEPLOYMENT, fakeProtocol, feeOnlySimulator, policy } from '../test-support/fake-protocol';
import { ALICE, BOB, harness, SECRET, storedGrant, walletsSigner } from '../test-support/harness';
import { createAutomations } from './automations';
import { scheduleSlot } from './time';

runStoreConformance('memoryStore', async () => memoryStore(), { describe, it, expect });

/** Deposit for each wallet named in the dispatch payload. */
function onDeposits(automations: ReturnType<typeof harness>['automations'], options: { cooldown?: '1h' } = {}) {
  automations.on<{ wallets: string[] }>('deposits', (payload, ctx) =>
    payload.wallets.map((wallet) =>
      ctx.step('deposit', { wallet, instance: 'inst-1', args: { amount: '1000' } }, options.cooldown ? { cooldown: options.cooldown } : {}),
    ),
  );
}

describe('dispatch and handlers', () => {
  it('runs a dispatched trigger through to a completed step', async () => {
    const h = harness();
    await h.grant(ALICE);
    onDeposits(h.automations);
    expect(await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 'tick-1' })).toBe(true);
    await h.automations.drain();
    expect(h.outcomes).toHaveLength(1);
    expect(h.outcomes[0]).toMatchObject({ state: 'completed', code: 'completed', wallet: ALICE, step: 'deposit' });
    expect(h.outcomes[0]!.executionId).toBe('exec-1');
    expect(h.outcomes[0]!.signatures[0]).toBeTypeOf('string');
    expect(h.fake.prepareCalls[0]!.options).toMatchObject({
      transactionFormat: 'v1',
      idempotencyKey: `step:dispatch%3Atick-1:deposit:${ALICE}:inst-1:0`,
    });
    expect(h.fake.prepareCalls[0]!.input).toEqual({ amount: '1000', wallet: ALICE, instance: 'inst-1' });
  });

  it('accepts a repeated dispatch id once', async () => {
    const h = harness();
    await h.grant(ALICE);
    onDeposits(h.automations);
    expect(await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: scheduleSlot('weekly', 'week', Date.UTC(2026, 9, 22)) })).toBe(true);
    expect(await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: scheduleSlot('weekly', 'week', Date.UTC(2026, 9, 23)) })).toBe(false);
    await h.automations.drain();
    expect(h.fake.submitCalls).toHaveLength(1);
  });

  it('fans one trigger out to one step job per wallet, deduplicated', async () => {
    const h = harness();
    await h.grant(ALICE);
    await h.grant(BOB);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE, BOB, ALICE] }, { id: 't' });
    await h.automations.drain();
    expect((await h.stepJobs()).map((job) => job.state)).toEqual(['completed', 'completed']);
    expect(h.fake.submitCalls).toHaveLength(2);
  });

  it('runs the same step for one wallet on several instances', async () => {
    const h = harness();
    await h.grant(ALICE);
    h.automations.on('claims', (_payload, ctx) => [
      ctx.step('claim', { wallet: ALICE, instance: 'inst-1' }),
      ctx.step('claim', { wallet: ALICE, instance: 'inst-2' }),
      ctx.step('claim', { wallet: ALICE, instance: 'inst-2' }),
    ]);
    await h.automations.dispatch('claims', {}, { id: 't' });
    await h.automations.drain();
    expect((await h.stepJobs()).map((job) => job.id)).toEqual([
      `step:dispatch%3At:claim:${ALICE}:inst-1`,
      `step:dispatch%3At:claim:${ALICE}:inst-2`,
    ]);
    expect(h.outcomes.map((outcome) => [outcome.instanceId, outcome.state])).toEqual([
      ['inst-1', 'completed'],
      ['inst-2', 'completed'],
    ]);
  });

  it('keeps step job ids apart from delivery ids', async () => {
    const h = harness();
    await h.grant(ALICE);
    h.automations.on('claims', (_payload, ctx) => [ctx.step('claim', { wallet: ALICE, instance: 'inst-1' })]);
    await h.automations.dispatch('claims', {}, { id: 't' });
    await h.automations.drain();
    // A dispatch whose id spells out the first one's step job is a new delivery.
    expect(await h.automations.dispatch('claims', {}, { id: `t:claim:${ALICE}:inst-1` })).toBe(true);
    await h.automations.drain();
    expect(h.outcomes.map((outcome) => outcome.state)).toEqual(['completed', 'completed']);
  });

  it('retries a handler that throws, without duplicating step jobs', async () => {
    const h = harness();
    await h.grant(ALICE);
    await h.grant(BOB);
    let calls = 0;
    h.automations.on<{ wallets: string[] }>('deposits', (payload, ctx) => {
      calls += 1;
      if (calls === 1) throw new Error('read failed halfway');
      return payload.wallets.map((wallet) => ctx.step('deposit', { wallet, instance: 'inst-1' }));
    });
    await h.automations.dispatch('deposits', { wallets: [ALICE, BOB] }, { id: 't' });
    await h.automations.drain();
    expect(await h.stepJobs()).toHaveLength(0);
    h.clock.advance(60_000);
    await h.automations.drain();
    expect(calls).toBe(2);
    expect((await h.stepJobs()).map((job) => job.state)).toEqual(['completed', 'completed']);
    expect(h.fake.prepareCalls).toHaveLength(2);
  });

  it('lets a lapsed handler run again on another worker without duplicating steps', async () => {
    const h = harness();
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    // Worker w0 claims the delivery and dies before completing it.
    const dead = await h.store.jobs.claimNext({ worker: 'w0', leaseMs: 60_000, now: h.clock.now() });
    expect(dead?.kind).toBe('delivery');
    h.clock.advance(61_000);
    await h.automations.drain();
    expect(await h.store.jobs.completeDelivery(dead!.id, 'w0', [], h.clock.now())).toBe(false);
    expect((await h.stepJobs()).map((job) => job.state)).toEqual(['completed']);
  });

  it('fails a handler after the maximum attempts', async () => {
    const h = harness({}, { maxAttempts: 2 });
    h.automations.on('broken', () => {
      throw new Error('always');
    });
    await h.automations.dispatch('broken', {}, { id: 'b' });
    await h.automations.drain();
    h.clock.advance(3_600_000);
    await h.automations.drain();
    const [job] = await h.store.jobs.list({ kind: 'delivery' });
    expect(job).toMatchObject({ state: 'failed', outcome: { code: 'handler_failed', reason: 'always' } });
  });
});

describe('the step pipeline', () => {
  it('skips a wallet without consent, and one whose consent was revoked before signing', async () => {
    const h = harness({
      onPrepare: async ({ input }) => {
        if (input.wallet === BOB) await h.store.consent.revoke({ wallet: BOB, deploymentId: DEPLOYMENT, now: h.clock.now() });
      },
    });
    await h.grant(BOB);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE, BOB] }, { id: 't' });
    await h.automations.drain();
    expect(h.outcomes.map((outcome) => [outcome.wallet, outcome.state, outcome.code])).toEqual([
      [ALICE, 'skipped', 'no_consent'],
      [BOB, 'skipped', 'no_consent'],
    ]);
    expect(h.outcomes[1]!.reason).toBe('consent was revoked before signing');
    expect(h.fake.submitCalls).toHaveLength(0);
  });

  it('treats a guard rejection as a normal outcome', async () => {
    const h = harness({
      allowed: (step) => (step === 'deposit' ? { step, allowed: false, reason: 'raise_closed' } : { step, allowed: true }),
    });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    expect(h.rejected).toHaveLength(1);
    expect(h.rejected[0]).toMatchObject({ state: 'rejected', code: 'raise_closed' });
    expect(h.fake.prepareCalls).toHaveLength(0);
    expect(h.errors).toHaveLength(0);
  });

  it('records a prepare rejection', async () => {
    const h = harness({ rejectPrepare: () => 'insufficient_funds' });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    expect(h.rejected[0]).toMatchObject({ state: 'rejected', code: 'insufficient_funds' });
  });

  it('refuses a step the policy does not allow', async () => {
    const h = harness({ program: 'Stake11111111111111111111111111111111111111' });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    expect(h.refused).toHaveLength(1);
    expect(h.refused[0]).toMatchObject({ state: 'refused', code: 'programs' });
    expect(h.fake.submitCalls).toHaveLength(0);
  });

  it('refuses when the policy does not list the cluster', async () => {
    const h = harness({}, { cluster: 'mainnet' });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    expect(h.refused[0]).toMatchObject({ code: 'clusters' });
    expect(h.fake.prepareCalls).toHaveLength(0);
  });

  it('refuses when the wallet provider declines', async () => {
    const declining: ProtocolSigner = {
      ...walletsSigner(),
      signTransactions: async () => {
        throw new SignerRefusal('policy denied: program not allowlisted');
      },
    };
    const h = harness({}, { signer: declining });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    expect(h.refused[0]).toMatchObject({ code: 'provider_policy', reason: 'policy denied: program not allowlisted' });
  });

  it('skips a step inside its cooldown', async () => {
    const h = harness();
    await h.grant(ALICE);
    onDeposits(h.automations, { cooldown: '1h' });
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 'a' });
    await h.automations.drain();
    h.clock.advance(30 * 60_000);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 'b' });
    await h.automations.drain();
    h.clock.advance(31 * 60_000);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 'c' });
    await h.automations.drain();
    expect(h.outcomes.map((outcome) => outcome.code)).toEqual(['completed', 'cooldown', 'completed']);
  });

  it('records a transaction that failed on chain', async () => {
    const h = harness({ script: () => ['submitting', 'chain_failed'] });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    expect(h.outcomes[0]).toMatchObject({ state: 'chain_failed', detail: [{ index: 0, error: 'custom program error: 0x1' }] });
  });
});

describe('the signer boundary', () => {
  it('checks a signer against a baseline it could not change', async () => {
    const sneaky: ProtocolSigner = {
      ...walletsSigner(),
      async signTransactions(input) {
        const signed = await walletsSigner().signTransactions(input);
        // Rewrite what it was given to match a changed result.
        const changed = signed.map((transaction) => transaction.slice(0, -4) + 'AAAA');
        (input.transactions as unknown as { transactionBase64: string }[]).forEach((transaction, index) => {
          transaction.transactionBase64 = changed[index]!;
        });
        return changed;
      },
    };
    const h = harness({}, { signer: sneaky });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    expect(h.refused[0]).toMatchObject({ code: 'signer_modified' });
    expect(h.fake.submitCalls).toHaveLength(0);
  });
});

describe('expiry and reconciliation', () => {
  it('prepares an expired intent once more with a new idempotency key', async () => {
    const h = harness({ script: (key) => (key.endsWith(':0') ? ['expired'] : ['submitting', 'completed']) });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    expect(h.fake.prepareCalls.map((call) => call.options?.idempotencyKey?.slice(-2))).toEqual([':0', ':1']);
    expect(h.outcomes[0]).toMatchObject({ state: 'completed', executionId: 'exec-2' });
  });

  it('gives up after a second expiry', async () => {
    const h = harness({ script: () => ['expired'] });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    expect(h.fake.prepareCalls).toHaveLength(2);
    expect(h.outcomes[0]).toMatchObject({ state: 'expired', code: 'expired' });
  });

  it('reconciles a lost submit response through the intent instead of submitting again', async () => {
    const h = harness({ submitMode: (call) => (call === 1 ? 'fail-after' : 'accept') });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    expect((await h.stepJobs())[0]!.state).toBe('signed');
    h.clock.advance(60_000);
    // A different worker picks it up after a restart.
    await h.make('w2').drain();
    expect(h.fake.submitCalls).toHaveLength(1);
    expect(h.outcomes[0]).toMatchObject({ state: 'completed' });
  });

  it('submits the saved signed bytes when the API never received them', async () => {
    let signs = 0;
    const counting: ProtocolSigner = {
      ...walletsSigner(),
      signTransactions: async (input) => {
        signs += 1;
        return walletsSigner().signTransactions(input);
      },
    };
    const h = harness({ submitMode: (call) => (call === 1 ? 'fail-before' : 'accept') }, { signer: counting });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.drain();
    h.clock.advance(60_000);
    await h.automations.drain();
    expect(signs).toBe(1);
    expect(h.fake.submitCalls.map((call) => call.signed.length)).toEqual([0, 1]);
    expect(h.outcomes[0]).toMatchObject({ state: 'completed' });
  });

  it('recovers a worker that died after signing and before saving', async () => {
    const h = harness();
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    // A store whose write of the signed transactions never completes: the worker hangs, as if it died.
    const hanging: AutomationStore = {
      ...h.store,
      jobs: {
        ...h.store.jobs,
        transition: (id, input) => (input.to === 'signed' ? new Promise<boolean>(() => {}) : h.store.jobs.transition(id, input)),
      },
    };
    const dying = createAutomations({
      client: h.fake.client,
      signer: walletsSigner(),
      policy,
      simulator: feeOnlySimulator,
      store: hanging,
      cluster: 'devnet',
      clock: h.clock,
      worker: 'dying',
    });
    dying.on('deposits', () => []);
    await h.automations.runOnce(); // the delivery
    void dying.runOnce(); // signs, then never saves
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect((await h.stepJobs())[0]).toMatchObject({ state: 'running', lease: { worker: 'dying#1' } });
    h.clock.advance(61_000);
    await h.make('w2').drain();
    expect(h.fake.prepareCalls).toHaveLength(2);
    expect(new Set(h.fake.prepareCalls.map((call) => call.options?.idempotencyKey)).size).toBe(1);
    expect(h.fake.submitCalls).toHaveLength(1);
    expect((await h.stepJobs())[0]!.state).toBe('completed');
  });

  it('keeps the wallet locked while an intent is submitted_unknown', async () => {
    const h = harness({
      script: (key) =>
        key.includes(':deposit:') ? [...Array<'submitted_unknown'>(12).fill('submitted_unknown'), 'completed'] : ['completed'],
    });
    await h.grant(ALICE);
    h.automations.on('both', (_payload, ctx) => [
      ctx.step('deposit', { wallet: ALICE, instance: 'inst-1' }),
      ctx.step('withdraw', { wallet: ALICE, instance: 'inst-1' }),
    ]);
    await h.automations.dispatch('both', {}, { id: 't' });
    await h.automations.drain();
    let jobs = await h.stepJobs();
    expect(jobs.map((job) => job.state)).toEqual(['submitted_unknown', 'queued']);
    expect(h.fake.prepareCalls).toHaveLength(1);
    // Each later claim reads the intent first; the withdraw waits until it settles.
    for (let round = 0; round < 5; round += 1) {
      h.clock.advance(3_600_000);
      await h.automations.drain();
      jobs = await h.stepJobs();
      if (jobs[0]!.state === 'submitted_unknown') expect(jobs[1]!.state).toBe('queued');
    }
    expect(jobs.map((job) => job.state)).toEqual(['completed', 'completed']);
    expect(h.fake.submitCalls).toHaveLength(2);
  });

  it("keeps a lapsed claim's late failure off the job another claim took over", async () => {
    const gates: Array<() => void> = [];
    const gate = () => new Promise<void>((resolve) => gates.push(resolve));
    let prepares = 0;
    const h = harness({
      onPrepare: async () => {
        prepares += 1;
        if (prepares === 1) {
          await gate();
          throw new Error('the slow prepare finally fails');
        }
      },
      onSubmit: async () => gate(),
    });
    await h.grant(ALICE);
    onDeposits(h.automations);
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 't' });
    await h.automations.runOnce(); // the delivery
    const slow = h.automations.runOnce(); // blocks in prepare
    await new Promise((resolve) => setTimeout(resolve, 5));
    h.clock.advance(61_000); // its lease lapses
    const taker = h.automations.runOnce(); // another loop of this process: signs, then blocks in submit
    await new Promise((resolve) => setTimeout(resolve, 20));
    const [during] = await h.stepJobs();
    expect(during).toMatchObject({ state: 'signed', lease: { worker: expect.stringMatching(/#3$/) } });
    gates[0]!(); // the slow claim fails now
    await slow;
    // It must not have parked or retried the job the other claim holds.
    expect((await h.stepJobs())[0]).toMatchObject({ state: 'signed', attempts: 0, lease: { worker: during!.lease!.worker } });
    gates[1]!();
    await taker;
    expect((await h.stepJobs())[0]!.state).toBe('completed');
    expect(h.fake.submitCalls).toHaveLength(1);
  });

  it('refuses a store that belongs to another deployment', async () => {
    const h = harness();
    await h.automations.dispatch('x', {}, { id: '1' });
    const other = fakeProtocol();
    const foreign = createAutomations({
      client: { ...other.client, deploymentId: 'dep-other' },
      signer: walletsSigner(),
      policy,
      simulator: feeOnlySimulator,
      store: h.store,
      cluster: 'devnet',
    });
    await expect(foreign.runOnce()).rejects.toThrow('belongs to deployment dep-1');
    await expect(foreign.dispatch('x', {}, { id: '2' })).rejects.toThrow('use one store per deployment');
  });

  it('never runs two step jobs of one wallet at once', async () => {
    let inFlight = 0;
    let most = 0;
    const h = harness({
      onPrepare: async () => {
        inFlight += 1;
        most = Math.max(most, inFlight);
        await new Promise((resolve) => setTimeout(resolve, 5));
        inFlight -= 1;
      },
    });
    await h.grant(ALICE);
    h.automations.on('many', (_payload, ctx) =>
      ['deposit', 'withdraw', 'claim', 'rebalance'].map((step) => ctx.step(step as 'deposit', { wallet: ALICE, instance: 'inst-1' })),
    );
    await h.automations.dispatch('many', {}, { id: 't' });
    await h.automations.runOnce();
    const workers = ['a', 'b', 'c', 'd'].map((worker) => h.make(worker));
    for (let round = 0; round < 4; round += 1) await Promise.all(workers.map((worker) => worker.runOnce()));
    expect(most).toBe(1);
    expect((await h.stepJobs()).every((job) => job.state === 'completed')).toBe(true);
  });
});

describe('webhooks and streams', () => {
  function signed(body: string, timestamp: number, id = 'dlv-1') {
    return {
      'arete-webhook-id': id,
      'arete-webhook-timestamp': String(timestamp),
      'arete-webhook-signature': `v1=${signWebhook(SECRET, timestamp, body)}`,
    };
  }

  const payload = (event = 'vault.rebalanceDue', deploymentId = DEPLOYMENT) =>
    JSON.stringify({
      id: 'em-1',
      event,
      deploymentId,
      instanceId: 'inst-1',
      participantId: null,
      state: 'open',
      occurredAt: '2026-10-22T00:00:00Z',
    });

  it('verifies, accepts once and answers straight away', async () => {
    const h = harness();
    await h.grant(ALICE);
    h.automations.on('vault.rebalanceDue', (event, ctx) => {
      expect(event).toMatchObject({ instanceId: 'inst-1', state: 'open', source: 'webhook' });
      return [ctx.step('rebalance', { wallet: ALICE, instance: event.instanceId })];
    });
    const endpoint = h.automations.webhookHandler({ secret: SECRET });
    const body = payload();
    const now = Math.floor(h.clock.now() / 1000);
    const request = () => new Request('https://app.example/arete', { method: 'POST', body, headers: signed(body, now) });
    const first = await endpoint(request());
    expect(first.status).toBe(200);
    expect(await first.json()).toEqual({ accepted: true, duplicate: false });
    expect(await (await endpoint(request())).json()).toEqual({ accepted: false, duplicate: true });
    expect(h.fake.prepareCalls).toHaveLength(0); // nothing ran on the request path
    await h.automations.drain();
    expect(h.outcomes[0]).toMatchObject({ state: 'completed', step: 'rebalance' });
  });

  it('dedupes on the signed emission id, not the unsigned delivery header', async () => {
    const h = harness();
    await h.grant(ALICE);
    h.automations.on('vault.rebalanceDue', (event, ctx) => [ctx.step('rebalance', { wallet: ALICE, instance: event.instanceId })]);
    const body = payload();
    const now = Math.floor(h.clock.now() / 1000);
    expect((await h.automations.acceptWebhook(body, signed(body, now, 'dlv-1'), { secret: SECRET })).body).toEqual({
      accepted: true,
      duplicate: false,
    });
    // A replay of the same signed body under a new header id.
    expect((await h.automations.acceptWebhook(body, signed(body, now, 'dlv-forged'), { secret: SECRET })).body).toEqual({
      accepted: false,
      duplicate: true,
    });
    await h.automations.drain();
    expect(h.fake.submitCalls).toHaveLength(1);
  });

  it('refuses forged, stale and malformed deliveries', async () => {
    const h = harness();
    h.automations.on('vault.rebalanceDue', () => []);
    const body = payload();
    const now = Math.floor(h.clock.now() / 1000);
    const accept = (headers: Record<string, string>, text = body) => h.automations.acceptWebhook(text, headers, { secret: SECRET });
    expect((await accept({ ...signed(body, now), 'arete-webhook-signature': 'v1=00' })).status).toBe(401);
    expect((await accept(signed(body, now - 3600))).status).toBe(401);
    expect((await accept({ 'arete-webhook-timestamp': String(now) })).status).toBe(400);
    expect((await accept(signed('[]', now), '[]')).status).toBe(400);
    expect(await h.store.jobs.list()).toHaveLength(0);
  });

  it('ignores other deployments and events without a handler', async () => {
    const h = harness();
    h.automations.on('vault.rebalanceDue', () => []);
    const now = Math.floor(h.clock.now() / 1000);
    const other = payload('vault.rebalanceDue', 'dep-other');
    expect((await h.automations.acceptWebhook(other, signed(other, now), { secret: SECRET })).body).toEqual({
      accepted: false,
      ignored: 'other-deployment',
    });
    const unhandled = payload('vault.closed');
    expect((await h.automations.acceptWebhook(unhandled, signed(unhandled, now), { secret: SECRET })).body).toEqual({
      accepted: false,
      ignored: 'no-handler',
    });
  });

  it('serves Express behind a raw body parser', async () => {
    const h = harness();
    h.automations.on('vault.rebalanceDue', () => []);
    const endpoint = h.automations.expressWebhookHandler({ secret: SECRET });
    const body = payload();
    const reply = { code: 0, body: undefined as unknown };
    const res = {
      status(code: number) {
        reply.code = code;
        return res;
      },
      json(value: unknown) {
        reply.body = value;
      },
    };
    await endpoint({ body: JSON.parse(body), headers: signed(body, Math.floor(h.clock.now() / 1000)) }, res);
    expect(reply).toMatchObject({ code: 500, body: { error: 'raw-body-required' } });
    await endpoint({ body: Buffer.from(body), headers: signed(body, Math.floor(h.clock.now() / 1000)) }, res);
    expect(reply).toEqual({ code: 200, body: { accepted: true, duplicate: false } });
  });

  it('retries a stream event whose accept failed, instead of dropping it', async () => {
    const h = harness();
    let failures = 2;
    const flaky: AutomationStore = {
      ...h.store,
      deliveries: {
        ...h.store.deliveries,
        accept: async (delivery, now) => {
          if (failures > 0) {
            failures -= 1;
            throw new Error('database restarting');
          }
          return h.store.deliveries.accept(delivery, now);
        },
      },
    };
    const automations = createAutomations({
      client: h.fake.client,
      signer: walletsSigner(),
      policy,
      simulator: feeOnlySimulator,
      store: flaky,
      cluster: 'devnet',
      clock: h.clock,
      onError: () => undefined,
    });
    automations.on('vault.rebalanceDue', () => []);
    const subscription = automations.follow('inst-1');
    h.fake.emit('vault.rebalanceDue', 7);
    h.fake.emit('vault.rebalanceDue', 8);
    for (let wait = 0; wait < 20; wait += 1) await new Promise((resolve) => setTimeout(resolve, 0));
    subscription.close();
    expect((await h.store.jobs.list({ kind: 'delivery' })).map((job) => job.id)).toEqual(['stream:inst-1:7', 'stream:inst-1:8']);
  });

  it('accepts emitted events from a followed instance once each', async () => {
    const h = harness();
    await h.grant(ALICE);
    h.automations.on('vault.rebalanceDue', (event, ctx) => {
      expect(event).toMatchObject({ source: 'stream', state: null, id: '7' });
      return [ctx.step('rebalance', { wallet: ALICE, instance: event.instanceId })];
    });
    h.automations.follow('inst-1');
    h.fake.emit('vault.rebalanceDue', 7);
    h.fake.emit('vault.rebalanceDue', 7);
    h.fake.emit('vault.closed', 8);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect((await h.store.jobs.list({ kind: 'delivery' })).map((job) => job.id)).toEqual(['stream:inst-1:7']);
    await h.automations.drain();
    expect(h.outcomes).toHaveLength(1);
  });
});

describe('scheduleSlot', () => {
  it('names a slot per period, in UTC', () => {
    const at = Date.UTC(2026, 9, 22, 9, 5);
    expect(scheduleSlot('buy', 'minute', at)).toBe('buy:2026-10-22T09:05');
    expect(scheduleSlot('buy', 'hour', at)).toBe('buy:2026-10-22T09');
    expect(scheduleSlot('buy', 'day', at)).toBe('buy:2026-10-22');
    expect(scheduleSlot('buy', 'week', at)).toBe('buy:2026-W43');
    expect(scheduleSlot('buy', 'month', at)).toBe('buy:2026-10');
    expect(scheduleSlot('buy', 'week', Date.UTC(2027, 0, 1))).toBe('buy:2026-W53');
  });
});

describe('the worker loop', () => {
  it('starts, runs jobs and stops', async () => {
    const h = harness({}, { clock: { now: () => Date.now(), sleep: (ms) => new Promise((resolve) => setTimeout(resolve, Math.min(ms, 5))) } });
    await h.store.consent.put(storedGrant(ALICE, Date.now(), { steps: ['deposit'] }));
    onDeposits(h.automations);
    h.automations.start({ pollInterval: 5, concurrency: 2 });
    await h.automations.dispatch('deposits', { wallets: [ALICE] }, { id: 'loop' });
    for (let wait = 0; wait < 100 && h.outcomes.length === 0; wait += 1) {
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
    await h.automations.stop();
    expect(h.outcomes[0]).toMatchObject({ state: 'completed' });
  });
});
