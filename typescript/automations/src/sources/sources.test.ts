import { describe, expect, it } from 'vitest';

import { createAutomations } from '../runtime/automations';
import type { Clock } from '../runtime/time';
import type { DeliveryJob } from '../runtime/types';
import { memoryStore } from '../store/memory';
import type { AutomationStore } from '../store/store';
import { fakeClock, fakeProtocol, feeOnlySimulator, policy } from '../test-support/fake-protocol';
import { FakeAppendView, FakeListView, manualClock, settle } from '../test-support/fake-views';
import { ALICE, BOB, storedGrant, walletsSigner } from '../test-support/harness';
import { fromAppendView, fromView, fromViewRows, type ViewTrigger } from './view';

interface Market {
  id: string;
  apyBps: number;
}

interface Position {
  owner: string;
  health: number;
}

/** The best market, with a 25 bps edge needed to displace it. */
function bestMarket(markets: readonly Market[]): string | null {
  const sorted = [...markets].sort((a, b) => b.apyBps - a.apyBps);
  return sorted[0]?.id ?? null;
}

function setup(
  store: AutomationStore = memoryStore(),
  clock: Clock & { advance(ms: number): void | Promise<void> } = manualClock(),
) {
  const fake = fakeProtocol();
  const make = (worker: string) =>
    createAutomations({
      client: fake.client,
      signer: walletsSigner(),
      policy,
      simulator: feeOnlySimulator,
      store,
      cluster: 'dev',
      clock,
      worker,
      lease: '30s',
    });
  return { store, clock, fake, make };
}

async function deliveries(store: AutomationStore) {
  return ((await store.jobs.list({ kind: 'delivery' })) as DeliveryJob[]).map((job) => ({
    id: job.id,
    payload: job.delivery.payload,
  }));
}

const markets = () => new FakeListView<Market>((market) => market.id);
const rateSource = (view: FakeListView<Market>, extra: { stableFor?: '2m' } = {}) =>
  fromView(view, {
    name: 'bestRateChanged',
    key: (market) => market.id,
    derive: bestMarket,
    changed: (previous, next) => previous !== undefined && previous !== next,
    ...extra,
  });

