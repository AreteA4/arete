import { describe, expect, it } from 'vitest';

import { fromAppendView, fromView } from '../sources/view';
import { FakeAppendView, FakeListView } from '../test-support/fake-views';
import { PROGRAM } from '../test-support/fake-protocol';
import { ALICE, BOB, harness } from '../test-support/harness';
import type { ScheduledTrigger } from './automations';
import { defineAutomations, describeDefinition } from './define';
import { scheduleSlotFor } from './time';

const MONDAY_NOON = Date.UTC(2026, 9, 19, 12, 0, 0);
const HOUR = 3_600_000;

function definition(sources?: (view: FakeListView<{ id: string; apyBps: number }>) => void) {
  const view = new FakeListView<{ id: string; apyBps: number }>((row) => row.id);
  sources?.(view);
  const triggers: ScheduledTrigger[] = [];
  const definition = defineAutomations(
    { name: 'vault', deployment: { deploymentId: 'dep' }, events: { 'vault.opened': { machine: 'instance' } } } as never,
    (automations) => {
      automations.schedule('daily', { every: 'day' });
      automations.on('daily', (trigger: ScheduledTrigger, ctx) => {
        triggers.push(trigger);
        return [ctx.step('deposit', { wallet: ALICE, instance: 'inst-1', args: { amount: '1000' } })];
      });
      automations.on('vault.opened', () => []);
      automations.source(
        fromView(view, {
          name: 'bestRate',
          key: (row) => row.id,
          derive: (rows) => [...rows].sort((a, b) => b.apyBps - a.apyBps)[0]?.id ?? null,
          changed: (previous, next) => previous !== undefined && previous !== next,
        }),
      );
      automations.on('bestRate', () => []);
    },
    { policy: { programs: [PROGRAM], maxSolPerTransaction: '0.01' } },
  );
  return { definition, view, triggers };
}

describe('schedules', () => {
  it('dispatches once per slot, from any number of ticks', async () => {
    const h = harness();
    await h.grant(ALICE);
    const { definition: daily, triggers } = definition();
    await h.automations.use(daily as never, { sources: 'poll' });

    const first = await h.automations.tick({ now: MONDAY_NOON });
    expect(first.scheduled).toEqual(['daily']);
    expect(first.ran).toBeGreaterThan(0);
    expect(triggers).toEqual([{ schedule: 'daily', slot: 'daily:2026-10-19', at: MONDAY_NOON }]);
    expect(h.outcomes).toMatchObject([{ state: 'completed', step: 'deposit', wallet: ALICE }]);

    // Later the same day, and a second replica ticking at once: nothing new.
    const again = await Promise.all([
      h.automations.tick({ now: MONDAY_NOON + HOUR }),
      h.make('w2').use(daily as never, { sources: 'poll' }).then((other) => other.tick({ now: MONDAY_NOON + 2 * HOUR })),
    ]);
    expect(again.map((result) => result.scheduled)).toEqual([[], []]);

    const tuesday = await h.automations.tick({ now: MONDAY_NOON + 24 * HOUR });
    expect(tuesday.scheduled).toEqual(['daily']);
    expect(triggers.map((trigger) => trigger.slot)).toEqual(['daily:2026-10-19', 'daily:2026-10-20']);
  });

  it('names interval slots by their start and refuses intervals under a second', () => {
    expect(scheduleSlotFor('sweep', '15m', Date.UTC(2026, 9, 19, 12, 29, 59))).toBe('sweep:2026-10-19T12:15:00.000Z');
    expect(scheduleSlotFor('sweep', 'week', MONDAY_NOON)).toBe('sweep:2026-W43');
    const h = harness();
    expect(() => h.automations.schedule('fast', { every: '500ms' })).toThrow(/at most once a second/);
    expect(() => h.automations.schedule('bad', { every: 'fortnight' as never })).toThrow(/invalid duration/);
  });

  it('reports a schedule without a handler and leaves jobs to a started worker with maxJobs 0', async () => {
    const h = harness();
    h.automations.schedule('orphan', { every: 'hour' });
    h.automations.schedule('hourly', { every: 'hour' });
    h.automations.on('hourly', (_trigger, ctx) => [ctx.step('deposit', { wallet: BOB, instance: 'inst-1' })]);
    const result = await h.automations.tick({ now: MONDAY_NOON, maxJobs: 0 });
    expect(result).toEqual({ scheduled: ['hourly'], polled: { delivered: 0, pending: 0 }, ran: 0 });
    expect(h.errors.map((error) => (error as Error).message)).toEqual([
      "schedule orphan has no handler: register one with on('orphan', ...)",
    ]);
    expect(await h.store.jobs.list({ kind: 'delivery' })).toHaveLength(1);
  });
});

