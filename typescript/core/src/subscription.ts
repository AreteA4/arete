import type {
  ConnectionState,
  QueryLease,
  QuerySnapshot,
  RichUpdate,
  Subscription,
  SubscriptionQuery,
  SubscriptionRequest,
  SubscriptionSnapshotOptions,
  UnsubscribeFn,
  Update,
} from './types';
import { AreteError } from './types';
import type { ConnectionManager } from './connection';
import type { QueryStore } from './query-store';

interface SubscriptionTracker {
  subscription: Subscription;
  queryKey: string;
  refCount: number;
  refreshPromise?: Promise<void>;
}

interface NormalizedSubscriptionRequest {
  query: SubscriptionQuery;
  snapshot: SubscriptionSnapshotOptions;
}

let fallbackId = 0;
const QUERY_FIELDS = new Set([
  'view',
  'key',
  'partition',
  'filters',
  'take',
  'skip',
  'after',
  'snapshotLimit',
]);
const SUBSCRIPTION_FIELDS = new Set([
  'type',
  'protocolVersion',
  'subscriptionId',
  'query',
  'snapshot',
]);

function canonicalJsonValue(value: unknown, path: string): unknown {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (Array.isArray(value)) {
    return value.map((entry, index) => canonicalJsonValue(entry, `${path}[${index}]`));
  }
  if (typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, entry]) => [key, canonicalJsonValue(entry, `${path}.${key}`)])
    );
  }
  throw new TypeError(`${path} must contain JSON values`);
}

function assertPositiveInteger(value: number | undefined, field: string): void {
  if (value !== undefined && (!Number.isInteger(value) || value <= 0)) {
    throw new TypeError(`${field} must be a positive integer`);
  }
}

export function validateSubscriptionId(subscriptionId: string): void {
  if (subscriptionId.length === 0 || subscriptionId.trim() !== subscriptionId) {
    throw new TypeError('subscriptionId must be non-empty with no surrounding whitespace');
  }
  if (/\p{Cc}/u.test(subscriptionId)) {
    throw new TypeError('subscriptionId must not contain control characters');
  }
  if (new TextEncoder().encode(subscriptionId).byteLength > 128) {
    throw new TypeError('subscriptionId must not exceed 128 bytes');
  }
}

export function createSubscriptionId(): string {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  if (randomUuid) return `a4-${randomUuid}`;
  fallbackId++;
  return `a4-${Date.now().toString(36)}-${fallbackId.toString(36)}`;
}

export function normalizeSubscriptionQuery(query: SubscriptionQuery): SubscriptionQuery {
  if (typeof query.view !== 'string' || query.view.length === 0) {
    throw new TypeError('query.view must be a non-empty string');
  }
  if (Object.keys(query).some((key) => !QUERY_FIELDS.has(key))) {
    throw new TypeError('query contains an unknown protocol v2 field');
  }
  if (query.key !== undefined && typeof query.key !== 'string') {
    throw new TypeError('query.key must be a string');
  }
  if (query.partition !== undefined && typeof query.partition !== 'string') {
    throw new TypeError('query.partition must be a string');
  }
  if (
    query.filters !== undefined
    && (query.filters === null || typeof query.filters !== 'object' || Array.isArray(query.filters))
  ) {
    throw new TypeError('query.filters must be an object');
  }
  if (query.after !== undefined && typeof query.after !== 'string') {
    throw new TypeError('query.after must be a string');
  }
  assertPositiveInteger(query.take, 'query.take');
  assertPositiveInteger(query.snapshotLimit, 'query.snapshotLimit');
  if (query.skip !== undefined && (!Number.isInteger(query.skip) || query.skip < 0)) {
    throw new TypeError('query.skip must be a non-negative integer');
  }

  const normalized: SubscriptionQuery = { view: query.view };
  if (query.key !== undefined) normalized.key = query.key;
  if (query.partition !== undefined) normalized.partition = query.partition;
  if (query.filters !== undefined) {
    normalized.filters = Object.fromEntries(
      Object.entries(query.filters)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([path, value]) => [path, canonicalJsonValue(value, `query.filters.${path}`)])
    );
  }
  if (query.take !== undefined) normalized.take = query.take;
  if (query.skip !== undefined) normalized.skip = query.skip;
  if (query.after !== undefined) normalized.after = query.after;
  if (query.snapshotLimit !== undefined) normalized.snapshotLimit = query.snapshotLimit;
  return normalized;
}