describe('list-view sources', () => {
  it('produces one delivery per transition across replicas', async () => {
    const { store, make } = setup();
    const view = markets();
    view.set({ id: 'kamino', apyBps: 500 }, { id: 'save', apyBps: 450 });
    const replicas = [make('r1'), make('r2'), make('r3')].map((replica) => replica.source(rateSource(view)));
    await settle();
    view.set({ id: 'save', apyBps: 520 });
    await settle();
    view.set({ id: 'kamino', apyBps: 530 });
    await settle();
    await Promise.all(replicas.map((replica) => replica.stop()));
    expect(await deliveries(store)).toEqual([
      { id: 'view:bestRateChanged:all:2', payload: { source: 'bestRateChanged', key: 'all', value: 'save', previous: 'kamino' } },
      { id: 'view:bestRateChanged:all:3', payload: { source: 'bestRateChanged', key: 'all', value: 'kamino', previous: 'save' } },
    ]);
  });

  it('fires once for a change made while every replica was down, and not again', async () => {
    const { store, make } = setup();
    const view = markets();
    view.set({ id: 'kamino', apyBps: 500 }, { id: 'save', apyBps: 450 });
    const first = make('r1').source(rateSource(view));
    await settle();
    await first.stop();
    view.set({ id: 'save', apyBps: 600 });
    const second = make('r2').source(rateSource(view));
    await settle();
    await second.stop();
    const third = make('r3').source(rateSource(view));
    await settle();
    await third.stop();
    expect((await deliveries(store)).map((delivery) => delivery.id)).toEqual(['view:bestRateChanged:all:2']);
  });

  it('gives every transition its own delivery, A → B → A → B', async () => {
    const { store, make } = setup();
    const view = markets();
    view.set({ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 400 });
    const handle = make('r1').source(rateSource(view));
    await settle();
    for (const [a, b] of [
      [400, 500],
      [500, 400],
      [400, 500],
    ]) {
      view.set({ id: 'a', apyBps: a! }, { id: 'b', apyBps: b! });
      await settle();
    }
    await handle.stop();
    expect((await deliveries(store)).map((delivery) => (delivery.payload as ViewTrigger).value)).toEqual(['b', 'a', 'b']);
  });

  it('waits out stableFor, so a one-update blip does not fire', async () => {
    const { store, make, clock } = setup();
    const view = markets();
    view.set({ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 400 });
    const handle = make('r1').source(rateSource(view, { stableFor: '2m' }));
    await settle();
    await clock.advance(120_000); // the first value settles
    view.set({ id: 'b', apyBps: 900 }); // a blip
    await settle();
    await clock.advance(30_000);
    view.set({ id: 'b', apyBps: 400 }); // gone again
    await settle();
    await clock.advance(120_000);
    expect(await deliveries(store)).toEqual([]);
    view.set({ id: 'b', apyBps: 900 }); // a real change
    await settle();
    await clock.advance(60_000);
    expect(await deliveries(store)).toEqual([]);
    await clock.advance(60_000);
    await handle.stop();
    expect((await deliveries(store)).map((delivery) => (delivery.payload as ViewTrigger).value)).toEqual(['b']);
  });

  it('fires per row on the crossing, not on every update while it holds', async () => {
    // Steps run here too, and their tracking sleeps need a clock that moves on its own.
    const { store, make, fake } = setup(memoryStore(), fakeClock());
    await store.consent.put(storedGrant(ALICE, Date.now()));
    const view = new FakeListView<Position>((position) => position.owner);
    const automations = make('r1');
    automations.on<ViewTrigger<{ wallet: string; low: boolean }>>('healthLow', (trigger, ctx) => [
      ctx.step('withdraw', { wallet: trigger.value.wallet, instance: 'inst-1' }),
    ]);
    view.set({ owner: ALICE, health: 1.4 }, { owner: BOB, health: 1.1 }, { owner: 'Carol', health: 1.0 });
    const handle = automations.source(
      fromViewRows(view, {
        name: 'healthLow',
        filter: (position) => position.owner !== 'Carol',
        key: (position) => position.owner,
        derive: (position) => ({ wallet: position.owner, low: position.health < 1.15 }),
        changed: (previous, next) => !previous?.low && next.low,
      }),
    );
    await settle();
    view.set({ owner: ALICE, health: 1.12 });
    await settle();
    view.set({ owner: ALICE, health: 1.05 }, { owner: BOB, health: 1.0 });
    await settle();
    await handle.stop();
    expect((await deliveries(store)).map((delivery) => delivery.id)).toEqual([
      `view:healthLow:${BOB}:1`,
      `view:healthLow:${ALICE}:2`,
    ]);
    await automations.drain();
    expect(fake.prepareCalls.map((call) => call.input.wallet)).toEqual([ALICE]);
  });

  it('subscribes on one replica only with exclusive', async () => {
    const { make, clock } = setup();
    const view = markets();
    view.set({ id: 'a', apyBps: 500 });
    const handles = [make('r1'), make('r2')].map((replica) => replica.source(rateSource(view), { exclusive: true }));
    await settle();
    expect(view.open).toBe(1);
    await handles[0]!.stop();
    await clock.advance(30_000);
    await settle();
    expect(view.open).toBe(1);
    expect(view.subscriptions).toBe(2);
    await handles[1]!.stop();
  });

  it('gives the same deliveries polled as subscribed', async () => {
    const snapshots: Market[][] = [
      [{ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 400 }],
      [{ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 450 }],
      [{ id: 'a', apyBps: 300 }, { id: 'b', apyBps: 450 }],
      [{ id: 'a', apyBps: 300 }, { id: 'b', apyBps: 450 }],
      [{ id: 'a', apyBps: 600 }, { id: 'b', apyBps: 450 }],
    ];
    const polled = setup();
    const polledView = markets();
    const poller = polled.make('p');
    for (const snapshot of snapshots) {
      polledView.set(...snapshot);
      await poller.poll(rateSource(polledView));
    }
    const subscribed = setup();
    const view = markets();
    view.set(...snapshots[0]!);
    const handle = subscribed.make('s').source(rateSource(view));
    await settle();
    for (const snapshot of snapshots.slice(1)) {
      view.set(...snapshot);
      await settle();
    }
    await handle.stop();
    expect(await deliveries(subscribed.store)).toEqual(await deliveries(polled.store));
    expect((await deliveries(polled.store)).length).toBe(2);
  });

  it('holds stableFor across polls', async () => {
    const { make, clock } = setup();
    const view = markets();
    const poller = make('p');
    const source = rateSource(view, { stableFor: '2m' });
    view.set({ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 400 });
    expect(await poller.poll(source)).toEqual({ delivered: 0, pending: 1 });
    await clock.advance(120_000);
    expect(await poller.poll(source)).toEqual({ delivered: 0, pending: 0 });
    view.set({ id: 'b', apyBps: 900 });
    expect(await poller.poll(source)).toEqual({ delivered: 0, pending: 1 });
    await clock.advance(120_000);
    expect(await poller.poll(source)).toEqual({ delivered: 1, pending: 0 });
  });
});