describe('poll-mode sources', () => {
  it('polls the definition’s list sources on each tick instead of subscribing', async () => {
    const h = harness();
    const { definition: polled, view } = definition((rows) => rows.set({ id: 'kamino', apyBps: 500 }, { id: 'save', apyBps: 450 }));
    await h.automations.use(polled as never, { sources: 'poll' });
    expect(view.subscriptions).toBe(0);

    expect((await h.automations.tick({ now: MONDAY_NOON, maxJobs: 0 })).polled).toEqual({ delivered: 0, pending: 0 });
    view.set({ id: 'save', apyBps: 600 });
    expect((await h.automations.tick({ now: MONDAY_NOON + 60_000, maxJobs: 0 })).polled).toEqual({ delivered: 1, pending: 0 });
    expect((await h.automations.tick({ now: MONDAY_NOON + 120_000, maxJobs: 0 })).polled).toEqual({ delivered: 0, pending: 0 });
    expect(await h.automations.tick({ now: MONDAY_NOON, poll: false, maxJobs: 0 })).toMatchObject({ polled: { delivered: 0 } });
  });

  it('stops the live sources a definition started along with the workers', async () => {
    const h = harness();
    const { definition: live, view } = definition((rows) => rows.set({ id: 'kamino', apyBps: 500 }));
    await h.automations.use(live as never);
    for (let i = 0; i < 100 && view.open === 0; i += 1) await new Promise((resolve) => setTimeout(resolve, 5));
    expect(view.open).toBe(1);
    await h.automations.stop();
    expect(view.open).toBe(0);
  });

  it('disconnects the stacks a definition connected when stopped', async () => {
    const h = harness();
    let disconnected = 0;
    const connected = defineAutomations({ name: 'vault', deployment: {} } as never, async (automations) => {
      await automations.connect({ name: 'rates' } as never);
    });
    await h.automations.use(connected as never, { sources: 'poll', connect: async () => ({ disconnect: () => void (disconnected += 1) }) as never });
    expect(disconnected).toBe(0);
    await h.automations.stop();
    expect(disconnected).toBe(1);
    await h.automations.stop();
    expect(disconnected).toBe(1);
  });

  it('refuses an append source, which needs a live subscription', async () => {
    const h = harness();
    const appendOnly = defineAutomations({ name: 'vault', deployment: {} } as never, (automations) => {
      automations.source(fromAppendView(new FakeAppendView<{ id: string }>(), { name: 'fills' }));
    });
    await expect(h.automations.use(appendOnly as never, { sources: 'poll' })).rejects.toThrow(/fills reads an append view/);
  });
});

describe('describeDefinition', () => {
  it('lists the events, schedules, sources and other triggers without connecting', async () => {
    const connected = defineAutomations(
      { name: 'vault', events: { 'vault.opened': { machine: 'instance' }, 'vault.closed': { machine: 'instance' } } } as never,
      async (automations) => {
        // A definition that connects a stack for its sources gets a stand-in.
        const stack = await automations.connect({ name: 'rates' } as never);
        const view = (stack as unknown as { views: { rates: { markets: { list: FakeListView<{ id: string }> } } } }).views.rates.markets.list;
        automations.source(fromView(view, { name: 'bestRate', key: (row) => row.id, derive: () => null }));
        automations.on('bestRate', () => []);
        automations.schedule('weekly', { every: 'week' });
        automations.on('weekly', () => []);
        automations.on('vault.opened', () => []);
        automations.on('manual', () => []);
      },
    );
    expect(await describeDefinition(connected as never)).toEqual({
      protocol: 'vault',
      events: ['vault.opened'],
      schedules: [{ name: 'weekly', every: 'week' }],
      sources: [{ name: 'bestRate', kind: 'list' }],
      triggers: ['manual'],
    });
  });
});
