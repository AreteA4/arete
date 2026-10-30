import { isApiCredentialAuth, withSessionMintEndpoint } from '../api-credential';
import { ARETE_API_URL_ENV, withTrustedApiUrl } from '../credential-destination';
import { classifyApiKey, isBrowserEnvironment, readEnvironmentVariable, resolveAuthConfig } from '../auth-keys';
import { ConnectionManager } from '../connection';
import {
  createDevServiceFetch,
  createEdgeFetch,
  pathSegment,
  resolveEnvironmentSwitch,
  systemEnvironmentSwitchHost,
  type ClientEnvironment,
  type DevEnvironmentSwitch,
  type EnvironmentOption,
} from '../dev-environment';
import {
  protocolIdentity,
  protocolTarget,
  resolveDevTargets,
  type DevProtocolTarget,
} from '../dev-targets';
import {
  AreteError,
  parseErrorCode,
  parseWireErrorCode,
  shouldRefreshToken,
  type AuthConfig,
  type AuthTokenRequest,
} from '../types';
import {
  protocolMetadataProofMessage,
  type ProtocolMetadataProofMessage,
  type ProtocolMetadataWalletProof,
} from './metadata-proof';
import type { ProtocolExecutionReceipt, ProtocolReceiptKeySet } from './receipt';
import { ServerSentEventParser, type ServerSentEvent } from './sse';
import {
  PROTOCOL_ACCESS_REJECTION_CODES,
  PROTOCOL_STATE_REJECTION_CODES,
  SQUADS_PROPOSER_MISSING_EVENT,
  type AnyProtocolDefinition,
  type ExecutionIntentV1,
  type ProtocolAccessRejectionCode,
  type ProtocolDefinition,
  type ProtocolInstanceDetail,
  type ProtocolInstancePage,
  type ProtocolIntentState,
  type ProtocolKeyClass,
  type ProtocolParticipantDetail,
  type ProtocolPrepareResult,
  type ProtocolProgramsOf,
  type ProtocolSquadsProposer,
  type ProtocolSquadsProposerMissingData,
  type ProtocolSquadsStatus,
  type ProtocolStacksOf,
  type ProtocolStateRejectionCode,
  type ProtocolStepAvailabilityFor,
  type ProtocolStepTypes,
  type ProtocolStreamEvent,
  type ProtocolTransactionFormat,
  type ProtocolTypeMap,
  type ProtocolTypesOf,
  type ProtocolWalletActions,
  type ProtocolWalletInstanceActions,
  type ProtocolWalletRelation,
} from './types';

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** Session scopes a protocol client asks for. */
export const PROTOCOL_READ_SCOPE = 'protocol:read';
export const PROTOCOL_SUBMIT_SCOPE = 'protocol:submit';
export const PROTOCOL_PREPARE_ANY_SCOPE = 'protocol:prepare:*';

/** The most instances one wallet actions page returns. */
const WALLET_ACTIONS_PAGE_LIMIT = 50;

export interface ProtocolClientOptions<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  /**
   * `publishableKey` in browsers (origin-bound, `public` steps only) or
   * `secretKey` on servers (every client-callable step). `token`, `getToken`,
   * `tokenEndpoint` and the `ARETE_API_KEY` fallback work too.
   */
  auth?: AuthConfig;
  /**
   * The key class to type the client as when it cannot be read from `auth`
   * (`token`, `getToken`, `tokenEndpoint`, `ARETE_API_KEY`). Defaults to
   * `publishable`, which exposes only `public` steps.
   */
  keyClass?: ProtocolKeyClass;
  /**
   * Steps a publishable-key client may prepare. A session that can prepare is
   * minted on the first prepare or submit and covers these steps; defaults
   * to every `public` step. Secret-key clients may prepare every step.
   */
  steps?: readonly Extract<keyof TTypes['steps'], string>[];
  fetch?: typeof fetch;
  /**
   * The protocol API, `<origin>/api`, instead of the generated one. Outside
   * browsers `ARETE_API_URL` (the API origin, e.g. `https://api.arete.run`)
   * is the default when this is unset, so one generated client runs against
   * another API by environment alone. The session endpoint follows:
   * `<origin>/ws/sessions`.
   */
  apiUrl?: string;
  /** The session endpoint, instead of the one derived from `apiUrl` or generated. */
  sessionEndpoint?: string;
  /**
   * Call this deployment instead of the generated `deploymentId` (advanced:
   * another deployment of the same release). The generated client already
   * carries the installed deployment (the dev environment's, while `a4 dev`
   * generates it).
   */
  deploymentId?: string;
  /**
   * Production (the default) or the project's dev environment (see the
   * stack client's `environment` option). In dev mode the environment's
   * deployment of this protocol is called, through the `a4 dev up` edge or
   * directly with a secret key; `apiUrl`, `sessionEndpoint` and
   * `deploymentId` cannot be combined with it.
   */
  environment?: EnvironmentOption;
}

/** Environment variable read (outside browsers) for the API origin. */
export { ARETE_API_URL_ENV };

/**
 * The deployment, protocol API and session endpoint a client calls: options
 * first, then `ARETE_API_URL` outside browsers, then the generated
 * descriptor. An `apiUrl` without a `sessionEndpoint` carries its origin's
 * session endpoint, so overriding the API never mints sessions elsewhere.
 */
export function resolveProtocolEndpoints(
  deployment: { deploymentId: string; apiUrl: string; sessionEndpoint: string },
  options?: Pick<ProtocolClientOptions, 'apiUrl' | 'sessionEndpoint' | 'deploymentId'>,
  environment: (name: string) => string | undefined = (name) =>
    isBrowserEnvironment() ? undefined : readEnvironmentVariable(name),
): { deploymentId: string; apiUrl: string; sessionEndpoint: string } {
  const deploymentId = options?.deploymentId?.trim() || deployment.deploymentId;
  const optionApi = options?.apiUrl?.trim();
  const origin = optionApi ? undefined : trimSlash(environment(ARETE_API_URL_ENV)?.trim() ?? '');
  let apiUrl = deployment.apiUrl;
  let sessionEndpoint = deployment.sessionEndpoint;
  if (optionApi) {
    apiUrl = optionApi;
    sessionEndpoint = `${trimSlash(optionApi).replace(/\/api$/, '')}/ws/sessions`;
  } else if (origin) {
    apiUrl = `${origin}/api`;
    sessionEndpoint = `${origin}/ws/sessions`;
  }
  return {
    deploymentId,
    apiUrl: trimSlash(apiUrl),
    sessionEndpoint: options?.sessionEndpoint?.trim() || sessionEndpoint,
  };
}

/** The key class a client created with `TOptions` is typed as. */
export type ProtocolKeyClassOf<TOptions> =
  TOptions extends { readonly keyClass: infer TKeyClass extends ProtocolKeyClass }
    ? TKeyClass
    : TOptions extends { readonly auth: { readonly secretKey: unknown } }
      ? 'secret'
      : 'publishable';

export interface ProtocolPrepareOptions {
  /** `v1` (default) or `v0`; `v0` may yield several transactions. */
  transactionFormat?: ProtocolTransactionFormat;
  /** Reuse to make a retried prepare return the same intent. Generated when absent. */
  idempotencyKey?: string;
  clientContext?: { requestId?: string };
  signal?: AbortSignal;
}