describe('robust list-view sources', () => {
  it('ignores an observation older than the one stored', async () => {
    const { store, make } = setup();
    const view = new FakeListView<Market & { slot: number }>((market) => market.id);
    const source = fromView(view, {
      name: 'best',
      key: (market) => market.id,
      derive: bestMarket,
      observedAt: (rows) => Math.max(...rows.map((row) => row.slot)),
      changed: (previous, next) => previous !== undefined && previous !== next,
    });
    const poller = make('p');
    view.set({ id: 'a', apyBps: 500, slot: 10 }, { id: 'b', apyBps: 400, slot: 10 });
    await poller.poll(source);
    view.set({ id: 'b', apyBps: 600, slot: 12 });
    await poller.poll(source); // B, at slot 12
    // A replica that is behind still sees A at slot 11.
    const lagging = new FakeListView<Market & { slot: number }>((market) => market.id);
    lagging.set({ id: 'a', apyBps: 500, slot: 11 }, { id: 'b', apyBps: 400, slot: 10 });
    await poller.poll({ ...source, view: lagging });
    await poller.poll(source); // B again, nothing new
    expect((await deliveries(store)).map((delivery) => (delivery.payload as ViewTrigger).value)).toEqual(['b']);
  });

  it('evaluates queued aggregate updates in order', async () => {
    const { store, make } = setup();
    let slow = false;
    const slowStore: AutomationStore = {
      ...store,
      sources: {
        ...store.sources,
        advance: async (input) => {
          if (slow) await new Promise((resolve) => setTimeout(resolve, 10));
          return store.sources.advance(input);
        },
      },
    };
    const view = markets();
    view.set({ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 400 });
    const handle = setup(slowStore).make('r').source(rateSource(view));
    void make;
    await settle();
    slow = true;
    view.set({ id: 'b', apyBps: 600 }); // B leads...
    view.set({ id: 'b', apyBps: 400 }); // ...then A again, before the first write lands
    await settle(40);
    await handle.stop();
    expect((await deliveries(store)).map((delivery) => (delivery.payload as ViewTrigger).value)).toEqual(['b', 'a']);
  });

  it('applies what the stream delivered during the first read before evaluating', async () => {
    const { store, make } = setup();
    const view = markets();
    view.set({ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 600 });
    // An earlier read that saw a smaller view than the watcher's snapshot.
    view.get = async () => [{ id: 'a', apyBps: 500 }];
    const handle = make('r').source(rateSource(view));
    await settle();
    await handle.stop();
    expect(await deliveries(store)).toEqual([]);
    expect((await store.sources.get('bestRateChanged', 'all'))?.value).toBe('b');
  });

  it('seeds from the complete rows when the query is already open', async () => {
    const { store, make } = setup();
    const view = markets();
    view.set({ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 600 });
    // An open query replays nothing to a new stream (and a large one would overflow its buffer).
    view.replay = false;
    const handle = make('r').source(rateSource(view));
    await settle();
    view.set({ id: 'c', apyBps: 100 }); // a newcomer that never leads
    await settle();
    view.set({ id: 'a', apyBps: 650 });
    await settle();
    await handle.stop();
    // b led from the start; only a overtaking it is a change.
    expect((await deliveries(store)).map((delivery) => (delivery.payload as ViewTrigger).value)).toEqual(['a']);
    expect((await store.sources.get('bestRateChanged', 'all'))?.generation).toBe(2);
  });

  it('drops rows a reconnect removed', async () => {
    const { store, make } = setup();
    const view = markets();
    view.set({ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 600 });
    const handle = make('r').source(rateSource(view));
    await settle();
    view.reconnect({ id: 'a', apyBps: 500 }); // b left the view while disconnected
    await settle();
    await handle.stop();
    expect((await deliveries(store)).map((delivery) => (delivery.payload as ViewTrigger).value)).toEqual(['a']);
  });

  it('evaluates a reconnect snapshot as a whole', async () => {
    const { store, make } = setup();
    const view = markets();
    view.set({ id: 'a', apyBps: 400 }, { id: 'b', apyBps: 600 });
    const handle = make('r').source(rateSource(view));
    await settle();
    // While disconnected, b left and c arrived and leads; a was never best.
    view.reconnect({ id: 'a', apyBps: 500 }, { id: 'c', apyBps: 700 });
    await settle();
    await handle.stop();
    expect((await deliveries(store)).map((delivery) => (delivery.payload as ViewTrigger).value)).toEqual(['c']);
  });

  it('never fires a pending change of a row that stopped passing the filter', async () => {
    const { store, make, clock } = setup();
    const view = new FakeListView<Position & { watched: boolean }>((position) => position.owner);
    view.set({ owner: ALICE, health: 1.4, watched: true });
    const handle = make('r').source(
      fromViewRows(view, {
        name: 'healthLow',
        key: (position) => position.owner,
        filter: (position) => position.watched,
        derive: (position) => ({ low: position.health < 1.15 }),
        changed: (previous, next) => previous !== undefined && !previous.low && next.low,
        stableFor: '2m',
      }),
    );
    await settle();
    await clock.advance(120_000);
    view.set({ owner: ALICE, health: 1.0, watched: true }); // pending: low
    await settle();
    view.set({ owner: ALICE, health: 1.0, watched: false }); // excluded before it settles
    await settle();
    await clock.advance(120_000);
    await handle.stop();
    expect(await deliveries(store)).toEqual([]);
  });

  it('drops a snapshot-seeded row removed without lastKnown', async () => {
    const { store, make } = setup();
    const view = markets();
    view.set({ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 600 });
    view.replay = false; // seeded from the snapshot only: the stream never announced b
    const handle = make('r').source(rateSource(view));
    await settle();
    view.remove('b', false);
    await settle();
    await handle.stop();
    expect((await deliveries(store)).map((delivery) => (delivery.payload as ViewTrigger).value)).toEqual(['a']);
  });

  it('never promotes a pending change of a seeded row removed without lastKnown', async () => {
    const { store, make, clock } = setup();
    const view = new FakeListView<Position>((position) => position.owner);
    view.set({ owner: ALICE, health: 1.4 }, { owner: BOB, health: 1.4 });
    view.replay = false;
    const source = () =>
      fromViewRows(view, {
        name: 'healthLow',
        key: (position: Position) => position.owner,
        derive: (position: Position) => ({ low: position.health < 1.15 }),
        changed: (previous, next) => previous !== undefined && !previous.low && next.low,
        stableFor: '2m',
      });
    const first = make('r1').source(source());
    await settle();
    await clock.advance(120_000);
    await first.stop();
    // While nothing watched, Alice went low: the next replica sees it only in its snapshot.
    view.rows.set(ALICE, { owner: ALICE, health: 1.0 });
    const second = make('r2').source(source());
    await settle(); // pending: low
    view.remove(ALICE, false);
    await settle();
    await clock.advance(120_000);
    await settle();
    await second.stop();
    expect(await deliveries(store)).toEqual([]);
  });

  it('retries a pending value whose promotion failed', async () => {
    const { store, clock } = setup();
    let failNext = false;
    const flaky: AutomationStore = {
      ...store,
      sources: {
        ...store.sources,
        advance: async (input) => {
          if (failNext) {
            failNext = false;
            throw new Error('database restarting');
          }
          return store.sources.advance(input);
        },
      },
    };
    const view = markets();
    view.set({ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 400 });
    const handle = setup(flaky, clock).make('r').source(rateSource(view, { stableFor: '2m' }));
    await settle();
    await clock.advance(120_000);
    view.set({ id: 'b', apyBps: 900 });
    await settle();
    failNext = true;
    await clock.advance(120_000); // the promotion fails...
    await clock.advance(60_000); // ...and is retried
    await handle.stop();
    expect((await deliveries(store)).map((delivery) => (delivery.payload as ViewTrigger).value)).toEqual(['b']);
  });

  it('opens nothing after stop, even while the first read is loading', async () => {
    const { store, make } = setup();
    const view = markets();
    let release: (rows: Market[]) => void = () => undefined;
    view.get = () => new Promise<Market[]>((resolve) => (release = resolve));
    const handle = make('r').source(rateSource(view));
    await settle();
    await handle.stop();
    release([{ id: 'a', apyBps: 1 }]);
    await settle();
    expect(view.open).toBe(0);
    expect(await store.sources.get('bestRateChanged', 'all')).toBeNull();
  });

  it('keeps an exclusive source alive through lease errors', async () => {
    const { store, clock } = setup();
    let failures = 1;
    const flaky: AutomationStore = {
      ...store,
      sources: {
        ...store.sources,
        acquireLease: async (...args) => {
          if (failures > 0) {
            failures -= 1;
            throw new Error('database restarting');
          }
          return store.sources.acquireLease(...args);
        },
      },
    };
    const view = markets();
    view.set({ id: 'a', apyBps: 1 });
    const handle = setup(flaky, clock).make('r').source(rateSource(view), { exclusive: true });
    await settle();
    await clock.advance(15_000);
    expect(view.open).toBe(1);
    await handle.stop();
    expect(view.open).toBe(0);
  });

  it('closes a failed watch before starting the next', async () => {
    const { store, clock } = setup();
    let failNext = false;
    const flaky: AutomationStore = {
      ...store,
      sources: {
        ...store.sources,
        advance: async (input) => {
          if (failNext) {
            failNext = false;
            throw new Error('database restarting');
          }
          return store.sources.advance(input);
        },
      },
    };
    const view = markets();
    view.set({ id: 'a', apyBps: 500 }, { id: 'b', apyBps: 400 });
    const handle = setup(flaky, clock).make('r').source(rateSource(view));
    await settle();
    failNext = true;
    view.set({ id: 'b', apyBps: 600 });
    await settle();
    await clock.advance(20_000);
    expect(view.subscriptions).toBe(2);
    expect(view.open).toBe(1);
    await handle.stop();
    expect((await deliveries(store)).map((delivery) => (delivery.payload as ViewTrigger).value)).toEqual(['b']);
  });
});

