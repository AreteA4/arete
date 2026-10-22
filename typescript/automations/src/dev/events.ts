/**
 * Forwards a dev environment's protocol events as signed webhook deliveries.
 *
 * Dev environments deliver no webhooks, and production delivery refuses
 * local addresses, so `a4 dev` follows every instance's event stream and
 * turns each emission into the body and headers a production webhook sends,
 * signed with the project's dev secret. Handlers receive them through
 * `acceptWebhook` (or the app's own webhook route), so signature checks and
 * duplicate handling run in development too.
 */

import { signWebhook } from '@usearete/sdk';
import type { ProtocolEventsOptions, ProtocolEventSubscription } from '@usearete/sdk';

export interface PumpClient {
  readonly deploymentId: string;
  readonly instances: {
    list(filter?: { cursor?: string; limit?: number }): Promise<{
      readonly instances: readonly { readonly id: string }[];
      readonly nextCursor: string | null;
    }>;
  };
  events(instanceId: string, options: ProtocolEventsOptions): ProtocolEventSubscription;
}

/** A delivery as a production webhook sends it. */
export interface SignedDelivery {
  readonly body: string;
  readonly headers: Record<string, string>;
}

export interface EventPumpOptions {
  readonly client: PumpClient;
  readonly secret: string;
  /** Deliver one event. Throwing retries it, in order, before the instance's later events. */
  deliver(delivery: SignedDelivery, event: { readonly event: string; readonly instanceId: string }): Promise<void>;
  /** Each instance's last forwarded event id, so a restart resumes after it. */
  readonly cursors: Map<string, string>;
  /** Events that occurred before this time (ms) are not forwarded. */
  readonly since: number;
  /** Between instance listings. Defaults to 3s. */
  readonly listIntervalMs?: number;
  onError?(error: unknown, context: { phase: string; instanceId?: string }): void;
  readonly now?: () => number;
  /** Waits between delivery retries. */
  readonly sleep?: (ms: number) => Promise<void>;
}

export interface EventPump {
  /** Instances being followed. */
  readonly following: number;
  /** List instances now and follow new ones. */
  refresh(): Promise<void>;
  stop(): void;
}

/** The body and headers of one delivery, signed with `secret`. */
export function signDelivery(secret: string, payload: Record<string, unknown>, timestamp: number): SignedDelivery {
  const body = JSON.stringify(payload);
  return {
    body,
    headers: {
      'content-type': 'application/json',
      'arete-webhook-id': String(payload.id),
      'arete-webhook-timestamp': String(timestamp),
      'arete-webhook-signature': `v1=${signWebhook(secret, timestamp, body)}`,
    },
  };
}

export function startEventPump(options: EventPumpOptions): EventPump {
  const { client, cursors } = options;
  const now = options.now ?? Date.now;
  const sleep = options.sleep ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)));
  const subscriptions = new Map<string, ProtocolEventSubscription>();
  const report = (error: unknown, context: { phase: string; instanceId?: string }) => {
    try {
      options.onError?.(error, context);
    } catch {
      // A failing error hook must not stop the pump.
    }
  };
  let stopped = false;

  const follow = (instanceId: string) => {
    let queue = Promise.resolve();
    const forward = async (cursor: string, payload: Record<string, unknown>, event: string) => {
      for (let failures = 0; !stopped; failures += 1) {
        try {
          // Signed at each attempt, so a retry is inside the timestamp tolerance.
          await options.deliver(signDelivery(options.secret, payload, Math.floor(now() / 1000)), { event, instanceId });
          cursors.set(instanceId, cursor);
          return;
        } catch (error) {
          report(error, { phase: 'deliver', instanceId });
          await sleep(Math.min(1000 * 2 ** failures, 30_000));
        }
      }
    };
    const lastEventId = cursors.get(instanceId);
    const subscription = client.events(instanceId, {
      ...(lastEventId !== undefined ? { lastEventId } : {}),
      onEvent(streamEvent) {
        if (streamEvent.type !== 'emit') return;
        const data = streamEvent.data;
        const occurred = Date.parse(data.at);
        if (Number.isFinite(occurred) && occurred < options.since) {
          queue = queue.then(() => void cursors.set(instanceId, streamEvent.id));
          return;
        }
        // The payload of a production delivery. The stream does not carry the
        // machine's state at the emission, so `state` is absent: handlers
        // read current state rather than trust the trigger.
        const payload = {
          id: String(data.id),
          event: streamEvent.event,
          deploymentId: client.deploymentId,
          instanceId: data.instanceId,
          participantId: data.participantId,
          occurredAt: data.at,
        };
        queue = queue.then(() => forward(streamEvent.id, payload, streamEvent.event));
      },
      onError(error) {
        report(error, { phase: 'stream', instanceId });
      },
    });
    subscriptions.set(instanceId, subscription);
  };

  const refresh = async () => {
    let cursor: string | undefined;
    do {
      const page = await client.instances.list({ limit: 100, ...(cursor !== undefined ? { cursor } : {}) });
      for (const instance of page.instances) {
        if (!stopped && !subscriptions.has(instance.id)) follow(instance.id);
      }
      cursor = page.nextCursor ?? undefined;
    } while (cursor !== undefined && !stopped);
  };

  const interval = options.listIntervalMs ?? 3000;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const list = async () => {
    try {
      await refresh();
    } catch (error) {
      report(error, { phase: 'list' });
    }
    if (!stopped) timer = setTimeout(list, interval);
  };
  void list();

  return {
    get following() {
      return subscriptions.size;
    },
    refresh,
    stop() {
      stopped = true;
      clearTimeout(timer);
      for (const subscription of subscriptions.values()) subscription.close();
      subscriptions.clear();
    },
  };
}