export function normalizeSubscriptionRequest(request: SubscriptionRequest): NormalizedSubscriptionRequest {
  const enabled = request.snapshot?.enabled ?? true;
  if (typeof enabled !== 'boolean') {
    throw new TypeError('snapshot.enabled must be a boolean');
  }
  return {
    query: normalizeSubscriptionQuery(request.query),
    snapshot: { enabled },
  };
}

export function canonicalQueryKey(request: SubscriptionRequest): string {
  const normalized = normalizeSubscriptionRequest(request);
  return JSON.stringify({ query: normalized.query, snapshot: normalized.snapshot });
}

export function normalizeSubscription(subscription: Subscription): Subscription {
  if (subscription.type !== 'subscribe' || subscription.protocolVersion !== 2) {
    throw new TypeError('Subscription must use WebSocket protocolVersion 2');
  }
  if (Object.keys(subscription).some((key) => !SUBSCRIPTION_FIELDS.has(key))) {
    throw new TypeError('Subscription contains an unknown protocol v2 field');
  }
  validateSubscriptionId(subscription.subscriptionId);
  const request = normalizeSubscriptionRequest(subscription);
  return {
    type: 'subscribe',
    protocolVersion: 2,
    subscriptionId: subscription.subscriptionId,
    query: request.query,
    snapshot: request.snapshot,
  };
}

export function subscriptionIdentityKey(subscription: Subscription): string {
  return canonicalQueryKey(subscription);
}

export function subscriptionOptionsKey(subscription: Subscription): string {
  return canonicalQueryKey(subscription);
}

function refreshError(value: unknown): AreteError {
  if (value instanceof AreteError) return value;
  return new AreteError(
    value instanceof Error ? value.message : 'Subscription refresh failed',
    'SUBSCRIPTION_ERROR',
    value
  );
}

export class SubscriptionRegistry {
  private readonly subscriptions = new Map<string, SubscriptionTracker>();
  private readonly subscriptionsById = new Map<string, SubscriptionTracker>();

  constructor(
    private readonly connection: ConnectionManager,
    private readonly queryStore: QueryStore
  ) {}

  subscribe(query: SubscriptionQuery, snapshotEnabled = true): QueryLease {
    const request = normalizeSubscriptionRequest({
      query,
      snapshot: { enabled: snapshotEnabled },
    });
    const queryKey = canonicalQueryKey(request);
    let tracker = this.subscriptions.get(queryKey);

    if (tracker) {
      tracker.refCount++;
    } else {
      const subscription: Subscription = {
        type: 'subscribe',
        protocolVersion: 2,
        subscriptionId: createSubscriptionId(),
        query: request.query,
        snapshot: request.snapshot,
      };
      tracker = { subscription, queryKey, refCount: 1 };
      this.queryStore.register(subscription, queryKey);
      try {
        this.connection.subscribe(subscription);
      } catch (error) {
        this.queryStore.unregister(subscription.subscriptionId);
        throw error;
      }
      this.subscriptions.set(queryKey, tracker);
      this.subscriptionsById.set(subscription.subscriptionId, tracker);
    }

    return this.createLease(tracker);
  }

  refresh(query: SubscriptionQuery, snapshotEnabled = true): Promise<void> {
    const tracker = this.getTracker(query, snapshotEnabled);
    if (!tracker) {
      return Promise.reject(new AreteError(
        `Cannot refresh inactive query '${query.view}'`,
        'SUBSCRIPTION_NOT_FOUND'
      ));
    }
    return this.refreshTracker(tracker);
  }

  /**
   * Refresh every active subscription for a view, optionally narrowed to one
   * serialized key. Resolves immediately when nothing matches — refreshing a
   * view with no active subscriptions is a no-op, not an error.
   */
  refreshView(view: string, key?: string): Promise<void> {
    const matches = Array.from(this.subscriptions.values()).filter(({ subscription }) =>
      subscription.query.view === view
      && (key === undefined || subscription.query.key === key)
    );
    return Promise.all(matches.map((tracker) => this.refreshTracker(tracker))).then(() => undefined);
  }

  getRefCount(query: SubscriptionQuery, snapshotEnabled = true): number {
    return this.getTracker(query, snapshotEnabled)?.refCount ?? 0;
  }