/** What a step's `prepare` takes: the wallet, the instance for non-create steps, and the args. */
export type ProtocolStepPrepareInput<TStep extends ProtocolStepTypes> =
  (TStep['scope'] extends 'create'
    ? { readonly wallet: string }
    : { readonly instance: string; readonly wallet: string })
  & TStep['args'];

export interface ProtocolStepClient<TStep extends ProtocolStepTypes = ProtocolStepTypes> {
  readonly name: string;
  readonly scope: TStep['scope'];
  readonly access: TStep['access'];
  /**
   * Ask the hosted API to build the step's transactions. Resolves with the
   * execution intent and the partially signed transactions for the wallet to
   * sign, or with the rejection (a guard code or an access code).
   */
  prepare(
    input: ProtocolStepPrepareInput<TStep>,
    options?: ProtocolPrepareOptions
  ): Promise<ProtocolPrepareResult<TStep['guardCode']>>;
}

/**
 * The steps a client can call: every client-callable step for a secret-key
 * client, only the `public` ones for a publishable-key client.
 */
export type ProtocolStepsClient<TTypes extends ProtocolTypeMap, TKeyClass extends ProtocolKeyClass> = {
  readonly [K in keyof TTypes['steps'] as TKeyClass extends 'secret'
    ? K
    : TTypes['steps'][K]['access'] extends 'public'
      ? K
      : never]: ProtocolStepClient<TTypes['steps'][K]>;
};

export interface ProtocolInstanceListFilter<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  state?: TTypes['instanceState'];
  /** Exact-match filters on `filterable` instance metadata fields. */
  metadata?: Partial<Record<Extract<keyof TTypes['instanceMetadata'], string>, string>>;
  cursor?: string;
  limit?: number;
  signal?: AbortSignal;
}

export interface ProtocolInstancesForWalletOptions {
  /** Only instances the wallet relates to this way. */
  relation?: ProtocolWalletRelation;
  signal?: AbortSignal;
}

export interface ProtocolWalletActionsInput {
  /** The wallet to evaluate steps for. */
  wallet: string;
  /** Only this instance, instead of every instance the wallet relates to. */
  instance?: string;
  /** `nextCursor` from the previous page. */
  cursor?: string;
  /** Instances per page, at most 50. */
  limit?: number;
  signal?: AbortSignal;
}

export interface ProtocolInstanceGetOptions {
  /** Evaluate `allowedSteps` for this wallet. */
  wallet?: string;
  signal?: AbortSignal;
}

/** What an instance handle holds: the canonical instance and its live stack row. */
export interface ProtocolInstanceSnapshot<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly instance: ProtocolInstanceDetail<TTypes>;
  /**
   * The protocol stack's row for this instance, when the definition names a
   * live stack and that stack is connected (inside a session). Otherwise
   * `undefined`.
   */
  readonly live: unknown | undefined;
}

/**
 * A reactive view of one instance, refreshed from the hosted API whenever the
 * instance's event stream reports a change. Works with `useSyncExternalStore`:
 * it starts on the first `subscribe` and stops when the last listener leaves.
 */
export interface ProtocolInstanceHandle<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly instanceId: string;
  get(): ProtocolInstanceSnapshot<TTypes> | undefined;
  /** The last read or stream failure, cleared by the next successful read. */
  getError(): Error | undefined;
  subscribe(listener: () => void): () => void;
  refresh(): Promise<ProtocolInstanceSnapshot<TTypes> | undefined>;
  close(): void;
}

export type ProtocolMetadataPatch<TMetadata> = {
  [K in keyof TMetadata]?: TMetadata[K] | null;
};

export interface ProtocolMetadataState<TMetadata> {
  readonly metadata: Partial<TMetadata>;
  readonly version: number | null;
}

export interface ProtocolMetadataUpdateOptions {
  /** Proof for participant-writer fields; see `protocolMetadataProof`. */
  walletProof?: ProtocolMetadataWalletProof;
  /** Update this participant's metadata instead of the instance's. */
  participant?: string;
  signal?: AbortSignal;
}

export interface ProtocolMetadataUpdateResult<TMetadata> {
  readonly instanceId: string;
  readonly participantId: string | null;
  readonly version: number;
  readonly metadata: Partial<TMetadata>;
}

export interface ProtocolMetadataHandle<TMetadata> {
  get(): Promise<ProtocolMetadataState<TMetadata>>;
  /** Apply a JSON merge patch (`null` removes a field). */
  update(
    patch: ProtocolMetadataPatch<TMetadata>,
    options?: ProtocolMetadataUpdateOptions
  ): Promise<ProtocolMetadataUpdateResult<TMetadata>>;
  /**
   * The message `wallet` signs to authorise `patch`. `expiresAt` defaults to
   * four minutes from now.
   */
  proofMessage(
    patch: ProtocolMetadataPatch<TMetadata>,
    options: { wallet: string; expiresAt?: number }
  ): ProtocolMetadataProofMessage;
}

export interface ProtocolEventsOptions<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  /** Resume after this event id. */
  lastEventId?: string;
  onEvent(event: ProtocolStreamEvent<TTypes>): void;
  /**
   * Stream failures. The subscription keeps reconnecting unless the hosted
   * API refuses it outright (for example the instance does not exist).
   */
  onError?(error: Error): void;
  signal?: AbortSignal;
}

export interface ProtocolEventSubscription {
  /** The id of the last event received; reconnects resume after it. */
  readonly lastEventId: string | undefined;
  readonly closed: boolean;
  close(): void;
}

/**
 * Squads onboarding. Admin steps are proposed to a team's Squads v4 multisig
 * by a proposer key that belongs to the calling account and may only
 * initiate. Arete never votes or executes, and builds no Squads
 * transactions: the team adds the proposer on the dashboard, in the Squads
 * app, or from code with the squads-v4 program SDK.
 */
export interface ProtocolSquadsClient {
  /** The calling account's proposer and the permission it needs. */
  proposer(options?: { signal?: AbortSignal }): Promise<ProtocolSquadsProposer>;
  /** Whether the proposer is an Initiate-only member of `multisig`. */
  status(multisig: string, options?: { signal?: AbortSignal }): Promise<ProtocolSquadsStatus>;
}

export interface ProtocolClient<
  TTypes extends ProtocolTypeMap = ProtocolTypeMap,
  TKeyClass extends ProtocolKeyClass = ProtocolKeyClass,
  TStacks = Record<string, unknown>,
  TPrograms = Record<string, unknown>,
