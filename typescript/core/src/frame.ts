import { inflate } from 'pako';
import type { SubscriptionQuery } from './types';

export type FrameMode = 'state' | 'append' | 'list';
export type FrameOp =
  | 'subscribed'
  | 'unsubscribed'
  | 'snapshot'
  | 'upsert'
  | 'patch'
  | 'remove'
  | 'delete';
export type SortOrder = 'asc' | 'desc';

export interface SortConfig {
  field: string[];
  order: SortOrder;
}

interface IdentifiedFrame {
  protocolVersion: 2;
  subscriptionId: string;
}

/**
 * Offsets an append view can still replay. Offsets restart whenever a tape is
 * rebuilt, so they only mean anything inside `epoch`.
 */
export interface ReplayWindow {
  epoch: string;
  /** Oldest retained offset. */
  earliest: number;
  /** Offset the next event will take; a consumer at `next - 1` is caught up. */
  next: number;
  /** Last offset before a known discontinuity. */
  gapAfter?: number;
}

export interface SubscribedFrame extends IdentifiedFrame {
  op: 'subscribed';
  query: SubscriptionQuery;
  mode: FrameMode;
  sort?: SortConfig;
  /** Present on append views backed by the journal. */
  replayWindow?: ReplayWindow;
}

export interface UnsubscribedFrame extends IdentifiedFrame {
  op: 'unsubscribed';
}

export interface EntityFrame<T = unknown> extends IdentifiedFrame {
  mode: FrameMode;
  entity: string;
  op: 'upsert' | 'patch' | 'remove' | 'delete';
  key: string;
  data: T | null;
  append?: string[];
  seq?: string;
  /** Dense, monotonic position on the view's tape. Absent on state/list views. */
  offset?: number;
}

export interface SnapshotEntity<T = unknown> {
  key: string;
  data: T;
}

export interface SnapshotFrame<T = unknown> extends IdentifiedFrame {
  snapshotId: string;
  authoritative: boolean;
  mode: FrameMode;
  entity: string;
  op: 'snapshot';
  key?: string;
  data: SnapshotEntity<T>[];
  complete: boolean;
}

export interface ErrorFrame {
  type: 'error';
  protocolVersion: 2;
  subscriptionId: string | null;
  error?: string;
  message?: string;
  code: string;
  retryable?: boolean;
  fatal: boolean;
  /** Seconds to wait before retrying, as the server sends it. */
  retryAfter?: number;
  suggestedAction?: string;
  docsUrl?: string;
  /** @deprecated The server sends `retryAfter`; kept for older servers. */
  retry_after?: number;
  /** @deprecated The server sends `suggestedAction`; kept for older servers. */
  suggested_action?: string;
  /** @deprecated The server sends `docsUrl`; kept for older servers. */
  docs_url?: string;
  /** Present on cursor refusals: what the view can still serve. */
  replayWindow?: ReplayWindow;
  /** Present on `replay-lagged`: cursor of the last record delivered before the gap. */
  recoverFrom?: string;
}

export type Frame<T = unknown> =
  | EntityFrame<T>
  | SnapshotFrame<T>
  | SubscribedFrame
  | UnsubscribedFrame
  | ErrorFrame;

const GZIP_MAGIC_0 = 0x1f;
const GZIP_MAGIC_1 = 0x8b;
const FRAME_MODES = new Set(['state', 'append', 'list']);
const LIVE_OPS = new Set(['upsert', 'patch', 'remove', 'delete']);
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function isGzipData(data: Uint8Array): boolean {
  return data.length >= 2 && data[0] === GZIP_MAGIC_0 && data[1] === GZIP_MAGIC_1;
}

function isSubscriptionId(value: unknown): value is string {
  if (typeof value !== 'string' || value.length === 0 || value.trim() !== value) return false;
  if (/\p{Cc}/u.test(value)) return false;
  return new TextEncoder().encode(value).byteLength <= 128;
}

function isMode(value: unknown): value is FrameMode {
  return typeof value === 'string' && FRAME_MODES.has(value);
}

function isOffset(value: unknown): value is number {
  return Number.isInteger(value) && (value as number) >= 0;
}

function isReplayWindow(value: unknown): value is ReplayWindow {
  return isRecord(value)
    && typeof value['epoch'] === 'string'
    && value['epoch'].length > 0
    && isOffset(value['earliest'])
    && isOffset(value['next'])
    && (value['gapAfter'] === undefined || isOffset(value['gapAfter']));
}

function isSort(value: unknown): value is SortConfig {
  if (!isRecord(value)) return false;
  return Array.isArray(value['field'])
    && value['field'].every((entry) => typeof entry === 'string')
    && (value['order'] === 'asc' || value['order'] === 'desc');
}

function isQuery(value: unknown): value is SubscriptionQuery {
  if (!isRecord(value) || typeof value['view'] !== 'string' || value['view'].length === 0) {
    return false;
  }
  if (Object.keys(value).some((key) => !QUERY_FIELDS.has(key))) return false;
  if (value['key'] !== undefined && typeof value['key'] !== 'string') return false;
  if (value['partition'] !== undefined && typeof value['partition'] !== 'string') return false;
  if (value['filters'] !== undefined && !isRecord(value['filters'])) return false;
  if (value['take'] !== undefined && (!Number.isInteger(value['take']) || Number(value['take']) <= 0)) {
    return false;
  }
  if (value['skip'] !== undefined && (!Number.isInteger(value['skip']) || Number(value['skip']) < 0)) {
    return false;
  }
  if (value['after'] !== undefined && typeof value['after'] !== 'string') return false;
  return value['snapshotLimit'] === undefined
    || (Number.isInteger(value['snapshotLimit']) && Number(value['snapshotLimit']) > 0);
}

