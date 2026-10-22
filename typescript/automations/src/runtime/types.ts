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
  readonly format?: ProtocolTransactionFormat;
  readonly intent?: ExecutionIntentV1;
  readonly prepared?: readonly ProtocolPreparedTransaction[];
  /** The signed transactions, saved before submit so a crash never re-signs blind. */
  readonly signed?: readonly string[];
  /** Transaction signatures once the intent has them. */
  readonly signatures?: readonly (string | null)[];
}

interface JobBase {
  /** Unique: the delivery id for a delivery job, {@link stepJobId} for a step job. */
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
 * A step job's id: `${deliveryId}:${step}:${wallet}`, then `:${instance}`
 * for a step on an instance, so one trigger can run the same step for one
 * wallet on several instances.
 */
export function stepJobId(deliveryId: string, request: Pick<StepRequest, 'step' | 'wallet' | 'instance'>): string {
  const base = `${deliveryId}:${encodeURIComponent(request.step)}:${request.wallet}`;
  return request.instance === undefined ? base : `${base}:${encodeURIComponent(request.instance)}`;
}
