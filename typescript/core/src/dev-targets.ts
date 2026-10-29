/**
 * Where each surface goes in dev mode (contract §3, §4, §5): through the
 * edge, or to the targets `resolve` answers for direct mode.
 *
 * Everything here maps installed identities to targets; the transports use
 * the result. Production never reaches this module.
 */

import { DEFAULT_A4_API_URL } from './a4-profile';
import { resolveAuthConfig, tokenEndpointApiKey } from './auth-keys';
import { ARETE_API_URL_ENV } from './credential-destination';
import {
  DEV_ENVIRONMENT_CODES,
  DevEnvironmentError,
  devSurfaceUnavailable,
  edgeWebsocketOrigin,
  parseDevRefusal,
  pathSegment,
  type DevEnvironmentSwitch,
} from './dev-environment';
import { readEnvironmentVariable } from './environment';
import {
  AreteError,
  parseWireErrorCode,
  type AuthConfig,
  type HostedSolanaGatewayCapabilityBinding,
  type ProgramReadBinding,
  type ProgramReleaseReference,
  type StackRelease,
} from './types';

/** The session endpoint direct-mode stack sessions are minted at. */
export const DEV_STACK_SESSION_ENDPOINT = 'https://api.arete.run/ws/sessions';

// ── Resolve (§4) ────────────────────────────────────────────────────────────

export interface DevProgramReadIdentity {
  readonly programId: string;
  readonly programReadBindingId: string;
  readonly programReleaseHash: string;
}

export interface DevProtocolIdentity {
  readonly package: string;
  readonly protocolReleaseHash: string;
  /** Absent before the protocol's first publish. */
  readonly deploymentId?: string;
}

/** The identities a client was built with; every member is optional. */
export interface DevResolveRequest {
  chain?: { readonly solanaGatewayBindingId: string };
  transactions?: { readonly solanaGatewayBindingId: string };
  programReads?: DevProgramReadIdentity[];
  stacks?: StackRelease[];
  protocols?: DevProtocolIdentity[];
}

/** One answer: a target (surface-specific fields) or a refusal. */
export type DevResolveEntry = Readonly<Record<string, unknown>> & {
  readonly status?: unknown;
};

export interface DevResolveAnswer {
  readonly environment?: { readonly id?: string; readonly name?: string };
  readonly chain?: DevResolveEntry;
  readonly transactions?: DevResolveEntry;
  readonly programReads?: readonly DevResolveEntry[];
  readonly stacks?: readonly DevResolveEntry[];
  readonly protocols?: readonly DevResolveEntry[];
}

function trimSlash(url: string): string {
  return url.replace(/\/+$/, '');
}

/**
 * The API origin direct mode resolves against: the client's `apiUrl`, then
 * `ARETE_API_URL`, then `https://api.arete.run`.
 */
export function devApiOrigin(apiUrl: string | undefined): string {
  const configured = apiUrl?.trim() || readEnvironmentVariable(ARETE_API_URL_ENV)?.trim();
  return trimSlash(configured || DEFAULT_A4_API_URL).replace(/\/api$/, '');
}

/** `POST <api>/api/dev/environments/{env}/resolve`. */
export function devResolveUrl(environment: string, apiUrl?: string): string {
  return `${devApiOrigin(apiUrl)}/api/dev/environments/${encodeURIComponent(environment)}/resolve`;
}

/** Whether a request names anything to resolve. */
export function isEmptyResolveRequest(request: DevResolveRequest): boolean {
  return !request.chain && !request.transactions
    && !request.programReads?.length && !request.stacks?.length && !request.protocols?.length;
}

/**
 * Call resolve once with a secret key. A refusal of the whole request
 * (`devenv-not-found`, a publishable key) throws.
 */
