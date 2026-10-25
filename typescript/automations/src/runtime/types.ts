import { sha256 } from '@noble/hashes/sha2';
import { bytesToHex, utf8ToBytes } from '@noble/hashes/utils';
import type { ExecutionIntentV1, ProtocolPreparedTransaction, ProtocolTransactionFormat } from '@usearete/sdk';

/** Where a trigger came from. */
export type TriggerSource = 'webhook' | 'stream' | 'dispatch' | 'view';

/** One accepted trigger: a webhook delivery, a stream event or a dispatch. */
export interface Delivery {
  /** Unique per trigger, namespaced by source (`webhook:…`, `stream:…`, `dispatch:…`, `view:…`). */
  readonly id: string;
  /** The handler name: an event name or a dispatch name. */
  readonly name: string;
  readonly source: TriggerSource;
  readonly payload: unknown;
  /** Milliseconds since the epoch. */
  readonly receivedAt: number;
}

/** A step a handler asks for. */
export interface StepRequest {
  readonly step: string;
  readonly wallet: string;
  /** The instance, for steps that act on one; omitted for create steps. */
  readonly instance?: string;
  readonly args: Readonly<Record<string, unknown>>;
  /** Skip the step when the same (wallet, step) completed this recently, in milliseconds. */
  readonly cooldownMs?: number;
}

/** Delivery jobs run a handler; they never sign. */
export type DeliveryJobState = 'queued' | 'running' | 'done' | 'failed';

/**
 * Step jobs run the signing pipeline. `running`, `signed`, `submitted` and
 * `submitted_unknown` are active: while one is, no other step job of the
 * same wallet may start.
 */
export type StepJobState =
  | 'queued'
  | 'running'
  | 'signed'
  | 'submitted'
  | 'submitted_unknown'
  | 'completed'
  | 'chain_failed'
  | 'expired'
  | 'skipped'
  | 'rejected'
  | 'refused'
  | 'failed';

export type JobState = DeliveryJobState | StepJobState;

export const ACTIVE_STEP_STATES: readonly StepJobState[] = [
  'running',
  'signed',
  'submitted',
  'submitted_unknown',
];

export const TERMINAL_STEP_STATES: readonly StepJobState[] = [
  'completed',
  'chain_failed',
  'expired',
  'skipped',
  'rejected',
  'refused',
  'failed',
];

export const TERMINAL_DELIVERY_STATES: readonly DeliveryJobState[] = ['done', 'failed'];

/** Why a job ended where it did. */
export interface JobOutcome {
  /** A refusal rule, a guard or rejection code, `no_consent`, `cooldown`, or an error class. */
  readonly code: string;
  readonly reason?: string;
  readonly detail?: unknown;
}

/** What a step job carries once it has prepared and signed. */
export interface StepProgress {
  /** Increments when an expired intent is prepared again. */
  readonly generation: number;
  /**
   * The args the step was prepared with, as the API received them (integers
   * as decimal strings). Saved when the step is prepared.
   */
  readonly args?: Readonly<Record<string, unknown>>;
  readonly format?: ProtocolTransactionFormat;
  readonly intent?: ExecutionIntentV1;
  readonly prepared?: readonly ProtocolPreparedTransaction[];
  /** The signed transactions, saved before submit so a crash never re-signs blind. */
  readonly signed?: readonly string[];
  /** Transaction signatures once the intent has them. */
  readonly signatures?: readonly (string | null)[];
  /** When the step was signed: its spending counts against windows from then. */
  readonly signedAt?: number;
  /** What each transaction moves out of the wallet (decimal amounts per asset), saved at signing. */
  readonly outflows?: readonly { readonly index: number; readonly amounts: Readonly<Record<string, string>> }[];
}

interface JobBase {
  /** Unique across both kinds: the delivery id for a delivery job, {@link stepJobId} for a step job. */
  readonly id: string;
  readonly attempts: number;
  /** Not claimable before this time (milliseconds since the epoch). */
  readonly notBefore: number;
  readonly lease: { readonly worker: string; readonly until: number } | null;
  readonly createdAt: number;
  readonly updatedAt: number;
  readonly outcome: JobOutcome | null;
}

export interface DeliveryJob extends JobBase {
  readonly kind: 'delivery';
  readonly state: DeliveryJobState;
  readonly delivery: Delivery;
}

export interface StepJob extends JobBase {
  readonly kind: 'step';
  readonly state: StepJobState;
  readonly deliveryId: string;
  readonly deploymentId: string;
  readonly request: StepRequest;
  readonly progress: StepProgress;
}

export type Job = DeliveryJob | StepJob;

/**
 * A step job's id: `step:${deliveryId}:${step}:${wallet}`, then
 * `:${instance}` for a step on an instance, so one trigger can run the same
 * step for one wallet on several instances. The `step:` namespace and the
 * encoded components keep step job ids apart from delivery ids (which start
 * `webhook:`, `stream:`, `dispatch:` or `view:`) and from each other.
 */
export function stepJobId(deliveryId: string, request: Pick<StepRequest, 'step' | 'wallet' | 'instance'>): string {
  const base = `step:${encodeURIComponent(deliveryId)}:${encodeURIComponent(request.step)}:${request.wallet}`;
  return request.instance === undefined ? base : `${base}:${encodeURIComponent(request.instance)}`;
}

/**
 * The idempotency key a step job's prepare sends for one generation. The API
 * accepts `[A-Za-z0-9._:-]{1,128}`, and a job id is percent-encoded and can be
 * longer (a webhook delivery, a wallet and an instance id), so the key
 * carries the id's SHA-256 instead. It is deterministic, so a retry of the
 * same generation gets the same intent back.
 */
export function prepareIdempotencyKey(jobId: string, generation: number): string {
  return `step:${bytesToHex(sha256(utf8ToBytes(jobId)))}:${generation}`;
}

/**
 * Step args as the protocol API receives them: a JSON copy with bigints as
 * decimal strings, as the protocol client sends them. `undefined` fields are
 * dropped.
 */
export function preparedArgs(args: Readonly<Record<string, unknown>>): Record<string, unknown> {
  return JSON.parse(
    JSON.stringify(args, (_key, item: unknown) => (typeof item === 'bigint' ? item.toString(10) : item)),
  ) as Record<string, unknown>;
}