  getActiveSubscriptions(): Subscription[] {
    return Array.from(this.subscriptions.values(), ({ subscription }) =>
      normalizeSubscription(subscription)
    );
  }

  getSnapshot<T>(query: SubscriptionQuery, snapshotEnabled = true): QuerySnapshot<T> | undefined {
    const tracker = this.getTracker(query, snapshotEnabled);
    return tracker
      ? this.queryStore.getSnapshot<T>(tracker.subscription.subscriptionId)
      : undefined;
  }

  handleConnectionState(state: ConnectionState): void {
    if (state === 'reconnecting') this.queryStore.beginReconnect();
    if (state === 'error') {
      this.queryStore.failRefreshing(
        new AreteError('Connection failed while refreshing subscriptions', 'CONNECTION_ERROR')
      );
    }
  }

  clear(): void {
    for (const { subscription } of this.subscriptions.values()) {
      try {
        this.connection.unsubscribe(subscription.subscriptionId);
      } catch {
        // Local release must complete even when the socket can no longer send.
      } finally {
        this.queryStore.unregister(subscription.subscriptionId);
      }
    }
    this.subscriptions.clear();
    this.subscriptionsById.clear();
  }

  private createLease(tracker: SubscriptionTracker): QueryLease {
    let released = false;
    const subscriptionId = tracker.subscription.subscriptionId;
    return {
      subscription: tracker.subscription,
      queryKey: tracker.queryKey,
      getSnapshot: <T>() => {
        const snapshot = this.queryStore.getSnapshot<T>(subscriptionId);
        if (!snapshot) {
          throw new AreteError('Query lease has been released', 'SUBSCRIPTION_NOT_FOUND');
        }
        return snapshot;
      },
      onChange: (callback: () => void): UnsubscribeFn =>
        this.queryStore.onChange(subscriptionId, callback),
      onUpdate: <T>(callback: (update: Update<T>) => void): UnsubscribeFn =>
        this.queryStore.onUpdate(subscriptionId, callback),
      onRichUpdate: <T>(callback: (update: RichUpdate<T>) => void): UnsubscribeFn =>
        this.queryStore.onRichUpdate(subscriptionId, callback),
      refresh: () => this.refreshTracker(tracker),
      release: () => {
        if (released) return;
        released = true;
        this.releaseTracker(tracker);
      },
    };
  }

  private getTracker(
    query: SubscriptionQuery,
    snapshotEnabled: boolean
  ): SubscriptionTracker | undefined {
    return this.subscriptions.get(canonicalQueryKey({
      query,
      snapshot: { enabled: snapshotEnabled },
    }));
  }

  private refreshTracker(tracker: SubscriptionTracker): Promise<void> {
    if (this.subscriptionsById.get(tracker.subscription.subscriptionId) !== tracker) {
      return Promise.reject(
        new AreteError('Cannot refresh a released query lease', 'SUBSCRIPTION_NOT_FOUND')
      );
    }
    if (tracker.refreshPromise) return tracker.refreshPromise;

    const subscriptionId = tracker.subscription.subscriptionId;
    const waitForSnapshot = tracker.subscription.snapshot.enabled;
    const completion = this.queryStore.beginRefresh(subscriptionId, waitForSnapshot);
    try {
      this.connection.refresh(tracker.subscription);
    } catch (value) {
      const error = refreshError(value);
      this.queryStore.failRefresh(subscriptionId, error);
      return waitForSnapshot ? completion : Promise.reject(error);
    }

    const promise = completion;
    tracker.refreshPromise = promise;
    const clear = () => {
      if (tracker.refreshPromise === promise) tracker.refreshPromise = undefined;
    };
    void promise.then(clear, clear);
    return promise;
  }

  private releaseTracker(tracker: SubscriptionTracker): void {
    if (this.subscriptionsById.get(tracker.subscription.subscriptionId) !== tracker) return;
    tracker.refCount--;
    if (tracker.refCount > 0) return;

    this.subscriptions.delete(tracker.queryKey);
    this.subscriptionsById.delete(tracker.subscription.subscriptionId);
    try {
      this.connection.unsubscribe(tracker.subscription.subscriptionId);
    } catch {
      // ConnectionManager already removed local membership before sending.
    } finally {
      this.queryStore.unregister(tracker.subscription.subscriptionId);
    }
  }
}