export async function resolveDevTargets(options: {
  readonly environment: string;
  readonly request: DevResolveRequest;
  readonly auth: AuthConfig | undefined;
  readonly apiUrl?: string;
  readonly fetch: typeof fetch;
}): Promise<DevResolveAnswer> {
  const url = devResolveUrl(options.environment, options.apiUrl);
  const auth = resolveAuthConfig(options.auth);
  // Asserts that the key may go to this origin.
  const key = tokenEndpointApiKey(auth, url);
  if (!key) {
    throw new DevEnvironmentError(
      DEV_ENVIRONMENT_CODES.notConfigured,
      `Dev mode with environment ${options.environment} needs a secret key`,
      { fix: 'Set ARETE_API_KEY, run `a4 auth login`, or use the `a4 dev up` edge.' }
    );
  }
  const response = await options.fetch(url, {
    method: 'POST',
    headers: { Authorization: `Bearer ${key}`, 'Content-Type': 'application/json' },
    body: JSON.stringify(options.request),
  });
  const text = await response.text();
  let body: unknown;
  try {
    body = text ? JSON.parse(text) as unknown : undefined;
  } catch {
    body = text;
  }
  if (!response.ok) {
    const refusal = parseDevRefusal(response.status, response.headers, body);
    if (refusal) throw refusal;
    const record = typeof body === 'object' && body !== null ? body as Record<string, unknown> : {};
    const wire = response.headers.get('X-Error-Code')
      ?? (typeof record['code'] === 'string' ? record['code'] : undefined);
    const message = typeof record['error'] === 'string' ? record['error'] : response.statusText;
    throw new AreteError(
      `Resolving dev environment ${options.environment} returned ${response.status}: ${message || 'request failed'}`,
      wire ? parseWireErrorCode(wire) : 'DEVENV_RESOLVE_FAILED',
      { status: response.status, wireErrorCode: wire ?? null, body }
    );
  }
  if (typeof body !== 'object' || body === null || Array.isArray(body)) {
    throw new AreteError('Resolve returned an unexpected body', 'INVALID_RESPONSE', { body });
  }
  return body as DevResolveAnswer;
}

const REFUSAL_FIELDS = new Set(['status', 'code', 'message', 'fix']);

/**
 * The target of one answer, or its refusal as an error. A missing answer is
 * `devenv-surface-unavailable`.
 */
export function devTargetOf(
  entry: DevResolveEntry | undefined,
  surface: string
): Readonly<Record<string, unknown>> {
  if (entry?.status === 'target') return entry;
  if (entry?.status === 'refused') {
    const code = typeof entry['code'] === 'string' ? entry['code'] : DEV_ENVIRONMENT_CODES.surfaceUnavailable;
    const refusal = Object.fromEntries(
      Object.entries(entry).filter(([key]) => !REFUSAL_FIELDS.has(key))
    );
    throw new DevEnvironmentError(
      code,
      typeof entry['message'] === 'string' ? entry['message'] : `The environment refused ${surface}`,
      { fix: typeof entry['fix'] === 'string' ? entry['fix'] : undefined, refusal }
    );
  }
  throw devSurfaceUnavailable(
    `The environment did not answer for ${surface}`,
    'Update the SDK and a4 to versions that agree on the dev environment contract.'
  );
}

function stringField(target: Readonly<Record<string, unknown>>, field: string, surface: string): string {
  const value = target[field];
  if (typeof value !== 'string' || value.length === 0) {
    throw devSurfaceUnavailable(
      `The environment's ${surface} target has no ${field}`,
      'Update the SDK and a4 to versions that agree on the dev environment contract.'
    );
  }
  return value;
}

// ── Per-surface targets ─────────────────────────────────────────────────────

/** How one HTTP surface is reached in dev mode. */
export interface DevHttpTarget {
  readonly endpoint: string;
  /** Edge mode: the `X-Arete-Dev-Identity` value. */
  readonly identity?: Record<string, unknown>;
  /** Direct mode: the environment named in token requests. */
  readonly environment?: string;
  /** Direct mode: where tokens are minted (the descriptor's). */
  readonly sessionEndpoint?: string;
}

export function gatewayTarget(
  dev: DevEnvironmentSwitch,
  answer: DevResolveAnswer | undefined,
  capability: 'chain' | 'transactions',
  binding: HostedSolanaGatewayCapabilityBinding
): DevHttpTarget {
  if (dev.via === 'edge') {
    return {
      endpoint: dev.edgeUrl,
      identity: { [capability]: { solanaGatewayBindingId: binding.solanaGatewayBindingId } },
    };
  }
  const label = capability === 'chain' ? 'chain reads' : 'transactions';
  const target = devTargetOf(answer?.[capability], label);
  return {
    endpoint: stringField(target, 'endpoint', label),
    environment: dev.environment,
    sessionEndpoint: binding.auth.sessionEndpoint,
  };
}

export interface DevProgramReadTarget extends DevHttpTarget {
  /** The release in the request path: the one the environment serves. */
  readonly pathRelease: ProgramReleaseReference;
}

export function programReadIdentity(
  programId: string,
  binding: ProgramReadBinding,
  release: ProgramReleaseReference
): DevProgramReadIdentity {
  return {
    programId,
    programReadBindingId: binding.programReadBindingId,
    programReleaseHash: release.programReleaseHash,
  };
}

