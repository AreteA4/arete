import type {
  Update,
  RichUpdate,
  TypedStateView,
  TypedListView,
  ViewDef,
  StackDefinition,
  TypedViews,
  WatchOptions,
  GetOptions,
  DefaultViewKey,
  QuerySnapshot,
  SubscriptionQuery,
  UnsubscribeFn,
} from './types';
import { AreteError } from './types';
import type { StorageAdapter } from './storage/adapter';
import type { SubscriptionRegistry } from './subscription';
import { createUpdateStream, createEntityStream, createRichUpdateStream } from './stream';

/** How long `get` and `getOne` wait for an initial snapshot, as in the Rust and Python SDKs. */
export const DEFAULT_INITIAL_DATA_TIMEOUT_MS = 5_000;

/**
 * `get` or `getOne` gave up waiting for the initial snapshot: the socket never
 * delivered it (no connection, a reconnect storm, a server that never
 * answers). The read's subscription is released before this is thrown.
 */
export class InitialDataTimeoutError extends AreteError {
  constructor(readonly view: string, readonly timeoutMs: number) {
    super(
      `Timed out after ${timeoutMs}ms waiting for the initial snapshot of view '${view}'`,
      'INITIAL_DATA_TIMEOUT',
      { view, timeoutMs }
    );
    this.name = 'InitialDataTimeoutError';
  }
}

function queryOptions(options?: GetOptions): {
  query: Omit<SubscriptionQuery, 'view' | 'key'>;
  snapshotEnabled: boolean;
} {
  const {
    schema: _schema,
    withSnapshot,
    timeoutMs: _timeoutMs,
    ...query
  } = options ?? {};
  return { query, snapshotEnabled: withSnapshot ?? true };
}

function resolveTimeout(options?: GetOptions): number | null {
  const timeoutMs = options?.timeoutMs === undefined
    ? DEFAULT_INITIAL_DATA_TIMEOUT_MS
    : options.timeoutMs;
  if (timeoutMs !== null && (!Number.isFinite(timeoutMs) || timeoutMs <= 0)) {
    throw new RangeError('timeoutMs must be a positive finite number or null');
  }
  return timeoutMs;
}

/**
 * Open (or reuse) the equivalent subscription, wait until its snapshot has
 * resolved or its query has failed, and release it. Bounded by `timeoutMs`
 * (`null` waits forever) so a socket that never delivers cannot hang the caller.
 */
async function readResolved<T>(
  registry: SubscriptionRegistry,
  query: SubscriptionQuery,
  snapshotEnabled: boolean,
  timeoutMs: number | null
): Promise<QuerySnapshot<T>> {
  const lease = registry.subscribe(query, snapshotEnabled);
  try {
    await new Promise<void>((resolve, reject) => {
      let settled = false;
      let stop: UnsubscribeFn = () => {};
      let timer: ReturnType<typeof setTimeout> | undefined;
      const finish = (error?: Error) => {
        if (settled) return;
        settled = true;
        stop();
        if (timer !== undefined) clearTimeout(timer);
        if (error) reject(error);
        else resolve();
      };
      const check = () => {
        const error = lease.getError();
        if (error) finish(error);
        else if (!lease.getSnapshot().isLoading) finish();
      };
      check();
      if (settled) return;
      stop = lease.onChange(check);
      if (timeoutMs !== null) {
        timer = setTimeout(
          () => finish(new InitialDataTimeoutError(query.view, timeoutMs)),
          timeoutMs
        );
      }
    });
    return lease.getSnapshot<T>();
  } finally {
    lease.release();
  }
}

function serializeViewKeyValue(value: unknown, view: string, field?: string): string {
  const location = field === undefined ? `view '${view}'` : `key field '${field}' for view '${view}'`;
  if (typeof value === 'string') return value;
  if (typeof value === 'bigint') return value.toString(10);
  if (typeof value === 'number') {
    if (!Number.isSafeInteger(value)) {
      throw new TypeError(`${location} must be a safe integer`);
    }
    return value.toString(10);
  }
  throw new TypeError(`${location} must be a string, safe integer, or bigint`);
}

export function serializeViewKey<TKey>(
  viewDef: ViewDef<unknown, 'state', TKey>,
  key: TKey
): string {
  const keyFields = viewDef.keyFields ?? [];
  if (keyFields.length === 0) {
    return serializeViewKeyValue(key, viewDef.view);
  }
  if (keyFields.length !== 1) {
    throw new TypeError(
      `View '${viewDef.view}' has an unsupported composite key with fields [${keyFields.join(', ')}]`
    );
  }
  if (key === null || typeof key !== 'object' || Array.isArray(key)) {
    throw new TypeError(`View '${viewDef.view}' requires an object key`);
  }

  const field = keyFields[0]!;
  if (!Object.prototype.hasOwnProperty.call(key, field)) {
    throw new TypeError(`View '${viewDef.view}' key is missing field '${field}'`);
  }
  return serializeViewKeyValue((key as Record<string, unknown>)[field], viewDef.view, field);
}

