import { verifyWebhook, WebhookVerificationError } from '@usearete/sdk';
import type {
  ExecutionIntentV1,
  ProtocolDefinition,
  ProtocolEventsOptions,
  ProtocolEventSubscription,
  ProtocolIntentState,
  ProtocolPrepareOptions,
  ProtocolPrepareResult,
  ProtocolStepAvailability,
  ProtocolTransactionFormat,
  ProtocolTypeMap,
} from '@usearete/sdk';

import type { Cluster, Refusal, SignerPolicy } from '../policy/policy';
import { checkPrepared, checkTarget } from '../policy/policy';
import type { Simulator } from '../policy/simulator';
import { summarizePrepared } from '../policy/summary';
import { DEFAULT_TRANSACTION_FORMATS, negotiateTransactionFormat } from '../signer/format';
import type { ProtocolSigner } from '../signer/signer';
import { SignerRefusal } from '../signer/signer';
import { checkPreparedAgainstIntent, checkSignedTransactions } from '../signer/verify';
import type { AutomationStore, NewStepJob } from '../store/store';
import { type Clock, type Duration, durationMs, systemClock } from './time';
import {
  type Delivery,
  type DeliveryJob,
  type Job,
  type JobOutcome,
  type StepJob,
  type StepJobState,
  type StepRequest,
  stepJobId,
  type TriggerSource,
} from './types';

/** The parts of a protocol client automations use. A secret-key `ProtocolClient` is one. */
export interface AutomationClient<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly deploymentId: string;
  /** Present on a real client; only used to infer the protocol's types. */
  readonly definition?: ProtocolDefinition<TTypes, any, any>;
  readonly steps: object;
  allowedSteps(input: { instance: string; wallet: string }): Promise<readonly ProtocolStepAvailability[]>;
  submit(intent: ExecutionIntentV1 | string, signedTransactions: readonly (string | Uint8Array)[]): Promise<ProtocolIntentState>;
  readonly intents: { get(executionId: string): Promise<ProtocolIntentState> };
  events?(instanceId: string, options: ProtocolEventsOptions<TTypes>): ProtocolEventSubscription;
}

interface PreparableStep {
  prepare(input: Record<string, unknown>, options?: ProtocolPrepareOptions): Promise<ProtocolPrepareResult>;
}

type EventName<TTypes extends ProtocolTypeMap> = Extract<keyof TTypes['events'], string>;
type StepName<TTypes extends ProtocolTypeMap> = Extract<keyof TTypes['steps'], string>;

type EventState<TTypes extends ProtocolTypeMap, E extends EventName<TTypes>> =
  TTypes['events'][E]['machine'] extends 'participant' ? TTypes['participantState'] : TTypes['instanceState'];

/** An emitted protocol event, from a webhook or an instance's event stream. */
export interface AutomationEvent<TEvent extends string = string, TState extends string = string> {
  /** The emission id. */
  readonly id: string;
  readonly event: TEvent;
  readonly deploymentId: string;
  readonly instanceId: string;
  readonly participantId: string | null;
  /** The machine's state at emission; `null` for stream events, which do not carry it. */
  readonly state: TState | null;
  readonly occurredAt: string;
  readonly source: 'webhook' | 'stream';
}

export interface TriggerInfo {
  readonly name: string;
  readonly id: string;
  readonly source: TriggerSource;
  readonly receivedAt: number;
}

export interface StepOptions {
  /** Skip the step when the same wallet completed it this recently. */
  readonly cooldown?: Duration;
}

export interface HandlerContext<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly client: AutomationClient<TTypes>;
  readonly trigger: TriggerInfo;
  /** A step request for the handler to return. Handlers never prepare steps themselves. */
  step<S extends StepName<TTypes>>(
    step: S,
    request: { wallet: string; instance?: string; args?: Partial<TTypes['steps'][S]['args']> & Record<string, unknown> },
    options?: StepOptions,
  ): StepRequest;
}

export type Handler<TPayload, TTypes extends ProtocolTypeMap> = (
  payload: TPayload,
  ctx: HandlerContext<TTypes>,
) => readonly StepRequest[] | void | Promise<readonly StepRequest[] | void>;