> {
  readonly definition: ProtocolDefinition<TTypes, any, any>;
  /**
   * The deployment called. In dev mode through the edge it is the installed
   * one (empty before the first publish: the edge finds the environment's
   * deployment by package); directly, the environment's once resolved.
   */
  readonly deploymentId: string;
  /** `'dev'` when this client calls the project's dev environment. */
  readonly environment: ClientEnvironment;
  /**
   * The deployment this client calls. In production it is `deploymentId`.
   * In dev mode it is the environment's deployment of the protocol, asked
   * of the edge or of resolve once; `deploymentId` names it afterwards.
   * Metadata proofs and automations need it in dev mode.
   */
  resolveDeployment(options?: { signal?: AbortSignal }): Promise<string>;
  /** The key class this client authenticates with at runtime. */
  readonly keyClass: TKeyClass;
  readonly steps: ProtocolStepsClient<TTypes, TKeyClass>;
  readonly instances: {
    list(filter?: ProtocolInstanceListFilter<TTypes>): Promise<ProtocolInstancePage<TTypes['instanceState']>>;
    get(id: string, options?: ProtocolInstanceGetOptions): Promise<ProtocolInstanceDetail<TTypes>>;
    use(id: string, options?: { wallet?: string }): ProtocolInstanceHandle<TTypes>;
    /**
     * Every instance `wallet` relates to, read through all pages of
     * {@link ProtocolClient.actions}. With `relation`, only the instances it
     * relates to that way: `creator` for the ones it created, `role:<name>`
     * for the ones where it holds that role, `participant` for the ones it
     * joined.
     */
    forWallet(
      wallet: string,
      options?: ProtocolInstancesForWalletOptions
    ): Promise<ProtocolWalletInstanceActions<TTypes>[]>;
  };
  readonly participants: {
    get(input: { instance: string; wallet: string; signal?: AbortSignal }): Promise<ProtocolParticipantDetail<TTypes>>;
  };
  /** Which steps `wallet` can call on `instance` now, and why not. */
  allowedSteps(input: {
    instance: string;
    wallet: string;
    signal?: AbortSignal;
  }): Promise<ProtocolStepAvailabilityFor<TTypes>[]>;
  /**
   * Which steps `wallet` can call now across the deployment: create steps and
   * the instances it relates to (as a participant, creator or role holder),
   * each with its availability, the reason it is not allowed, and the step's
   * typed arguments. Create steps are evaluated without an instance, so
   * guards that read the arguments or the instance are listed in
   * `argsDependent`. Instances are paged; pass `nextCursor` back as `cursor`.
   */
  actions(input: ProtocolWalletActionsInput): Promise<ProtocolWalletActions<TTypes>>;
  /** Submit the wallet-signed transactions of a prepared intent. */
  submit(
    intent: ExecutionIntentV1 | string,
    signedTransactions: readonly (string | Uint8Array)[]
  ): Promise<ProtocolIntentState>;
  readonly intents: {
    get(executionId: string): Promise<ProtocolIntentState>;
  };
  /**
   * The signed execution receipt of a transaction this deployment landed.
   * Check it with `verifyReceipt` against `getReceiptKeys()` and with
   * `verifyReceiptOnChain` against the transaction from any RPC. Throws with
   * status 404 and code `protocol-receipt-not-found` when there is none.
   */
  getReceipt(signature: string, options?: { signal?: AbortSignal }): Promise<ProtocolExecutionReceipt>;
  /** The published receipt keys, including retired ones. */
  getReceiptKeys(options?: { signal?: AbortSignal }): Promise<ProtocolReceiptKeySet>;
  readonly metadata: {
    instance(id: string): ProtocolMetadataHandle<TTypes['instanceMetadata']>;
    participant(input: { instance: string; wallet: string }): ProtocolMetadataHandle<TTypes['participantMetadata']>;
  };
  /** Follow an instance's event stream; reconnects and resumes transparently. */
  events(instanceId: string, options: ProtocolEventsOptions<TTypes>): ProtocolEventSubscription;
  /** Bring-your-own Squads multisig onboarding for the calling account. */
  readonly squads: ProtocolSquadsClient;
  /** The protocol's stacks: definitions, or connected clients inside a session. */
  readonly stacks: TStacks;
  /** The protocol's programs: definitions, or connected programs inside a session. */
  readonly programs: TPrograms;
  close(): void;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ACCESS_CODES = new Set<string>(PROTOCOL_ACCESS_REJECTION_CODES);
const STATE_CODES = new Set<string>(PROTOCOL_STATE_REJECTION_CODES);
const STREAM_EVENT_TYPES = new Set(['transition', 'participant', 'observation']);
const MAX_RECONNECT_DELAY_MS = 30_000;
const DEFAULT_PROOF_TTL_SECONDS = 240;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function trimSlash(url: string): string {
  return url.replace(/\/+$/, '');
}

function segment(value: string, name: string): string {
  if (typeof value !== 'string' || value.length === 0) {
    throw new AreteError(`${name} must be a non-empty string`, 'INVALID_CONFIG');
  }
  return encodeURIComponent(value);
}

function jsonBody(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) =>
    typeof item === 'bigint' ? item.toString(10) : item
  );
}

function randomId(): string {
  const crypto = (globalThis as { crypto?: Crypto }).crypto;
  if (typeof crypto?.randomUUID === 'function') return crypto.randomUUID();
  const bytes = new Uint8Array(16);
  if (typeof crypto?.getRandomValues === 'function') {
    crypto.getRandomValues(bytes);
  } else {
    for (let i = 0; i < bytes.length; i += 1) bytes[i] = Math.floor(Math.random() * 256);
  }
  bytes[6] = (bytes[6]! & 0x0f) | 0x40;
  bytes[8] = (bytes[8]! & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function bytesToBase64(bytes: Uint8Array): string {
  let out = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i]!;
    const b = bytes[i + 1];
    const c = bytes[i + 2];
    const triple = (a << 16) | ((b ?? 0) << 8) | (c ?? 0);
    out += BASE64_ALPHABET[(triple >> 18) & 63]!;
    out += BASE64_ALPHABET[(triple >> 12) & 63]!;
    out += b === undefined ? '=' : BASE64_ALPHABET[(triple >> 6) & 63]!;
    out += c === undefined ? '=' : BASE64_ALPHABET[triple & 63]!;
  }
  return out;
}

function resolveFetch(fetchImpl: typeof fetch | undefined): typeof fetch {
  if (fetchImpl) return fetchImpl;
  if (typeof globalThis.fetch === 'function') return globalThis.fetch.bind(globalThis);
  throw new AreteError('A fetch implementation is required for protocol clients', 'INVALID_CONFIG');
}

function hasRuntimeAuthStrategy(auth: AuthConfig | undefined): boolean {
  return auth?.token !== undefined
    || auth?.getToken !== undefined
    || auth?.tokenEndpoint !== undefined;
}

function runtimeKeyClass(
  auth: AuthConfig | undefined,
  declared: ProtocolKeyClass | undefined
): ProtocolKeyClass {
  if (auth?.secretKey !== undefined) return 'secret';
  if (auth?.publishableKey !== undefined) {
    return classifyApiKey(auth.publishableKey) === 'secret' ? 'secret' : 'publishable';
  }
  return declared ?? 'publishable';
}

function normalizeWireCode(code: string): string {
  return code.trim().toLowerCase();
}

function isRefreshCode(code: string | null | undefined): boolean {
  return typeof code === 'string'
    && shouldRefreshToken(parseErrorCode(code.trim().replace(/_/g, '-')));
}

async function readJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (!text) return undefined;
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
}

function wireCodeOf(response: Response, body: unknown): string | undefined {
  const header = response.headers.get('X-Error-Code');
  if (header) return header.trim();
  return isRecord(body) && typeof body['code'] === 'string' ? body['code'] : undefined;
}

function responseError(what: string, response: Response, body: unknown): AreteError {
  const wireErrorCode = wireCodeOf(response, body);
  const message = isRecord(body) && typeof body['error'] === 'string'
    ? body['error']
    : isRecord(body) && typeof body['message'] === 'string'
      ? body['message']
      : typeof body === 'string' && body.length > 0
        ? body
        : response.statusText || 'request failed';
  return new AreteError(
    `${what} returned ${response.status}: ${message}`,
    wireErrorCode ? parseWireErrorCode(wireErrorCode) : 'PROTOCOL_REQUEST_FAILED',
    { status: response.status, wireErrorCode: wireErrorCode ?? null, body }
  );
}

function delay(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve();
      return;
    }
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      resolve();
    };
    signal.addEventListener('abort', onAbort, { once: true });
  });
}

