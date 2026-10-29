/**
 * The dev environment switch: which world a client talks to.
 *
 * - **Production** (the default): the generated descriptors exactly as they
 *   are. Nothing in this module is read on that path beyond the switch
 *   itself (`ARETE_ENVIRONMENT`, outside browsers).
 * - **Dev**: the project's cloud dev environment (a fork of mainnet), either
 *   through the loopback edge `a4 dev up` serves (no credential in the app),
 *   or directly with a secret key (servers, CI and agents).
 *
 * The contract is `docs/internal/dev-environment-switch.md`; the shared
 * vectors are `test-vectors/dev-environment-switch/v1/`.
 */

import { classifyApiKey, resolveAuthConfig } from './auth-keys';
import { systemProfileHost, type A4ProfileHost } from './a4-profile';
import { isBrowserEnvironment, readEnvironmentVariable } from './environment';
import { AreteError, type AuthConfig } from './types';

/** Process environment variable that selects the world, outside browsers. */
export const ARETE_ENVIRONMENT_ENV = 'ARETE_ENVIRONMENT';
/** The edge origin `a4 dev up` serves, outside browsers. */
export const ARETE_DEV_URL_ENV = 'ARETE_DEV_URL';
/** An environment id or name for direct mode, outside browsers. */
export const ARETE_DEV_ENV_ENV = 'ARETE_DEV_ENV';

/** Header every edge response carries, with the value `1`. */
export const DEV_EDGE_HEADER = 'X-Arete-Dev-Edge';
/** Header naming the installed identity an edge request is for. */
export const DEV_IDENTITY_HEADER = 'X-Arete-Dev-Identity';
/** Checked once before the first socket to an edge is opened. */
export const DEV_EDGE_WELL_KNOWN_PATH = '/.well-known/arete-dev-edge';

/** Refusal codes the SDK itself raises. */
export const DEV_ENVIRONMENT_CODES = {
  notConfigured: 'devenv-not-configured',
  edgeUnreachable: 'devenv-edge-unreachable',
  stackNotLoaded: 'devenv-stack-not-loaded',
  surfaceUnavailable: 'devenv-surface-unavailable',
  walletSendUnsupported: 'devenv-wallet-send-unsupported',
} as const;

/**
 * The `environment` client option: `'production'`, or dev through an edge
 * (`url`) or directly (`env`, servers with a secret key only). `{ kind: 'dev' }`
 * alone takes the target from the process environment or the running
 * `a4 dev up` session.
 */
export type EnvironmentOption =
  | 'production'
  | { readonly kind: 'dev'; readonly url?: string; readonly env?: string };

/** What `client.environment` reports. */
export type ClientEnvironment = 'production' | 'dev';

/** The switch, decided once per client at construction. */
export type EnvironmentSwitch =
  | { readonly mode: 'production' }
  | { readonly mode: 'dev'; readonly via: 'edge'; readonly edgeUrl: string }
  | { readonly mode: 'dev'; readonly via: 'direct'; readonly environment: string };

export type DevEnvironmentSwitch = Exclude<EnvironmentSwitch, { mode: 'production' }>;

/**
 * A refusal from the dev environment, the edge, or the SDK's switch. `code`
 * is the wire code; `fix` names the command or change that resolves it, and
 * `refusal` holds every other detail field the answer carried.
 */
export class DevEnvironmentError extends AreteError {
  readonly fix: string | undefined;
  readonly refusal: Readonly<Record<string, unknown>>;
  readonly status: number | undefined;

  constructor(
    code: string,
    message: string,
    options: {
      fix?: string;
      refusal?: Record<string, unknown>;
      status?: number;
      cause?: unknown;
    } = {}
  ) {
    const refusal = options.refusal ?? {};
    super(message, code, {
      ...refusal,
      ...(options.fix !== undefined ? { fix: options.fix } : {}),
      ...(options.status !== undefined ? { status: options.status } : {}),
    });
    this.name = 'DevEnvironmentError';
    this.fix = options.fix;
    this.refusal = refusal;
    this.status = options.status;
    if (options.cause !== undefined) {
      (this as { cause?: unknown }).cause = options.cause;
    }
  }
}

const NOT_CONFIGURED_FIX =
  'Run the app with `a4 dev run -- <command>`, start `a4 dev up`, or set ARETE_DEV_ENV with a secret key.';
const EDGE_UNREACHABLE_FIX = 'Start `a4 dev up` in the project (it serves the edge).';