/** Where a step job ended, for notifications. */
export interface AutomationOutcome {
  readonly jobId: string;
  readonly deliveryId: string;
  readonly deploymentId: string;
  readonly step: string;
  readonly wallet: string;
  readonly instanceId: string | null;
  readonly state: StepJobState;
  readonly code: string;
  readonly reason?: string;
  readonly detail?: unknown;
  readonly executionId: string | null;
  readonly signatures: readonly (string | null)[];
}

export interface AutomationsOptions<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly client: AutomationClient<TTypes>;
  readonly signer: ProtocolSigner;
  readonly policy: SignerPolicy;
  /** Measures what each transaction moves out of the wallet; the policy refuses what it cannot measure. */
  readonly simulator: Simulator;
  readonly store: AutomationStore;
  /** The cluster the deployment runs on; the policy must list it. */
  readonly cluster: Cluster;
  /** Transaction formats in the deployment's order of preference. Defaults to v1, then v0. */
  readonly transactionFormats?: readonly ProtocolTransactionFormat[];
  /** Identifies this worker in leases. Defaults to a random id. */
  readonly worker?: string;
  /** How long a claim lasts before another worker may take the job over. Defaults to 60s. */
  readonly lease?: Duration;
  /** Attempts before a job that has not signed yet fails. Defaults to 5. */
  readonly maxAttempts?: number;
  /** How long one claim waits for a submitted intent to settle. Defaults to 30s. */
  readonly trackFor?: Duration;
  /** Between intent status reads while tracking. Defaults to 2s. */
  readonly trackInterval?: Duration;
  /** Before a pending intent is read again in a later claim. Defaults to 10s, doubling up to 5m. */
  readonly reconcileInterval?: Duration;
  readonly clock?: Clock;
  /** Every step job that reaches a final state. */
  readonly onOutcome?: (outcome: AutomationOutcome) => void | Promise<void>;
  /** Step jobs refused by the policy or the signer checks. */
  readonly onRefused?: (outcome: AutomationOutcome) => void | Promise<void>;
  /** Step jobs the protocol rejected (a guard, an access or a state code). */
  readonly onRejected?: (outcome: AutomationOutcome) => void | Promise<void>;
  /** Errors the runtime recovers from by retrying, and hook failures. */
  readonly onError?: (error: unknown, context: { jobId?: string; phase: string }) => void;
}

export interface WebhookHandlerOptions {
  /** The endpoint's signing secret. */
  readonly secret: string | Uint8Array;
  /** Allowed clock skew in seconds; defaults to 300. */
  readonly toleranceSeconds?: number;
}

export interface WebhookResult {
  readonly status: number;
  readonly body: Record<string, unknown>;
}

export interface Automations<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  /** Handle an emitted protocol event. */
  on<E extends EventName<TTypes>>(event: E, handler: Handler<AutomationEvent<E, EventState<TTypes, E>>, TTypes>): this;
  /** Handle a dispatched trigger (a cron tick, a view source). */
  on<TPayload = unknown>(name: string, handler: Handler<TPayload, TTypes>): this;
  /**
   * Accept a trigger. `id` makes it idempotent: a repeated dispatch with the
   * same id is ignored. Resolves `false` for a duplicate.
   */
  dispatch(name: string, payload: unknown, options: { id: string }): Promise<boolean>;
  /** A fetch-standard webhook endpoint (Workers, Next.js, Hono, Bun). */
  webhookHandler(options: WebhookHandlerOptions): (request: Request) => Promise<Response>;
  /** An Express endpoint. Mount it behind `express.raw({ type: 'application/json' })`. */
  expressWebhookHandler(options: WebhookHandlerOptions): (req: ExpressLikeRequest, res: ExpressLikeResponse) => Promise<void>;
  /** Verify and accept a webhook delivery from its raw body and headers. */
  acceptWebhook(body: string | Uint8Array, headers: WebhookHeadersLike, options: WebhookHandlerOptions): Promise<WebhookResult>;
  /** Follow one instance's event stream and accept its events. */
  follow(instanceId: string, options?: { lastEventId?: string; signal?: AbortSignal }): ProtocolEventSubscription;
  /** Claim and run one job. `false` when none was ready. */
  runOnce(): Promise<boolean>;
  /** Run jobs until none is ready (or `maxJobs` ran). Returns how many ran. */
  drain(options?: { maxJobs?: number }): Promise<number>;
  /** Run jobs in the background until `stop()`. */
  start(options?: { pollInterval?: Duration; concurrency?: number }): void;
  stop(): Promise<void>;
}