describe('append-view sources', () => {
  it('resumes from its cursor with no gaps or repeats', async () => {
    const { store, make } = setup();
    const tape = new FakeAppendView<{ amount: number }>();
    const source = fromAppendView(tape, { name: 'deposits', filter: (row) => row.amount > 0 });
    tape.append({ amount: 1 });
    tape.append({ amount: 0 });
    const first = make('r1').source(source);
    await settle();
    tape.append({ amount: 3 });
    await settle();
    await first.stop();
    tape.append({ amount: 4 });
    tape.append({ amount: 5 });
    const second = make('r2').source(source);
    await settle();
    await second.stop();
    expect(tape.resumedFrom).toEqual([undefined, '1:2']);
    expect((await deliveries(store)).map((delivery) => delivery.id)).toEqual([
      'view:deposits:1:0',
      'view:deposits:1:2',
      'view:deposits:1:3',
      'view:deposits:1:4',
    ]);
    expect((await deliveries(store))[1]!.payload).toEqual({ source: 'deposits', cursor: '1:2', row: { amount: 3 } });
  });

  it('refuses a view whose updates carry no cursor', async () => {
    const errors: unknown[] = [];
    const fake = fakeProtocol();
    const automations = createAutomations({
      client: fake.client,
      signer: walletsSigner(),
      policy,
      simulator: feeOnlySimulator,
      store: memoryStore(),
      cluster: 'dev',
      clock: manualClock(),
      onError: (error) => void errors.push(error),
    });
    const view = {
      watch: () => ({
        [Symbol.asyncIterator]: () => ({
          next: async () => ({ value: { type: 'upsert' as const, key: 'k', data: {} }, done: false }),
          return: async () => ({ value: undefined, done: true as const }),
        }),
      }),
    };
    const handle = automations.source(fromAppendView(view, { name: 'nope' }));
    await settle();
    await handle.stop();
    expect(String(errors[0])).toContain('not an append view');
  });
});