export function devNotConfigured(message: string): DevEnvironmentError {
  return new DevEnvironmentError(DEV_ENVIRONMENT_CODES.notConfigured, message, {
    fix: NOT_CONFIGURED_FIX,
  });
}

export function devEdgeUnreachable(edgeUrl: string, why: string, cause?: unknown): DevEnvironmentError {
  return new DevEnvironmentError(
    DEV_ENVIRONMENT_CODES.edgeUnreachable,
    `The dev edge at ${edgeUrl} did not answer as an edge: ${why}`,
    { fix: EDGE_UNREACHABLE_FIX, cause }
  );
}

export function devSurfaceUnavailable(message: string, fix: string): DevEnvironmentError {
  return new DevEnvironmentError(DEV_ENVIRONMENT_CODES.surfaceUnavailable, message, { fix });
}

function invalidConfig(message: string): AreteError {
  return new AreteError(message, 'INVALID_CONFIG');
}

// ── The switch ──────────────────────────────────────────────────────────────

/**
 * What the switch may read. Every dev input goes through here, so a test can
 * record that production reads none of them.
 */
export interface EnvironmentSwitchHost {
  /** In a browser no variable or file is read. */
  readonly browser: boolean;
  readVariable(name: string): string | undefined;
  /** `edgeUrl` of the nearest `.arete/dev/session.json`, if any. */
  readSessionEdgeUrl(): string | undefined;
  /** Whether the SDK's key resolution finds a secret key (direct mode). */
  hasSecretKey(): boolean;
}

const EDGE_URL = /^(https?):\/\/(127\.0\.0\.1|\[::1\]|localhost):([0-9]{1,5})\/?$/;
const ENVIRONMENT_ID = /^denv_[0-7][0-9abcdefghjkmnpqrstvwxyz]{25}$/;
const ENVIRONMENT_NAME = /^[a-z0-9-]{1,63}$/;

/**
 * An edge URL as the switch accepts it: `http(s)://`, a loopback host
 * (`127.0.0.1`, `[::1]`, `localhost`), an explicit port, nothing after `/`.
 * Returns it without the trailing slash; throws `INVALID_CONFIG` otherwise.
 */
export function normalizeEdgeUrl(value: string, source: string): string {
  const match = EDGE_URL.exec(value);
  const port = match ? Number(match[3]) : 0;
  if (!match || port < 1 || port > 65535) {
    throw invalidConfig(
      `${source} must be the loopback edge \`a4 dev up\` serves, like http://127.0.0.1:8911 `
        + '(http or https, host 127.0.0.1, [::1] or localhost, an explicit port, no path)'
    );
  }
  return `${match[1]}://${match[2]}:${match[3]}`;
}

/** An environment id (`denv_…`) or name; throws `INVALID_CONFIG` otherwise. */
export function validateEnvironmentReference(value: string, source: string): string {
  if (ENVIRONMENT_ID.test(value) || ENVIRONMENT_NAME.test(value)) return value;
  throw invalidConfig(
    `${source} must be a dev environment id (denv_…) or name (lowercase letters, digits and -)`
  );
}

interface ParsedOption {
  readonly kind: 'production' | 'dev';
  readonly url?: string;
  readonly env?: string;
}

function parseOption(option: unknown): ParsedOption | undefined {
  if (option === undefined || option === null) return undefined;
  if (option === 'production') return { kind: 'production' };
  const invalid = () => invalidConfig(
    "environment must be 'production' or { kind: 'dev', url?, env? }"
  );
  if (typeof option !== 'object' || Array.isArray(option)) throw invalid();
  const record = option as Record<string, unknown>;
  if (record['kind'] !== 'dev') throw invalid();
  for (const key of Object.keys(record)) {
    if (key !== 'kind' && key !== 'url' && key !== 'env') throw invalid();
  }
  const url = record['url'];
  const env = record['env'];
  if ((url !== undefined && typeof url !== 'string') || (env !== undefined && typeof env !== 'string')) {
    throw invalid();
  }
  return { kind: 'dev', url, env };
}

/**
 * Decide the switch (contract §2): the option, then `ARETE_ENVIRONMENT`
 * outside browsers, then production. In dev mode the target is the option's
 * `url`, the option's `env`, `ARETE_DEV_URL`, `ARETE_DEV_ENV`, then the
 * nearest session file's `edgeUrl`, the first that is set.
 */