export function createTypedStateView<T, TKey = unknown>(
  viewDef: ViewDef<T, 'state', TKey>,
  storage: StorageAdapter,
  subscriptionRegistry: SubscriptionRegistry
): TypedStateView<T, DefaultViewKey<TKey>> {
  type Key = DefaultViewKey<TKey>;
  const wireKey = (key: Key): string => serializeViewKey(viewDef, key as TKey);

  return {
    use<TSchema = T>(key: Key, options?: WatchOptions<TSchema>): AsyncIterable<TSchema> {
      const serializedKey = wireKey(key);
      const { query } = queryOptions(options);
      return createEntityStream<T>(
        storage,
        subscriptionRegistry,
        { view: viewDef.view, key: serializedKey, ...query },
        options,
        serializedKey
      ) as AsyncIterable<TSchema>;
    },

    watch(key: Key, options?: WatchOptions): AsyncIterable<Update<T>> {
      const serializedKey = wireKey(key);
      const { query, snapshotEnabled } = queryOptions(options);
      return createUpdateStream<T>(
        storage,
        subscriptionRegistry,
        { view: viewDef.view, key: serializedKey, ...query },
        serializedKey,
        snapshotEnabled
      );
    },

    watchRich(key: Key, options?: WatchOptions): AsyncIterable<RichUpdate<T>> {
      const serializedKey = wireKey(key);
      const { query, snapshotEnabled } = queryOptions(options);
      return createRichUpdateStream<T>(
        storage,
        subscriptionRegistry,
        { view: viewDef.view, key: serializedKey, ...query },
        serializedKey,
        snapshotEnabled
      );
    },

    async get(key: Key, options?: GetOptions): Promise<T | null> {
      const timeoutMs = resolveTimeout(options);
      const { query, snapshotEnabled } = queryOptions(options);
      const snapshot = await readResolved<T>(
        subscriptionRegistry,
        { view: viewDef.view, key: wireKey(key), ...query },
        snapshotEnabled,
        timeoutMs
      );
      return snapshot.data[0] ?? null;
    },

    getSync(key: Key, options?: WatchOptions): T | null | undefined {
      const { query, snapshotEnabled } = queryOptions(options);
      const snapshot = subscriptionRegistry.getSnapshot<T>({
        view: viewDef.view,
        key: wireKey(key),
        ...query,
      }, snapshotEnabled);
      return snapshot ? snapshot.data[0] ?? null : undefined;
    },
  };
}

export function createTypedListView<T>(
  viewDef: ViewDef<T, 'list'>,
  storage: StorageAdapter,
  subscriptionRegistry: SubscriptionRegistry
): TypedListView<T> {
  return {
    use<TSchema = T>(options?: WatchOptions<TSchema>): AsyncIterable<TSchema> {
      const { query } = queryOptions(options);
      return createEntityStream<T>(
        storage,
        subscriptionRegistry,
        { view: viewDef.view, ...query },
        options
      ) as AsyncIterable<TSchema>;
    },

    watch(options?: WatchOptions): AsyncIterable<Update<T>> {
      const { query, snapshotEnabled } = queryOptions(options);
      return createUpdateStream<T>(
        storage,
        subscriptionRegistry,
        { view: viewDef.view, ...query },
        undefined,
        snapshotEnabled
      );
    },

    watchRich(options?: WatchOptions): AsyncIterable<RichUpdate<T>> {
      const { query, snapshotEnabled } = queryOptions(options);
      return createRichUpdateStream<T>(
        storage,
        subscriptionRegistry,
        { view: viewDef.view, ...query },
        undefined,
        snapshotEnabled
      );
    },

    async get(options?: GetOptions): Promise<T[]> {
      const timeoutMs = resolveTimeout(options);
      const { query, snapshotEnabled } = queryOptions(options);
      const snapshot = await readResolved<T>(
        subscriptionRegistry,
        { view: viewDef.view, ...query },
        snapshotEnabled,
        timeoutMs
      );
      return [...snapshot.data];
    },

    async getOne(options?: Omit<GetOptions, 'take'>): Promise<T | null> {
      const items = await this.get({ ...options, take: 1 });
      return items[0] ?? null;
    },

    getSync(options?: WatchOptions): T[] | undefined {
      const { query, snapshotEnabled } = queryOptions(options);
      const snapshot = subscriptionRegistry.getSnapshot<T>(
        { view: viewDef.view, ...query },
        snapshotEnabled
      );
      return snapshot ? [...snapshot.data] : undefined;
    },
  };
}

export function createTypedViews<TStack extends StackDefinition>(
  stack: TStack,
  storage: StorageAdapter,
  subscriptionRegistry: SubscriptionRegistry
): TypedViews<TStack['views']> {
  const views = {} as Record<string, Record<string, unknown>>;

  for (const [entityName, viewGroup] of Object.entries(stack.views)) {
    const group = viewGroup as Record<string, ViewDef<unknown, 'state' | 'list', unknown>>;
    const typedGroup: Record<string, unknown> = {};

    for (const [viewName, viewDef] of Object.entries(group)) {
      if (viewDef.mode === 'state') {
        typedGroup[viewName] = createTypedStateView(viewDef as ViewDef<unknown, 'state', unknown>, storage, subscriptionRegistry);
      } else if (viewDef.mode === 'list') {
        typedGroup[viewName] = createTypedListView(viewDef as ViewDef<unknown, 'list'>, storage, subscriptionRegistry);
      }
    }

    views[entityName] = typedGroup;
  }

  return views as TypedViews<TStack['views']>;
}
