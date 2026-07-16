import type { Subscription, UnsubscribeFn } from './types';
import { AreteError } from './types';
import type { ConnectionManager } from './connection';

interface SubscriptionTracker {
  subscription: Subscription;
  optionsKey: string;
  refCount: number;
}

type SubKey = string;

export function normalizeSubscription(subscription: Subscription): Subscription {
  const normalized: Subscription = { view: subscription.view };

  if (subscription.key !== undefined) normalized.key = subscription.key;
  if (subscription.partition !== undefined) normalized.partition = subscription.partition;
  if (subscription.filters !== undefined) {
    normalized.filters = Object.fromEntries(
      Object.entries(subscription.filters).sort(([left], [right]) => left.localeCompare(right))
    );
  }
  if (subscription.take !== undefined) normalized.take = subscription.take;
  if (subscription.skip !== undefined) normalized.skip = subscription.skip;
  if (subscription.withSnapshot !== undefined) normalized.withSnapshot = subscription.withSnapshot;
  if (subscription.after !== undefined) normalized.after = subscription.after;
  if (subscription.snapshotLimit !== undefined) {
    normalized.snapshotLimit = subscription.snapshotLimit;
  }

  return normalized;
}

export function subscriptionIdentityKey(subscription: Subscription): SubKey {
  return JSON.stringify([subscription.view, subscription.key ?? null]);
}

export function subscriptionOptionsKey(subscription: Subscription): string {
  const normalized = normalizeSubscription(subscription);
  return JSON.stringify([
    normalized.partition ?? null,
    normalized.filters ?? null,
    normalized.take ?? null,
    normalized.skip ?? null,
    normalized.withSnapshot ?? null,
    normalized.after ?? null,
    normalized.snapshotLimit ?? null,
  ]);
}

export function incompatibleSubscriptionError(subscription: Subscription): AreteError {
  const key = subscription.key === undefined ? '*' : subscription.key;
  return new AreteError(
    `Subscription '${subscription.view}' with key '${key}' already uses incompatible options`,
    'INCOMPATIBLE_SUBSCRIPTION'
  );
}

export class SubscriptionRegistry {
  private subscriptions: Map<SubKey, SubscriptionTracker> = new Map();
  private connection: ConnectionManager;

  constructor(connection: ConnectionManager) {
    this.connection = connection;
  }

  subscribe(subscription: Subscription): UnsubscribeFn {
    const normalized = normalizeSubscription(subscription);
    const subKey = subscriptionIdentityKey(normalized);
    const optionsKey = subscriptionOptionsKey(normalized);
    const existing = this.subscriptions.get(subKey);

    if (existing) {
      if (existing.optionsKey !== optionsKey) {
        throw incompatibleSubscriptionError(normalized);
      }
      existing.refCount++;
    } else {
      try {
        this.connection.subscribe(normalized);
      } catch (error) {
        if (!(error instanceof AreteError && error.code === 'INCOMPATIBLE_SUBSCRIPTION')) {
          try {
            this.connection.unsubscribe(normalized.view, normalized.key);
          } catch {
            // Preserve the registration error; rollback is best-effort.
          }
        }
        throw error;
      }
      this.subscriptions.set(subKey, {
        subscription: normalized,
        optionsKey,
        refCount: 1,
      });
    }

    let subscribed = true;
    return () => {
      if (!subscribed) return;
      subscribed = false;
      this.unsubscribe(normalized);
    };
  }

  unsubscribe(subscription: Subscription): void {
    const normalized = normalizeSubscription(subscription);
    const subKey = subscriptionIdentityKey(normalized);
    const existing = this.subscriptions.get(subKey);

    if (existing) {
      if (existing.optionsKey !== subscriptionOptionsKey(normalized)) {
        throw incompatibleSubscriptionError(normalized);
      }
      existing.refCount--;
      if (existing.refCount <= 0) {
        this.subscriptions.delete(subKey);
        this.connection.unsubscribe(normalized.view, normalized.key);
      }
    }
  }

  refresh(subscription: Subscription): void {
    const normalized = normalizeSubscription(subscription);
    const existing = this.subscriptions.get(subscriptionIdentityKey(normalized));

    if (!existing) {
      const key = normalized.key === undefined ? '*' : normalized.key;
      throw new AreteError(
        `Cannot refresh inactive subscription '${normalized.view}' with key '${key}'`,
        'SUBSCRIPTION_NOT_FOUND'
      );
    }
    if (existing.optionsKey !== subscriptionOptionsKey(normalized)) {
      throw incompatibleSubscriptionError(normalized);
    }

    this.connection.refresh(existing.subscription);
  }

  getRefCount(subscription: Subscription): number {
    const normalized = normalizeSubscription(subscription);
    const existing = this.subscriptions.get(subscriptionIdentityKey(normalized));
    return existing?.optionsKey === subscriptionOptionsKey(normalized) ? existing.refCount : 0;
  }

  getActiveSubscriptions(): Subscription[] {
    return Array.from(this.subscriptions.values()).map((tracker) =>
      normalizeSubscription(tracker.subscription)
    );
  }

  clear(): void {
    for (const { subscription } of this.subscriptions.values()) {
      this.connection.unsubscribe(subscription.view, subscription.key);
    }
    this.subscriptions.clear();
  }
}
