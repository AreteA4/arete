import { describe, expect, it } from 'vitest';

import type { ConnectionManager } from './connection';
import { FrameProcessor } from './frame-processor';
import { QueryStore } from './query-store';
import { MemoryAdapter } from './storage/memory-adapter';
import { SubscriptionRegistry } from './subscription';
import { StreamGapError, createUpdateStream } from './stream';
import type { SubscriptionQuery, Update } from './types';

const EPOCH = '0f8c2b31-6a4e-4f0b-9a77-1d2c3e4f5a6b';

function harness(query: SubscriptionQuery) {
  const storage = new MemoryAdapter();
  const queries = new QueryStore(storage);
  const registry = new SubscriptionRegistry({
    subscribe: () => undefined,
    unsubscribe: () => undefined,
    refresh: () => undefined,
  } as unknown as ConnectionManager, queries);
  const processor = new FrameProcessor(storage, { queryStore: queries, maxEntriesPerView: null });
  const stream = createUpdateStream<{ amount: number }>(storage, registry, query, undefined, false);
  const iterator = stream[Symbol.asyncIterator]();
  const subscriptionId = registry.getActiveSubscriptions()[0]!.subscriptionId;
  return { processor, iterator, subscriptionId };
}

function ack(subscriptionId: string, query: SubscriptionQuery, replayable: boolean) {
  return {
    protocolVersion: 2 as const,
    subscriptionId,
    op: 'subscribed' as const,
    mode: 'append' as const,
    query,
    ...(replayable
      ? { replayWindow: { epoch: EPOCH, earliest: 0, next: 0 } }
      : {}),
  };
}

function event(subscriptionId: string, view: string, offset: number | undefined) {
  return {
    protocolVersion: 2 as const,
    subscriptionId,
    mode: 'append' as const,
    entity: view,
    op: 'upsert' as const,
    key: `trade-${offset ?? 'live'}`,
    data: { amount: offset ?? 0 },
    ...(offset === undefined ? {} : { offset }),
  };
}

describe('update stream overflow', () => {
  it('ends the stream at the last delivered cursor when a replayable record is dropped', async () => {
    const query: SubscriptionQuery = { view: 'Trade/append' };
    const { processor, iterator, subscriptionId } = harness(query);
    processor.handleFrame(ack(subscriptionId, query, true));

    for (let offset = 1; offset <= 3; offset++) {
      processor.handleFrame(event(subscriptionId, query.view, offset));
    }
    const read: Update<{ amount: number }>[] = [];
    for (let index = 0; index < 3; index++) {
      read.push((await iterator.next()).value);
    }
    expect(read.map((update) => update.cursor)).toEqual([
      `${EPOCH}:1`,
      `${EPOCH}:2`,
      `${EPOCH}:3`,
    ]);

    // Nothing reads while 1001 more arrive: the queue holds 1000.
    for (let offset = 4; offset <= 1004; offset++) {
      processor.handleFrame(event(subscriptionId, query.view, offset));
    }

    const failure = await iterator.next().then(
      (result) => result,
      (error: unknown) => error
    );
    expect(failure).toBeInstanceOf(StreamGapError);
    expect((failure as StreamGapError).cursor).toBe(`${EPOCH}:3`);
    expect(await iterator.next()).toEqual({ value: undefined, done: true });
  });

  it('keeps dropping the oldest update when the view has no tape', async () => {
    const query: SubscriptionQuery = { view: 'Trade/live' };
    const { processor, iterator, subscriptionId } = harness(query);
    processor.handleFrame(ack(subscriptionId, query, false));

    for (let index = 0; index < 1200; index++) {
      processor.handleFrame(event(subscriptionId, query.view, undefined));
    }

    const next = await iterator.next();
    expect(next.done).toBe(false);
    expect(next.value.cursor).toBeUndefined();
  });

  it('delivers everything queued before a server refusal, then the refusal', async () => {
    const query: SubscriptionQuery = { view: 'Trade/append' };
    const { processor, iterator, subscriptionId } = harness(query);
    processor.handleFrame(ack(subscriptionId, query, true));

    // Queued while nothing is reading. `replay-lagged` reports records skipped
    // *after* everything the server had already sent, so these are real and
    // contiguous — dropping them would lose data the consumer was never told
    // about, and leave `recoverFrom` pointing past it.
    for (let offset = 1; offset <= 3; offset++) {
      processor.handleFrame(event(subscriptionId, query.view, offset));
    }
    processor.handleFrame({
      type: 'error' as const,
      protocolVersion: 2 as const,
      subscriptionId,
      code: 'replay-lagged',
      fatal: false,
      recoverFrom: `${EPOCH}:4180`,
    });

    const delivered: (string | undefined)[] = [];
    let failure: unknown;
    for (;;) {
      const result = await iterator.next().then(
        (value) => value,
        (error: unknown) => {
          failure = error;
          return { done: true, value: undefined } as IteratorResult<Update<{ amount: number }>>;
        }
      );
      if (result.done) break;
      delivered.push(result.value.cursor);
    }

    expect(delivered).toEqual([`${EPOCH}:1`, `${EPOCH}:2`, `${EPOCH}:3`]);
    expect(failure).toMatchObject({
      code: 'replay-lagged',
      details: { recoverFrom: `${EPOCH}:4180` },
    });
  });

  it('ends the stream with the wire code when the server refuses a replay', async () => {
    const query: SubscriptionQuery = { view: 'Trade/append', after: `${EPOCH}:9` };
    const { processor, iterator, subscriptionId } = harness(query);
    processor.handleFrame(ack(subscriptionId, query, true));

    const pending = iterator.next();
    processor.handleFrame({
      type: 'error' as const,
      protocolVersion: 2 as const,
      subscriptionId,
      code: 'replay-lagged',
      fatal: false,
      recoverFrom: `${EPOCH}:4180`,
    });

    const failure = await pending.then(
      (result) => result,
      (error: unknown) => error
    );
    expect(failure).toMatchObject({
      code: 'replay-lagged',
      details: { recoverFrom: `${EPOCH}:4180` },
    });
  });
});
