/**
 * The store conformance suite. Any `AutomationStore` (memory, Postgres, or
 * your own over Redis, DynamoDB, D1 or Durable Objects) must pass it.
 *
 * ```ts
 * import { describe, expect, it } from 'vitest';
 * import { runStoreConformance } from '@usearete/automations/testing';
 *
 * runStoreConformance('my store', async () => myStore(), { describe, it, expect });
 * ```
 */

import type { Delivery, StepJob } from '../runtime/types';
import type { AutomationStore, ConsentGrant, NewStepJob } from './store';

/** The subset of a test framework the suite uses (vitest and jest both fit). */
export interface ConformanceTestApi {
  describe(name: string, body: () => void): void;
  it(name: string, body: () => Promise<void>): void;
  expect(value: unknown): {
    toBe(expected: unknown): void;
    toEqual(expected: unknown): void;
    toBeNull(): void;
    not: { toBeNull(): void };
  };
}

const LEASE = 30_000;
const T0 = 1_700_000_000_000;

function grantFor(wallet: string, overrides: Partial<ConsentGrant> = {}): ConsentGrant {
  return {
    wallet,
    deploymentId: 'dep-1',
    instanceId: null,
    steps: ['deposit', 'withdraw'],
    limits: { perExecution: { SOL: '10000' }, perWindow: { SOL: { amount: '30000', windowMs: 86_400_000 } } },
    params: { amount: '1000', nested: { weekday: 'fri' } },
    expiresAt: T0 + 1000,
    grantedAt: T0,
    revokedAt: null,
    proof: { message: 'm', signature: 's', nonce: 'n', issuedAt: T0 },
    ...overrides,
  };
}

function delivery(id: string, name = 'tick'): Delivery {
  return { id, name, source: 'dispatch', payload: { n: id }, receivedAt: T0 };
}

function step(deliveryId: string, wallet: string, name = 'deposit'): NewStepJob {
  return {
    id: `${deliveryId}:${name}:${wallet}`,
    deploymentId: 'dep-1',
    request: { step: name, wallet, instance: 'inst-1', args: { amount: '1' } },
  };
}

/** Accept a delivery and complete it with `steps`, returning the step job ids. */
async function seedSteps(store: AutomationStore, deliveryId: string, steps: NewStepJob[], now = T0) {
  await store.deliveries.accept(delivery(deliveryId), now);
  const claimed = await store.jobs.claimNext({ worker: 'seed', leaseMs: LEASE, now });
  if (claimed?.id !== deliveryId) throw new Error(`expected to claim ${deliveryId}, got ${claimed?.id}`);
  await store.jobs.completeDelivery(deliveryId, 'seed', steps, now);
  return steps.map((entry) => entry.id);
}

