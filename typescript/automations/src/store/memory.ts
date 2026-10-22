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
import type { AutomationStore, ConsentGrant, NewStepJob, TransitionInput } from './store';

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

export function memoryStore(): AutomationStore {
  const deliveries = new Map<string, number>();
  const jobs = new Map<string, Job>();
  let order = 0;
  const sequence = new Map<string, number>();
  const grants: ConsentGrant[] = [];

  const insert = (job: Job) => {
    jobs.set(job.id, job);
    sequence.set(job.id, order++);
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

  return {
    deliveries: {
      async accept(delivery: Delivery, now: number) {
        if (deliveries.has(delivery.id)) return false;
        deliveries.set(delivery.id, now);
        insert({
          kind: 'delivery',
          id: delivery.id,
          state: 'queued',
          delivery: clone(delivery),
          attempts: 0,
          notBefore: now,
          lease: null,
          createdAt: now,
          updatedAt: now,
          outcome: null,
        });
        return true;
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
        jobs.set(id, { ...job, state: 'done', lease: null, updatedAt: now, outcome: null });
        for (const step of steps) {
          if (jobs.has(step.id)) continue;
          const created: StepJob = {
            kind: 'step',
            id: step.id,
            state: 'queued',
            deliveryId: id,
            deploymentId: step.deploymentId,
            request: clone(step.request),
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
        const match = grants.find(
          (grant) =>
            grant.wallet === wallet &&
            grant.deploymentId === deploymentId &&
            (grant.instanceId === null || grant.instanceId === instanceId) &&
            grant.steps.includes(step) &&
            grant.revokedAt === null &&
            grant.expiresAt > now,
        );
        return match ? clone(match) : null;
      },
      async revoke({ wallet, deploymentId, steps, now }) {
        let revoked = 0;
        for (const [index, grant] of grants.entries()) {
          if (grant.wallet !== wallet || grant.deploymentId !== deploymentId || grant.revokedAt !== null) continue;
          if (steps && !grant.steps.some((step) => steps.includes(step))) continue;
          const remaining = steps ? grant.steps.filter((step) => !steps.includes(step)) : [];
          grants[index] =
            remaining.length > 0 ? { ...grant, steps: remaining } : { ...grant, revokedAt: now };
          revoked += 1;
        }
        return revoked;
      },
      async list({ wallet }) {
        return grants.filter((grant) => grant.wallet === wallet).map(clone);
      },
    },
  };
}
