import { inflate } from 'pako';

export type FrameMode = 'state' | 'append' | 'list';
export type FrameOp = 'create' | 'upsert' | 'patch' | 'delete' | 'snapshot' | 'subscribed';
export type SortOrder = 'asc' | 'desc';

export interface SortConfig {
  field: string[];
  order: SortOrder;
}

export interface SubscribedFrame {
  op: 'subscribed';
  view: string;
  mode: FrameMode;
  sort?: SortConfig;
}

export interface EntityFrame<T = unknown> {
  mode: FrameMode;
  entity: string;
  op: FrameOp;
  key: string;
  data: T;
  append?: string[];
  /** Sequence cursor for ordering and resume capability */
  seq?: string;
}

export interface SnapshotEntity<T = unknown> {
  key: string;
  data: T;
}

export interface SnapshotFrame<T = unknown> {
  mode: FrameMode;
  entity: string;
  op: 'snapshot';
  data: SnapshotEntity<T>[];
  /** Indicates if this is the final snapshot batch. When false, more batches will follow. */
  complete?: boolean;
}

export type Frame<T = unknown> = EntityFrame<T> | SnapshotFrame<T> | SubscribedFrame;

const GZIP_MAGIC_0 = 0x1f;
const GZIP_MAGIC_1 = 0x8b;

function isGzipData(data: Uint8Array): boolean {
  return data.length >= 2 && data[0] === GZIP_MAGIC_0 && data[1] === GZIP_MAGIC_1;
}

export function isSnapshotFrame<T>(frame: Frame<T>): frame is SnapshotFrame<T> {
  return frame.op === 'snapshot';
}

export function isSubscribedFrame(frame: Frame): frame is SubscribedFrame {
  return frame.op === 'subscribed';
}

export function isEntityFrame<T>(frame: Frame<T>): frame is EntityFrame<T> {
  return ['create', 'upsert', 'patch', 'delete'].includes(frame.op);
}

export function parseFrame(data: ArrayBuffer | string): Frame {
  if (typeof data === 'string') {
    return JSON.parse(data) as Frame;
  }

  const bytes = new Uint8Array(data);

  if (isGzipData(bytes)) {
    const decompressed = inflate(bytes);
    const jsonString = new TextDecoder().decode(decompressed);
    return JSON.parse(jsonString) as Frame;
  }

  const jsonString = new TextDecoder('utf-8').decode(data);
  return JSON.parse(jsonString) as Frame;
}

export async function parseFrameFromBlob(blob: Blob): Promise<Frame> {
  const arrayBuffer = await blob.arrayBuffer();
  return parseFrame(arrayBuffer);
}

export function isValidFrame(frame: unknown): frame is Frame {
  if (typeof frame !== 'object' || frame === null) {
    return false;
  }

  const f = frame as Record<string, unknown>;

  if (
    typeof f['entity'] !== 'string' ||
    typeof f['op'] !== 'string' ||
    typeof f['mode'] !== 'string' ||
    !['state', 'append', 'list'].includes(f['mode'] as string)
  ) {
    return false;
  }

  if (f['op'] === 'snapshot') {
    return Array.isArray(f['data']);
  }

  return (
    typeof f['key'] === 'string' &&
    ['create', 'upsert', 'patch', 'delete'].includes(f['op'] as string)
  );
}
