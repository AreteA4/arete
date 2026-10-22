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

/** Spending limits of a consent grant, per asset: `SOL` (in lamports) or a mint (in base units). */
export interface StoredConsentLimits {
  /** The most one step may move out of the wallet. An asset without one may not leave. */
  readonly perExecution: Readonly<Record<string, string>>;
  /** The most all steps together may move out within a rolling window. */
  readonly perWindow: Readonly<Record<string, { readonly amount: string; readonly windowMs: number }>>;
}

/** The signed proof a grant was made from. */
export interface ConsentProof {
  /** The exact message the wallet signed. */
  readonly message: string;
  /** Base58 Ed25519 signature by the wallet. */
  readonly signature: string;
  readonly nonce: string;
  readonly issuedAt: number;
}

/** A user's consent for automations to run steps for their wallet. */
export interface ConsentGrant {
  readonly wallet: string;
  readonly deploymentId: string;
  /** `null` for every instance of the deployment. */
  readonly instanceId: string | null;
  readonly steps: readonly string[];
  readonly limits: StoredConsentLimits;
  /** The user's own settings for the steps (amount, weekday, ...). */
  readonly params: Readonly<Record<string, unknown>>;
  /** Milliseconds since the epoch. */
  readonly expiresAt: number;
  readonly grantedAt: number;
  readonly revokedAt: number | null;
  readonly proof: ConsentProof;
}

/** What one step moved, counted against a grant's windows. */
export interface LedgerEntry {
  readonly jobId: string;
  readonly wallet: string;
  readonly deploymentId: string;
  /** `SOL` or a mint. */
  readonly asset: string;
  /** Lamports or base units, as a decimal string. */
  readonly amount: string;
  readonly recordedAt: number;
  /** Set when the step's intent never landed, so it no longer counts. */
  readonly voidedAt: number | null;
}

export interface ConsentQuery {
  readonly wallet: string;
  readonly deploymentId: string;
  readonly step: string;
  readonly instanceId: string | null;
  readonly now: number;
}

/** What a view source last saw for one key. */
export interface SourceState {
  readonly source: string;
  readonly key: string;
  /** Canonical-JSON SHA-256 of the stable value. */
  readonly hash: string;
  /** The stable derived value (JSON). */
  readonly value: unknown;
  /** Increments with every stable change; delivery ids carry it. */
  readonly generation: number;
  /** A new value waiting out `stableFor`, if any. */
  readonly pending: { readonly hash: string; readonly value: unknown; readonly since: number } | null;
  /** Increments with every write; compare-and-set reads it. */
  readonly version: number;
  readonly updatedAt: number;
}

export interface SourceAdvance {
  readonly source: string;
  readonly key: string;
  /** The version read; `null` when there was no state yet. The write fails if it moved. */
  readonly expectedVersion: number | null;
  readonly next: Pick<SourceState, 'hash' | 'value' | 'pending'>;
  /**
   * Accept a delivery in the same atomic step, with id
   * `view:${source}:${key}:${generation}` (the generation after the write).
   */
  readonly deliver?: { readonly name: string; readonly payload: unknown };
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
  /**
   * Ledger changes made in the same atomic step as the transition:
   * `record` adds entries for this job, `void` voids its entries except
   * those for `keepAssets`.
   */
  readonly ledger?: {
    readonly record?: readonly Omit<LedgerEntry, 'jobId' | 'recordedAt' | 'voidedAt'>[];
    readonly void?: boolean;
    readonly keepAssets?: readonly string[];
  };
}

/**
 * Where automations keep their state. A store must provide:
 *
 * - a unique insert for deliveries, so a trigger is accepted once;
 * - exclusive job claims with a lease;
 * - at most one active step job per wallet (`running`, `signed`,
 *   `submitted`, `submitted_unknown`);
 * - compare-and-set transitions, by state and lease holder;
 * - completing a delivery job and enqueuing its step jobs atomically;
 * - ledger writes atomic with the job transition they belong to;
 * - single-use consent nonces;
 * - compare-and-set on view-source state, with its delivery in the same step.
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
    /** Store a grant, replacing the wallet's grant for the same deployment and instance. */
    put(grant: ConsentGrant): Promise<void>;
    /** The active grant covering the query, or `null`. */
    find(query: ConsentQuery): Promise<ConsentGrant | null>;
    /** Revoke grants (or only some of their steps). Returns how many grants changed. */
    revoke(input: {
      wallet: string;
      deploymentId: string;
      steps?: readonly string[];
      /** Only the grant scoped to this instance. */
      instanceId?: string;
      now: number;
    }): Promise<number>;
    /** Every grant of a wallet, active or not. */
    list(input: { wallet: string }): Promise<ConsentGrant[]>;
    /** Active grants in a deployment that cover `step`, for fan-out. */
    active(input: { deploymentId: string; step?: string; now: number }): Promise<ConsentGrant[]>;
    /** Remember a nonce handed out with a proof message. */
    issueNonce(nonce: string, expiresAt: number): Promise<void>;
    /** Use an issued, unexpired nonce once. `false` when it was not issued, expired or used. */
    useNonce(nonce: string, now: number): Promise<boolean>;
  };
  readonly sources: {
    get(source: string, key: string): Promise<SourceState | null>;
    /**
     * Write a key's state if its version is still `expectedVersion`. The
     * generation increments when the stable hash changes. Returns the new
     * state, or `null` when another writer got there first.
     */
    advance(input: SourceAdvance): Promise<SourceState | null>;
    /** The last cursor an append-view source recorded. */
    cursor(source: string): Promise<string | null>;
    putCursor(source: string, cursor: string, now: number): Promise<void>;
    /** Hold a source exclusively until `until`. `false` while another holder's lease runs. */
    acquireLease(source: string, holder: string, until: number, now: number): Promise<boolean>;
    releaseLease(source: string, holder: string): Promise<void>;
  };
  readonly ledger: {
    /** The sum of unvoided entries for the wallet and asset recorded at or after `since`. */
    spent(input: { wallet: string; deploymentId: string; asset: string; since: number }): Promise<bigint>;
    list(input: { wallet: string }): Promise<LedgerEntry[]>;
  };
}

export type { DeliveryJob, StepJob };

export class StoreDeploymentError extends Error {
  constructor(bound: string, requested: string) {
    super(`this store belongs to deployment ${bound}, not ${requested}; use one store per deployment`);
    this.name = 'StoreDeploymentError';
  }
}
