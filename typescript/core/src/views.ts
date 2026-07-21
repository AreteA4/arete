import type {
  Update,
  RichUpdate,
  TypedStateView,
  TypedListView,
  ViewDef,
  StackDefinition,
  TypedViews,
  WatchOptions,
  DefaultViewKey,
} from './types';
import type { StorageAdapter } from './storage/adapter';
import type { SubscriptionRegistry } from './subscription';
import { createUpdateStream, createEntityStream, createRichUpdateStream } from './stream';

function queryOptions(options?: WatchOptions): {
  query: Omit<import('./types').SubscriptionQuery, 'view' | 'key'>;
  snapshotEnabled: boolean;
} {
  const {
    schema: _schema,
    withSnapshot,
    ...query
  } = options ?? {};
  return { query, snapshotEnabled: withSnapshot ?? true };
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

    async get(key: Key, options?: WatchOptions): Promise<T | null> {
      return this.getSync(key, options) ?? null;
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

    async get(options?: WatchOptions): Promise<T[]> {
      return this.getSync(options) ?? [];
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