export function resolveEnvironmentSwitch(
  option: unknown,
  host: EnvironmentSwitchHost = systemEnvironmentSwitchHost()
): EnvironmentSwitch {
  const parsed = parseOption(option);
  if (parsed?.kind === 'production') return { mode: 'production' };
  if (!parsed) {
    if (host.browser) return { mode: 'production' };
    const value = host.readVariable(ARETE_ENVIRONMENT_ENV);
    if (value === undefined || value === '' || value === 'production') return { mode: 'production' };
    if (value !== 'dev') {
      throw invalidConfig(`${ARETE_ENVIRONMENT_ENV} must be 'production' or 'dev'`);
    }
  }

  const direct = (environment: string): DevEnvironmentSwitch => {
    if (host.browser) {
      throw devNotConfigured(
        'Dev mode talks to an environment directly only on servers: a browser uses the `a4 dev up` edge (environment: { kind: \'dev\', url })'
      );
    }
    if (!host.hasSecretKey()) {
      throw devNotConfigured(
        `Dev mode with environment ${environment} needs a secret key (ARETE_API_KEY, the a4 login or auth.secretKey)`
      );
    }
    return { mode: 'dev', via: 'direct', environment };
  };

  if (parsed?.url !== undefined) {
    return { mode: 'dev', via: 'edge', edgeUrl: normalizeEdgeUrl(parsed.url, 'environment.url') };
  }
  if (parsed?.env !== undefined) {
    return direct(validateEnvironmentReference(parsed.env, 'environment.env'));
  }
  if (!host.browser) {
    const url = host.readVariable(ARETE_DEV_URL_ENV);
    if (url !== undefined) {
      return { mode: 'dev', via: 'edge', edgeUrl: normalizeEdgeUrl(url, ARETE_DEV_URL_ENV) };
    }
    const env = host.readVariable(ARETE_DEV_ENV_ENV);
    if (env !== undefined) return direct(validateEnvironmentReference(env, ARETE_DEV_ENV_ENV));
    const session = host.readSessionEdgeUrl();
    if (session !== undefined) {
      return {
        mode: 'dev',
        via: 'edge',
        edgeUrl: normalizeEdgeUrl(session, 'edgeUrl in .arete/dev/session.json'),
      };
    }
  }
  throw devNotConfigured(
    host.browser
      ? "Dev mode in a browser needs the edge URL: environment: { kind: 'dev', url: 'http://127.0.0.1:<port>' }"
      : 'Dev mode is on, but no edge URL or environment is set and no `a4 dev up` session was found'
  );
}

/** `client.environment` for a switch. */
export function clientEnvironment(value: EnvironmentSwitch): ClientEnvironment {
  return value.mode;
}

/** The secret key the SDK's key resolution finds for `auth`, if any. */
export function resolveSecretKey(auth: AuthConfig | undefined): string | undefined {
  const resolved = resolveAuthConfig(auth);
  if (resolved?.secretKey !== undefined) return resolved.secretKey;
  if (resolved?.publishableKey !== undefined && classifyApiKey(resolved.publishableKey) === 'secret') {
    return resolved.publishableKey;
  }
  return undefined;
}

function parentDirectory(dir: string): string | undefined {
  const trimmed = dir.replace(/[\\/]+$/, '');
  const index = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  if (index < 0) return undefined;
  if (index === 0) return trimmed === '' ? undefined : '/';
  const parent = trimmed.slice(0, index);
  // `C:` is a drive root.
  return /^[A-Za-z]:$/.test(parent) ? `${parent}\\` : parent;
}

/**
 * `edgeUrl` from the nearest `.arete/dev/session.json` at or above the
 * working directory. A file without one (an `a4` that serves no edge) counts
 * as absent; an unreadable or malformed file is `INVALID_CONFIG`.
 */