// ---------------------------------------------------------------------------
// Authenticated HTTP
// ---------------------------------------------------------------------------

interface SendOptions {
  readonly method: string;
  readonly scopes: readonly string[];
  readonly body?: unknown;
  readonly headers?: Record<string, string>;
  readonly signal?: AbortSignal;
  readonly forceRefresh?: boolean;
}

interface ProtocolSender {
  send(url: string, options: SendOptions): Promise<Response>;
}

class ProtocolHttp implements ProtocolSender {
  private readonly fetchImpl: typeof fetch;
  private readonly staticToken?: string;
  private readonly manager?: ConnectionManager;

  constructor(
    private readonly deploymentId: string,
    sessionEndpoint: string,
    configuredAuth: AuthConfig | undefined,
    fetchImpl: typeof fetch | undefined
  ) {
    this.fetchImpl = resolveFetch(fetchImpl);
    const auth = resolveAuthConfig(configuredAuth);
    const apiCredential = isApiCredentialAuth(auth);
    if (auth?.token !== undefined && !apiCredential) {
      if (!auth.token) throw new AreteError('Authentication token is empty', 'TOKEN_INVALID');
      this.staticToken = auth.token;
      return;
    }
    // Keys mint through the generated session endpoint unless the app
    // brought its own token strategy.
    const managerAuth = apiCredential
      ? withSessionMintEndpoint(auth!, sessionEndpoint)
      : hasRuntimeAuthStrategy(auth)
        ? auth
        : { ...auth, tokenEndpoint: sessionEndpoint };
    this.manager = new ConnectionManager({
      websocketUrl: null,
      auth: managerAuth,
      fetch: this.fetchImpl,
    });
  }

  request(scopes: readonly string[]): AuthTokenRequest {
    return {
      targetKind: 'protocol-deployment',
      targetId: this.deploymentId,
      scopes,
    };
  }

  /** Mint (or reuse) a session for `scopes`. Throws the mint's refusal. */
  async token(scopes: readonly string[], forceRefresh = false): Promise<string | undefined> {
    if (this.staticToken !== undefined) return this.staticToken;
    return this.manager?.getHttpAuthToken(this.request(scopes), forceRefresh);
  }

  async send(url: string, options: SendOptions): Promise<Response> {
    const body = options.body === undefined ? undefined : jsonBody(options.body);
    const attempt = async (force: boolean): Promise<Response> => {
      const token = await this.token(options.scopes, force);
      const headers = new Headers(options.headers);
      if (token) headers.set('Authorization', `Bearer ${token}`);
      if (body !== undefined) headers.set('Content-Type', 'application/json');
      return this.fetchImpl(url, {
        method: options.method,
        headers,
        body,
        signal: options.signal,
      });
    };

    let response = await attempt(options.forceRefresh ?? false);
    if (!response.ok && this.manager && await this.isRefreshable(response)) {
      this.manager.clearHttpAuthToken(this.request(options.scopes));
      response = await attempt(true);
    }
    return response;
  }

  private async isRefreshable(response: Response): Promise<boolean> {
    if (isRefreshCode(response.headers.get('X-Error-Code'))) return true;
    if (response.status !== 401) return false;
    try {
      const body = await response.clone().json() as unknown;
      return isRecord(body) && isRefreshCode(body['code'] as string | undefined);
    } catch {
      return false;
    }
  }
}

/** Dev mode through the edge: no credential, the protocol named in a header. */
class EdgeProtocolSender implements ProtocolSender {
  private readonly edgeFetch: typeof fetch;

  constructor(
    edgeUrl: string,
    identity: Record<string, unknown> | undefined,
    fetchImpl: typeof fetch | undefined,
    private readonly onDeployment: (deploymentId: string) => void
  ) {
    this.edgeFetch = createEdgeFetch(edgeUrl, resolveFetch(fetchImpl), identity);
  }

  async send(url: string, options: SendOptions): Promise<Response> {
    const headers = new Headers(options.headers);
    const body = options.body === undefined ? undefined : jsonBody(options.body);
    if (body !== undefined) headers.set('Content-Type', 'application/json');
    const response = await this.edgeFetch(url, {
      method: options.method,
      headers,
      body,
      signal: options.signal,
    });
    // The edge names the environment's deployment on the routes it resolved.
    const deployment = response.headers.get(DEV_DEPLOYMENT_HEADER)?.trim();
    if (deployment) this.onDeployment(deployment);
    return response;
  }
}

/**
 * The environment's deployment the edge called, on every protocol answer it
 * resolved. Metadata proofs and automations name this deployment.
 */
const DEV_DEPLOYMENT_HEADER = 'x-arete-dev-deployment';

/** Placeholders a direct-mode client builds URLs with until resolve answers. */
const DIRECT_API_PLACEHOLDER = 'https://resolving.invalid/api';
const DIRECT_DEPLOYMENT_PLACEHOLDER = '~deployment~';

/**
 * Dev mode direct: resolve once (on the first request), then call the
 * environment's deployment with ordinary protocol sessions for it.
 */
class DirectProtocolSender implements ProtocolSender {
  private resolved?: Promise<{ target: DevProtocolTarget; http: ProtocolHttp }>;

  constructor(
    private readonly dev: Extract<DevEnvironmentSwitch, { via: 'direct' }>,
    private readonly definition: AnyProtocolDefinition,
    private readonly auth: AuthConfig | undefined,
    private readonly fetchImpl: typeof fetch | undefined,
    private readonly onResolved: (target: DevProtocolTarget) => void
  ) {}

  private target(): Promise<{ target: DevProtocolTarget; http: ProtocolHttp }> {
    this.resolved ??= (async () => {
      const fetchImpl = resolveFetch(this.fetchImpl);
      const answer = await resolveDevTargets({
        environment: this.dev.environment,
        request: { protocols: [protocolIdentity(this.definition)] },
        auth: this.auth,
        fetch: fetchImpl,
      });
      const target = protocolTarget(this.dev, answer.protocols?.[0], this.definition);
      this.onResolved(target);
      return {
        target,
        http: new ProtocolHttp(
          target.deploymentId!,
          target.sessionEndpoint!,
          this.auth,
          createDevServiceFetch(fetchImpl)
        ),
      };
    })();
    // A failed resolve is not cached: the next call asks again.
    this.resolved.catch(() => {
      this.resolved = undefined;
    });
    return this.resolved;
  }

  /** Resolve (once) and name the environment's deployment. */
  async resolvedDeployment(): Promise<string | null> {
    return (await this.target()).target.deploymentId;
  }

