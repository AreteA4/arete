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

const MAX_QUEUE_SIZE = 1000;

function enqueue<T>(queue: T[], value: T): void {
  if (queue.length >= MAX_QUEUE_SIZE) queue.shift();
  queue.push(value);
}

function createIterator<T>(
  start: (push: (value: T) => void) => UnsubscribeFn
): AsyncIterator<T> {
  const queue: T[] = [];
  let waitingResolve: ((value: IteratorResult<T>) => void) | null = null;
  let done = false;
  const push = (value: T) => {
    if (waitingResolve) {
      const resolve = waitingResolve;
      waitingResolve = null;
      resolve({ value, done: false });
    } else {
      enqueue(queue, value);
    }
  };
  const stop = start(push);
  const cleanup = () => {
    if (done) return;
    done = true;
    stop();
    waitingResolve?.({ value: undefined, done: true });
    waitingResolve = null;
  };

  return {
    async next(): Promise<IteratorResult<T>> {
      if (done) return { value: undefined, done: true };
      const value = queue.shift();
      if (value !== undefined) return { value, done: false };
      return new Promise((resolve) => {
        waitingResolve = resolve;
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
      return createIterator((push) => {
        const lease = acquire(subscriptionRegistry, query, snapshotEnabled);
        const unsubscribe = lease.onUpdate<T>((update) => {
          if (keyFilter === undefined || update.key === keyFilter) push(update);
        });
        return () => {
          unsubscribe();
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
      return createIterator((push) => {
        const lease = acquire(subscriptionRegistry, query, options?.withSnapshot ?? true);
        const emit = (entity: T) => {
          if (!schema) {
            push(entity as TOut);
            return;
          }
          const parsed = schema.safeParse(entity);
          if (parsed.success) push(parsed.data as TOut);
        };
        for (const [index, entity] of lease.getSnapshot<T>().data.entries()) {
          const key = lease.getSnapshot<T>().keys[index];
          if (keyFilter === undefined || key === keyFilter) emit(entity);
        }
        const unsubscribe = lease.onRichUpdate<T>((update) => {
          if (keyFilter !== undefined && update.key !== keyFilter) return;
          if (update.type === 'created') emit(update.data);
          if (update.type === 'updated') emit(update.after);
        });
        return () => {
          unsubscribe();
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
      return createIterator((push) => {
        const lease = acquire(subscriptionRegistry, query, snapshotEnabled);
        const unsubscribe = lease.onRichUpdate<T>((update) => {
          if (keyFilter === undefined || update.key === keyFilter) push(update);
        });
        return () => {
          unsubscribe();
          lease.release();
        };
      });
    },
  };
}
