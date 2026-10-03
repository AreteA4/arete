import {
  FrameProcessor,
  MemoryAdapter,
  QueryStore,
  canonicalQueryKey,
  parseFrame,
  type FrameMode,
  type FrameValidationDiagnostic,
  type QuerySnapshot,
  type ReplayWindow,
  type Schema,
  type SortConfig,
  type Subscription,
  type SubscriptionQuery,
} from '../index';

/** The subscription a frame answers: a client `subscribe` message, or its id and query. */
export interface FrameTarget {
  readonly subscriptionId: string;
  readonly query: SubscriptionQuery;
}

interface LiveFrameOptions {
  /** Defaults to `'state'` for keyed queries and `'list'` otherwise. */
  readonly mode?: FrameMode;
  /** `"<slot>:<index>"` sequence. */
  readonly seq?: string;
  /** Tape offset, for replayable append views. */
  readonly offset?: number;
}

function modeOf(target: FrameTarget, mode: FrameMode | undefined): FrameMode {
  return mode ?? (target.query.key !== undefined ? 'state' : 'list');
}

function live(
  target: FrameTarget,
  op: 'upsert' | 'patch' | 'remove' | 'delete',
  key: string,
  data: unknown,
  options: LiveFrameOptions & { readonly append?: readonly string[] } = {},
) {
  return {
    protocolVersion: 2 as const,
    subscriptionId: target.subscriptionId,
    mode: modeOf(target, options.mode),
    entity: target.query.view,
    op,
    key,
    data,
    ...(options.seq !== undefined ? { seq: options.seq } : {}),
    ...(options.offset !== undefined ? { offset: options.offset } : {}),
    ...(options.append !== undefined ? { append: [...options.append] } : {}),
  };
}

let snapshotCounter = 0;

/**
 * Builders for WebSocket protocol v2 server frames, addressed to a
 * subscription. Payloads use the wire shape (snake_case, u64 as decimal
 * strings); the SDK's generated schemas normalize them as they would live.
 */
export const frames = {
  subscribed(
    target: FrameTarget,
    options: {
      readonly mode?: FrameMode;
      readonly sort?: SortConfig;
      readonly replayWindow?: ReplayWindow;
      /**
       * Advertise the whole-entity guarantee, as current servers do (the
       * default). Pass `false` to act as an older server, whose patches for
       * keys the client does not hold are kept.
       */
      readonly wholeEntities?: boolean;
    } = {},
  ) {
    return {
      protocolVersion: 2 as const,
      subscriptionId: target.subscriptionId,
      op: 'subscribed' as const,
      query: target.query,
      mode: modeOf(target, options.mode),
      ...(options.sort ? { sort: options.sort } : {}),
      ...(options.replayWindow ? { replayWindow: options.replayWindow } : {}),
      ...(options.wholeEntities === false ? {} : { wholeEntities: true }),
    };
  },
  snapshot(
    target: FrameTarget,
    rows: readonly { readonly key: string; readonly data: unknown }[],
    options: {
      readonly mode?: FrameMode;
      readonly snapshotId?: string;
      readonly authoritative?: boolean;
      readonly complete?: boolean;
    } = {},
  ) {
    snapshotCounter += 1;
    return {
      protocolVersion: 2 as const,
      subscriptionId: target.subscriptionId,
      snapshotId: options.snapshotId ?? `snapshot-${snapshotCounter}`,
      authoritative: options.authoritative ?? true,
      mode: modeOf(target, options.mode),
      entity: target.query.view,
      op: 'snapshot' as const,
      ...(target.query.key !== undefined ? { key: target.query.key } : {}),
      data: rows.map(({ key, data }) => ({ key, data })),
      complete: options.complete ?? true,
    };
  },
  upsert(target: FrameTarget, key: string, data: unknown, options: LiveFrameOptions = {}) {
    return live(target, 'upsert', key, data, options);
  },
  patch(
    target: FrameTarget,
    key: string,
    data: unknown,
    options: LiveFrameOptions & { readonly append?: readonly string[] } = {},
  ) {
    return live(target, 'patch', key, data, options);
  },
  remove(target: FrameTarget, key: string, options: LiveFrameOptions = {}) {
    return live(target, 'remove', key, null, options);
  },
  delete(target: FrameTarget, key: string, options: LiveFrameOptions = {}) {
    return live(target, 'delete', key, null, options);
  },
  error(
    target: FrameTarget | null,
    code: string,
    options: { readonly message?: string; readonly fatal?: boolean; readonly retryable?: boolean } = {},
  ) {
    return {
      type: 'error' as const,
      protocolVersion: 2 as const,
      subscriptionId: target?.subscriptionId ?? null,
      code,
      message: options.message ?? code,
      fatal: options.fatal ?? false,
      retryable: options.retryable ?? false,
    };
  },
};

export interface FrameHarnessOptions {
  readonly maxEntriesPerView?: number | null;
  readonly schemas?: Record<string, Schema<unknown>>;
  readonly patchSchemas?: Record<string, Schema<unknown>>;
  readonly onValidationError?: (diagnostic: FrameValidationDiagnostic) => void;
}

export interface FrameHarness {
  readonly storage: MemoryAdapter;
  readonly queries: QueryStore;
  readonly processor: FrameProcessor;
  /** Register a local subscription, as a view lease would, and return it. */
  subscribe(
    subscriptionId: string,
    query: SubscriptionQuery,
    options?: { readonly snapshot?: boolean },
  ): Subscription;
  /** Process one server frame exactly as it would arrive on the wire. */
  process(frame: unknown): void;
  /** The query's current snapshot, as view verbs read it. */
  snapshot<T = unknown>(subscriptionId: string): QuerySnapshot<T> | undefined;
}

/**
 * The client store engine without a socket: feed it protocol v2 frames and
 * read query snapshots. Every frame is serialized and parsed as it would be
 * on the wire, so malformed fixtures fail the same way live frames do.
 */
export function createFrameHarness(options: FrameHarnessOptions = {}): FrameHarness {
  const storage = new MemoryAdapter();
  const queries = new QueryStore(storage);
  const processor = new FrameProcessor(storage, {
    queryStore: queries,
    ...(options.maxEntriesPerView !== undefined ? { maxEntriesPerView: options.maxEntriesPerView } : {}),
    ...(options.schemas ? { schemas: options.schemas } : {}),
    ...(options.patchSchemas ? { patchSchemas: options.patchSchemas } : {}),
    ...(options.onValidationError ? { onValidationError: options.onValidationError } : {}),
  });
  return {
    storage,
    queries,
    processor,
    subscribe(subscriptionId, query, subscribeOptions = {}) {
      const subscription: Subscription = {
        type: 'subscribe',
        protocolVersion: 2,
        subscriptionId,
        query,
        snapshot: { enabled: subscribeOptions.snapshot ?? true },
      };
      queries.register(subscription, canonicalQueryKey(subscription));
      return subscription;
    },
    process(frame) {
      processor.handleFrame(parseFrame(JSON.stringify(frame)));
    },
    snapshot<T>(subscriptionId: string) {
      return queries.getSnapshot<T>(subscriptionId);
    },
  };
}
