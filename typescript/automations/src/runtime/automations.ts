import { verifyWebhook, WebhookVerificationError } from '@usearete/sdk';
import type {
  ExecutionIntentV1,
  ProtocolDefinition,
  ProtocolEventsOptions,
  ProtocolEventSubscription,
  ProtocolIntentState,
  ProtocolParticipantDetail,
  ProtocolPrepareOptions,
  ProtocolPrepareResult,
  ProtocolStepAvailability,
  ProtocolTransactionFormat,
  ProtocolTypeMap,
  RichUpdate,
} from '@usearete/sdk';

import { ed25519 } from '@noble/curves/ed25519';

import {
  type ConsentFields,
  ConsentError,
  consentMessage,
  normalizeConsent,
  SOL,
  stepOutflows,
  transactionOutflows,
} from '../consent/consent';
import { decodeTransaction } from '../policy/decode';
import { decodeBase58, decodeBase64, encodeBase58 } from '../policy/encoding';
import type { Cluster, Refusal, SignerPolicy } from '../policy/policy';
import { checkPrepared, checkTarget } from '../policy/policy';
import type { Simulator } from '../policy/simulator';
import { summarizePrepared } from '../policy/summary';
import { DEFAULT_TRANSACTION_FORMATS, negotiateTransactionFormat } from '../signer/format';
import type { ProtocolSigner } from '../signer/signer';
import { isSignerRefusal } from '../signer/signer';
import { checkPreparedAgainstIntent, checkSignedTransactions } from '../signer/verify';
import { type AppendSource, type AppendTrigger, evaluateSignal, type ListSource, type ViewSource } from '../sources/view';
import type { AutomationStore, ConsentGrant, NewStepJob } from '../store/store';
import { type AutomationsDefinition, type AutomationsSetup, connectStack, type StackConnector } from './define';
import { type Clock, type Duration, durationMs, systemClock } from './time';
import {
  type Delivery,
  type DeliveryJob,
  type Job,
  type JobOutcome,
  prepareIdempotencyKey,
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
  /** Used to end instance-scoped consent when the instance is terminal. */
  readonly instances?: { get(id: string): Promise<{ readonly terminal: boolean }> };
  /** Present on a real client: a handler reads a participant's current state with it. */
  readonly participants?: {
    get(input: { instance: string; wallet: string; signal?: AbortSignal }): Promise<ProtocolParticipantDetail<TTypes>>;
  };
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
  /**
   * Active consent grants in this deployment, for fan-out: every wallet the
   * automations may act for. A starting set only; each step job checks
   * consent again.
   */
  readonly consents: {
    list(query?: { step?: StepName<TTypes> | (string & {}); instanceId?: string }): Promise<ConsentGrant[]>;
  };
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
  /**
   * Where the deployment runs; the policy must list it. Defaults to the
   * client's: `dev` for a client `a4 dev` generated (the deployment in a dev
   * environment), else `mainnet`.
   */
  readonly cluster?: Cluster;
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
  /** Consent proofs: the host name users see in the message they sign. */
  readonly consent?: {
    readonly domain: string;
    /** How long a proof message may wait to be signed and granted. Defaults to 10 minutes. */
    readonly proofTtl?: Duration;
  };
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

/** A consent proof message for the user's wallet to sign. */
export interface ConsentProofRequest {
  /** The UTF-8 message, lines joined by `\n`. */
  readonly message: string;
  /** `message` as bytes, for `wallet.signMessage`. */
  readonly bytes: Uint8Array;
  readonly nonce: string;
  /** Milliseconds since the epoch; send it back with the grant. */
  readonly issuedAt: number;
}

export interface ConsentGrantInput extends ConsentFields {
  readonly nonce: string;
  readonly issuedAt: number;
  /** The wallet's 64-byte Ed25519 signature over the message, or its base58 text. */
  readonly signature: Uint8Array | string;
}

export interface ConsentApi {
  /** Build the message to sign. Issues a single-use nonce. */
  proofMessage(fields: ConsentFields): Promise<ConsentProofRequest>;
  /**
   * Verify the wallet's signature over the message rebuilt from `fields`,
   * use the nonce, and store the grant. `warnings` lists what the signer
   * reports missing on the provider side (no delegation, no provider policy).
   */
  grant(input: ConsentGrantInput): Promise<{ grant: ConsentGrant; warnings: string[] }>;
  /** Revoke without a signature. Queued and running jobs see it before signing. */
  revoke(input: { wallet: string; steps?: readonly string[]; instanceId?: string }): Promise<number>;
  list(input: { wallet: string }): Promise<ConsentGrant[]>;
}

export interface WebhookResult {
  readonly status: number;
  readonly body: Record<string, unknown>;
}

export interface Automations<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly consent: ConsentApi;
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
  /**
   * Watch a view source and accept its triggers until `stop()`. With
   * `exclusive`, only the replica holding the source's lease subscribes
   * (the compare-and-set already makes several subscribers correct; the
   * lease only saves bandwidth).
   */
  source(source: ViewSource, options?: { exclusive?: boolean }): SourceHandle;
  /**
   * Register a definition's handlers and view sources (see
   * `defineAutomations`). `connect` opens the stacks its sources read;
   * it defaults to the SDK's own connection.
   */
  use(definition: AutomationsDefinition<TTypes>, options?: { connect?: StackConnector }): Promise<this>;
  /** Read a list-view source's snapshot once and accept what changed (for a serverless cron). */
  poll(source: ListSource): Promise<{ delivered: number; pending: number }>;
  /** Claim and run one job. `false` when none was ready. */
  runOnce(): Promise<boolean>;
  /** Run jobs until none is ready (or `maxJobs` ran). Returns how many ran. */
  drain(options?: { maxJobs?: number }): Promise<number>;
  /** Run jobs in the background until `stop()`. */
  start(options?: { pollInterval?: Duration; concurrency?: number }): void;
  stop(): Promise<void>;
}

