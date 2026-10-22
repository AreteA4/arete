/**
 * View sources: watch a live view and dispatch a trigger when a condition
 * becomes true.
 *
 * Exactly-once comes from the store, not from ids derived from the view's
 * data (list and state views carry no cursor or version). Each source key
 * keeps its last stable value; a change is written by compare-and-set and,
 * when `changed(previous, next)` holds, its delivery
 * (`view:${source}:${key}:${generation}`) is accepted in the same step. So:
 *
 * - replicas watching one view produce one delivery per transition;
 * - after a restart the first snapshot is compared with the stored value,
 *   so a crossing during downtime fires once and nothing fires twice;
 * - every transition has its own generation, so A → B → A → B fires three
 *   times.
 *
 * A value that changed and changed back while no replica was watching is
 * not seen: list and state views hold current state, not history.
 *
 * Replicas whose views lag each other can still disagree about the order of
 * values: one stores B while another, behind, still sees A. Give the source
 * `observedAt` (for example the slot a row was updated at) so an older
 * observation never replaces a newer one, or run it on one replica
 * (`exclusive`).
 *
 * A trigger means "go and look", not the truth: handlers read live state
 * before returning steps, and every step is checked again before signing.
 */

import type { RichUpdate, Update } from '@usearete/sdk';
import { canonicalJsonSha256 } from '@usearete/sdk';

import type { Clock, Duration } from '../runtime/time';
import { durationMs } from '../runtime/time';
import type { AutomationStore } from '../store/store';

/** The parts of a list view a source reads. A `TypedListView` is one. */
export interface ListViewLike<T> {
  /** The complete current rows (the source seeds from these). */
  get(): Promise<T[]>;
  /** The rows the open subscription holds now; used to drop rows a reconnect removed. */
  getSync?(): T[] | undefined;
  watchRich(options?: { withSnapshot?: boolean }): AsyncIterable<RichUpdate<T>>;
}

/** The parts of an append view a source reads: updates with `{epoch}:{offset}` cursors. */
export interface AppendViewLike<T> {
  watch(options?: { after?: string; withSnapshot?: boolean }): AsyncIterable<Update<T>>;
}

interface SourceOptionsBase<V> {
  /** The handler name the source dispatches to. Unique per source. */
  readonly name: string;
  /**
   * Whether a change from `previous` (undefined the first time) to `next`
   * is a trigger. Defaults to every change. Put hysteresis here or in
   * `derive`.
   */
  readonly changed?: (previous: V | undefined, next: V) => boolean;
  /** A new value must hold this long before it counts. Stops one bad update from firing. */
  readonly stableFor?: Duration;
}

type Observed = number | bigint | null | undefined;

function observedOrder(value: Observed): bigint | null {
  if (value === null || value === undefined) return null;
  if (typeof value === 'number' && !Number.isSafeInteger(value)) throw new RangeError('observedAt must be an integer');
  return BigInt(value);
}

/** One signal for the whole view (for example: which market pays best). */
export interface AggregateSourceOptions<T, V> extends SourceOptionsBase<V> {
  /** A row's identity in the view (its id), so updates and removals find the row. */
  readonly key: (row: T) => string;
  /** The signal, as JSON. */
  readonly derive: (rows: readonly T[]) => V;
  /** How recent the rows are (a slot, a sequence); an older observation is ignored. */
  readonly observedAt?: (rows: readonly T[]) => Observed;
}

/** One signal per row, kept separately (for example: is this position's health low). */
export interface RowSourceOptions<T, V> extends SourceOptionsBase<V> {
  readonly key: (row: T) => string;
  /** The row's signal, as JSON. */
  readonly derive: (row: T) => V;
  /** Rows to watch; others are ignored. */
  readonly filter?: (row: T) => boolean;
  /** How recent the row is (a slot, a sequence); an older observation is ignored. */
  readonly observedAt?: (row: T) => Observed;
}

export interface ListSource<T = any, V = any> {
  readonly kind: 'list';
  readonly name: string;
  readonly view: ListViewLike<T>;
  /** A row's identity in the view. */
  readonly rowKey: (row: T) => string;
  readonly stableForMs: number;
  readonly changed: (previous: V | undefined, next: V) => boolean;
  /** The keys, values and observation orders the rows give. */
  readonly signals: (rows: readonly T[]) => Array<[string, V, bigint | null]>;
  /** The key, value and order one changed row gives, or `null` for an aggregate (re-read every row). */
  readonly rowSignal: ((row: T) => [string, V, bigint | null] | undefined) | null;
}

export interface AppendSource<T = any> {
  readonly kind: 'append';
  readonly name: string;
  readonly view: AppendViewLike<T>;
  readonly filter?: (row: T) => boolean;
}

export type ViewSource = ListSource | AppendSource;