export function programReadTarget(
  dev: DevEnvironmentSwitch,
  answer: DevResolveEntry | undefined,
  programId: string | undefined,
  binding: ProgramReadBinding,
  release: ProgramReleaseReference
): DevProgramReadTarget {
  if (!programId) {
    throw devSurfaceUnavailable(
      'This program SDK names no program id, so a dev environment cannot tell which program it reads',
      'Install the program from the registry (`a4 install program`) to read it in dev mode.'
    );
  }
  if (dev.via === 'edge') {
    return {
      endpoint: dev.edgeUrl,
      pathRelease: release,
      identity: { programReads: [programReadIdentity(programId, binding, release)] },
    };
  }
  const target = devTargetOf(answer, `Program Read for ${programId}`);
  return {
    endpoint: stringField(target, 'endpoint', 'Program Read'),
    pathRelease: {
      ...release,
      programReleaseHash: stringField(target, 'programReleaseHash', 'Program Read'),
    },
    environment: dev.environment,
    sessionEndpoint: binding.auth.sessionEndpoint,
  };
}

/** How a stack is reached in dev mode. */
export interface DevStackTarget {
  readonly websocketUrl: string;
  /** Where HTTP queries go, or why they can't. */
  readonly queryBaseUrl: string | DevEnvironmentError;
  /** Query parameters added to every HTTP query. */
  readonly queryParams: Readonly<Record<string, string>>;
  /** Direct mode: the session names this release and environment. */
  readonly sessionRelease?: StackRelease;
  readonly environment?: string;
}

export function stackTarget(
  dev: DevEnvironmentSwitch,
  answer: DevResolveEntry | undefined,
  release: StackRelease | undefined
): DevStackTarget {
  if (!release) {
    throw new DevEnvironmentError(
      DEV_ENVIRONMENT_CODES.stackNotLoaded,
      'This stack definition has no release, so a dev environment cannot tell which stack it is',
      { fix: 'Use a stack SDK installed from the registry (`a4 install stack`).' }
    );
  }
  if (dev.via === 'edge') {
    const path = `/stacks/${pathSegment(release.liveAlias)}`;
    const hash = pathSegment(release.stackManifestHash);
    return {
      websocketUrl: `${edgeWebsocketOrigin(dev.edgeUrl)}${path}?stackManifestHash=${hash}`,
      queryBaseUrl: `${dev.edgeUrl}${path}`,
      queryParams: { stackManifestHash: release.stackManifestHash },
    };
  }
  const target = devTargetOf(answer, `stack ${release.liveAlias}`);
  const httpUrl = target['httpUrl'];
  return {
    websocketUrl: stringField(target, 'websocketUrl', 'stack'),
    queryBaseUrl: typeof httpUrl === 'string' && httpUrl.length > 0
      ? trimSlash(httpUrl)
      : devSurfaceUnavailable(
          "The environment's stack serves its stream only, not HTTP queries",
          'Read the stack through its views (the WebSocket) in dev mode.'
        ),
    queryParams: {},
    sessionRelease: {
      stackManifestHash: stringField(target, 'stackManifestHash', 'stack'),
      liveAlias: release.liveAlias,
    },
    environment: dev.environment,
  };
}

/** How a protocol is reached in dev mode. */
export interface DevProtocolTarget {
  readonly apiUrl: string;
  /** `null` through the edge: nothing is minted. */
  readonly sessionEndpoint: string | null;
  /** `null` through the edge before the first publish. */
  readonly deploymentId: string | null;
  /** What the API path names in the deployment segment. */
  readonly deploymentSegment: string;
  readonly identity?: Record<string, unknown>;
}

export function protocolIdentity(protocol: {
  readonly package: string;
  readonly protocolReleaseHash: string;
  readonly deployment: { readonly deploymentId: string } | null | undefined;
}): DevProtocolIdentity {
  return {
    package: protocol.package,
    protocolReleaseHash: protocol.protocolReleaseHash,
    ...(protocol.deployment?.deploymentId ? { deploymentId: protocol.deployment.deploymentId } : {}),
  };
}

export function protocolTarget(
  dev: DevEnvironmentSwitch,
  answer: DevResolveEntry | undefined,
  protocol: Parameters<typeof protocolIdentity>[0]
): DevProtocolTarget {
  if (dev.via === 'edge') {
    const deploymentId = protocol.deployment?.deploymentId || null;
    return {
      apiUrl: `${dev.edgeUrl}/api`,
      sessionEndpoint: null,
      deploymentId,
      deploymentSegment: deploymentId ?? `package:${protocol.package}`,
      identity: { protocols: [protocolIdentity(protocol)] },
    };
  }
  const target = devTargetOf(answer, `protocol ${protocol.package}`);
  const deploymentId = stringField(target, 'deploymentId', 'protocol');
  return {
    apiUrl: trimSlash(stringField(target, 'apiUrl', 'protocol')),
    sessionEndpoint: stringField(target, 'sessionEndpoint', 'protocol'),
    deploymentId,
    deploymentSegment: deploymentId,
  };
}
