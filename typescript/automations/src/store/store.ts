import type {
  Delivery,
  DeliveryJob,
  Job,
  JobOutcome,
  JobState,
  StepJob,
  StepProgress,
  StepRequest,
} from '../runtime/types';

/** A user's consent for automations to run a step for their wallet. */
export interface ConsentGrant {
  readonly wallet: string;
  readonly deploymentId: string;
  /** `null` for every instance of the deployment. */
  readonly instanceId: string | null;
  readonly steps: readonly string[];
  /** Milliseconds since the epoch. */
  readonly expiresAt: number;
  readonly grantedAt: number;
  readonly revokedAt: number | null;
}

export interface ConsentQuery {
  readonly wallet: string;
  readonly deploymentId: string;
  readonly step: string;
  readonly instanceId: string | null;
  readonly now: number;
}

/** A step job to create, without its bookkeeping. */
export interface NewStepJob {
  readonly id: string;
  readonly deploymentId: string;
  readonly request: StepRequest;
}

export interface ClaimInput {
  readonly worker: string;
  readonly leaseMs: number;
  readonly now: number;
}

export interface TransitionInput {
  /** The worker that holds the lease. */
  readonly worker: string;
  /** The state the job must be in; the transition fails otherwise. */
  readonly from: JobState;
  readonly to: JobState;
  readonly now: number;
  /** Merged into a step job's progress. */
  readonly progress?: Partial<StepProgress>;
  readonly outcome?: JobOutcome | null;
  /** Keep the lease (for more work in the same claim) instead of releasing it. */
  readonly keepLease?: { readonly until: number };
  /** Not claimable again before this time. */
  readonly notBefore?: number;
  readonly attempts?: number;
}

/**
 * Where automations keep their state. A store must provide:
 *
 * - a unique insert for deliveries, so a trigger is accepted once;
 * - exclusive job claims with a lease;
 * - at most one active step job per wallet (`running`, `signed`,
 *   `submitted`, `submitted_unknown`);
 * - compare-and-set transitions, by state and lease holder;
 * - completing a delivery job and enqueuing its step jobs atomically.
 *
 * `runStoreConformance` from `@usearete/automations/testing` checks all of
 * these; a custom store must pass it.
 */
export interface AutomationStore {
  /**
   * Tie the store to one deployment. The first call records it; a call for
   * another deployment throws `StoreDeploymentError`. Use a separate store
   * (for Postgres, a separate schema) per deployment.
   */
  bindDeployment(deploymentId: string): Promise<void>;
  readonly deliveries: {
    /**
     * Record the delivery and queue its delivery job, atomically. `false`
     * when a delivery with this id was already accepted, or its job still
     * exists after the delivery id was pruned.
     */
    accept(delivery: Delivery, now: number): Promise<boolean>;
    /** Forget accepted delivery ids older than `before`; returns how many. */
    prune(before: number): Promise<number>;
  };
  readonly jobs: {
    get(id: string): Promise<Job | null>;
    /**
     * Claim the next job: a queued delivery job; a queued step job whose
     * wallet has no active step job (it becomes `running`); or an active job
     * whose lease has lapsed. Jobs before their `notBefore` are skipped.
     */
    claimNext(input: ClaimInput): Promise<Job | null>;
    /** Move a job from `from` to `to` if it is still there and leased by `worker`. */
    transition(id: string, input: TransitionInput): Promise<boolean>;
    /** Extend `worker`'s lease. `false` when it no longer holds it. */
    renew(id: string, worker: string, until: number): Promise<boolean>;
    /**
     * Mark a running delivery job `done` and create its step jobs, in one
     * step. Step jobs whose id exists already are left alone.
     */
    completeDelivery(id: string, worker: string, steps: readonly NewStepJob[], now: number): Promise<boolean>;
    /** When this wallet last completed this step in this deployment, or `null`. */
    lastCompleted(input: { wallet: string; deploymentId: string; step: string }): Promise<number | null>;
    list(filter?: { kind?: Job['kind']; state?: JobState }): Promise<Job[]>;
    /** Delete finished jobs last updated before `before`; returns how many. */
    prune(before: number): Promise<number>;
  };
  readonly consent: {
    put(grant: ConsentGrant): Promise<void>;
    /** The active grant covering the query, or `null`. */
    find(query: ConsentQuery): Promise<ConsentGrant | null>;
    revoke(input: { wallet: string; deploymentId: string; steps?: readonly string[]; now: number }): Promise<number>;
    list(input: { wallet: string }): Promise<ConsentGrant[]>;
  };
}

export type { DeliveryJob, StepJob };

export class StoreDeploymentError extends Error {
  constructor(bound: string, requested: string) {
    super(`this store belongs to deployment ${bound}, not ${requested}; use one store per deployment`);
    this.name = 'StoreDeploymentError';
  }
}