function hasV2Identity(value: Record<string, unknown>): boolean {
  return value['protocolVersion'] === 2 && isSubscriptionId(value['subscriptionId']);
}

export function isSnapshotFrame<T>(frame: Frame<T>): frame is SnapshotFrame<T> {
  return 'op' in frame && frame.op === 'snapshot';
}

export function isSubscribedFrame(frame: Frame): frame is SubscribedFrame {
  return 'op' in frame && frame.op === 'subscribed';
}

export function isUnsubscribedFrame(frame: Frame): frame is UnsubscribedFrame {
  return 'op' in frame && frame.op === 'unsubscribed';
}

export function isErrorFrame(frame: Frame): frame is ErrorFrame {
  return 'type' in frame && frame.type === 'error';
}

export function isEntityFrame<T>(frame: Frame<T>): frame is EntityFrame<T> {
  return 'op' in frame && LIVE_OPS.has(frame.op);
}

function decodeFrame(data: ArrayBuffer | string): unknown {
  if (typeof data === 'string') return JSON.parse(data) as unknown;
  const bytes = new Uint8Array(data);
  const decoded = isGzipData(bytes) ? inflate(bytes) : bytes;
  return JSON.parse(new TextDecoder('utf-8').decode(decoded)) as unknown;
}

export function parseFrame(data: ArrayBuffer | string): Frame {
  const frame = decodeFrame(data);
  if (!isValidFrame(frame)) {
    throw new Error('Invalid WebSocket protocol v2 frame');
  }
  return frame;
}

export async function parseFrameFromBlob(blob: Blob): Promise<Frame> {
  return parseFrame(await blob.arrayBuffer());
}

export function isValidFrame(frame: unknown): frame is Frame {
  if (!isRecord(frame) || frame['protocolVersion'] !== 2) return false;

  if (frame['type'] === 'error') {
    return (frame['subscriptionId'] === null || isSubscriptionId(frame['subscriptionId']))
      && typeof frame['code'] === 'string'
      && typeof frame['fatal'] === 'boolean'
      && (frame['message'] === undefined || typeof frame['message'] === 'string')
      && (frame['error'] === undefined || typeof frame['error'] === 'string')
      && (frame['retryable'] === undefined || typeof frame['retryable'] === 'boolean')
      && (frame['replayWindow'] === undefined || isReplayWindow(frame['replayWindow']))
      && (frame['recoverFrom'] === undefined || typeof frame['recoverFrom'] === 'string');
  }

  if (!hasV2Identity(frame) || typeof frame['op'] !== 'string') return false;
  if (frame['op'] === 'unsubscribed') return true;
  if (frame['op'] === 'subscribed') {
    return isQuery(frame['query'])
      && isMode(frame['mode'])
      && (frame['sort'] === undefined || isSort(frame['sort']))
      && (frame['replayWindow'] === undefined || isReplayWindow(frame['replayWindow']));
  }
  if (!isMode(frame['mode']) || typeof frame['entity'] !== 'string') return false;
  if (frame['op'] === 'snapshot') {
    return typeof frame['snapshotId'] === 'string'
      && frame['snapshotId'].length > 0
      && typeof frame['authoritative'] === 'boolean'
      && typeof frame['complete'] === 'boolean'
      && (frame['key'] === undefined || typeof frame['key'] === 'string')
      && Array.isArray(frame['data'])
      && frame['data'].every((entry) =>
        isRecord(entry) && typeof entry['key'] === 'string' && 'data' in entry
      );
  }
  return LIVE_OPS.has(frame['op'])
    && typeof frame['key'] === 'string'
    && 'data' in frame
    && (frame['seq'] === undefined || typeof frame['seq'] === 'string')
    && (frame['offset'] === undefined || isOffset(frame['offset']))
    && (frame['append'] === undefined
      || (Array.isArray(frame['append'])
        && frame['append'].every((entry) => typeof entry === 'string')));
}

/**
 * Builds the `{epoch}:{offset}` cursor a consumer stores and sends back as
 * `query.after`. An offset only means something inside the epoch the
 * acknowledgement named, so the epoch has to be carried across frames.
 */
export class CursorTracker {
  private readonly epochs = new Map<string, string>();
  private readonly cursors = new Map<string, string>();

  /** Records the frame and returns the cursor it carries, if any. */
  observe(frame: Frame): string | undefined {
    if (isSubscribedFrame(frame)) {
      const epoch = frame.replayWindow?.epoch;
      if (epoch === undefined) this.epochs.delete(frame.subscriptionId);
      else this.epochs.set(frame.subscriptionId, epoch);
      return undefined;
    }
    if (!isEntityFrame(frame) || frame.offset === undefined) return undefined;
    const epoch = this.epochs.get(frame.subscriptionId);
    if (epoch === undefined) return undefined;
    const cursor = `${epoch}:${frame.offset}`;
    this.cursors.set(frame.subscriptionId, cursor);
    return cursor;
  }

  /** The last cursor delivered on a subscription, for resuming after a drop. */
  last(subscriptionId: string): string | undefined {
    return this.cursors.get(subscriptionId);
  }

  forget(subscriptionId: string): void {
    this.epochs.delete(subscriptionId);
    this.cursors.delete(subscriptionId);
  }
}
