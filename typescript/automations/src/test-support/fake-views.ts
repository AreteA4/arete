// In-memory list and append views for view-source tests. Not part of the package.

import type { RichUpdate, Update } from '@usearete/sdk';

import type { Clock } from '../runtime/time';
import type { AppendViewLike, ListViewLike } from '../sources/view';

/** An async iterable fed by `push`, closed by `return`. */
class Feed<T> implements AsyncIterator<T> {
  private queue: T[] = [];
  private waiting: ((result: IteratorResult<T>) => void) | null = null;
  closed = false;

  constructor(private readonly onClose: () => void) {}

  push(value: T) {
    if (this.closed) return;
    if (this.waiting) {
      const resolve = this.waiting;
      this.waiting = null;
      resolve({ value, done: false });
    } else {
      this.queue.push(value);
    }
  }

  next(): Promise<IteratorResult<T>> {
    if (this.queue.length > 0) return Promise.resolve({ value: this.queue.shift()!, done: false });
    if (this.closed) return Promise.resolve({ value: undefined, done: true });
    return new Promise((resolve) => {
      this.waiting = resolve;
    });
  }

  return(): Promise<IteratorResult<T>> {
    this.closed = true;
    this.onClose();
    this.waiting?.({ value: undefined, done: true });
    this.waiting = null;
    return Promise.resolve({ value: undefined, done: true });
  }
}

export class FakeListView<T> implements ListViewLike<T> {
  readonly rows = new Map<string, T>();
  private feeds = new Set<Feed<RichUpdate<T>>>();
  subscriptions = 0;
  /** Whether a new stream replays the current rows (an already-open query does not). */
  replay = true;

  constructor(private readonly keyOf: (row: T) => string) {}

  get open(): number {
    return this.feeds.size;
  }

  async get(): Promise<T[]> {
    return [...this.rows.values()];
  }

  getSync(): T[] | undefined {
    return [...this.rows.values()];
  }

  watchRich(): AsyncIterable<RichUpdate<T>> {
    this.subscriptions += 1;
    const feed: Feed<RichUpdate<T>> = new Feed(() => this.feeds.delete(feed));
    this.feeds.add(feed);
    if (this.replay) for (const [key, data] of this.rows) feed.push({ type: 'created', key, data });
    return { [Symbol.asyncIterator]: () => feed };
  }

  /**
   * A reconnect: the list is replaced by `rows`, and subscribers hear only
   * `created` for the rows present, as the SDK does after a resync.
   */
  reconnect(...rows: T[]) {
    this.rows.clear();
    for (const row of rows) this.rows.set(this.keyOf(row), row);
    for (const feed of this.feeds) for (const [key, data] of this.rows) feed.push({ type: 'created', key, data });
  }

  /** Change rows and tell every subscriber. */
  set(...rows: T[]) {
    for (const row of rows) {
      const key = this.keyOf(row);
      const before = this.rows.get(key);
      this.rows.set(key, row);
      for (const feed of this.feeds) {
        feed.push(before === undefined ? { type: 'created', key, data: row } : { type: 'updated', key, before, after: row });
      }
    }
  }
}

export class FakeAppendView<T> implements AppendViewLike<T> {
  readonly tape: Array<{ cursor: string; row: T }> = [];
  private feeds = new Set<{ feed: Feed<Update<T>> }>();
  readonly resumedFrom: Array<string | undefined> = [];

  watch(options: { after?: string } = {}): AsyncIterable<Update<T>> {
    this.resumedFrom.push(options.after);
    const holder = { feed: undefined as unknown as Feed<Update<T>> };
    holder.feed = new Feed(() => this.feeds.delete(holder));
    this.feeds.add(holder);
    const start = options.after === undefined ? 0 : this.tape.findIndex((entry) => entry.cursor === options.after) + 1;
    for (const entry of this.tape.slice(start)) {
      holder.feed.push({ type: 'upsert', key: entry.cursor, data: entry.row, cursor: entry.cursor });
    }
    return { [Symbol.asyncIterator]: () => holder.feed };
  }

  append(row: T) {
    const cursor = `1:${this.tape.length}`;
    this.tape.push({ cursor, row });
    for (const { feed } of this.feeds) feed.push({ type: 'upsert', key: cursor, data: row, cursor });
  }
}

/** A clock whose sleeps wait for `advance` to pass their wake time. */
export function manualClock(start = 1_700_000_000_000): Clock & { advance(ms: number): Promise<void> } {
  let now = start;
  const sleepers: Array<{ at: number; wake: () => void }> = [];
  return {
    now: () => now,
    sleep: (ms) =>
      new Promise<void>((resolve) => {
        sleepers.push({ at: now + ms, wake: resolve });
      }),
    async advance(ms) {
      now += ms;
      for (const sleeper of [...sleepers]) {
        if (sleeper.at <= now) {
          sleepers.splice(sleepers.indexOf(sleeper), 1);
          sleeper.wake();
        }
      }
      await settle();
    },
  };
}

/** Let pending promise chains run. */
export async function settle(rounds = 20) {
  for (let round = 0; round < rounds; round += 1) await new Promise((resolve) => setTimeout(resolve, 0));
}