export function readSessionEdgeUrl(host: A4ProfileHost | undefined): string | undefined {
  let dir = host?.cwd();
  if (!host || !dir) return undefined;
  for (let depth = 0; depth < 256 && dir !== undefined; depth += 1) {
    const file = host.joinPath(dir, '.arete', 'dev', 'session.json');
    let text: string | null;
    try {
      text = host.readTextFile(file);
    } catch (error) {
      throw new AreteError(`Could not read ${file}`, 'INVALID_CONFIG', error);
    }
    if (text !== null) {
      let parsed: unknown;
      try {
        parsed = JSON.parse(text) as unknown;
      } catch (error) {
        throw new AreteError(`${file} is not JSON`, 'INVALID_CONFIG', error);
      }
      const edgeUrl = typeof parsed === 'object' && parsed !== null
        ? (parsed as Record<string, unknown>)['edgeUrl']
        : undefined;
      return typeof edgeUrl === 'string' ? edgeUrl : undefined;
    }
    const parent = parentDirectory(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return undefined;
}

/** The host for this runtime: process variables, the session file and `auth`'s key. */
export function systemEnvironmentSwitchHost(auth?: AuthConfig): EnvironmentSwitchHost {
  const browser = isBrowserEnvironment();
  return {
    browser,
    readVariable: (name) => (browser ? undefined : readEnvironmentVariable(name)),
    readSessionEdgeUrl: () => (browser ? undefined : readSessionEdgeUrl(systemProfileHost())),
    hasSecretKey: () => !browser && resolveSecretKey(auth) !== undefined,
  };
}

// ── Refusals ────────────────────────────────────────────────────────────────

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Codes a dev answer refuses with (contract §6), not ordinary request errors. */
export function isDevRefusalCode(code: string): boolean {
  return code.startsWith('devenv-') || code.startsWith('environment-token-');
}

/**
 * The refusal in a response body, from either envelope: the API problem
 * envelope (`{ schemaVersion, error, code, retryable, fix, … }`) or Program
 * Read's (`{ error: { code, detail: { message, fix, … } } }`, the code also
 * in `X-Error-Code`). `undefined` when the body is not a dev refusal.
 */
export function parseDevRefusal(
  status: number,
  headers: Headers,
  body: unknown
): DevEnvironmentError | undefined {
  const headerCode = headers.get('X-Error-Code')?.trim();
  if (isRecord(body) && isRecord(body['error'])) {
    const error = body['error'];
    const code = typeof error['code'] === 'string' ? error['code'] : headerCode;
    if (!code || !isDevRefusalCode(code)) return undefined;
    const detail = isRecord(error['detail']) ? { ...error['detail'] } : {};
    const message = typeof detail['message'] === 'string' ? detail['message'] : code;
    const fix = typeof detail['fix'] === 'string' ? detail['fix'] : undefined;
    delete detail['message'];
    delete detail['fix'];
    return new DevEnvironmentError(code, message, { fix, refusal: detail, status });
  }
  const code = isRecord(body) && typeof body['code'] === 'string' ? body['code'] : headerCode;
  if (!code || !isDevRefusalCode(code)) return undefined;
  const fields = isRecord(body) ? { ...body } : {};
  const message = typeof fields['error'] === 'string'
    ? fields['error']
    : typeof fields['message'] === 'string' ? fields['message'] : code;
  const fix = typeof fields['fix'] === 'string' ? fields['fix'] : undefined;
  for (const key of ['schemaVersion', 'error', 'message', 'code', 'retryable', 'fix']) {
    delete fields[key];
  }
  return new DevEnvironmentError(code, message, { fix, refusal: fields, status });
}

async function refusalOf(response: Response): Promise<DevEnvironmentError | undefined> {
  if (response.ok) return undefined;
  let body: unknown;
  try {
    const text = await response.clone().text();
    body = text ? JSON.parse(text) as unknown : undefined;
  } catch {
    body = undefined;
  }
  return parseDevRefusal(response.status, response.headers, body);
}

function requestUrl(input: RequestInfo | URL): string {
  return typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
}

function isAbort(error: unknown): boolean {
  return (error as { name?: unknown } | null)?.name === 'AbortError';
}

/**
 * A fetch for dev services reached directly: a response that carries a dev
 * refusal throws it as a {@link DevEnvironmentError}; every other response
 * passes through.
 */
export function createDevServiceFetch(fetchImpl: typeof fetch): typeof fetch {
  return (async (input: RequestInfo | URL, init?: RequestInit) => {
    const response = await fetchImpl(input, init);
    const refusal = await refusalOf(response);
    if (refusal) throw refusal;
    return response;
  }) as typeof fetch;
}

/**
 * A fetch for one surface through the edge. It sends no `Authorization`,
 * names `identity` in `X-Arete-Dev-Identity` (stacks name none), refuses any
 * URL off the edge, fails with `devenv-edge-unreachable` when nothing answers
 * or the answer lacks `X-Arete-Dev-Edge: 1`, and throws dev refusals.
 */
export function createEdgeFetch(
  edgeUrl: string,
  fetchImpl: typeof fetch,
  identity: Record<string, unknown> | undefined
): typeof fetch {
  const origin = new URL(edgeUrl).origin;
  const identityHeader = identity === undefined ? undefined : JSON.stringify(identity);
  return (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = requestUrl(input);
    let target: URL | undefined;
    try {
      target = new URL(url);
    } catch {
      target = undefined;
    }
    if (target?.origin !== origin) {
      throw invalidConfig(`A dev request to ${url} is not on the edge ${edgeUrl}`);
    }
    const headers = new Headers(init?.headers);
    headers.delete('authorization');
    if (identityHeader !== undefined) headers.set(DEV_IDENTITY_HEADER, identityHeader);
    let response: Response;
    try {
      response = await fetchImpl(url, { ...init, headers });
    } catch (error) {
      if (isAbort(error)) throw error;
      throw devEdgeUnreachable(edgeUrl, 'nothing answered', error);
    }
    if (response.headers.get(DEV_EDGE_HEADER)?.trim() !== '1') {
      throw devEdgeUnreachable(edgeUrl, `the answer to ${target.pathname} has no ${DEV_EDGE_HEADER} header`);
    }
    const refusal = await refusalOf(response);
    if (refusal) throw refusal;
    return response;
  }) as typeof fetch;
}

/**
 * Check the edge before a socket is opened: `GET /.well-known/arete-dev-edge`
 * must answer `200`, `{"edge": 1}` and `X-Arete-Dev-Edge: 1`.
 */
export async function checkDevEdge(edgeUrl: string, fetchImpl: typeof fetch): Promise<void> {
  let response: Response;
  try {
    response = await fetchImpl(`${edgeUrl}${DEV_EDGE_WELL_KNOWN_PATH}`, { method: 'GET' });
  } catch (error) {
    throw devEdgeUnreachable(edgeUrl, 'nothing answered', error);
  }
  let body: unknown;
  try {
    body = await response.json() as unknown;
  } catch {
    body = undefined;
  }
  if (
    response.status !== 200
    || response.headers.get(DEV_EDGE_HEADER)?.trim() !== '1'
    || !isRecord(body)
    || body['edge'] !== 1
  ) {
    throw devEdgeUnreachable(edgeUrl, `${DEV_EDGE_WELL_KNOWN_PATH} did not answer as an edge`);
  }
}

/** A path segment, percent-encoded except for `:` (release hashes, `package:`). */
export function pathSegment(value: string): string {
  return encodeURIComponent(value).replace(/%3A/gi, ':');
}

/** `url` with `name=value` appended to its query, `:` kept readable. */
export function withQueryParameter(url: string, name: string, value: string): string {
  const hash = url.indexOf('#');
  const base = hash < 0 ? url : url.slice(0, hash);
  const fragment = hash < 0 ? '' : url.slice(hash);
  const separator = base.includes('?') ? (base.endsWith('?') || base.endsWith('&') ? '' : '&') : '?';
  return `${base}${separator}${encodeURIComponent(name)}=${pathSegment(value)}${fragment}`;
}

/** The WebSocket origin of an edge: `ws://` for `http://`, `wss://` for `https://`. */
export function edgeWebsocketOrigin(edgeUrl: string): string {
  return edgeUrl.replace(/^http/, 'ws');
}

/**
 * In dev mode a transaction is signed by the wallet and sent through the
 * client's transaction transport, never by the wallet itself: a wallet's
 * own RPC is mainnet. An adapter that does not declare it submits through
 * the client's transport is refused.
 */
export function assertDevWalletSubmission(wallet: { readonly submitsThrough?: string }): void {
  if (wallet.submitsThrough === 'context') return;
  throw new DevEnvironmentError(
    DEV_ENVIRONMENT_CODES.walletSendUnsupported,
    'In dev mode the wallet must only sign: this wallet adapter sends transactions itself, '
      + 'which would reach mainnet instead of the dev environment',
    {
      fix: 'Use a sign-only adapter (createWalletAdapter from @usearete/adapter-kit or '
        + '@usearete/adapter-web3js without a transport option, or useWalletStandardWallet in React).',
    }
  );
}

/**
 * The option that decides `value` again without reading anything: what a
 * session hands its members so the whole session is in one world.
 */
export function environmentOptionOf(value: EnvironmentSwitch): EnvironmentOption {
  if (value.mode === 'production') return 'production';
  return value.via === 'edge'
    ? { kind: 'dev', url: value.edgeUrl }
    : { kind: 'dev', env: value.environment };
}