export type WebhookHeadersLike = { get(name: string): string | null } | Record<string, string | readonly string[] | undefined>;

export interface ExpressLikeRequest {
  readonly body: unknown;
  readonly headers: Record<string, string | readonly string[] | undefined>;
}

export interface ExpressLikeResponse {
  status(code: number): ExpressLikeResponse;
  json(body: unknown): unknown;
}

/** Thrown when another worker took the job over; the current run stops quietly. */
class LostLease extends Error {}

const ACTIVE_RECONCILE_CAP_MS = 5 * 60_000;

function randomId(): string {
  const crypto = (globalThis as { crypto?: { randomUUID?: () => string } }).crypto;
  return crypto?.randomUUID?.() ?? `worker-${Math.random().toString(36).slice(2)}-${Date.now().toString(36)}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function createAutomations<TTypes extends ProtocolTypeMap = ProtocolTypeMap>(
  options: AutomationsOptions<TTypes>,
): Automations<TTypes> {
  const { client, signer, policy, simulator, store, cluster } = options;
  const clock = options.clock ?? systemClock;
  const worker = options.worker ?? randomId();
  const leaseMs = durationMs(options.lease ?? 60_000);
  const maxAttempts = options.maxAttempts ?? 5;
  const trackForMs = durationMs(options.trackFor ?? 30_000);
  const trackIntervalMs = durationMs(options.trackInterval ?? 2_000);
  const reconcileMs = durationMs(options.reconcileInterval ?? 10_000);
  const formats = options.transactionFormats ?? DEFAULT_TRANSACTION_FORMATS;
  const handlers = new Map<string, Handler<any, TTypes>>();

  const report = (error: unknown, context: { jobId?: string; phase: string }) => {
    try {
      options.onError?.(error, context);
    } catch {
      // A failing error hook must not stop the worker.
    }
  };

  const backoff = (attempts: number) => Math.min(reconcileMs * 2 ** Math.max(0, attempts - 1), ACTIVE_RECONCILE_CAP_MS);
  const leaseUntil = () => ({ until: clock.now() + leaseMs });

  // -------------------------------------------------------------------------
  // Accepting triggers
  // -------------------------------------------------------------------------

  const accept = (delivery: Omit<Delivery, 'receivedAt'>) =>
    store.deliveries.accept({ ...delivery, receivedAt: clock.now() }, clock.now());

  const acceptWebhook = async (
    body: string | Uint8Array,
    headers: WebhookHeadersLike,
    webhook: WebhookHandlerOptions,
  ): Promise<WebhookResult> => {
    let verified;
    try {
      verified = verifyWebhook(webhook.secret, body, headers as never, {
        ...(webhook.toleranceSeconds !== undefined ? { toleranceSeconds: webhook.toleranceSeconds } : {}),
        now: Math.floor(clock.now() / 1000),
      });
    } catch (error) {
      if (error instanceof WebhookVerificationError) {
        const status = error.code === 'invalid-signature' || error.code === 'timestamp-out-of-tolerance' ? 401 : 400;
        return { status, body: { error: error.code } };
      }
      throw error;
    }
    const payload = verified.payload as unknown;
    if (!isRecord(payload) || typeof payload.event !== 'string' || typeof payload.instanceId !== 'string') {
      return { status: 400, body: { error: 'invalid-payload' } };
    }
    if (payload.deploymentId !== client.deploymentId) {
      return { status: 200, body: { accepted: false, ignored: 'other-deployment' } };
    }
    if (!handlers.has(payload.event)) {
      return { status: 200, body: { accepted: false, ignored: 'no-handler' } };
    }
    const event: AutomationEvent = {
      id: String(payload.id),
      event: payload.event,
      deploymentId: payload.deploymentId,
      instanceId: payload.instanceId,
      participantId: typeof payload.participantId === 'string' ? payload.participantId : null,
      state: typeof payload.state === 'string' ? payload.state : null,
      occurredAt: typeof payload.occurredAt === 'string' ? payload.occurredAt : '',
      source: 'webhook',
    };
    const accepted = await accept({ id: `webhook:${verified.id}`, name: payload.event, source: 'webhook', payload: event });
    return { status: 200, body: { accepted, duplicate: !accepted } };
  };

  // -------------------------------------------------------------------------
  // Step jobs
  // -------------------------------------------------------------------------

  const outcomeOf = (job: StepJob, state: StepJobState, outcome: JobOutcome): AutomationOutcome => ({
    jobId: job.id,
    deliveryId: job.deliveryId,
    deploymentId: job.deploymentId,
    step: job.request.step,
    wallet: job.request.wallet,
    instanceId: job.request.instance ?? null,
    state,
    code: outcome.code,
    ...(outcome.reason !== undefined ? { reason: outcome.reason } : {}),
    ...(outcome.detail !== undefined ? { detail: outcome.detail } : {}),
    executionId: job.progress.intent?.executionId ?? null,
    signatures: job.progress.signatures ?? [],
  });

  const notify = async (outcome: AutomationOutcome) => {
    const hooks = [options.onOutcome];
    if (outcome.state === 'refused') hooks.push(options.onRefused);
    if (outcome.state === 'rejected') hooks.push(options.onRejected);
    for (const hook of hooks) {
      if (!hook) continue;
      try {
        await hook(outcome);
      } catch (error) {
        report(error, { jobId: outcome.jobId, phase: 'hook' });
      }
    }
  };

  /** Move `job` to a final state and release it. */
  const finish = async (
    job: StepJob,
    state: StepJobState,
    outcome: JobOutcome,
    progress: StepJob['progress'] | undefined = undefined,
  ) => {
    const moved = await store.jobs.transition(job.id, {
      worker,
      from: job.state,
      to: state,
      now: clock.now(),
      outcome,
      ...(progress ? { progress } : {}),
    });
    if (!moved) throw new LostLease();
    await notify(outcomeOf(progress ? { ...job, progress: { ...job.progress, ...progress } } : job, state, outcome));
  };

  /** Move `job` to another active state, keeping the lease. */
  const advance = async (job: StepJob, state: StepJobState, progress?: Partial<StepJob['progress']>): Promise<StepJob> => {
    const moved = await store.jobs.transition(job.id, {
      worker,
      from: job.state,
      to: state,
      now: clock.now(),
      keepLease: leaseUntil(),
      ...(progress ? { progress } : {}),
    });
    if (!moved) throw new LostLease();
    return { ...job, state, progress: { ...job.progress, ...progress } };
  };

  /** Release `job` in its current state, to be claimed again later. */
  const park = async (job: StepJob, delayMs: number, attempts = job.attempts) => {
    const moved = await store.jobs.transition(job.id, {
      worker,
      from: job.state,
      to: job.state,
      now: clock.now(),
      notBefore: clock.now() + delayMs,
      attempts,
    });
    if (!moved) throw new LostLease();
  };

  const refuse = (job: StepJob, refusal: Refusal) =>
    finish(job, 'refused', { code: refusal.rule, reason: refusal.reason });

  const consentFor = (job: StepJob) =>
    store.consent.find({
      wallet: job.request.wallet,
      deploymentId: job.deploymentId,
      step: job.request.step,
      instanceId: job.request.instance ?? null,
      now: clock.now(),
    });

  /** Prepare, check, sign and submit a `running` step job. */
  const attempt = async (job: StepJob): Promise<void> => {
    const { request } = job;
    const wallet = request.wallet;
    if (!(await consentFor(job))) {
      return finish(job, 'skipped', { code: 'no_consent', reason: `no active consent for ${request.step}` });
    }
    if (request.cooldownMs !== undefined) {
      const last = await store.jobs.lastCompleted({ wallet, deploymentId: job.deploymentId, step: request.step });
      if (last !== null && clock.now() - last < request.cooldownMs) {
        return finish(job, 'skipped', {
          code: 'cooldown',
          reason: `${request.step} completed for this wallet ${Math.round((clock.now() - last) / 1000)}s ago`,
        });
      }
    }
    const target = checkTarget(policy, job.deploymentId, cluster);
    if (target) return refuse(job, target);
    if (request.instance !== undefined) {
      const availability = (await client.allowedSteps({ instance: request.instance, wallet })).find(
        (entry) => entry.step === request.step,
      );
      if (!availability) {
        return finish(job, 'rejected', { code: 'step_not_available', reason: `${request.step} is not a step of this instance` });
      }
      if (!availability.allowed) {
        return finish(job, 'rejected', {
          code: availability.reason ?? 'not_allowed',
          ...(availability.detail !== undefined ? { detail: availability.detail } : {}),
        });
      }
    }
    const format = negotiateTransactionFormat(signer.supportedTransactionFormats, formats);
    if (format === null) {
      return refuse(job, {
        rule: 'transaction_format',
        reason: `the ${signer.kind} signer supports ${signer.supportedTransactionFormats.join(', ')}, the deployment ${formats.join(', ')}`,
      });
    }
    if (!(await signer.canSign(wallet))) {
      return refuse(job, { rule: 'signer_unavailable', reason: `the ${signer.kind} signer cannot sign for ${wallet}` });
    }
    const stepClient = (client.steps as Record<string, PreparableStep | undefined>)[request.step];
    if (!stepClient) {
      return finish(job, 'rejected', { code: 'step_not_client_callable', reason: `${request.step} cannot be prepared by this client` });
    }
    const result = await stepClient.prepare(
      { ...request.args, wallet, ...(request.instance !== undefined ? { instance: request.instance } : {}) },
      { transactionFormat: format, idempotencyKey: `${job.id}:${job.progress.generation}` },
    );
    if (!result.ok) {
      return finish(job, 'rejected', {
        code: result.rejection.rejected,
        ...(result.rejection.detail !== undefined ? { detail: result.rejection.detail } : {}),
      });
    }
    const { intent, transactions } = result;
    const mismatch = checkPreparedAgainstIntent({ wallet, intent, transactions, format });
    if (mismatch) return refuse(job, mismatch);
    const summary = await summarizePrepared({ intent, transactions }, wallet, simulator);
    const refusal = checkPrepared(policy, {
      deploymentId: job.deploymentId,
      prepared: { intent, transactions },
      summary,
      simulates: true,
    });
    if (refusal) return refuse(job, refusal);
    if (!(await store.jobs.renew(job.id, worker, leaseUntil().until))) throw new LostLease();
    // Consent can be revoked while the step was prepared and simulated.
    if (!(await consentFor(job))) {
      return finish(job, 'skipped', { code: 'no_consent', reason: 'consent was revoked before signing' });
    }
    let signed: string[];
    try {
      signed = await signer.signTransactions({ wallet, intent, transactions });
    } catch (error) {
      if (error instanceof SignerRefusal) return refuse(job, { rule: error.rule, reason: error.message });
      throw error;
    }
    const tampered = checkSignedTransactions({ wallet, intent, prepared: transactions, signed });
    if (tampered) return refuse(job, tampered);
    const saved = await advance(job, 'signed', { format, intent, prepared: transactions, signed });
    return submit(saved);
  };

  /** Submit a `signed` job's saved transactions, then track the intent. */
  const submit = async (job: StepJob): Promise<void> => {
    let state: ProtocolIntentState;
    try {
      state = await client.submit(job.progress.intent!, job.progress.signed!);
    } catch (error) {
      // The API may have accepted it; the next claim reads the intent first.
      report(error, { jobId: job.id, phase: 'submit' });
      return park(job, backoff(job.attempts + 1), job.attempts + 1);
    }
    return track(await advance(job, 'submitted'), state);
  };

  /** Follow a submitted intent until it settles or this claim's time is up. */
  const track = async (initial: StepJob, first?: ProtocolIntentState): Promise<void> => {
    let job = initial;
    const deadline = clock.now() + trackForMs;
    let state = first;
    for (;;) {
      if (state === undefined) {
        try {
          state = await client.intents.get(job.progress.intent!.executionId);
        } catch (error) {
          report(error, { jobId: job.id, phase: 'intent' });
        }
      }
      if (state !== undefined) {
        const signatures = state.transactions.map((transaction) => transaction.signature);
        switch (state.state) {
          case 'completed':
            return finish(job, 'completed', { code: 'completed' }, { ...job.progress, signatures });
          case 'chain_failed':
            return finish(
              job,
              'chain_failed',
              {
                code: 'chain_failed',
                detail: state.transactions.filter((transaction) => transaction.error).map(({ index, error }) => ({ index, error })),
              },
              { ...job.progress, signatures },
            );
          case 'rejected':
            return finish(job, 'rejected', { code: 'intent_rejected' }, { ...job.progress, signatures });
          case 'expired':
            return prepareAgain(job);
          case 'prepared':
            // Not submitted yet: submit the saved bytes (never signs again).
            if (job.state === 'signed') return submit(job);
            break;
          case 'submitted_unknown':
            if (job.state !== 'submitted_unknown') job = await advance(job, 'submitted_unknown', { signatures });
            break;
          default:
            if (job.state === 'signed') job = await advance(job, 'submitted', { signatures });
        }
      }
      if (clock.now() + trackIntervalMs > deadline) {
        return park(job, backoff(job.attempts + 1), job.attempts + 1);
      }
      await clock.sleep(trackIntervalMs);
      if (!(await store.jobs.renew(job.id, worker, leaseUntil().until))) throw new LostLease();
      state = undefined;
    }
  };

  /** An expired intent is prepared again once, with a new idempotency key. */
  const prepareAgain = async (job: StepJob): Promise<void> => {
    if (job.progress.generation >= 1) {
      return finish(job, 'expired', { code: 'expired', reason: 'the intent expired again after preparing it a second time' });
    }
    const fresh = await advance(job, 'running', {
      generation: job.progress.generation + 1,
      intent: undefined,
      prepared: undefined,
      signed: undefined,
      signatures: undefined,
    });
    return attempt(fresh);
  };

  /** A job that had signed: read the intent before doing anything else. */
  const reconcile = async (job: StepJob): Promise<void> => {
    let state: ProtocolIntentState;
    try {
      state = await client.intents.get(job.progress.intent!.executionId);
    } catch (error) {
      report(error, { jobId: job.id, phase: 'reconcile' });
      return park(job, backoff(job.attempts + 1), job.attempts + 1);
    }
    return track(job, state);
  };

  const runStep = async (job: StepJob) => {
    try {
      if (job.state === 'running') {
        // A lapsed claim before signing starts over: prepare is idempotent.
        await attempt(job);
      } else {
        await reconcile(job);
      }
    } catch (error) {
      if (error instanceof LostLease) return;
      report(error, { jobId: job.id, phase: job.state });
      const current = await store.jobs.get(job.id);
      if (!current || current.kind !== 'step' || current.lease?.worker !== worker) return;
      const attempts = current.attempts + 1;
      if (current.state === 'running') {
        if (attempts >= maxAttempts) {
          await finish(current, 'failed', { code: 'error', reason: message(error) }).catch(() => undefined);
        } else {
          // Nothing was signed: let other jobs of the wallet run meanwhile.
          await store.jobs.transition(current.id, {
            worker,
            from: 'running',
            to: 'queued',
            now: clock.now(),
            notBefore: clock.now() + backoff(attempts),
            attempts,
          });
        }
      } else {
        // Signed or later: never fail blind; read the intent next time.
        await park(current, backoff(attempts), attempts).catch(() => undefined);
      }
    }
  };

  // -------------------------------------------------------------------------
  // Delivery jobs
  // -------------------------------------------------------------------------

  const runDelivery = async (job: DeliveryJob) => {
    const { delivery } = job;
    const handler = handlers.get(delivery.name);
    const ctx: HandlerContext<TTypes> = {
      client,
      trigger: { name: delivery.name, id: delivery.id, source: delivery.source, receivedAt: delivery.receivedAt },
      step(step, request, stepOptions = {}) {
        return {
          step,
          wallet: request.wallet,
          ...(request.instance !== undefined ? { instance: request.instance } : {}),
          args: { ...(request.args ?? {}) },
          ...(stepOptions.cooldown !== undefined ? { cooldownMs: durationMs(stepOptions.cooldown) } : {}),
        };
      },
    };
    try {
      const requests = handler ? ((await handler(delivery.payload, ctx)) ?? []) : [];
      const steps = new Map<string, NewStepJob>();
      for (const request of requests) {
        if (typeof request?.step !== 'string' || typeof request.wallet !== 'string' || request.wallet === '') {
          throw new TypeError(`handler for ${delivery.name} returned a step request without a step or wallet`);
        }
        const id = stepJobId(delivery.id, request);
        if (!steps.has(id)) steps.set(id, { id, deploymentId: client.deploymentId, request });
      }
      if (!(await store.jobs.completeDelivery(job.id, worker, [...steps.values()], clock.now()))) {
        throw new LostLease();
      }
    } catch (error) {
      if (error instanceof LostLease) return;
      report(error, { jobId: job.id, phase: 'handler' });
      const attempts = job.attempts + 1;
      await store.jobs.transition(job.id, {
        worker,
        from: 'running',
        to: attempts >= maxAttempts ? 'failed' : 'queued',
        now: clock.now(),
        attempts,
        notBefore: clock.now() + backoff(attempts),
        ...(attempts >= maxAttempts ? { outcome: { code: 'handler_failed', reason: message(error) } } : {}),
      });
    }
  };

  // -------------------------------------------------------------------------
  // Workers
  // -------------------------------------------------------------------------

  const runOnce = async () => {
    const job: Job | null = await store.jobs.claimNext({ worker, leaseMs, now: clock.now() });
    if (!job) return false;
    if (job.kind === 'delivery') await runDelivery(job);
    else await runStep(job);
    return true;
  };

  let running = false;
  let loops: Promise<void>[] = [];

  const automations: Automations<TTypes> = {
    on(name: string, handler: Handler<any, TTypes>) {
      handlers.set(name, handler);
      return automations;
    },
    dispatch(name, payload, { id }) {
      return accept({ id: `dispatch:${id}`, name, source: 'dispatch', payload });
    },
    acceptWebhook,
    webhookHandler(webhook) {
      return async (request: Request) => {
        const body = new Uint8Array(await request.arrayBuffer());
        const result = await acceptWebhook(body, request.headers, webhook);
        return new Response(JSON.stringify(result.body), {
          status: result.status,
          headers: { 'content-type': 'application/json' },
        });
      };
    },
    expressWebhookHandler(webhook) {
      return async (req, res) => {
        const raw = req.body;
        if (!(raw instanceof Uint8Array) && typeof raw !== 'string') {
          res.status(500).json({
            error: 'raw-body-required',
            message: "mount the webhook route behind express.raw({ type: 'application/json' })",
          });
          return;
        }
        const result = await acceptWebhook(raw, req.headers, webhook);
        res.status(result.status).json(result.body);
      };
    },
    follow(instanceId, followOptions = {}) {
      if (!client.events) throw new Error('this client cannot follow event streams');
      return client.events(instanceId, {
        ...(followOptions.lastEventId !== undefined ? { lastEventId: followOptions.lastEventId } : {}),
        ...(followOptions.signal ? { signal: followOptions.signal } : {}),
        onEvent(streamEvent) {
          if (streamEvent.type !== 'emit' || !handlers.has(streamEvent.event)) return;
          const data = streamEvent.data;
          const event: AutomationEvent = {
            id: String(data.id),
            event: streamEvent.event,
            deploymentId: client.deploymentId,
            instanceId: data.instanceId,
            participantId: data.participantId,
            state: null,
            occurredAt: data.at,
            source: 'stream',
          };
          accept({ id: `stream:${data.instanceId}:${data.id}`, name: streamEvent.event, source: 'stream', payload: event }).catch(
            (error) => report(error, { phase: 'stream' }),
          );
        },
        onError(error) {
          report(error, { phase: 'stream' });
        },
      });
    },
    runOnce,
    async drain(drainOptions = {}) {
      const max = drainOptions.maxJobs ?? Number.POSITIVE_INFINITY;
      let ran = 0;
      while (ran < max && (await runOnce())) ran += 1;
      return ran;
    },
    start(startOptions = {}) {
      if (running) return;
      running = true;
      const pollMs = durationMs(startOptions.pollInterval ?? 1_000);
      const loop = async () => {
        while (running) {
          let ran = false;
          try {
            ran = await runOnce();
          } catch (error) {
            report(error, { phase: 'worker' });
          }
          if (!ran && running) await clock.sleep(pollMs);
        }
      };
      loops = Array.from({ length: Math.max(1, startOptions.concurrency ?? 1) }, loop);
    },
    async stop() {
      running = false;
      await Promise.all(loops);
      loops = [];
    },
  };
  return automations;
}