  async send(url: string, options: SendOptions): Promise<Response> {
    const { target, http } = await this.target();
    const resolvedUrl = url
      .replace(DIRECT_API_PLACEHOLDER, target.apiUrl)
      .replace(
        `/protocol-deployments/${DIRECT_DEPLOYMENT_PLACEHOLDER}`,
        `/protocol-deployments/${encodeURIComponent(target.deploymentSegment)}`
      );
    return http.send(resolvedUrl, options);
  }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/** @internal Connected members a session hands to its protocol clients. */
export interface ProtocolClientMembers {
  readonly stacks: Record<string, unknown>;
  readonly programs: Record<string, unknown>;
}

/**
 * Create a client for a generated protocol definition.
 *
 * The client is typed by key class: with `auth.secretKey` (or
 * `keyClass: 'secret'`) every client-callable step is on `client.steps`;
 * otherwise only the `public` ones are. Read calls mint a read-only session
 * (`protocol:read`); the first prepare or submit mints a session that can
 * prepare the client's steps and submit.
 */
export function createProtocolClient<
  TDef extends AnyProtocolDefinition,
  TOptions extends ProtocolClientOptions<ProtocolTypesOf<TDef>> = Record<never, never>,
>(
  definition: TDef,
  options?: TOptions
): ProtocolClient<
  ProtocolTypesOf<TDef>,
  ProtocolKeyClassOf<TOptions>,
  ProtocolStacksOf<TDef>,
  ProtocolProgramsOf<TDef>
> {
  return createProtocolClientWithMembers(definition, options, undefined) as unknown as ProtocolClient<
    ProtocolTypesOf<TDef>,
    ProtocolKeyClassOf<TOptions>,
    ProtocolStacksOf<TDef>,
    ProtocolProgramsOf<TDef>
  >;
}

/** @internal */
export function createProtocolClientWithMembers(
  definition: AnyProtocolDefinition,
  options: ProtocolClientOptions | undefined,
  members: ProtocolClientMembers | undefined
): ProtocolClient<ProtocolTypeMap, ProtocolKeyClass, Record<string, unknown>, Record<string, unknown>> {
  if (!definition || definition.kind !== 'protocol') {
    throw new AreteError('createProtocolClient requires a generated protocol definition', 'INVALID_CONFIG');
  }
  const environmentSwitch = resolveEnvironmentSwitch(
    options?.environment,
    systemEnvironmentSwitchHost(options?.auth)
  );
  // A protocol before its first publish has no production deployment: only
  // dev mode can call it, where the environment names the deployment.
  if (!definition.deployment && environmentSwitch.mode === 'production') {
    throw new AreteError(
      `${definition.package} is not deployed to production yet. Publish it with \`a4 protocol deploy\`, `
        + 'or run against your dev environment with `a4 dev run -- <command>`.',
      'PROTOCOL_NOT_DEPLOYED'
    );
  }
  let deploymentId: string;
  // In dev mode, whether `deploymentId` is the environment's deployment yet.
  let deploymentKnown = environmentSwitch.mode === 'production';
  let resolveDeployment: (signal?: AbortSignal) => Promise<string>;
  let apiUrl: string;
  let deploymentPath: () => string;
  let http: ProtocolSender;
  if (environmentSwitch.mode === 'production') {
    const endpoints = resolveProtocolEndpoints(definition.deployment!, options);
    deploymentId = endpoints.deploymentId;
    apiUrl = endpoints.apiUrl;
    deploymentPath = () => `${apiUrl}/protocol-deployments/${segment(deploymentId, 'deploymentId')}`;
    // Only an apiUrl the app passed in code is trusted for its credential; the
    // descriptor's URLs and a sessionEndpoint override are not.
    http = new ProtocolHttp(
      deploymentId,
      endpoints.sessionEndpoint,
      withTrustedApiUrl(options?.auth, options?.apiUrl),
      options?.fetch
    );
    resolveDeployment = async () => deploymentId;
  } else {
    if (
      options?.apiUrl !== undefined
      || options?.sessionEndpoint !== undefined
      || options?.deploymentId !== undefined
    ) {
      throw new AreteError(
        'apiUrl, sessionEndpoint and deploymentId cannot be combined with dev mode: the dev environment decides which deployment is called',
        'INVALID_CONFIG'
      );
    }
    if (environmentSwitch.via === 'edge') {
      const target = protocolTarget(environmentSwitch, undefined, definition);
      deploymentId = target.deploymentId ?? '';
      apiUrl = target.apiUrl;
      deploymentPath = () => `${apiUrl}/protocol-deployments/${pathSegment(target.deploymentSegment)}`;
      const edge = new EdgeProtocolSender(
        environmentSwitch.edgeUrl,
        target.identity,
        options?.fetch,
        (environmentDeployment) => {
          deploymentId = environmentDeployment;
          deploymentKnown = true;
        }
      );
      http = edge;
      let asked: Promise<string> | undefined;
      resolveDeployment = (signal) => {
        if (deploymentKnown) return Promise.resolve(deploymentId);
        // Any deployment route answers with the environment's deployment.
        asked ??= edge.send(deploymentPath(), { method: 'GET', scopes: [PROTOCOL_READ_SCOPE], signal })
          .then(async (response) => {
            if (!response.ok) throw responseError('Protocol deployment', response, await readJson(response));
            await response.body?.cancel();
            if (!deploymentKnown) {
              throw new AreteError(
                'The a4 dev edge did not name the environment\'s deployment of this protocol; update a4',
                'INVALID_CONFIG'
              );
            }
            return deploymentId;
          })
          .finally(() => {
            asked = undefined;
          });
        return asked;
      };
    } else {
      deploymentId = definition.deployment?.deploymentId ?? '';
      apiUrl = DIRECT_API_PLACEHOLDER;
      deploymentPath = () => `${apiUrl}/protocol-deployments/${DIRECT_DEPLOYMENT_PLACEHOLDER}`;
      http = new DirectProtocolSender(
        environmentSwitch,
        definition,
        options?.auth,
        options?.fetch,
        (target) => {
          deploymentId = target.deploymentId ?? deploymentId;
          deploymentKnown = true;
        }
      );
      const direct = http as DirectProtocolSender;
      resolveDeployment = async () => (await direct.resolvedDeployment()) ?? deploymentId;
    }
  }
  const resolvedAuth = resolveAuthConfig(options?.auth);
  const keyClass = runtimeKeyClass(resolvedAuth, options?.keyClass);
  const stepDefinitions = definition.steps as Record<
    string,
    { scope: string; access: string; guards: readonly string[]; args: readonly string[] }
  >;

  const readScopes = [PROTOCOL_READ_SCOPE] as const;
  const preparableSteps = keyClass === 'secret'
    ? undefined
    : (options?.steps ?? Object.keys(stepDefinitions).filter(
        (step) => stepDefinitions[step]!.access === 'public'
      ));
  const transactionScopes = [
    PROTOCOL_READ_SCOPE,
    PROTOCOL_SUBMIT_SCOPE,
    ...(preparableSteps === undefined
      ? [PROTOCOL_PREPARE_ANY_SCOPE]
      : preparableSteps.map((step) => `protocol:prepare:${step}`)),
  ];

  const subscriptions = new Set<ProtocolEventSubscription>();
  const handles = new Set<ProtocolInstanceHandle>();

  const getJson = async (what: string, url: string, signal?: AbortSignal): Promise<unknown> => {
    const response = await http.send(url, { method: 'GET', scopes: readScopes, signal });
    const body = await readJson(response);
    if (!response.ok) throw responseError(what, response, body);
    return body;
  };

  const instanceUrl = (id: string) => `${apiUrl}/protocol-instances/${segment(id, 'instance')}`;

  const getInstance = async (
    id: string,
    getOptions?: ProtocolInstanceGetOptions
  ): Promise<ProtocolInstanceDetail> => {
    const query = getOptions?.wallet ? `?wallet=${encodeURIComponent(getOptions.wallet)}` : '';
    return await getJson('Protocol instance read', `${instanceUrl(id)}${query}`, getOptions?.signal) as ProtocolInstanceDetail;
  };

  const getParticipant = async (input: {
    instance: string;
    wallet: string;
    signal?: AbortSignal;
  }): Promise<ProtocolParticipantDetail> =>
    await getJson(
      'Protocol participant read',
      `${instanceUrl(input.instance)}/participants/${segment(input.wallet, 'wallet')}`,
      input.signal
    ) as ProtocolParticipantDetail;

  const rejection = (
    rejected: string,
    step: string,
    extra?: { guard?: unknown; detail?: unknown }
  ): ProtocolPrepareResult => ({
    ok: false,
    rejection: {
      rejected,
      step,
      ...(typeof extra?.guard === 'string' ? { guard: extra.guard } : {}),
      ...(isRecord(extra?.detail) ? { detail: extra.detail } : {}),
    },
  });

  const prepare = async (
    stepName: string,
    input: Record<string, unknown>,
    prepareOptions?: ProtocolPrepareOptions
  ): Promise<ProtocolPrepareResult> => {
    const step = Object.prototype.hasOwnProperty.call(stepDefinitions, stepName)
      ? stepDefinitions[stepName]
      : undefined;
    if (!step) return rejection('step_not_client_callable', stepName);
    if (keyClass === 'publishable' && step.access === 'secret') {
      return rejection('step_requires_secret_key', stepName);
    }
    if (!isRecord(input)) {
      throw new AreteError(`${stepName}.prepare requires an input object`, 'INVALID_CONFIG');
    }
    const { instance, wallet, ...args } = input;
    if (typeof wallet !== 'string' || wallet.length === 0) {
      throw new AreteError(`${stepName}.prepare requires a wallet`, 'INVALID_CONFIG');
    }
    let url: string;
    if (step.scope === 'create') {
      url = deploymentPath()
        + `/steps/${segment(stepName, 'step')}/prepare`;
    } else {
      if (typeof instance !== 'string' || instance.length === 0) {
        throw new AreteError(`${stepName}.prepare requires an instance id`, 'INVALID_CONFIG');
      }
      url = `${instanceUrl(instance)}/steps/${segment(stepName, 'step')}/prepare`;
    }
    const body = {
      idempotencyKey: prepareOptions?.idempotencyKey ?? randomId(),
      wallet,
      args,
      transactionFormat: prepareOptions?.transactionFormat ?? 'v1',
      ...(prepareOptions?.clientContext ? { clientContext: prepareOptions.clientContext } : {}),
    };

    let response: Response;
    try {
      response = await http.send(url, {
        method: 'POST',
        scopes: transactionScopes,
        body,
        signal: prepareOptions?.signal,
      });
    } catch (error) {
      // The session mint refuses a step the key cannot prepare.
      if (error instanceof AreteError && isRecord(error.details)) {
        const wire = error.details['wireErrorCode'];
        if (typeof wire === 'string' && ACCESS_CODES.has(normalizeWireCode(wire))) {
          return rejection(normalizeWireCode(wire), stepName);
        }
      }
      throw error;
    }

    const responseBody = await readJson(response);
    if (response.ok) {
      if (!isRecord(responseBody) || !isRecord(responseBody['intent']) || !Array.isArray(responseBody['transactions'])) {
        throw new AreteError('Prepare returned an unexpected body', 'PROTOCOL_REQUEST_FAILED', {
          status: response.status,
          body: responseBody,
        });
      }
      return {
        ok: true,
        intent: responseBody['intent'] as unknown as ExecutionIntentV1,
        transactions: responseBody['transactions'] as { index: number; transactionBase64: string }[],
      };
    }
    if (response.status === 422 && isRecord(responseBody) && typeof responseBody['rejected'] === 'string') {
      return rejection(
        responseBody['rejected'],
        typeof responseBody['step'] === 'string' ? responseBody['step'] : stepName,
        { guard: responseBody['guard'], detail: responseBody['detail'] }
      );
    }
    if (response.status === 403) {
      const code = wireCodeOf(response, responseBody);
      if (code && ACCESS_CODES.has(normalizeWireCode(code))) {
        return rejection(normalizeWireCode(code), stepName, {
          detail: isRecord(responseBody) ? responseBody['detail'] : undefined,
        });
      }
    }
    throw responseError(`Prepare of step '${stepName}'`, response, responseBody);
  };

  const steps: Record<string, ProtocolStepClient> = {};
  for (const [name, step] of Object.entries(stepDefinitions)) {
    steps[name] = {
      name,
      scope: step.scope as ProtocolStepClient['scope'],
      access: step.access as ProtocolStepClient['access'],
      prepare: (input, prepareOptions) =>
        prepare(name, input as Record<string, unknown>, prepareOptions),
    };
  }

  const intentState = async (what: string, response: Response): Promise<ProtocolIntentState> => {
    const body = await readJson(response);
    if (!response.ok) throw responseError(what, response, body);
    return body as ProtocolIntentState;
  };

  const submit = async (
    intent: ExecutionIntentV1 | string,
    signedTransactions: readonly (string | Uint8Array)[]
  ): Promise<ProtocolIntentState> => {
    const executionId = typeof intent === 'string' ? intent : intent?.executionId;
    if (!Array.isArray(signedTransactions) || signedTransactions.length === 0) {
      throw new AreteError('submit requires the signed transactions', 'INVALID_CONFIG');
    }
    const encoded = signedTransactions.map((transaction) =>
      typeof transaction === 'string' ? transaction : bytesToBase64(transaction)
    );
    const response = await http.send(
      `${apiUrl}/execution-intents/${segment(executionId as string, 'executionId')}/submit`,
      { method: 'POST', scopes: transactionScopes, body: { signedTransactions: encoded } }
    );
    return intentState('Submit', response);
  };

  const metadataHandle = (
    instanceId: string,
    participant: string | undefined
  ): ProtocolMetadataHandle<Record<string, unknown>> => ({
    async get() {
      if (participant !== undefined) {
        const detail = await getParticipant({ instance: instanceId, wallet: participant });
        return { metadata: detail.metadata as Record<string, unknown>, version: null };
      }
      const detail = await getInstance(instanceId);
      return {
        metadata: detail.metadata as Record<string, unknown>,
        version: detail.metadataVersion,
      };
    },
    async update(patch, updateOptions) {
      if (!isRecord(patch)) {
        throw new AreteError('A metadata update requires a patch object', 'INVALID_CONFIG');
      }
      const target = updateOptions?.participant ?? participant;
      const response = await http.send(`${instanceUrl(instanceId)}/metadata`, {
        method: 'PATCH',
        scopes: readScopes,
        body: {
          patch,
          ...(target !== undefined ? { participant: target } : {}),
          ...(updateOptions?.walletProof ? { proof: updateOptions.walletProof } : {}),
        },
        signal: updateOptions?.signal,
      });
      const body = await readJson(response);
      if (!response.ok) throw responseError('Metadata update', response, body);
      return body as ProtocolMetadataUpdateResult<Record<string, unknown>>;
    },
    proofMessage(patch, proofOptions) {
      // The API checks the proof against the instance's deployment: in dev
      // mode the environment's, not the installed one.
      if (!deploymentKnown) {
        throw new AreteError(
          'In dev mode a metadata proof names the dev environment\'s deployment: '
            + '`await client.resolveDeployment()` before building one',
          'INVALID_CONFIG'
        );
      }
      return protocolMetadataProofMessage({
        deploymentId,
        instanceId,
        wallet: proofOptions.wallet,
        patch: patch as Record<string, unknown>,
        expiresAt: proofOptions.expiresAt ?? Math.floor(Date.now() / 1000) + DEFAULT_PROOF_TTL_SECONDS,
      });
    },
  });

  const events = (
    instanceId: string,
    eventOptions: ProtocolEventsOptions
  ): ProtocolEventSubscription => {
    const url = `${instanceUrl(instanceId)}/events`;
    const controller = new AbortController();
    let lastEventId = eventOptions.lastEventId;
    let closed = false;
    const report = (error: unknown) => {
      if (closed) return;
      try {
        eventOptions.onError?.(error instanceof Error ? error : new Error(String(error)));
      } catch {
        // A throwing error callback must not end the stream.
      }
    };
    const subscription: ProtocolEventSubscription = {
      get lastEventId() {
        return lastEventId;
      },
      get closed() {
        return closed;
      },
      close() {
        if (closed) return;
        closed = true;
        controller.abort();
        subscriptions.delete(subscription);
        eventOptions.signal?.removeEventListener('abort', onExternalAbort);
      },
    };
    const onExternalAbort = () => subscription.close();
    subscriptions.add(subscription);
    if (eventOptions.signal?.aborted) {
      subscription.close();
      return subscription;
    }
    eventOptions.signal?.addEventListener('abort', onExternalAbort, { once: true });

    let forceRefresh = false;
    const dispatch = (event: ServerSentEvent) => {
      let data: unknown;
      try {
        data = JSON.parse(event.data) as unknown;
      } catch {
        report(new AreteError(`Event stream sent invalid JSON for '${event.event}'`, 'PROTOCOL_STREAM_INVALID'));
        return;
      }
      if (event.event === 'error') {
        const code = isRecord(data) && typeof data['code'] === 'string' ? data['code'] : 'unknown';
        if (isRefreshCode(code) || /expired/i.test(code)) {
          forceRefresh = true;
          return;
        }
        report(new AreteError(`Event stream error: ${code}`, parseWireErrorCode(code), { wireErrorCode: code }));
        return;
      }
      const id = event.id ?? '';
      let streamEvent: ProtocolStreamEvent | undefined;
      if (event.event === 'emit' && isRecord(data) && typeof data['event'] === 'string') {
        streamEvent = {
          type: 'emit',
          id,
          event: data['event'],
          data: data as unknown as Extract<ProtocolStreamEvent, { type: 'emit' }>['data'],
        };
      } else if (event.event === SQUADS_PROPOSER_MISSING_EVENT && isRecord(data)) {
        streamEvent = {
          type: SQUADS_PROPOSER_MISSING_EVENT,
          id,
          data: data as unknown as ProtocolSquadsProposerMissingData,
        };
      } else if (STREAM_EVENT_TYPES.has(event.event) && isRecord(data)) {
        streamEvent = {
          type: event.event as 'transition' | 'participant' | 'observation',
          id,
          data,
        };
      }
      if (!streamEvent) return;
      try {
        eventOptions.onEvent(streamEvent);
      } catch (error) {
        report(error);
      }
    };

    const run = async () => {
      let failures = 0;
      while (!closed) {
        try {
          const response = await http.send(url, {
            method: 'GET',
            scopes: readScopes,
            headers: {
              Accept: 'text/event-stream',
              'Cache-Control': 'no-cache',
              ...(lastEventId !== undefined ? { 'Last-Event-ID': lastEventId } : {}),
            },
            signal: controller.signal,
            forceRefresh,
          });
          forceRefresh = false;
          if (!response.ok) {
            const error = responseError('Event stream', response, await readJson(response));
            const fatal = response.status >= 400 && response.status < 500
              && response.status !== 408 && response.status !== 429;
            if (fatal) {
              report(error);
              subscription.close();
              return;
            }
            throw error;
          }
          if (!response.body) {
            throw new AreteError('Event stream response has no body', 'PROTOCOL_STREAM_INVALID');
          }
          const parser = new ServerSentEventParser(lastEventId);
          const reader = response.body.getReader();
          const decoder = new TextDecoder();
          failures = 0;
          try {
            while (!closed) {
              const { done, value } = await reader.read();
              if (done) break;
              for (const event of parser.push(decoder.decode(value, { stream: true }))) {
                if (event.id !== undefined) lastEventId = event.id;
                dispatch(event);
                if (closed) break;
              }
            }
          } finally {
            parser.end();
            if (closed) void reader.cancel().catch(() => undefined);
          }
          // The server ends streams periodically and when the session expires:
          // reconnect after the last id.
          if (!closed) await delay(parser.retryMs ?? 0, controller.signal);
        } catch (error) {
          if (closed) return;
          report(error);
          failures += 1;
          await delay(Math.min(MAX_RECONNECT_DELAY_MS, 1000 * 2 ** (failures - 1)), controller.signal);
        }
      }
    };
    void run();
    return subscription;
  };

  const liveStack = definition.liveStack;
  const memberStacks = members?.stacks ?? (definition.stacks as Record<string, unknown>);
  const memberPrograms = members?.programs ?? (definition.programs as Record<string, unknown>);

  const liveView = (): { use(key: string): AsyncIterable<unknown> } | undefined => {
    if (!liveStack) return undefined;
    const [entity, mode] = liveStack.view.split('/');
    const member = memberStacks[liveStack.stack] as { views?: unknown } | undefined;
    const views = member?.views as Record<string, Record<string, unknown>> | undefined;
    const view = entity && mode ? views?.[entity]?.[mode] as { use?: unknown } | undefined : undefined;
    return typeof view?.use === 'function'
      ? view as { use(key: string): AsyncIterable<unknown> }
      : undefined;
  };

  const useInstance = (id: string, useOptions?: { wallet?: string }): ProtocolInstanceHandle => {
    const listeners = new Set<() => void>();
    let snapshot: ProtocolInstanceSnapshot | undefined;
    let error: Error | undefined;
    let running = false;
    let stream: ProtocolEventSubscription | undefined;
    let inFlight: Promise<ProtocolInstanceSnapshot | undefined> | undefined;
    let queued = false;
    let liveKey: string | undefined;
    let liveIterator: AsyncIterator<unknown> | undefined;

    const emit = () => {
      for (const listener of [...listeners]) listener();
    };
    const stopLive = () => {
      const iterator = liveIterator;
      liveIterator = undefined;
      liveKey = undefined;
      void iterator?.return?.()?.catch?.(() => undefined);
    };
    const startLive = (instance: ProtocolInstanceDetail) => {
      if (!running || !liveStack) return;
      const key = instance.accounts?.[liveStack.keyAccount];
      if (key === liveKey) return;
      stopLive();
      if (!key) return;
      const view = liveView();
      if (!view) return;
      liveKey = key;
      const iterator = view.use(key)[Symbol.asyncIterator]();
      liveIterator = iterator;
      void (async () => {
        try {
          for (;;) {
            const next = await iterator.next();
            if (next.done || liveIterator !== iterator) return;
            if (snapshot) {
              snapshot = { ...snapshot, live: next.value };
              emit();
            }
          }
        } catch (liveError) {
          if (liveIterator !== iterator) return;
          error = liveError instanceof Error ? liveError : new Error(String(liveError));
          emit();
        }
      })();
    };

    const load = async (): Promise<ProtocolInstanceSnapshot | undefined> => {
      try {
        const instance = await getInstance(id, { wallet: useOptions?.wallet });
        const key = liveStack && instance.accounts?.[liveStack.keyAccount];
        snapshot = { instance, live: key === liveKey ? snapshot?.live : undefined };
        error = undefined;
        emit();
        startLive(instance);
      } catch (loadError) {
        error = loadError instanceof Error ? loadError : new Error(String(loadError));
        emit();
      }
      return snapshot;
    };
    const refresh = (): Promise<ProtocolInstanceSnapshot | undefined> => {
      if (inFlight) {
        queued = true;
        return inFlight;
      }
      const current = (async () => {
        let result = await load();
        while (queued) {
          queued = false;
          result = await load();
        }
        return result;
      })();
      inFlight = current;
      void current.finally(() => {
        if (inFlight === current) inFlight = undefined;
      });
      return current;
    };
    const start = () => {
      if (running) return;
      running = true;
      handles.add(handle);
      void refresh();
      stream = events(id, {
        onEvent: () => {
          void refresh();
        },
        onError: (streamError) => {
          error = streamError;
          emit();
        },
      });
    };
    const stop = () => {
      running = false;
      handles.delete(handle);
      stream?.close();
      stream = undefined;
      stopLive();
    };

    const handle: ProtocolInstanceHandle = {
      instanceId: id,
      get: () => snapshot,
      getError: () => error,
      subscribe(listener) {
        listeners.add(listener);
        start();
        return () => {
          listeners.delete(listener);
          if (listeners.size === 0) stop();
        };
      },
      refresh,
      close() {
        listeners.clear();
        stop();
      },
    };
    return handle;
  };

  const filterableInstanceFields = new Set(
    Object.entries((definition.metadata?.instance ?? {}) as Record<string, { filterable?: boolean }>)
      .filter(([, field]) => field?.filterable === true)
      .map(([name]) => name)
  );

  const squadsUrl = `${deploymentPath()}/squads`;
  const squads: ProtocolSquadsClient = {
    async proposer(proposerOptions) {
      return await getJson(
        'Squads proposer read',
        `${squadsUrl}/proposer`,
        proposerOptions?.signal
      ) as ProtocolSquadsProposer;
    },
    async status(multisig, statusOptions) {
      if (typeof multisig !== 'string' || multisig.length === 0) {
        throw new AreteError('status requires a multisig', 'INVALID_CONFIG');
      }
      const status = await getJson(
        'Squads proposer status',
        `${squadsUrl}/status?multisig=${encodeURIComponent(multisig)}`,
        statusOptions?.signal
      ) as ProtocolSquadsStatus & { mask?: number | null };
      return { ...status, mask: status.mask ?? null };
    },
  };

  const walletActions: ProtocolClient['actions'] = async (input) => {
    const wallet = segment(input.wallet, 'wallet');
    const params = new URLSearchParams();
    if (input.instance !== undefined) params.set('instance', input.instance);
    if (input.cursor !== undefined) params.set('cursor', input.cursor);
    if (input.limit !== undefined) params.set('limit', String(input.limit));
    const query = params.toString();
    const actions = await getJson(
      'Protocol wallet actions',
      `${deploymentPath()}/wallets/${wallet}/actions${query ? `?${query}` : ''}`,
      input.signal
    ) as ProtocolWalletActions;
    return {
      ...actions,
      createSteps: actions.createSteps ?? [],
      instances: actions.instances ?? [],
      nextCursor: actions.nextCursor ?? null,
    };
  };

  const instancesForWallet: ProtocolClient['instances']['forWallet'] = async (wallet, options = {}) => {
    const found: ProtocolWalletInstanceActions[] = [];
    const seen = new Set<string>();
    let cursor: string | undefined;
    for (;;) {
      const page = await walletActions({
        wallet,
        limit: WALLET_ACTIONS_PAGE_LIMIT,
        ...(cursor !== undefined ? { cursor } : {}),
        ...(options.signal ? { signal: options.signal } : {}),
      });
      for (const instance of page.instances) {
        if (options.relation !== undefined && !(instance.relation ?? []).includes(options.relation)) continue;
        found.push(instance);
      }
      if (page.nextCursor === null) return found;
      if (seen.has(page.nextCursor)) {
        throw new AreteError('Protocol wallet actions returned a cursor it already returned', 'PROTOCOL_REQUEST_FAILED', {
          cursor: page.nextCursor,
        });
      }
      seen.add(page.nextCursor);
      cursor = page.nextCursor;
    }
  };

  const client: ProtocolClient<ProtocolTypeMap, ProtocolKeyClass, Record<string, unknown>, Record<string, unknown>> = {
    definition: definition as ProtocolDefinition,
    get deploymentId() {
      return deploymentId;
    },
    environment: environmentSwitch.mode,
    resolveDeployment(resolveOptions) {
      return resolveDeployment(resolveOptions?.signal);
    },
    keyClass,
    steps: steps as ProtocolClient['steps'],
    instances: {
      async list(filter) {
        const params = new URLSearchParams();
        if (filter?.state !== undefined) params.set('state', filter.state);
        if (filter?.cursor !== undefined) params.set('cursor', filter.cursor);
        if (filter?.limit !== undefined) params.set('limit', String(filter.limit));
        for (const [field, value] of Object.entries(filter?.metadata ?? {})) {
          if (value === undefined) continue;
          if (!filterableInstanceFields.has(field)) {
            throw new AreteError(
              `Instance metadata field '${field}' is not filterable`,
              'INVALID_CONFIG'
            );
          }
          params.set(`metadata.${field}`, String(value));
        }
        const query = params.toString();
        return await getJson(
          'Protocol instance list',
          `${deploymentPath()}/instances${query ? `?${query}` : ''}`,
          filter?.signal
        ) as ProtocolInstancePage;
      },
      get: getInstance,
      use: useInstance,
      forWallet: instancesForWallet,
    },
    participants: {
      get: getParticipant,
    },
    async allowedSteps(input) {
      segment(input.wallet, 'wallet');
      const detail = await getInstance(input.instance, { wallet: input.wallet, signal: input.signal });
      return detail.allowedSteps ?? [];
    },
    actions: walletActions,
    submit,
    async getReceipt(signature, receiptOptions) {
      return (await getJson(
        'Protocol receipt read',
        `${apiUrl}/protocol-receipts/${segment(signature, 'signature')}`,
        receiptOptions?.signal
      )) as ProtocolExecutionReceipt;
    },
    async getReceiptKeys(receiptOptions) {
      return (await getJson(
        'Protocol receipt keys read',
        `${apiUrl}/protocol-receipts/keys`,
        receiptOptions?.signal
      )) as ProtocolReceiptKeySet;
    },
    intents: {
      async get(executionId) {
        const response = await http.send(
          `${apiUrl}/execution-intents/${segment(executionId, 'executionId')}`,
          { method: 'GET', scopes: readScopes }
        );
        return intentState('Execution intent read', response);
      },
    },
    metadata: {
      instance: (id) => metadataHandle(id, undefined),
      participant: (input) => metadataHandle(input.instance, input.wallet),
    },
    events,
    squads,
    stacks: memberStacks,
    programs: memberPrograms,
    close() {
      for (const subscription of [...subscriptions]) subscription.close();
      for (const handle of [...handles]) handle.close();
    },
  };
  return client;
}

/** True when `code` is one of {@link PROTOCOL_ACCESS_REJECTION_CODES}. */
export function isProtocolAccessRejectionCode(code: string): code is ProtocolAccessRejectionCode {
  return ACCESS_CODES.has(code);
}

/**
 * True when `code` is one of {@link PROTOCOL_STATE_REJECTION_CODES}: the instance,
 * or the preparing wallet's participant, is in a terminal state.
 */
export function isProtocolStateRejectionCode(code: string): code is ProtocolStateRejectionCode {
  return STATE_CODES.has(code);
}