/** The payload a list-view source dispatches. */
export interface ViewTrigger<V = unknown> {
  readonly source: string;
  /** The row key, or `all` for an aggregate source. */
  readonly key: string;
  readonly value: V;
  /** The stable value before; `undefined` the first time the key is seen. */
  readonly previous: V | undefined;
}

/** The payload an append-view source dispatches, one per row. */
export interface AppendTrigger<T = unknown> {
  readonly source: string;
  readonly cursor: string;
  readonly row: T;
}

export const AGGREGATE_KEY = 'all';
/** The stable hash of a key no value has settled on yet. */
const NO_VALUE = '';

function common<V>(options: SourceOptionsBase<V>) {
  if (!options.name) throw new TypeError('a view source needs a name');
  return {
    name: options.name,
    changed: options.changed ?? (() => true),
    stableForMs: options.stableFor === undefined ? 0 : durationMs(options.stableFor),
  };
}

/** A source with one signal for the whole list view, under the key `all`. */
export function fromView<T, V>(view: ListViewLike<T>, options: AggregateSourceOptions<T, V>): ListSource<T, V> {
  const { derive, observedAt, key } = options;
  return {
    kind: 'list',
    view,
    rowKey: key,
    ...common(options),
    signals: (rows) => [[AGGREGATE_KEY, derive(rows), observedOrder(observedAt?.(rows))]],
    rowSignal: null,
  };
}

/** A source with one signal per row of a list view, keyed by `key(row)`. */
export function fromViewRows<T, V>(view: ListViewLike<T>, options: RowSourceOptions<T, V>): ListSource<T, V> {
  const { key, derive, filter, observedAt } = options;
  const rowSignal = (row: T): [string, V, bigint | null] | undefined =>
    filter && !filter(row) ? undefined : [key(row), derive(row), observedOrder(observedAt?.(row))];
  return {
    kind: 'list',
    view,
    rowKey: key,
    ...common(options),
    signals: (rows) => rows.map(rowSignal).filter((entry): entry is [string, V, bigint | null] => entry !== undefined),
    rowSignal,
  };
}

/** A source over an append view: one trigger per row, resumed from the stored cursor. */
export function fromAppendView<T>(
  view: AppendViewLike<T>,
  options: { readonly name: string; readonly filter?: (row: T) => boolean },
): AppendSource<T> {
  if (!options.name) throw new TypeError('a view source needs a name');
  return { kind: 'append', name: options.name, view, ...(options.filter ? { filter: options.filter } : {}) };
}

export type Evaluation = 'same' | 'pending' | 'changed' | 'delivered' | 'stale' | 'contended';

/**
 * Compare a key's new value with its stored state and move it on by
 * compare-and-set, accepting a delivery when the change is a trigger.
 * Returns `pending` while `stableFor` has not elapsed; evaluate again then.
 */
export async function evaluateSignal<V>(
  store: AutomationStore,
  clock: Clock,
  source: ListSource<unknown, V>,
  key: string,
  value: V,
  observed: bigint | null = null,
): Promise<Evaluation> {
  const json = value === undefined ? null : value;
  const hash = canonicalJsonSha256(json);
  for (let attempt = 0; attempt < 5; attempt += 1) {
    const now = clock.now();
    const state = await store.sources.get(source.name, key);
    const stored = state?.observed === null || state?.observed === undefined ? null : BigInt(state.observed);
    // A lagging reader must not move the key back to an older value.
    if (observed !== null && stored !== null && observed < stored) return 'stale';
    const order = observed === null ? stored : stored === null || observed > stored ? observed : stored;
    const expectedVersion = state?.version ?? null;
    const write = (
      next: { hash: string; value: unknown; pending: { hash: string; value: unknown; since: number } | null },
      deliver?: { name: string; payload: unknown },
    ) =>
      store.sources.advance({
        source: source.name,
        key,
        expectedVersion,
        next: { ...next, observed: order === null ? null : order.toString() },
        now,
        ...(deliver ? { deliver } : {}),
      });
    if (state && state.hash === hash) {
      if (state.pending === null) return 'same';
      // The value went back to the stable one before the new one held.
      if (await write({ hash, value: state.value, pending: null })) return 'same';
      continue;
    }
    if (source.stableForMs > 0) {
      const pending = state?.pending;
      if (!pending || pending.hash !== hash) {
        const stable = state ? { hash: state.hash, value: state.value } : { hash: NO_VALUE, value: null };
        if (await write({ ...stable, pending: { hash, value: json, since: now } })) return 'pending';
        continue;
      }
      if (now - pending.since < source.stableForMs) return 'pending';
    }
    const previous = state && state.hash !== NO_VALUE ? (state.value as V) : undefined;
    const fire = source.changed(previous, value);
    const payload: ViewTrigger<V> = { source: source.name, key, value, previous };
    if (await write({ hash, value: json, pending: null }, fire ? { name: source.name, payload } : undefined)) {
      return fire ? 'delivered' : 'changed';
    }
  }
  return 'contended';
}