export function runStoreConformance(
  name: string,
  createStore: () => Promise<AutomationStore>,
  { describe, it, expect }: ConformanceTestApi,
): void {
  describe(`${name}: store conformance`, () => {
    it('accepts a delivery once', async () => {
      const store = await createStore();
      expect(await store.deliveries.accept(delivery('d1'), T0)).toBe(true);
      expect(await store.deliveries.accept(delivery('d1'), T0 + 1)).toBe(false);
      const jobs = await store.jobs.list({ kind: 'delivery' });
      expect(jobs.length).toBe(1);
      expect(jobs[0]!.state).toBe('queued');
    });

    it('claims a job for one worker at a time', async () => {
      const store = await createStore();
      await store.deliveries.accept(delivery('d1'), T0);
      const job = await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      expect(job?.id).toBe('d1');
      expect(job?.state).toBe('running');
      expect(job?.lease).toEqual({ worker: 'w1', until: T0 + LEASE });
      expect(await store.jobs.claimNext({ worker: 'w2', leaseMs: LEASE, now: T0 + 1 })).toBeNull();
    });

    it('never hands one job to two concurrent claims', async () => {
      const store = await createStore();
      for (let index = 0; index < 5; index += 1) await store.deliveries.accept(delivery(`d${index}`), T0);
      const claims = await Promise.all(
        Array.from({ length: 12 }, (_, index) => store.jobs.claimNext({ worker: `w${index}`, leaseMs: LEASE, now: T0 })),
      );
      const ids = claims.filter((job) => job !== null).map((job) => job!.id);
      expect(ids.length).toBe(5);
      expect(new Set(ids).size).toBe(5);
    });

    it('lets another worker take a job over when its lease lapses', async () => {
      const store = await createStore();
      await store.deliveries.accept(delivery('d1'), T0);
      await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      const taken = await store.jobs.claimNext({ worker: 'w2', leaseMs: LEASE, now: T0 + LEASE + 1 });
      expect(taken?.id).toBe('d1');
      expect(taken?.lease?.worker).toBe('w2');
      expect(await store.jobs.completeDelivery('d1', 'w1', [], T0 + LEASE + 2)).toBe(false);
      expect(await store.jobs.renew('d1', 'w1', T0 + 2 * LEASE)).toBe(false);
      expect(await store.jobs.renew('d1', 'w2', T0 + 2 * LEASE)).toBe(true);
    });

    it('transitions only from the expected state, by the lease holder', async () => {
      const store = await createStore();
      const [id] = await seedSteps(store, 'd1', [step('d1', 'alice')]);
      const job = await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      expect(job?.id).toBe(id);
      const base = { now: T0 + 1, keepLease: { until: T0 + LEASE } };
      expect(await store.jobs.transition(id!, { ...base, worker: 'w1', from: 'queued', to: 'signed' })).toBe(false);
      expect(await store.jobs.transition(id!, { ...base, worker: 'w2', from: 'running', to: 'signed' })).toBe(false);
      expect(
        await store.jobs.transition(id!, {
          ...base,
          worker: 'w1',
          from: 'running',
          to: 'signed',
          progress: { signed: ['AAAA'] },
        }),
      ).toBe(true);
      const saved = (await store.jobs.get(id!)) as StepJob;
      expect(saved.state).toBe('signed');
      expect(saved.progress).toEqual({ generation: 0, signed: ['AAAA'] });
      expect(saved.lease?.worker).toBe('w1');
    });

    it('completes a delivery and creates its step jobs in one step', async () => {
      const store = await createStore();
      await store.deliveries.accept(delivery('d1'), T0);
      expect(await store.jobs.completeDelivery('d1', 'w1', [step('d1', 'alice')], T0)).toBe(false);
      expect((await store.jobs.list({ kind: 'step' })).length).toBe(0);
      await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      expect(
        await store.jobs.completeDelivery('d1', 'w1', [step('d1', 'alice'), step('d1', 'bob'), step('d1', 'alice')], T0),
      ).toBe(true);
      const steps = await store.jobs.list({ kind: 'step' });
      expect(steps.map((job) => job.id).sort()).toEqual(['d1:deposit:alice', 'd1:deposit:bob']);
      expect(steps.every((job) => job.state === 'queued')).toBe(true);
      expect((await store.jobs.get('d1'))?.state).toBe('done');
    });

    it('leaves an existing step job alone when a delivery is completed again', async () => {
      const store = await createStore();
      await seedSteps(store, 'd1', [step('d1', 'alice')]);
      await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      // A delivery that lapsed and is completed again by another worker.
      await store.deliveries.accept(delivery('d2'), T0);
      await store.jobs.claimNext({ worker: 'w2', leaseMs: LEASE, now: T0 });
      await store.jobs.completeDelivery('d2', 'w2', [{ ...step('d1', 'alice'), request: { ...step('d1', 'alice').request, args: { amount: '9' } } }], T0);
      const job = (await store.jobs.get('d1:deposit:alice')) as StepJob;
      expect(job.state).toBe('running');
      expect(job.request.args).toEqual({ amount: '1' });
    });

    it('runs at most one step job per wallet', async () => {
      const store = await createStore();
      await seedSteps(store, 'd1', [step('d1', 'alice'), step('d1', 'alice', 'withdraw'), step('d1', 'bob')]);
      const first = await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      const second = await store.jobs.claimNext({ worker: 'w2', leaseMs: LEASE, now: T0 });
      const third = await store.jobs.claimNext({ worker: 'w3', leaseMs: LEASE, now: T0 });
      expect(first?.id).toBe('d1:deposit:alice');
      expect(second?.id).toBe('d1:deposit:bob');
      expect(third).toBeNull();
      await store.jobs.transition(first!.id, { worker: 'w1', from: 'running', to: 'completed', now: T0 + 1 });
      expect((await store.jobs.claimNext({ worker: 'w3', leaseMs: LEASE, now: T0 + 2 }))?.id).toBe('d1:withdraw:alice');
    });

    it('never lets concurrent claims start two jobs of one wallet', async () => {
      const store = await createStore();
      await seedSteps(
        store,
        'd1',
        Array.from({ length: 6 }, (_, index) => step('d1', 'alice', `step${index}`)),
      );
      const claims = await Promise.all(
        Array.from({ length: 6 }, (_, index) => store.jobs.claimNext({ worker: `w${index}`, leaseMs: LEASE, now: T0 })),
      );
      expect(claims.filter((job) => job !== null).length).toBe(1);
    });

    it('keeps the wallet locked through signed, submitted and submitted_unknown', async () => {
      const store = await createStore();
      await seedSteps(store, 'd1', [step('d1', 'alice'), step('d1', 'alice', 'withdraw')]);
      const job = await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      let from: StepJob['state'] = 'running';
      for (const to of ['signed', 'submitted', 'submitted_unknown'] as const) {
        expect(
          await store.jobs.transition(job!.id, { worker: 'w1', from, to, now: T0 + 1, keepLease: { until: T0 + LEASE } }),
        ).toBe(true);
        from = to;
      }
      // Released and parked for a later reconcile.
      expect(
        await store.jobs.transition(job!.id, {
          worker: 'w1',
          from,
          to: from,
          now: T0 + 2,
          notBefore: T0 + 60_000,
        }),
      ).toBe(true);
      expect(await store.jobs.claimNext({ worker: 'w2', leaseMs: LEASE, now: T0 + 3 })).toBeNull();
      const again = await store.jobs.claimNext({ worker: 'w2', leaseMs: LEASE, now: T0 + 60_000 });
      expect(again?.id).toBe(job!.id);
      expect(again?.state).toBe('submitted_unknown');
    });

    it('frees the wallet when a job goes back to the queue', async () => {
      const store = await createStore();
      await seedSteps(store, 'd1', [step('d1', 'alice'), step('d1', 'alice', 'withdraw')]);
      const job = await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      await store.jobs.transition(job!.id, {
        worker: 'w1',
        from: 'running',
        to: 'queued',
        now: T0 + 1,
        notBefore: T0 + 10_000,
        attempts: 1,
      });
      const next = await store.jobs.claimNext({ worker: 'w2', leaseMs: LEASE, now: T0 + 2 });
      expect(next?.id).toBe('d1:withdraw:alice');
      expect((await store.jobs.get(job!.id))?.attempts).toBe(1);
    });

    it('skips jobs before their notBefore time', async () => {
      const store = await createStore();
      await store.deliveries.accept(delivery('d1'), T0);
      const job = await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      await store.jobs.transition(job!.id, { worker: 'w1', from: 'running', to: 'queued', now: T0, notBefore: T0 + 5_000 });
      expect(await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 + 4_999 })).toBeNull();
      expect((await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 + 5_000 }))?.id).toBe('d1');
    });

    it('reports when a wallet last completed a step', async () => {
      const store = await createStore();
      await seedSteps(store, 'd1', [step('d1', 'alice')]);
      const query = { wallet: 'alice', deploymentId: 'dep-1', step: 'deposit' };
      expect(await store.jobs.lastCompleted(query)).toBeNull();
      const job = await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      await store.jobs.transition(job!.id, { worker: 'w1', from: 'running', to: 'completed', now: T0 + 500 });
      expect(await store.jobs.lastCompleted(query)).toBe(T0 + 500);
      expect(await store.jobs.lastCompleted({ ...query, step: 'withdraw' })).toBeNull();
    });

    it('stores, finds and revokes consent grants', async () => {
      const store = await createStore();
      const grant = grantFor('alice');
      await store.consent.put(grant);
      await store.consent.put({ ...grant, instanceId: 'inst-2', steps: ['claim'] });
      const query = { wallet: 'alice', deploymentId: 'dep-1', step: 'deposit', instanceId: 'inst-1', now: T0 };
      expect((await store.consent.find(query))?.steps).toEqual(['deposit', 'withdraw']);
      expect(await store.consent.find({ ...query, step: 'claim' })).toBeNull();
      expect((await store.consent.find({ ...query, step: 'claim', instanceId: 'inst-2' }))?.instanceId).toBe('inst-2');
      expect(await store.consent.find({ ...query, now: T0 + 1000 })).toBeNull();
      expect(await store.consent.find({ ...query, wallet: 'bob' })).toBeNull();
      expect(await store.consent.revoke({ wallet: 'alice', deploymentId: 'dep-1', steps: ['deposit'], now: T0 + 1 })).toBe(1);
      expect(await store.consent.find(query)).toBeNull();
      expect(await store.consent.find({ ...query, step: 'withdraw' })).not.toBeNull();
      expect(await store.consent.revoke({ wallet: 'alice', deploymentId: 'dep-1', now: T0 + 2 })).toBe(2);
      expect(await store.consent.find({ ...query, step: 'withdraw' })).toBeNull();
      expect((await store.consent.list({ wallet: 'alice' })).length).toBe(2);
    });

    it('keeps every field of a grant and replaces it per deployment and instance', async () => {
      const store = await createStore();
      const grant = grantFor('alice');
      await store.consent.put(grant);
      expect(await store.consent.list({ wallet: 'alice' })).toEqual([grant]);
      const raised = grantFor('alice', { limits: { perExecution: { SOL: '20000' }, perWindow: {} } });
      await store.consent.put(raised);
      expect(await store.consent.list({ wallet: 'alice' })).toEqual([raised]);
    });

    it('prefers the instance grant and revokes by instance', async () => {
      const store = await createStore();
      await store.consent.put(grantFor('alice'));
      await store.consent.put(grantFor('alice', { instanceId: 'inst-1', params: { scoped: true } }));
      const query = { wallet: 'alice', deploymentId: 'dep-1', step: 'deposit', instanceId: 'inst-1', now: T0 };
      expect((await store.consent.find(query))?.params).toEqual({ scoped: true });
      expect(await store.consent.revoke({ wallet: 'alice', deploymentId: 'dep-1', instanceId: 'inst-1', now: T0 + 1 })).toBe(1);
      expect((await store.consent.find(query))?.instanceId).toBeNull();
    });

    it('lists the active grants of a deployment for fan-out', async () => {
      const store = await createStore();
      await store.consent.put(grantFor('alice'));
      await store.consent.put(grantFor('bob', { steps: ['claim'] }));
      await store.consent.put(grantFor('carol', { expiresAt: T0 }));
      await store.consent.put(grantFor('dave', { revokedAt: T0 }));
      await store.consent.put(grantFor('erin', { deploymentId: 'dep-2' }));
      const wallets = async (step?: string) =>
        (await store.consent.active({ deploymentId: 'dep-1', ...(step ? { step } : {}), now: T0 }))
          .map((grant) => grant.wallet)
          .sort();
      expect(await wallets()).toEqual(['alice', 'bob']);
      expect(await wallets('deposit')).toEqual(['alice']);
    });

    it('uses an issued nonce once, before it expires', async () => {
      const store = await createStore();
      expect(await store.consent.useNonce('never-issued', T0)).toBe(false);
      await store.consent.issueNonce('n1', T0 + 100);
      await store.consent.issueNonce('n2', T0 + 100);
      const uses = await Promise.all([store.consent.useNonce('n1', T0), store.consent.useNonce('n1', T0)]);
      expect(uses.filter(Boolean).length).toBe(1);
      expect(await store.consent.useNonce('n1', T0 + 1)).toBe(false);
      expect(await store.consent.useNonce('n2', T0 + 100)).toBe(false);
    });

    it('writes and voids ledger entries with the transition they belong to', async () => {
      const store = await createStore();
      await seedSteps(store, 'd1', [step('d1', 'alice')]);
      const job = await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      const spent = (since: number, asset = 'SOL') =>
        store.ledger.spent({ wallet: 'alice', deploymentId: 'dep-1', asset, since });
      const record = { record: [{ wallet: 'alice', deploymentId: 'dep-1', asset: 'SOL', amount: '5000' }] };
      // A transition that fails writes nothing.
      expect(
        await store.jobs.transition(job!.id, { worker: 'w2', from: 'running', to: 'signed', now: T0 + 10, ledger: record }),
      ).toBe(false);
      expect(await spent(T0)).toBe(0n);
      expect(
        await store.jobs.transition(job!.id, {
          worker: 'w1',
          from: 'running',
          to: 'signed',
          now: T0 + 10,
          keepLease: { until: T0 + LEASE },
          ledger: record,
        }),
      ).toBe(true);
      expect(await spent(T0)).toBe(5000n);
      expect(await spent(T0 + 11)).toBe(0n);
      expect(await spent(T0, 'other')).toBe(0n);
      const [entry] = await store.ledger.list({ wallet: 'alice' });
      expect(entry).toEqual({
        jobId: job!.id,
        wallet: 'alice',
        deploymentId: 'dep-1',
        asset: 'SOL',
        amount: '5000',
        recordedAt: T0 + 10,
        voidedAt: null,
      });
      expect(
        await store.jobs.transition(job!.id, { worker: 'w1', from: 'signed', to: 'expired', now: T0 + 20, ledger: { void: true } }),
      ).toBe(true);
      expect(await spent(T0)).toBe(0n);
      expect((await store.ledger.list({ wallet: 'alice' }))[0]!.voidedAt).toBe(T0 + 20);
    });

    it('advances view-source state by compare-and-set', async () => {
      const store = await createStore();
      const advance = (expectedVersion: number | null, hash: string, deliver = false) =>
        store.sources.advance({
          source: 'rates',
          key: 'all',
          expectedVersion,
          next: { hash, value: { best: hash }, pending: null },
          now: T0,
          ...(deliver ? { deliver: { name: 'rates', payload: { best: hash } } } : {}),
        });
      expect(await store.sources.get('rates', 'all')).toBeNull();
      const first = await advance(null, 'a', true);
      expect(first).toEqual({
        source: 'rates',
        key: 'all',
        hash: 'a',
        value: { best: 'a' },
        generation: 1,
        pending: null,
        observed: null,
        version: 1,
        updatedAt: T0,
      });
      expect(await advance(null, 'b')).toBeNull();
      // The same stable hash keeps the generation; a new one moves it on.
      expect((await advance(1, 'a'))?.generation).toBe(1);
      expect((await advance(2, 'b', true))?.generation).toBe(2);
      expect((await advance(3, 'a', true))?.generation).toBe(3);
      const ids = (await store.jobs.list({ kind: 'delivery' })).map((job) => job.id);
      expect(ids).toEqual(['view:rates:all:1', 'view:rates:all:2', 'view:rates:all:3']);
      const [job] = await store.jobs.list({ kind: 'delivery' });
      expect(job!.kind === 'delivery' && job!.delivery).toEqual({
        id: 'view:rates:all:1',
        name: 'rates',
        source: 'view',
        payload: { best: 'a' },
        receivedAt: T0,
      });
    });

    it('gives different sources and keys different delivery ids', async () => {
      const store = await createStore();
      const deliver = (source: string, key: string) =>
        store.sources.advance({
          source,
          key,
          expectedVersion: null,
          next: { hash: 'a', value: 1, pending: null },
          deliver: { name: source, payload: key },
          now: T0,
        });
      await deliver('rates:wallet', 'alice');
      await deliver('rates', 'wallet:alice');
      expect((await store.jobs.list({ kind: 'delivery' })).length).toBe(2);
    });

    it('lets one of several concurrent writers advance a source key', async () => {
      const store = await createStore();
      await store.sources.advance({ source: 's', key: 'k', expectedVersion: null, next: { hash: 'a', value: 1, pending: null }, now: T0 });
      const results = await Promise.all(
        ['b', 'c', 'd', 'e'].map((hash) =>
          store.sources.advance({
            source: 's',
            key: 'k',
            expectedVersion: 1,
            next: { hash, value: hash, pending: null },
            deliver: { name: 's', payload: hash },
            now: T0,
          }),
        ),
      );
      expect(results.filter((result) => result !== null).length).toBe(1);
      expect((await store.jobs.list({ kind: 'delivery' })).length).toBe(1);
    });

    it('keeps a pending value and source cursors', async () => {
      const store = await createStore();
      const pending = { hash: 'b', value: 'B', since: T0 + 5 };
      await store.sources.advance({ source: 's', key: 'k', expectedVersion: null, next: { hash: '', value: null, pending }, now: T0 });
      expect((await store.sources.get('s', 'k'))?.pending).toEqual(pending);
      expect(await store.sources.cursor('tape')).toBeNull();
      await store.sources.putCursor('tape', '1:4', T0);
      await store.sources.putCursor('tape', '1:9', T0);
      expect(await store.sources.cursor('tape')).toBe('1:9');
      // A slow replica never moves the cursor back.
      await store.sources.putCursor('tape', '1:5', T0);
      await store.sources.putCursor('tape', '0:99', T0);
      expect(await store.sources.cursor('tape')).toBe('1:9');
      await store.sources.putCursor('tape', '1:10', T0);
      await store.sources.putCursor('tape', '2:0', T0);
      expect(await store.sources.cursor('tape')).toBe('2:0');
    });

    it('holds a source lease for one holder until it lapses or is released', async () => {
      const store = await createStore();
      expect(await store.sources.acquireLease('s', 'w1', T0 + 100, T0)).toBe(true);
      expect(await store.sources.acquireLease('s', 'w2', T0 + 100, T0 + 50)).toBe(false);
      expect(await store.sources.acquireLease('s', 'w1', T0 + 200, T0 + 50)).toBe(true);
      expect(await store.sources.acquireLease('s', 'w2', T0 + 300, T0 + 150)).toBe(false);
      expect(await store.sources.acquireLease('s', 'w2', T0 + 300, T0 + 201)).toBe(true);
      await store.sources.releaseLease('s', 'w1');
      expect(await store.sources.acquireLease('s', 'w1', T0 + 400, T0 + 202)).toBe(false);
      await store.sources.releaseLease('s', 'w2');
      expect(await store.sources.acquireLease('s', 'w1', T0 + 400, T0 + 203)).toBe(true);
    });

    it('keeps the ledger entries of the assets a void keeps', async () => {
      const store = await createStore();
      await seedSteps(store, 'd1', [step('d1', 'alice')]);
      const job = await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      await store.jobs.transition(job!.id, {
        worker: 'w1',
        from: 'running',
        to: 'signed',
        now: T0,
        keepLease: { until: T0 + LEASE },
        ledger: {
          record: [
            { wallet: 'alice', deploymentId: 'dep-1', asset: 'SOL', amount: '5000' },
            { wallet: 'alice', deploymentId: 'dep-1', asset: 'mint', amount: '7' },
          ],
        },
      });
      await store.jobs.transition(job!.id, {
        worker: 'w1',
        from: 'signed',
        to: 'chain_failed',
        now: T0 + 1,
        ledger: { void: true, keepAssets: ['SOL'] },
      });
      const spent = (asset: string) => store.ledger.spent({ wallet: 'alice', deploymentId: 'dep-1', asset, since: 0 });
      expect(await spent('SOL')).toBe(5000n);
      expect(await spent('mint')).toBe(0n);
    });

    it('belongs to one deployment', async () => {
      const store = await createStore();
      await store.bindDeployment('dep-1');
      await store.bindDeployment('dep-1');
      let refused = false;
      try {
        await store.bindDeployment('dep-2');
      } catch {
        refused = true;
      }
      expect(refused).toBe(true);
    });

    it('treats a pruned delivery id whose job remains as a duplicate', async () => {
      const store = await createStore();
      await store.deliveries.accept(delivery('d1'), T0);
      expect(await store.deliveries.prune(T0 + 1)).toBe(1);
      expect(await store.deliveries.accept(delivery('d1'), T0 + 2)).toBe(false);
      expect((await store.jobs.list({ kind: 'delivery' })).length).toBe(1);
    });

    it('prunes old deliveries and finished jobs only', async () => {
      const store = await createStore();
      await seedSteps(store, 'd1', [step('d1', 'alice'), step('d1', 'bob')]);
      const job = await store.jobs.claimNext({ worker: 'w1', leaseMs: LEASE, now: T0 });
      await store.jobs.transition(job!.id, { worker: 'w1', from: 'running', to: 'completed', now: T0 + 1 });
      expect(await store.jobs.prune(T0 + 10)).toBe(2); // the delivery job and one finished step job
      expect((await store.jobs.list()).map((entry) => entry.id)).toEqual(['d1:deposit:bob']);
      expect(await store.deliveries.prune(T0 + 10)).toBe(1);
      expect(await store.deliveries.accept(delivery('d1'), T0 + 20)).toBe(true);
    });
  });
}
