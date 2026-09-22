import type {
  QueryLease,
  RichUpdate,
  SubscriptionQuery,
  UnsubscribeFn,
  Update,
  WatchOptions,
} from './types';
import type { StorageAdapter } from './storage/adapter';
import type { SubscriptionRegistry } from './subscription';
import { AreteError, isReplayErrorCode } from './types';

const MAX_QUEUE_SIZE = 1000;

/**
 * A replayable record was dropped locally before the consumer read it.
 * `cursor` is the last one delivered, so the stream can be resumed from it
 * with `after` rather than silently skipping the lost records.
 */
export class StreamGapError extends AreteError {
  constructor(readonly cursor: string | undefined) {
    super(
      `Update stream dropped a replayable record; resume after ${cursor ?? 'the start of the window'}`,
      'stream-gap',
      { cursor }
    );
    this.name = 'StreamGapError';
  }
}

interface Queued<T> {
  value: T;
  cursor?: string;
}

/** Terminate the stream when a replay refusal makes further delivery impossible. */
function watchRefusals(lease: QueryLease, fail: (error: Error) => void): UnsubscribeFn {
  return lease.onChange(() => {
    const error = lease.getError();
    if (error && isReplayErrorCode(String(error.code))) fail(error);
  });
}

function createIterator<T>(
  start: (
    push: (value: T, cursor?: string) => void,
    fail: (error: Error) => void
  ) => UnsubscribeFn
): AsyncIterator<T> {
  const queue: Queued<T>[] = [];
  let waiting: {
    resolve: (result: IteratorResult<T>) => void;
    reject: (error: unknown) => void;
  } | null = null;
  let done = false;
  let failure: Error | null = null;
  let lastCursor: string | undefined;
  let stop: UnsubscribeFn = () => {};

  const cleanup = () => {
    if (done) return;
    done = true;
    stop();
    waiting?.resolve({ value: undefined, done: true });
    waiting = null;
  };
  const deliver = (item: Queued<T>): IteratorResult<T> => {
    if (item.cursor !== undefined) lastCursor = item.cursor;
    return { value: item.value, done: false };
  };
  /**
   * End the stream at `error`, after whatever is already queued.
   *
   * `discardQueued` is for a local overflow only: there the hole is at the
   * front of the queue, so everything behind it is no longer contiguous with
   * what the consumer has read. A server refusal is the opposite — it reports
   * records skipped *after* everything already sent, so the queue is real data
   * that arrived before the gap. Dropping it would lose records the consumer
   * was never told about and leave `recoverFrom` pointing past them.
   */
  const fail = (error: Error, discardQueued = false) => {
    if (done || failure) return;
    failure = error;
    if (discardQueued) queue.length = 0;
    const pending = waiting;
    waiting = null;
    // A waiter means the queue was empty, so there is nothing to drain first.
    if (pending && queue.length === 0) {
      failure = null;
      cleanup();
      pending.reject(error);
      return;
    }
    if (pending) {
      const item = queue.shift()!;
      pending.resolve(deliver(item));
    }
  };
  const push = (value: T, cursor?: string) => {
    if (done || failure) return;
    if (waiting) {
      const pending = waiting;
      waiting = null;
      pending.resolve(deliver({ value, cursor }));
      return;
    }
    if (queue.length >= MAX_QUEUE_SIZE) {
      const dropped = queue.shift();
      if (dropped?.cursor !== undefined) {
        fail(new StreamGapError(lastCursor), true);
        return;
      }
    }
    queue.push({ value, cursor });
  };
  stop = start(push, fail);
  if (done) stop();

  return {
    async next(): Promise<IteratorResult<T>> {
      const item = queue.shift();
      if (item !== undefined) return deliver(item);
      if (failure) {
        const error = failure;
        failure = null;
        cleanup();
        throw error;
      }
      if (done) return { value: undefined, done: true };
      return new Promise((resolve, reject) => {
        waiting = { resolve, reject };
      });
    },
    async return(): Promise<IteratorResult<T>> {
      cleanup();
      return { value: undefined, done: true };
    },
    async throw(error?: unknown): Promise<IteratorResult<T>> {
      cleanup();
      throw error;
    },
  };
}

function acquire(
  registry: SubscriptionRegistry,
  query: SubscriptionQuery,
  snapshotEnabled: boolean
): QueryLease {
  return registry.subscribe(query, snapshotEnabled);
}

export function createUpdateStream<T>(
  _storage: StorageAdapter,
  subscriptionRegistry: SubscriptionRegistry,
  query: SubscriptionQuery,
  keyFilter?: string,
  snapshotEnabled = true
): AsyncIterable<Update<T>> {
  return {
    [Symbol.asyncIterator](): AsyncIterator<Update<T>> {
      return createIterator((push, fail) => {
        const lease = acquire(subscriptionRegistry, query, snapshotEnabled);
        const unsubscribe = lease.onUpdate<T>((update) => {
          if (keyFilter === undefined || update.key === keyFilter) push(update, update.cursor);
        });
        const stopRefusals = watchRefusals(lease, fail);
        return () => {
          unsubscribe();
          stopRefusals();
          lease.release();
        };
      });
    },
  };
}

export function createEntityStream<T>(
  _storage: StorageAdapter,
  subscriptionRegistry: SubscriptionRegistry,
  query: SubscriptionQuery,
  options?: WatchOptions<any>,
  keyFilter?: string
): AsyncIterable<T> {
  type TOut = any;
  const schema = options?.schema;
  return {
    [Symbol.asyncIterator](): AsyncIterator<TOut> {
      return createIterator((push, fail) => {
        const lease = acquire(subscriptionRegistry, query, options?.withSnapshot ?? true);
        const emit = (entity: T, cursor?: string) => {
          if (!schema) {
            push(entity as TOut, cursor);
            return;
          }
          const parsed = schema.safeParse(entity);
          if (parsed.success) push(parsed.data as TOut, cursor);
        };
        for (const [index, entity] of lease.getSnapshot<T>().data.entries()) {
          const key = lease.getSnapshot<T>().keys[index];
          if (keyFilter === undefined || key === keyFilter) emit(entity);
        }
        const unsubscribe = lease.onRichUpdate<T>((update) => {
          if (keyFilter !== undefined && update.key !== keyFilter) return;
          if (update.type === 'created') emit(update.data, update.cursor);
          if (update.type === 'updated') emit(update.after, update.cursor);
        });
        const stopRefusals = watchRefusals(lease, fail);
        return () => {
          unsubscribe();
          stopRefusals();
          lease.release();
        };
      });
    },
  };
}

export function createRichUpdateStream<T>(
  _storage: StorageAdapter,
  subscriptionRegistry: SubscriptionRegistry,
  query: SubscriptionQuery,
  keyFilter?: string,
  snapshotEnabled = true
): AsyncIterable<RichUpdate<T>> {
  return {
    [Symbol.asyncIterator](): AsyncIterator<RichUpdate<T>> {
      return createIterator((push, fail) => {
        const lease = acquire(subscriptionRegistry, query, snapshotEnabled);
        const unsubscribe = lease.onRichUpdate<T>((update) => {
          if (keyFilter === undefined || update.key === keyFilter) push(update, update.cursor);
        });
        const stopRefusals = watchRefusals(lease, fail);
        return () => {
          unsubscribe();
          stopRefusals();
          lease.release();
        };
      });
    },
  };
}