export interface SourceHandle {
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

/** Where a client's deployment runs: `dev` when `a4 dev` generated it, else `mainnet`. */
export function clientCluster(client: Pick<AutomationClient, 'definition'>): Cluster {
  // Read structurally: SDKs before the field carry it untyped.
  const deployment = client.definition?.deployment as { readonly environment?: string } | undefined;
  return deployment?.environment === 'dev' ? 'dev' : 'mainnet';
}

export function createAutomations<TTypes extends ProtocolTypeMap = ProtocolTypeMap>(
  options: AutomationsOptions<TTypes>,
): Automations<TTypes> {
  const { client, signer, policy, simulator, store } = options;
  // Only SOL and SPL token balances are measured: a program outside the
  // allow-list could move anything else (NFTs, positions it holds), so
  // automations need one.
  if (policy.programs === null) {
    throw new TypeError(
      'createAutomations: the policy must list `programs`, the programs your steps call; without it any program could run with the signer',
    );
  }
  const cluster = options.cluster ?? clientCluster(client);
  const clock = options.clock ?? systemClock;
  const worker = options.worker ?? randomId();
  const leaseMs = durationMs(options.lease ?? 60_000);
  const maxAttempts = options.maxAttempts ?? 5;
  const trackForMs = durationMs(options.trackFor ?? 30_000);
  const trackIntervalMs = durationMs(options.trackInterval ?? 2_000);
  const reconcileMs = durationMs(options.reconcileInterval ?? 10_000);
  const formats = options.transactionFormats ?? DEFAULT_TRANSACTION_FORMATS;
  const handlers = new Map<string, Handler<any, TTypes>>();
  const proofTtlMs = durationMs(options.consent?.proofTtl ?? 600_000);

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

  // One store serves one deployment; claims and delivery ids are not scoped
  // by deployment, so the store refuses a second one.
  let binding: Promise<void> | null = null;
  const bound = () => {
    binding ??= store.bindDeployment(client.deploymentId).catch((error: unknown) => {
      binding = null;
      throw error;
    });
    return binding;
  };

  const accept = async (delivery: Omit<Delivery, 'receivedAt'>) => {
    await bound();
    return store.deliveries.accept({ ...delivery, receivedAt: clock.now() }, clock.now());
  };

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
    if (
      !isRecord(payload) ||
      typeof payload.event !== 'string' ||
      typeof payload.instanceId !== 'string' ||
      (typeof payload.id !== 'string' && typeof payload.id !== 'number')
    ) {
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
    // The id comes from the signed body: the delivery-id header is not covered
    // by the signature, so a replay under a new header id would otherwise
    // run again.
    const id = `webhook:${encodeURIComponent(String(payload.deploymentId))}:${encodeURIComponent(String(payload.id))}`;
    const accepted = await accept({ id, name: payload.event, source: 'webhook', payload: event });
    return { status: 200, body: { accepted, duplicate: !accepted } };
  };

  // -------------------------------------------------------------------------
  // Step jobs
  // -------------------------------------------------------------------------

  /**
   * The ledger after a chain failure, per transaction: one that landed
   * successfully keeps everything it moved; one that failed counts the fee
   * it was charged (all its SOL when the fee cannot be read); one that never
   * landed counts nothing.
   */
  const failedLedger = async (
    job: StepJob,
    state: ProtocolIntentState,
  ): Promise<NonNullable<Parameters<AutomationStore['jobs']['transition']>[1]['ledger']>> => {
    const saved = job.progress.outflows;
    if (!saved) {
      // Signed before outflows were saved per transaction: if any transaction
      // landed, keep every entry; otherwise keep the fees, or all the SOL.
      if (state.transactions.some((transaction) => transaction.signature !== null && !transaction.error)) return {};
      const fees = await chargedFees(state.transactions.map((transaction) => transaction.signature));
      return fees === null
        ? { void: true, keepAssets: [SOL] }
        : {
            void: true,
            record:
              fees > 0n ? [{ wallet: job.request.wallet, deploymentId: job.deploymentId, asset: SOL, amount: fees.toString() }] : [],
          };
    }
    const spent = new Map<string, bigint>();
    const add = (asset: string, amount: bigint) => {
      if (amount > 0n) spent.set(asset, (spent.get(asset) ?? 0n) + amount);
    };
    for (const transaction of state.transactions) {
      const amounts = saved.find((entry) => entry.index === transaction.index)?.amounts ?? {};
      if (transaction.signature === null) continue;
      if (!transaction.error) {
        for (const [asset, amount] of Object.entries(amounts)) add(asset, BigInt(amount));
        continue;
      }
      const fee = await chargedFees([transaction.signature]);
      add(SOL, fee ?? BigInt(amounts[SOL] ?? '0'));
    }
    // The rebuilt entries keep the signing time, so a late failure check
    // cannot restart the window for spending that is already ageing out.
    const recordedAt = job.progress.signedAt;
    return {
      void: true,
      record: [...spent].map(([asset, amount]) => ({
        wallet: job.request.wallet,
        deploymentId: job.deploymentId,
        asset,
        amount: amount.toString(),
        ...(recordedAt !== undefined ? { recordedAt } : {}),
      })),
    };
  };

  /** The fees the landed transactions paid, from the chain; `null` when one cannot be read. */
  const chargedFees = async (signatures: readonly (string | null)[]): Promise<bigint | null> => {
    let total = 0n;
    for (const signature of signatures) {
      if (signature === null) continue; // never landed, so never charged
      try {
        const transaction = await simulator.call('getTransaction', [
          signature,
          { encoding: 'json', commitment: 'confirmed', maxSupportedTransactionVersion: 0 },
        ]);
        const fee = (transaction as { meta?: { fee?: unknown } } | null)?.meta?.fee;
        if (typeof fee !== 'number' || !Number.isSafeInteger(fee) || fee < 0) return null;
        total += BigInt(fee);
      } catch (error) {
        report(error, { phase: 'fees' });
        return null;
      }
    }
    return total;
  };

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

  /**
   * The runners for one claim. `worker` here is the claim's own holder
   * token, so a lapsed claim's late writes can never touch the job after
   * another claim (even in this process) took it over.
   */
  const forClaim = (worker: string) => {
    /** Move `job` to a final state and release it. */
    const finish = async (
      job: StepJob,
      state: StepJobState,
      outcome: JobOutcome,
      progress: StepJob['progress'] | undefined = undefined,
      ledger: NonNullable<Parameters<AutomationStore['jobs']['transition']>[1]['ledger']> = { void: true },
    ) => {
      // Spending counts against a window only when the intent landed.
      const landed = state === 'completed';
      const voiding = ledger;
      const moved = await store.jobs.transition(job.id, {
        worker,
        from: job.state,
        to: state,
        now: clock.now(),
        outcome,
        ...(progress ? { progress } : {}),
        ...(landed ? {} : { ledger: voiding }),
      });
      if (!moved) throw new LostLease();
      await notify(outcomeOf(progress ? { ...job, progress: { ...job.progress, ...progress } } : job, state, outcome));
    };

    /** Move `job` to another active state, keeping the lease. */
    const advance = async (
      job: StepJob,
      state: StepJobState,
      progress?: Partial<StepJob['progress']>,
      ledger?: NonNullable<Parameters<AutomationStore['jobs']['transition']>[1]['ledger']>,
    ): Promise<StepJob> => {
      const moved = await store.jobs.transition(job.id, {
        worker,
        from: job.state,
        to: state,
        now: clock.now(),
        keepLease: leaseUntil(),
        ...(progress ? { progress } : {}),
        ...(ledger ? { ledger } : {}),
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

    /** The active grant for a job; instance-scoped grants end with their instance. */
    const consentFor = async (job: StepJob): Promise<ConsentGrant | null> => {
      const grant = await store.consent.find({
        wallet: job.request.wallet,
        deploymentId: job.deploymentId,
        step: job.request.step,
        instanceId: job.request.instance ?? null,
        now: clock.now(),
      });
      if (grant?.instanceId && client.instances) {
        const instance = await client.instances.get(grant.instanceId);
        if (instance.terminal) {
          await store.consent.revoke({
            wallet: grant.wallet,
            deploymentId: grant.deploymentId,
            instanceId: grant.instanceId,
            now: clock.now(),
          });
          return consentFor(job);
        }
      }
      return grant;
    };

    /** The grant's limits against what this step moves, and the window spent so far. */
    const checkLimits = async (job: StepJob, grant: ConsentGrant, outflows: Map<string, bigint>): Promise<Refusal | null> => {
      for (const [asset, amount] of outflows) {
        const limit = grant.limits.perExecution[asset];
        if (limit === undefined) {
          return {
            rule: 'consent_limit',
            reason: `the step moves ${amount} of ${asset} out of the wallet, and the consent sets no per-run limit for it`,
          };
        }
        if (amount > BigInt(limit)) {
          return {
            rule: 'consent_limit',
            reason: `the step moves ${amount} of ${asset} out of the wallet, over the consent's per-run limit of ${limit}`,
          };
        }
        const window = grant.limits.perWindow[asset];
        if (window) {
          const spent = await store.ledger.spent({
            wallet: job.request.wallet,
            deploymentId: job.deploymentId,
            asset,
            since: clock.now() - window.windowMs,
          });
          if (spent + amount > BigInt(window.amount)) {
            return {
              rule: 'consent_window',
              reason: `the step moves ${amount} of ${asset}; with ${spent} already moved in the window that exceeds the consent's limit of ${window.amount}`,
            };
          }
        }
      }
      return null;
    };

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
        { transactionFormat: format, idempotencyKey: prepareIdempotencyKey(job.id, job.progress.generation) },
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
      // Consent can be revoked or changed while the step was prepared and simulated.
      const grant = await consentFor(job);
      if (!grant) {
        return finish(job, 'skipped', { code: 'no_consent', reason: 'consent was revoked before signing' });
      }
      const outflows = stepOutflows(summary);
      const overLimit = await checkLimits(job, grant, outflows);
      if (overLimit) return refuse(job, overLimit);
      // The signer gets copies; the checks below compare against a baseline it never had.
      const baseline = transactions.map(({ index, transactionBase64 }) => ({ index, transactionBase64 }));
      let signed: string[];
      try {
        signed = await signer.signTransactions({
          wallet,
          intent: structuredClone(intent),
          transactions: baseline.map((transaction) => ({ ...transaction })),
        });
      } catch (error) {
        if (isSignerRefusal(error)) return refuse(job, { rule: error.rule, reason: error.message });
        throw error;
      }
      const tampered = checkSignedTransactions({ wallet, intent, prepared: baseline, signed: [...signed] });
      if (tampered) return refuse(job, tampered);
      // Recorded with the signed bytes, before submit, so a crash cannot let a
      // step escape its window; voided if the intent never lands.
      const perTransaction = transactionOutflows(summary).map(({ index, amounts }) => ({
        index,
        amounts: Object.fromEntries([...amounts].map(([asset, amount]) => [asset, amount.toString()])),
      }));
      const saved = await advance(job, 'signed', { format, intent, prepared: baseline, signed, outflows: perTransaction, signedAt: clock.now() }, {
        record: [...outflows].map(([asset, amount]) => ({
          wallet,
          deploymentId: job.deploymentId,
          asset,
          amount: amount.toString(),
        })),
      });
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
            case 'chain_failed': {
              const ledger = await failedLedger(job, state);
              return finish(
                job,
                'chain_failed',
                {
                  code: 'chain_failed',
                  detail: state.transactions.filter((transaction) => transaction.error).map(({ index, error }) => ({ index, error })),
                },
                { ...job.progress, signatures },
                ledger,
              );
            }
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

    /**
     * Whether the transactions an expired intent signed can still land. The
     * chain decides, not the intent: a transaction that landed keeps its
     * spending, and one whose blockhash is still valid could land after a
     * second signing, spending twice. Blockhashes are checked first and the
     * signatures after, on a view at least that recent, so a transaction
     * landing between the two reads is not missed.
     */
    const earlierAttempt = async (job: StepJob): Promise<'landed' | 'live' | 'gone'> => {
      const signed = (job.progress.signed ?? []).map((base64) => decodeTransaction(decodeBase64(base64)!));
      if (signed.length === 0) return 'gone';
      let expiredAt = 0;
      for (const blockhash of new Set(signed.map((transaction) => transaction.recentBlockhash))) {
        const valid = (await simulator.call('isBlockhashValid', [blockhash, { commitment: 'processed' }])) as {
          context?: { slot?: unknown };
          value?: unknown;
        } | null;
        if (valid?.value !== false) return 'live';
        const slot = valid.context?.slot;
        if (typeof slot !== 'number') throw new Error('isBlockhashValid returned no slot');
        expiredAt = Math.max(expiredAt, slot);
      }
      const signatures = signed.map((transaction) => encodeBase58(transaction.signatures[0]!));
      const statuses = (await simulator.call('getSignatureStatuses', [signatures, { searchTransactionHistory: true }])) as {
        context?: { slot?: unknown };
        value?: readonly unknown[];
      } | null;
      if (!Array.isArray(statuses?.value)) throw new Error('getSignatureStatuses returned no statuses');
      if (statuses.value.some((status) => status !== null)) return 'landed';
      const seenAt = statuses.context?.slot;
      // An RPC behind the expiry check could have missed a late landing.
      if (typeof seenAt !== 'number' || seenAt < expiredAt) return 'live';
      return 'gone';
    };

    /**
     * An expired intent is prepared again once, with a new idempotency key,
     * after the chain shows its transactions can no longer land. A second
     * expiry ends the job, keeping the spending of anything that landed.
     */
    const prepareAgain = async (job: StepJob): Promise<void> => {
      let earlier: 'landed' | 'live' | 'gone';
      try {
        earlier = await earlierAttempt(job);
      } catch (error) {
        report(error, { jobId: job.id, phase: 'expired' });
        return park(job, backoff(job.attempts + 1), job.attempts + 1);
      }
      if (earlier === 'landed') {
        // Keep its spending; never sign the step again.
        return finish(
          job,
          'failed',
          { code: 'landed_after_expiry', reason: 'the intent expired, but a transaction it signed landed on chain' },
          undefined,
          {},
        );
      }
      // Still able to land: wait for its blockhash to expire first.
      if (earlier === 'live') return park(job, backoff(job.attempts + 1), job.attempts + 1);
      if (job.progress.generation >= 1) {
        return finish(job, 'expired', { code: 'expired', reason: 'the intent expired again after preparing it a second time' });
      }
      const fresh = await advance(
        job,
        'running',
        { generation: job.progress.generation + 1, intent: undefined, prepared: undefined, signed: undefined, signatures: undefined },
        { void: true },
      );
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
        consents: {
          async list(query = {}) {
            const grants = await store.consent.active({
              deploymentId: client.deploymentId,
              ...(query.step !== undefined ? { step: query.step } : {}),
              now: clock.now(),
            });
            if (query.instanceId === undefined) return grants;
            // One effective grant per wallet: the instance's own grant wins over
            // one for every instance, as consent checks choose at run time.
            const effective = new Map<string, ConsentGrant>();
            for (const grant of grants) {
              if (grant.instanceId !== null && grant.instanceId !== query.instanceId) continue;
              const current = effective.get(grant.wallet);
              if (!current || (current.instanceId === null && grant.instanceId !== null)) effective.set(grant.wallet, grant);
            }
            return [...effective.values()];
          },
        },
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

    return { runStep, runDelivery };
  };

  // -------------------------------------------------------------------------
  // Workers
  // -------------------------------------------------------------------------

  let claims = 0;
  const runOnce = async () => {
    await bound();
    claims += 1;
    const holder = `${worker}#${claims}`;
    const job: Job | null = await store.jobs.claimNext({ worker: holder, leaseMs, now: clock.now() });
    if (!job) return false;
    const runner = forClaim(holder);
    if (job.kind === 'delivery') await runner.runDelivery(job);
    else await runner.runStep(job);
    return true;
  };

  let running = false;
  let loops: Promise<void>[] = [];

  // -------------------------------------------------------------------------
  // View sources
  // -------------------------------------------------------------------------

  const ENDED = Symbol('ended');

  /** One watch attempt: its iterator, and a promise that settles when it ends. */
  interface WatchAttempt {
    active: boolean;
    iterator?: AsyncIterator<unknown>;
    ended: Promise<typeof ENDED>;
    end: () => void;
  }

  const newAttempt = (): WatchAttempt => {
    let end = () => undefined as void;
    const ended = new Promise<typeof ENDED>((resolve) => {
      end = () => resolve(ENDED);
    });
    return { active: true, ended, end };
  };

  /** Close an attempt: no further work of it runs, and its stream is closed. */
  const closeAttempt = async (attempt: WatchAttempt) => {
    attempt.active = false;
    attempt.end();
    const iterator = attempt.iterator;
    attempt.iterator = undefined;
    try {
      await iterator?.return?.();
    } catch {
      // Closing a stream that already failed.
    }
  };

  /** `promise`, unless the attempt ends first. */
  const within = <T>(attempt: WatchAttempt, promise: Promise<T>): Promise<T | typeof ENDED> =>
    Promise.race([promise, attempt.ended]);

  const watchList = async (source: ListSource, attempt: WatchAttempt) => {
    // Signals are evaluated against the rows as of each event, in order.
    const latest = new Map<string, [unknown, bigint | null]>();
    const rowsByKey = new Map<string, unknown>();
    let rows: readonly unknown[] = [];
    const waiting = new Set<string>();
    const current = (key: string): [unknown, bigint | null] | undefined => {
      if (source.rowSignal) return latest.get(key);
      const signal = source.signals(rows).find(([candidate]) => candidate === key);
      return signal ? [signal[1], signal[2]] : undefined;
    };
    const evaluate = async (key: string, value: unknown, observed: bigint | null): Promise<void> => {
      const result = await evaluateSignal(store, clock, source, key, value, observed);
      if (result === 'pending' && !waiting.has(key)) {
        waiting.add(key);
        void clock.sleep(source.stableForMs).then(() => settle(key));
      }
    };
    // A pending value is promoted when its timer fires; a failed write is
    // retried until it lands or the attempt ends.
    const settle = async (key: string) => {
      waiting.delete(key);
      for (let failures = 0; attempt.active; failures += 1) {
        const signal = current(key);
        if (!signal) return;
        try {
          await evaluate(key, signal[0], signal[1]);
          return;
        } catch (error) {
          report(error, { phase: 'source' });
          if ((await within(attempt, clock.sleep(backoff(failures + 1)))) === ENDED) return;
        }
      }
    };
    // Read the stream into a local, unbounded queue from the moment it opens:
    // the SDK's own buffer is bounded and drops list updates silently.
    const iterator = source.view.watchRich({ withSnapshot: true })[Symbol.asyncIterator]();
    attempt.iterator = iterator;
    const queued: RichUpdate<unknown>[] = [];
    const waiter: { wake: (() => void) | null } = { wake: null };
    let finished = false;
    let failure: unknown = null;
    void (async () => {
      try {
        for (;;) {
          const next = await within(attempt, iterator.next());
          if (next === ENDED || next.done) break;
          queued.push(next.value);
          waiter.wake?.();
        }
      } catch (error) {
        failure = error;
      } finally {
        finished = true;
        waiter.wake?.();
      }
    })();
    const take = async (): Promise<RichUpdate<unknown> | null> => {
      while (queued.length === 0 && !finished) {
        await new Promise<void>((resolve) => {
          waiter.wake = resolve;
        });
        waiter.wake = null;
      }
      const update = queued.shift();
      if (update !== undefined) return update;
      if (failure !== null) throw failure;
      return null;
    };
    // Seed from the complete current rows: a query that is already open
    // replays nothing on a new stream, so the stream alone cannot be the
    // baseline.
    const snapshot = await within(attempt, source.view.get());
    if (snapshot === ENDED || !attempt.active) return;
    const rowKeys = new Map<string, string>();
    for (const row of snapshot) rowsByKey.set(source.rowKey(row), row);
    /** Apply an update; the row it leaves, or `undefined` for a removal. */
    /** A row's signal is forgotten, so no pending `stableFor` change of it can still fire. */
    const forget = (key: string) => {
      rowsByKey.delete(key);
      latest.delete(key);
    };
    // A removal of a row the stream never announced (seeded from the
    // snapshot) that carries no `lastKnown`: its key cannot be derived.
    let unmatchedRemoval = false;
    /** Apply an update: the row and its key, or `undefined` for a removal. */
    const apply = (update: RichUpdate<unknown>): { row: unknown; key: string } | undefined => {
      if (update.type === 'created' || update.type === 'updated') {
        const row = update.type === 'created' ? update.data : update.after;
        const key = source.rowKey(row);
        rowKeys.set(update.key, key);
        rowsByKey.set(key, row);
        return { row, key };
      }
      const key = update.lastKnown !== undefined ? source.rowKey(update.lastKnown) : rowKeys.get(update.key);
      if (key !== undefined) forget(key);
      else unmatchedRemoval = true;
      rowKeys.delete(update.key);
      return undefined;
    };
    /** Drop the rows the view no longer has, reading it when it has no synchronous copy. */
    const reconcileRemoval = async (): Promise<boolean> => {
      unmatchedRemoval = false;
      const current = source.view.getSync?.() ?? (await within(attempt, source.view.get()));
      if (current === ENDED) return false;
      const present = new Set(current.map((row) => source.rowKey(row)));
      for (const key of [...rowsByKey.keys()]) if (!present.has(key)) forget(key);
      return true;
    };
    /**
     * After a reconnect the SDK replaces the list with a new snapshot and
     * announces only the rows still present; drop the ones the view no
     * longer has.
     */
    const pruneGone = (): number => {
      const current = source.view.getSync?.();
      if (!current) return 0;
      const present = new Set(current.map((row) => source.rowKey(row)));
      let removed = 0;
      for (const key of [...rowsByKey.keys()]) {
        if (!present.has(key)) {
          forget(key);
          removed += 1;
        }
      }
      return removed;
    };
    const rowJson = (row: unknown) => JSON.stringify(row, (_key, value) => (typeof value === 'bigint' ? value.toString() : value));
    // Rows a reconnect's rebuild already took in, until the stream announces them.
    const resynced = new Map<string, string>();
    // Let the reader drain what the stream already delivered (buffered
    // events resolve as microtasks, before this timer), then apply it in order
    // without evaluating: it is already reflected in the snapshot, and
    // replaying it leaves every row at its latest value.
    if ((await within(attempt, new Promise<void>((resolve) => setTimeout(resolve, 0)))) === ENDED) return;
    while (queued.length > 0) apply(queued.shift()!);
    // A removal that arrived after the snapshot was read may name a row only
    // the snapshot holds.
    if (unmatchedRemoval && !(await reconcileRemoval())) return;
    rows = [...rowsByKey.values()];
    for (const [key, value, observed] of source.signals(rows)) {
      if (!attempt.active) return;
      if (source.rowSignal) latest.set(key, [value, observed]);
      await evaluate(key, value, observed);
    }
    while (attempt.active) {
      const update = await take();
      if (update === null || !attempt.active) return;
      if (!source.rowSignal && update.type === 'created') {
        const key = source.rowKey(update.data);
        if (resynced.get(key) === rowJson(update.data)) {
          // Part of a reconnect already evaluated as a whole.
          resynced.delete(key);
          rowKeys.set(update.key, key);
          continue;
        }
        const announcedAgain = rowsByKey.has(key);
        apply(update);
        const current = source.view.getSync?.();
        if ((announcedAgain || pruneGone() > 0) && current) {
          // A reconnect: the SDK replaced the whole list and now announces it
          // row by row. Evaluate the new list once, as a whole, not a mix of
          // old and new rows.
          rowsByKey.clear();
          resynced.clear();
          for (const row of current) {
            const rowKey = source.rowKey(row);
            rowsByKey.set(rowKey, row);
            if (rowKey !== key) resynced.set(rowKey, rowJson(row));
          }
        }
        rows = [...rowsByKey.values()];
        for (const [signalKey, value, observed] of source.signals(rows)) {
          if (!attempt.active) return;
          await evaluate(signalKey, value, observed);
        }
        continue;
      }
      const applied = apply(update);
      if (update.type === 'created') pruneGone();
      if (unmatchedRemoval && !(await reconcileRemoval())) return;
      rows = [...rowsByKey.values()];
      if (source.rowSignal) {
        if (applied === undefined) continue;
        const signal = source.rowSignal(applied.row);
        if (!signal) {
          // It no longer passes the filter: its old signal must not fire.
          latest.delete(applied.key);
          continue;
        }
        latest.set(signal[0], [signal[1], signal[2]]);
        await evaluate(signal[0], signal[1], signal[2]);
        continue;
      }
      for (const [key, value, observed] of source.signals(rows)) {
        if (!attempt.active) return;
        await evaluate(key, value, observed);
      }
    }
  };

  const watchAppend = async (source: AppendSource, attempt: WatchAttempt) => {
    const cursor = await within(attempt, store.sources.cursor(source.name));
    if (cursor === ENDED || !attempt.active) return;
    const iterator = source.view.watch(cursor ? { after: cursor } : {})[Symbol.asyncIterator]();
    attempt.iterator = iterator;
    while (attempt.active) {
      const next = await within(attempt, iterator.next());
      if (next === ENDED || next.done || !attempt.active) return;
      const update = next.value;
      if (update.type !== 'upsert') continue;
      if (!update.cursor) throw new Error(`${source.name}: the view is not an append view (its updates carry no cursor)`);
      if (!source.filter || source.filter(update.data)) {
        const payload: AppendTrigger = { source: source.name, cursor: update.cursor, row: update.data };
        await accept({
          id: `view:${encodeURIComponent(source.name)}:${update.cursor}`,
          name: source.name,
          source: 'view',
          payload,
        });
      }
      await store.sources.putCursor(source.name, update.cursor, clock.now());
    }
  };

  const startSource = (source: ViewSource, sourceOptions: { exclusive?: boolean } = {}): SourceHandle => {
    let stopped = false;
    const stopping = newAttempt();
    let attempt: WatchAttempt | null = null;
    let holding = !sourceOptions.exclusive;
    const pause = (ms: number) => within(stopping, clock.sleep(ms));
    const acquire = async () => {
      try {
        return await store.sources.acquireLease(source.name, worker, clock.now() + leaseMs, clock.now());
      } catch (error) {
        report(error, { phase: 'source' });
        return false;
      }
    };
    const keepLease = async () => {
      while (!stopped && holding) {
        if ((await pause(Math.max(1, Math.floor(leaseMs / 3)))) === ENDED) return;
        holding = await acquire();
        if (!holding && attempt) await closeAttempt(attempt);
      }
    };
    const loop = (async () => {
      let failures = 0;
      while (!stopped) {
        if (sourceOptions.exclusive && !holding) {
          holding = await acquire();
          if (!holding) {
            await pause(Math.max(1, Math.floor(leaseMs / 2)));
            continue;
          }
          void keepLease();
        }
        const current = newAttempt();
        attempt = current;
        try {
          await bound();
          if (source.kind === 'list') await watchList(source, current);
          else await watchAppend(source, current);
          failures = 0;
        } catch (error) {
          if (!stopped) {
            report(error, { phase: 'source' });
            failures += 1;
          }
        } finally {
          await closeAttempt(current);
        }
        if (!stopped) await pause(failures > 0 ? backoff(failures) : Math.min(reconcileMs, 1000));
      }
    })();
    return {
      async stop() {
        stopped = true;
        stopping.end();
        if (attempt) await closeAttempt(attempt);
        await loop;
        if (sourceOptions.exclusive) {
          await store.sources.releaseLease(source.name, worker).catch((error: unknown) => report(error, { phase: 'source' }));
        }
      },
    };
  };

  const domain = () => {
    if (!options.consent?.domain) {
      throw new ConsentError('no_domain', 'createAutomations needs consent.domain to build consent proofs');
    }
    return options.consent.domain;
  };

  const consent: ConsentApi = {
    async proofMessage(fields) {
      await bound();
      const normalized = normalizeConsent(fields);
      if (normalized.deploymentId !== client.deploymentId) {
        throw new ConsentError('invalid_fields', `consent is for ${normalized.deploymentId}, this client runs ${client.deploymentId}`);
      }
      const random = new Uint8Array(16);
      (globalThis.crypto as Crypto).getRandomValues(random);
      const nonce = encodeBase58(random);
      const issuedAt = clock.now();
      const message = consentMessage(domain(), normalized, nonce, issuedAt);
      await store.consent.issueNonce(nonce, issuedAt + proofTtlMs);
      return { message, bytes: new TextEncoder().encode(message), nonce, issuedAt };
    },
    async grant(input) {
      await bound();
      const now = clock.now();
      const normalized = normalizeConsent(input);
      if (normalized.deploymentId !== client.deploymentId) {
        throw new ConsentError('invalid_fields', `consent is for ${normalized.deploymentId}, this client runs ${client.deploymentId}`);
      }
      if (normalized.expiresAt <= now) throw new ConsentError('expired', 'the consent has already expired');
      if (!Number.isSafeInteger(input.issuedAt) || input.issuedAt > now + 60_000 || now - input.issuedAt > proofTtlMs) {
        throw new ConsentError('proof_stale', 'the proof was issued too long ago; build and sign a new one');
      }
      const message = consentMessage(domain(), normalized, input.nonce, input.issuedAt);
      const signature = typeof input.signature === 'string' ? decodeBase58(input.signature) : input.signature;
      const walletKey = decodeBase58(normalized.wallet);
      let valid = false;
      try {
        valid =
          signature !== null &&
          signature.length === 64 &&
          walletKey !== null &&
          ed25519.verify(signature, new TextEncoder().encode(message), walletKey);
      } catch {
        valid = false;
      }
      if (!valid) {
        throw new ConsentError('invalid_signature', 'the signature is not the wallet\'s over this consent');
      }
      if (!(await store.consent.useNonce(input.nonce, now))) {
        throw new ConsentError('nonce_not_usable', 'the nonce was not issued, has expired, or was already used');
      }
      const grant: ConsentGrant = {
        ...normalized,
        grantedAt: now,
        revokedAt: null,
        proof: { message, signature: encodeBase58(signature!), nonce: input.nonce, issuedAt: input.issuedAt },
      };
      await store.consent.put(grant);
      const warnings: string[] = [];
      let status: Awaited<ReturnType<NonNullable<ProtocolSigner['delegationStatus']>>> | undefined;
      try {
        status = signer.delegationStatus ? await signer.delegationStatus(normalized.wallet) : undefined;
      } catch (error) {
        // The grant is stored; a provider outage must not report it as failed.
        warnings.push(`the ${signer.kind} signer could not report its delegation: ${String((error as Error)?.message ?? error)}`);
        return { grant, warnings };
      }
      if (!status) {
        warnings.push(`the ${signer.kind} signer cannot report whether a provider policy bounds what it signs`);
      } else {
        if (!status.delegated) warnings.push(`the ${signer.kind} signer has no delegated access to ${normalized.wallet}`);
        if (!status.providerPolicy) {
          warnings.push(`no provider policy bounds what the ${signer.kind} signer may sign for ${normalized.wallet}`);
        }
      }
      return { grant, warnings };
    },
    async revoke(input) {
      await bound();
      return store.consent.revoke({ ...input, deploymentId: client.deploymentId, now: clock.now() });
    },
    async list(input) {
      await bound();
      return store.consent.list(input);
    },
  };

  const automations: Automations<TTypes> = {
    consent,
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
      let stopped = false;
      let queue: Promise<void> = Promise.resolve();
      // The stream moves past an event before its handler runs, so an event
      // whose accept fails is retried here, in order, until it is stored.
      const acceptUntilStored = async (delivery: Omit<Delivery, 'receivedAt'>) => {
        for (let failures = 0; !stopped && !followOptions.signal?.aborted; failures += 1) {
          try {
            await accept(delivery);
            return;
          } catch (error) {
            report(error, { phase: 'stream' });
            await clock.sleep(Math.min(1000 * 2 ** failures, 30_000));
          }
        }
      };
      const subscription = client.events(instanceId, {
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
          const delivery = {
            id: `stream:${encodeURIComponent(data.instanceId)}:${data.id}`,
            name: streamEvent.event,
            source: 'stream' as const,
            payload: event,
          };
          queue = queue.then(() => acceptUntilStored(delivery));
        },
        onError(error) {
          report(error, { phase: 'stream' });
        },
      });
      return {
        get lastEventId() {
          return subscription.lastEventId;
        },
        get closed() {
          return subscription.closed;
        },
        close() {
          stopped = true;
          subscription.close();
        },
      };
    },
    source: startSource,
    async use(definition, useOptions = {}) {
      // A definition for another protocol would register handlers for events
      // this deployment never emits.
      const own = client.definition?.name;
      if (own !== undefined && definition.protocol.name !== own) {
        throw new Error(`the definition is for protocol ${definition.protocol.name}, but the client is for ${own}`);
      }
      const setup: AutomationsSetup<TTypes> = {
        on: ((name: string, handler: Handler<any, TTypes>) => automations.on(name, handler)) as never,
        source: startSource,
        connect: useOptions.connect ?? connectStack,
      };
      await definition.setup(setup);
      return automations;
    },
    async poll(source) {
      if (source.kind !== 'list') throw new TypeError('only list-view sources can be polled');
      await bound();
      let delivered = 0;
      let pending = 0;
      for (const [key, value, observed] of source.signals(await source.view.get())) {
        const result = await evaluateSignal(store, clock, source, key, value, observed);
        if (result === 'delivered') delivered += 1;
        if (result === 'pending') pending += 1;
      }
      return { delivered, pending };
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
