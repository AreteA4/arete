/**
 * An in-memory store for tests and single-process development. State is
 * lost on restart, so it must not back automations that sign real funds.
 * Each method runs to completion without awaiting, which makes every
 * operation atomic under JavaScript's single-threaded event loop.
 */

import {
  ACTIVE_STEP_STATES,
  type Delivery,
  type Job,
  type StepJob,
  TERMINAL_DELIVERY_STATES,
  TERMINAL_STEP_STATES,
} from '../runtime/types';
import { type AutomationStore, type ConsentGrant, type LedgerEntry, type NewStepJob, type SourceState, StoreDeploymentError, type TransitionInput, cursorAfter } from './store';

function isActiveStep(job: Job): boolean {
  return job.kind === 'step' && (ACTIVE_STEP_STATES as readonly string[]).includes(job.state);
}

function isFinished(job: Job): boolean {
  return job.kind === 'step'
    ? (TERMINAL_STEP_STATES as readonly string[]).includes(job.state)
    : (TERMINAL_DELIVERY_STATES as readonly string[]).includes(job.state);
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

/**
 * A memory store's contents, for `a4 dev` to keep across restarts. Opaque:
 * write it with `v8.serialize` and read it back with `v8.deserialize`.
 */
export interface MemoryStoreState {
  readonly version: 1;
  readonly deliveries: Map<string, number>;
  readonly jobs: Map<string, Job>;
  readonly order: number;
  readonly sequence: Map<string, number>;
  readonly grants: ConsentGrant[];
  readonly deployment: string | null;
  readonly nonces: Map<string, { expiresAt: number; used: boolean }>;
  readonly ledger: LedgerEntry[];
  readonly sourceStates: Map<string, SourceState>;
  readonly cursors: Map<string, string>;
}

const STATES = new WeakMap<AutomationStore, () => MemoryStoreState>();

/** A copy of a memory store's contents (source leases are not kept). */
export function memoryStoreState(store: AutomationStore): MemoryStoreState {
  const state = STATES.get(store);
  if (!state) throw new TypeError('not a memory store');
  return state();
}

export function memoryStore(options: { state?: MemoryStoreState } = {}): AutomationStore {
  const initial = options.state ? clone(options.state) : null;
  if (initial && initial.version !== 1) throw new TypeError('unknown memory store state');
  const deliveries = initial?.deliveries ?? new Map<string, number>();
  const jobs = initial?.jobs ?? new Map<string, Job>();
  let order = initial?.order ?? 0;
  const sequence = initial?.sequence ?? new Map<string, number>();
  const grants: ConsentGrant[] = initial?.grants ?? [];
  let deployment: string | null = initial?.deployment ?? null;
  const nonces = initial?.nonces ?? new Map<string, { expiresAt: number; used: boolean }>();
  const ledger: LedgerEntry[] = initial?.ledger ?? [];
  const sourceStates = initial?.sourceStates ?? new Map<string, SourceState>();
  const cursors = initial?.cursors ?? new Map<string, string>();
  const sourceLeases = new Map<string, { holder: string; until: number }>();
  // A restored state's workers stopped with the process that saved it, so
  // their claims are released rather than left to expire.
  for (const [id, job] of jobs) if (job.lease) jobs.set(id, { ...job, lease: null });

  const insert = (job: Job) => {
    jobs.set(job.id, job);
    sequence.set(job.id, order++);
  };

  const acceptNow = (delivery: Delivery, now: number) => {
    if (deliveries.has(delivery.id)) return false;
    // Clone first: a value that cannot be stored leaves the store unchanged.
    const copy = clone(delivery);
    deliveries.set(delivery.id, now);
    if (jobs.has(delivery.id)) return false;
    insert({
      kind: 'delivery',
      id: delivery.id,
      state: 'queued',
      delivery: copy,
      attempts: 0,
      notBefore: now,
      lease: null,
      createdAt: now,
      updatedAt: now,
      outcome: null,
    });
    return true;
  };

  const walletBusy = (wallet: string, except: string) => {
    for (const job of jobs.values()) {
      if (job.id !== except && job.kind === 'step' && job.request.wallet === wallet && isActiveStep(job)) {
        return true;
      }
    }
    return false;
  };

  const claimable = (job: Job, now: number): boolean => {
    if (job.notBefore > now) return false;
    const leased = job.lease !== null && job.lease.until > now;
    if (leased) return false;
    if (job.kind === 'delivery') return job.state === 'queued' || job.state === 'running';
    if (job.state === 'queued') return !walletBusy(job.request.wallet, job.id);
    return isActiveStep(job);
  };

  const store: AutomationStore = {
    async bindDeployment(deploymentId) {
      if (deployment !== null && deployment !== deploymentId) throw new StoreDeploymentError(deployment, deploymentId);
      deployment = deploymentId;
    },
    deliveries: {
      async accept(delivery: Delivery, now: number) {
        return acceptNow(delivery, now);
      },
      async prune(before: number) {
        let removed = 0;
        for (const [id, at] of deliveries) {
          if (at < before) {
            deliveries.delete(id);
            removed += 1;
          }
        }
        return removed;
      },
    },
    jobs: {
      async get(id) {
        const job = jobs.get(id);
        return job ? clone(job) : null;
      },
      async claimNext({ worker, leaseMs, now }) {
        const candidates = [...jobs.values()]
          .filter((job) => claimable(job, now))
          .sort((a, b) => a.notBefore - b.notBefore || sequence.get(a.id)! - sequence.get(b.id)!);
        const job = candidates[0];
        if (!job) return null;
        const lease = { worker, until: now + leaseMs };
        const claimed: Job =
          job.state === 'queued'
            ? ({ ...job, state: 'running', lease, updatedAt: now } as Job)
            : ({ ...job, lease, updatedAt: now } as Job);
        jobs.set(job.id, claimed);
        return clone(claimed);
      },
      async transition(id, input: TransitionInput) {
        const job = jobs.get(id);
        if (!job || job.state !== input.from) return false;
        if (job.lease?.worker !== input.worker) return false;
        const next = {
          ...job,
          state: input.to,
          updatedAt: input.now,
          lease: input.keepLease ? { worker: input.worker, until: input.keepLease.until } : null,
          ...(input.notBefore !== undefined ? { notBefore: input.notBefore } : {}),
          ...(input.attempts !== undefined ? { attempts: input.attempts } : {}),
          ...(input.outcome !== undefined ? { outcome: clone(input.outcome) } : {}),
          ...(job.kind === 'step' && input.progress
            ? { progress: { ...job.progress, ...clone(input.progress) } }
            : {}),
        } as Job;
        if (next.kind === 'step' && isActiveStep(next) && walletBusy(next.request.wallet, id)) {
          return false;
        }
        jobs.set(id, next);
        if (input.ledger?.void) {
          for (const [index, entry] of ledger.entries()) {
            if (entry.jobId === id && entry.voidedAt === null && !input.ledger.keepAssets?.includes(entry.asset)) {
              ledger[index] = { ...entry, voidedAt: input.now };
            }
          }
        }
        for (const entry of input.ledger?.record ?? []) {
          ledger.push({ ...clone(entry), jobId: id, recordedAt: entry.recordedAt ?? input.now, voidedAt: null });
        }
        return true;
      },
      async renew(id, worker, until) {
        const job = jobs.get(id);
        if (!job || job.lease?.worker !== worker) return false;
        jobs.set(id, { ...job, lease: { worker, until } } as Job);
        return true;
      },
      async completeDelivery(id, worker, steps: readonly NewStepJob[], now) {
        const job = jobs.get(id);
        if (!job || job.kind !== 'delivery' || job.state !== 'running' || job.lease?.worker !== worker) {
          return false;
        }
        const requests = steps.map((step) => clone(step.request));
        jobs.set(id, { ...job, state: 'done', lease: null, updatedAt: now, outcome: null });
        for (const [index, step] of steps.entries()) {
          if (jobs.has(step.id)) continue;
          const created: StepJob = {
            kind: 'step',
            id: step.id,
            state: 'queued',
            deliveryId: id,
            deploymentId: step.deploymentId,
            request: requests[index]!,
            progress: { generation: 0 },
            attempts: 0,
            notBefore: now,
            lease: null,
            createdAt: now,
            updatedAt: now,
            outcome: null,
          };
          insert(created);
        }
        return true;
      },
      async lastCompleted({ wallet, deploymentId, step }) {
        let last: number | null = null;
        for (const job of jobs.values()) {
          if (
            job.kind === 'step' &&
            job.state === 'completed' &&
            job.request.wallet === wallet &&
            job.request.step === step &&
            job.deploymentId === deploymentId
          ) {
            last = last === null ? job.updatedAt : Math.max(last, job.updatedAt);
          }
        }
        return last;
      },
      async list(filter = {}) {
        return [...jobs.values()]
          .filter((job) => (filter.kind ? job.kind === filter.kind : true))
          .filter((job) => (filter.state ? job.state === filter.state : true))
          .filter((job) => (filter.wallet !== undefined ? job.kind === 'step' && job.request.wallet === filter.wallet : true))
          .filter((job) => (filter.finished !== undefined ? isFinished(job) === filter.finished : true))
          .sort((a, b) => sequence.get(a.id)! - sequence.get(b.id)!)
          .map(clone);
      },
      async prune(before) {
        let removed = 0;
        for (const job of [...jobs.values()]) {
          if (isFinished(job) && job.updatedAt < before) {
            jobs.delete(job.id);
            removed += 1;
          }
        }
        return removed;
      },
    },
    consent: {
      async put(grant) {
        const index = grants.findIndex(
          (existing) =>
            existing.wallet === grant.wallet &&
            existing.deploymentId === grant.deploymentId &&
            existing.instanceId === grant.instanceId,
        );
        if (index >= 0) grants.splice(index, 1);
        grants.push(clone(grant));
      },
      async find({ wallet, deploymentId, step, instanceId, now }) {
        const candidates = grants.filter(
          (grant) =>
            grant.wallet === wallet &&
            grant.deploymentId === deploymentId &&
            (grant.instanceId === null || grant.instanceId === instanceId) &&
            grant.steps.includes(step) &&
            grant.revokedAt === null &&
            grant.expiresAt > now,
        );
        // A grant scoped to the instance is more specific than one for every instance.
        const match = candidates.find((grant) => grant.instanceId !== null) ?? candidates[0];
        return match ? clone(match) : null;
      },
      async revoke({ wallet, deploymentId, steps, instanceId, now }) {
        let revoked = 0;
        for (const [index, grant] of grants.entries()) {
          if (grant.wallet !== wallet || grant.deploymentId !== deploymentId || grant.revokedAt !== null) continue;
          if (instanceId !== undefined && grant.instanceId !== instanceId) continue;
          if (steps && !grant.steps.some((step) => steps.includes(step))) continue;
          const remaining = steps ? grant.steps.filter((step) => !steps.includes(step)) : [];
          grants[index] = remaining.length > 0 ? { ...grant, steps: remaining } : { ...grant, revokedAt: now };
          revoked += 1;
        }
        return revoked;
      },
      async list({ wallet }) {
        return grants.filter((grant) => grant.wallet === wallet).map(clone);
      },
      async active({ deploymentId, step, now }) {
        return grants
          .filter(
            (grant) =>
              grant.deploymentId === deploymentId &&
              grant.revokedAt === null &&
              grant.expiresAt > now &&
              (step === undefined || grant.steps.includes(step)),
          )
          .map(clone);
      },
      async issueNonce(nonce, expiresAt) {
        if (!nonces.has(nonce)) nonces.set(nonce, { expiresAt, used: false });
      },
      async useNonce(nonce, now) {
        const issued = nonces.get(nonce);
        if (!issued || issued.used || issued.expiresAt <= now) return false;
        nonces.set(nonce, { ...issued, used: true });
        return true;
      },
    },
    sources: {
      async get(source, key) {
        const state = sourceStates.get(`${source}\u0000${key}`);
        return state ? clone(state) : null;
      },
      async advance({ source, key, expectedVersion, next, deliver, now }) {
        const id = `${source}\u0000${key}`;
        const current = sourceStates.get(id) ?? null;
        if ((current?.version ?? null) !== expectedVersion) return null;
        const generation = (current?.generation ?? 0) + (current?.hash === next.hash ? 0 : 1);
        const state: SourceState = {
          source,
          key,
          hash: next.hash,
          value: clone(next.value),
          generation,
          pending: next.pending ? clone(next.pending) : null,
          observed: next.observed ?? null,
          version: (current?.version ?? 0) + 1,
          updatedAt: now,
        };
        // Copy everything before changing anything: a payload that cannot be
        // stored must leave the source state as it was, or the trigger is lost.
        const delivery: Delivery | null = deliver
          ? {
              id: `view:${encodeURIComponent(source)}:${encodeURIComponent(key)}:${generation}`,
              name: deliver.name,
              source: 'view',
              payload: clone(deliver.payload),
              receivedAt: now,
            }
          : null;
        const result = clone(state);
        sourceStates.set(id, state);
        if (delivery) acceptNow(delivery, now);
        return result;
      },
      async cursor(source) {
        return cursors.get(source) ?? null;
      },
      async putCursor(source, cursor) {
        if (cursorAfter(cursor, cursors.get(source) ?? null)) cursors.set(source, cursor);
      },
      async acquireLease(source, holder, until, now) {
        const lease = sourceLeases.get(source);
        if (lease && lease.holder !== holder && lease.until > now) return false;
        sourceLeases.set(source, { holder, until });
        return true;
      },
      async releaseLease(source, holder) {
        if (sourceLeases.get(source)?.holder === holder) sourceLeases.delete(source);
      },
    },
    ledger: {
      async spent({ wallet, deploymentId, asset, since }) {
        let total = 0n;
        for (const entry of ledger) {
          if (
            entry.wallet === wallet &&
            entry.deploymentId === deploymentId &&
            entry.asset === asset &&
            entry.voidedAt === null &&
            entry.recordedAt >= since
          ) {
            total += BigInt(entry.amount);
          }
        }
        return total;
      },
      async list({ wallet }) {
        return ledger.filter((entry) => entry.wallet === wallet).map(clone);
      },
    },
  };
  STATES.set(store, () =>
    clone({ version: 1, deliveries, jobs, order, sequence, grants, deployment, nonces, ledger, sourceStates, cursors }),
  );
  return store;
}
