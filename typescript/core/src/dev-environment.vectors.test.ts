import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { Arete } from './client';
import {
  DevEnvironmentError,
  readSessionEdgeUrl,
  resolveEnvironmentSwitch,
  type EnvironmentOption,
  type EnvironmentSwitch,
  type EnvironmentSwitchHost,
} from './dev-environment';
import type { A4ProfileHost } from './a4-profile';
import { createProtocolClient } from './protocol/client';
import type { AnyProtocolDefinition } from './protocol/types';
import { AreteError, type StackDefinition } from './types';

// The cross-language vectors every SDK (TypeScript, Rust, Python), the edge
// and `a4` check themselves against.
function vectors(name: string): any {
  return JSON.parse(
    readFileSync(
      new URL(`../../../test-vectors/dev-environment-switch/v1/${name}`, import.meta.url),
      'utf8'
    )
  );
}

const DEV_INPUTS = ['ARETE_DEV_URL', 'ARETE_DEV_ENV', 'sessionFile'];
const SECRET_KEY = 'a4_sk_devenvironmentvectortest0000000000';
const ADDRESS = 'KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD';

/** A host that answers from a vector's input and records what is read. */
function recordingHost(input: any): { host: EnvironmentSwitchHost; reads: string[] } {
  const reads: string[] = [];
  const variables: Record<string, string> = input.variables ?? {};
  return {
    reads,
    host: {
      browser: input.browser,
      readVariable(name) {
        reads.push(name);
        return Object.prototype.hasOwnProperty.call(variables, name) ? variables[name] : undefined;
      },
      readSessionEdgeUrl() {
        reads.push('sessionFile');
        return input.sessionFile?.edgeUrl;
      },
      hasSecretKey() {
        reads.push('secretKey');
        return input.secretKey === true;
      },
    },
  };
}

function codeOf(run: () => unknown): string | undefined {
  try {
    run();
  } catch (error) {
    return (error as AreteError).code;
  }
  return undefined;
}

describe('dev-environment-switch v1: switch.json', () => {
  for (const testCase of vectors('switch.json').cases) {
    it(testCase.name, () => {
      const { input, expect: expected } = testCase;
      const { host, reads } = recordingHost(input);
      const run = () => resolveEnvironmentSwitch(input.option ?? undefined, host);
      if (expected.error) {
        expect(codeOf(run)).toBe(expected.error.code);
      } else {
        const result = run();
        const { devInputsRead, ...rest } = expected;
        expect(result).toEqual(rest);
        if (devInputsRead === false) {
          expect(reads.filter((name) => DEV_INPUTS.includes(name))).toEqual([]);
        }
      }
      // A browser reads no variable and no file, whatever the answer.
      if (input.browser) {
        expect(reads.filter((name) => name !== 'secretKey')).toEqual([]);
      }
    });
  }
});

describe('the nearest .arete/dev/session.json', () => {
  function host(files: Record<string, string>, cwd: string): A4ProfileHost {
    return {
      readEnv: () => undefined,
      readTextFile: (path) => files[path] ?? null,
      cwd: () => cwd,
      homeDir: () => undefined,
      joinPath: (...parts) => parts.join('/').replace(/\/+/g, '/'),
    };
  }

  it('is found at or above the working directory, the nearest first', () => {
    const files = {
      '/work/.arete/dev/session.json': JSON.stringify({ edgeUrl: 'http://127.0.0.1:9000' }),
      '/work/app/.arete/dev/session.json': JSON.stringify({ edgeUrl: 'http://127.0.0.1:8911' }),
    };
    expect(readSessionEdgeUrl(host(files, '/work/app/src'))).toBe('http://127.0.0.1:8911');
    expect(readSessionEdgeUrl(host(files, '/work/lib'))).toBe('http://127.0.0.1:9000');
    expect(readSessionEdgeUrl(host(files, '/elsewhere'))).toBeUndefined();
  });

  it('without an edgeUrl counts as absent; malformed is INVALID_CONFIG', () => {
    expect(readSessionEdgeUrl(host({ '/w/.arete/dev/session.json': '{"rpcUrl":"x"}' }, '/w'))).toBeUndefined();
    expect(codeOf(() => readSessionEdgeUrl(host({ '/w/.arete/dev/session.json': '{' }, '/w')))).toBe('INVALID_CONFIG');
  });
});

// ── Clients against a recording network ───────────────────────────────────

const targetsFile = vectors('targets.json');

interface Recorded {
  readonly url: string;
  readonly method: string;
  readonly headers: Headers;
  readonly body: any;
}

class MockWebSocket {
  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;

  constructor(readonly url: string) {
    queueMicrotask(() => {
      this.readyState = 1;
      this.onopen?.();
    });
  }

  send(): void {}

  close(code = 1000, reason = ''): void {
    this.readyState = 3;
    this.onclose?.({ code, reason });
  }
}

type EdgeAnswer =
  | { kind: 'connection-refused' }
  | { kind: 'response'; status: number; headers: Record<string, string>; body: unknown; request?: string };

interface NetworkOptions {
  resolveResponse?: unknown;
  /** The edge's answer to every request (or to `request` only). */
  edgeAnswer?: EdgeAnswer;
  /** A service's answer to every data request. */
  serviceResponse?: { status: number; headers: Record<string, string>; body: unknown };
}

function isLoopback(url: URL): boolean {
  return ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname);
}

function respond(status: number, headers: Record<string, string>, body: unknown): Response {
  const text = typeof body === 'string' ? body : JSON.stringify(body ?? {});
  return new Response(text, {
    status,
    headers: { 'content-type': 'application/json', ...headers },
  });
}

function network(options: NetworkOptions = {}) {
  const requests: Recorded[] = [];
  const sockets: string[] = [];
  let minted = 0;
  const fetchImpl = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
    const body = typeof init?.body === 'string'
      ? (() => {
          try {
            return JSON.parse(init.body as string);
          } catch {
            return init.body;
          }
        })()
      : undefined;
    requests.push({ url, method: init?.method ?? 'GET', headers: new Headers(init?.headers), body });
    const parsed = new URL(url);
    if (parsed.pathname.endsWith('/resolve')) {
      return respond(200, {}, options.resolveResponse);
    }
    if (parsed.pathname === '/ws/sessions' && !isLoopback(parsed)) {
      minted += 1;
      return respond(200, {}, {
        token: `token-${minted}`,
        expires_at: Math.floor(Date.now() / 1000) + 3600,
      });
    }
    if (isLoopback(parsed)) {
      const answer = options.edgeAnswer;
      const applies = answer?.kind === 'connection-refused'
        || (answer?.kind === 'response'
          && (answer.request === undefined || answer.request === `GET ${parsed.pathname}`));
      if (answer && applies) {
        if (answer.kind === 'connection-refused') throw new TypeError('fetch failed: ECONNREFUSED');
        return respond(answer.status, answer.headers, answer.body);
      }
      if (options.serviceResponse) {
        const { status, headers, body: answerBody } = options.serviceResponse;
        return respond(status, headers, answerBody);
      }
      if (parsed.pathname === '/.well-known/arete-dev-edge') {
        return respond(200, { 'X-Arete-Dev-Edge': '1' }, { edge: 1 });
      }
      return respond(200, { 'X-Arete-Dev-Edge': '1' }, {});
    }
    if (options.serviceResponse) {
      const { status, headers, body: answerBody } = options.serviceResponse;
      return respond(status, headers, answerBody);
    }
    return respond(200, {}, {});
  }) as typeof fetch;
  const websocketFactory = (url: string) => {
    sockets.push(url);
    return new MockWebSocket(url) as unknown as WebSocket;
  };
  return { requests, sockets, fetchImpl, websocketFactory };
}

function optionOf(mode: EnvironmentSwitch): EnvironmentOption {
  if (mode.mode === 'production') return 'production';
  return mode.via === 'edge' ? { kind: 'dev', url: mode.edgeUrl } : { kind: 'dev', env: mode.environment };
}

function stackOf(installed: any): StackDefinition {
  return {
    name: installed.stack.name,
    endpoints: installed.stack.endpoints,
    release: installed.stack.release,
    views: {},
    queries: { probe: { name: 'probe', path: '/probe', method: 'POST' } },
    programs: {
      klend: {
        name: 'klend',
        programId: installed.programs.klend.programId,
        accounts: { Reserve: { account: 'Reserve' } },
      },
    },
    programReads: { klend: installed.programReads.klend },
    gateway: installed.gateway,
  } as unknown as StackDefinition;
}

function protocolOf(installed: any): AnyProtocolDefinition {
  return {
    kind: 'protocol',
    name: 'tokenRaise',
    version: '1.0.0',
    package: installed.protocol.package,
    packageReleaseHash: 'arete:h1:package-release:sha256:0',
    protocolSpecHash: 'arete:h1:protocol-spec:sha256:0',
    protocolReleaseHash: installed.protocol.protocolReleaseHash,
    deployment: installed.protocol.deployment,
    instanceVisibility: 'shared',
    steps: {},
    events: {},
    metadata: {},
    stacks: {},
    programs: {},
  } as unknown as AnyProtocolDefinition;
}

async function settle(run: () => Promise<unknown>): Promise<unknown> {
  try {
    await run();
    return undefined;
  } catch (error) {
    return error;
  }
}

/** Use every surface of one stack client and one protocol client. */
async function exercise(mode: EnvironmentSwitch, installedKey: string, net: ReturnType<typeof network>) {
  const installed = targetsFile.installed[installedKey];
  const environment = optionOf(mode);
  const auth = { secretKey: SECRET_KEY, websocketFactory: net.websocketFactory };
  const errors: Record<string, unknown> = {};
  const client = await Arete.connect(stackOf(installed), {
    environment,
    auth,
    fetch: net.fetchImpl,
    autoConnect: false,
    autoReconnect: false,
  });
  errors.chain = await settle(() => client.chain.accountWithContext(ADDRESS));
  errors.transactions = await settle(() => client.transactions.getLatestBlockhash());
  errors['programReads.klend'] = await settle(
    () => (client.programs as any).klend.accounts.Reserve.fetch(ADDRESS)
  );
  errors.stackQuery = await settle(() => (client.queries as any).probe({}));
  errors.stack = await settle(() => client.connect());
  const protocol = createProtocolClient(protocolOf(installed), {
    environment,
    auth: { secretKey: SECRET_KEY },
    fetch: net.fetchImpl,
  });
  errors.protocol = await settle(() => protocol.instances.list());
  client.disconnect();
  return { client, protocol, errors };
}

function mintFor(requests: Recorded[], match: (body: any) => boolean): Recorded | undefined {
  return requests.find((request) => new URL(request.url).pathname === '/ws/sessions' && match(request.body));
}

function identityOf(request: Recorded | undefined): unknown {
  const value = request?.headers.get('X-Arete-Dev-Identity');
  return value === null || value === undefined ? null : JSON.parse(value);
}

function bearer(request: Recorded | undefined): string | null {
  return request?.headers.get('authorization') ?? null;
}

/** Compare a recorded mint with an expected token request; scopes may be a subset (one per call). */
function expectTokenRequest(mint: Recorded | undefined, expected: any, data: Recorded | undefined) {
  if (expected === null) {
    expect(mint).toBeUndefined();
    expect(bearer(data)).toBeNull();
    return;
  }
  expect(mint).toBeDefined();
  const { scopes, ...rest } = mint!.body;
  const { scopes: expectedScopes, ...expectedRest } = expected;
  expect(rest).toEqual(expectedRest);
  expect(scopes.length).toBeGreaterThan(0);
  for (const scope of scopes) expect(expectedScopes).toContain(scope);
  expect(bearer(data)).toMatch(/^Bearer token-\d+$/);
}

function stripToken(url: string): string {
  return url.replace(/[?&]hs_token=[^&]*$/, '').replace(/\?hs_token=[^&]*&/, '?');
}

describe('dev-environment-switch v1: targets.json', () => {
  for (const testCase of targetsFile.cases) {
    it(testCase.name, async () => {
      const { input, expect: expected } = testCase;
      const net = network({ resolveResponse: input.resolveResponse });
      const { protocol, errors } = await exercise(input.mode, input.installed, net);
      const { requests, sockets } = net;
      for (const [surface, error] of Object.entries(errors)) {
        // Bodies are minimal, so parsing may fail; no surface may be refused.
        expect(error instanceof DevEnvironmentError ? `${surface}: ${error.code}` : surface).toBe(surface);
      }
      // No credential ever goes to the edge.
      for (const request of requests) {
        if (isLoopback(new URL(request.url))) expect(bearer(request)).toBeNull();
      }

      // Resolve: one request per client, together the identities of both.
      const resolves = requests.filter((request) => new URL(request.url).pathname.endsWith('/resolve'));
      if (expected.resolveRequest === null) {
        expect(resolves).toEqual([]);
      } else {
        expect(resolves.length).toBeGreaterThan(0);
        const merged: Record<string, unknown> = {};
        for (const request of resolves) {
          expect(request.url).toBe(expected.resolveRequest.url);
          expect(request.method).toBe('POST');
          expect(bearer(request)).toBe(`Bearer ${SECRET_KEY}`);
          for (const [key, value] of Object.entries(request.body)) {
            merged[key] = Array.isArray(value) ? [...((merged[key] as unknown[]) ?? []), ...value] : value;
          }
        }
        expect(merged).toEqual(expected.resolveRequest.body);
      }

      // Chain reads and transactions.
      for (const capability of ['chain', 'transactions'] as const) {
        const marker = capability === 'chain' ? '/chain/' : '/transactions/v1/';
        const data = requests.find((request) => request.url.includes(marker));
        const want = expected[capability];
        expect(data, capability).toBeDefined();
        expect(data!.url.slice(0, data!.url.indexOf(marker))).toBe(want.endpoint);
        const mint = mintFor(requests, (body) =>
          body.targetKind === 'solana-gateway-binding'
          && (capability === 'chain' ? body.scopes.includes('read') : !body.scopes.includes('read')));
        expectTokenRequest(mint, want.tokenRequest, data);
        expect(mint?.url ?? null).toBe(want.sessionEndpoint);
        expect(identityOf(data)).toEqual(want.identityHeader);
      }

      // Program Read.
      {
        const want = expected.programReads.klend;
        const data = requests.find((request) => request.url.includes('/v1/releases/'));
        expect(data).toBeDefined();
        const at = data!.url.indexOf('/v1/releases/');
        expect(data!.url.slice(0, at)).toBe(want.endpoint);
        expect(decodeURIComponent(data!.url.slice(at + '/v1/releases/'.length).split('/')[0]!))
          .toBe(want.pathReleaseHash);
        const mint = mintFor(requests, (body) => body.targetKind === 'program-read-binding');
        expectTokenRequest(mint, want.tokenRequest, data);
        expect(mint?.url ?? null).toBe(want.sessionEndpoint);
        expect(identityOf(data)).toEqual(want.identityHeader);
      }

      // The stack: its socket and its HTTP queries.
      {
        const want = expected.stack;
        expect(sockets.map(stripToken)).toEqual([want.websocketUrl]);
        const mint = mintFor(requests, (body) => body.websocket_url !== undefined);
        expect(mint !== undefined).toBe(want.sessionRequired);
        expect(mint?.body ?? null).toEqual(want.sessionRequest);
        expect(mint?.url ?? null).toBe(want.sessionEndpoint);
        const query = requests.find((request) => new URL(request.url).pathname.endsWith('/probe'));
        expect(query).toBeDefined();
        const url = new URL(query!.url);
        expect(`${url.origin}${url.pathname.replace(/\/probe$/, '')}`).toBe(want.queryBaseUrl);
        expect(Object.fromEntries(url.searchParams)).toEqual(want.queryParams);
        expect(identityOf(query)).toEqual(want.identityHeader);
        if (input.mode.via === 'edge') {
          // The edge is checked before the socket opens.
          expect(requests.some((request) => request.url === `${input.mode.edgeUrl}/.well-known/arete-dev-edge`))
            .toBe(true);
        }
      }

      // The protocol.
      {
        const want = expected.protocol;
        const data = requests.find((request) => request.url.includes('/protocol-deployments/'));
        expect(data).toBeDefined();
        expect(data!.url).toBe(want.instancesUrl);
        const at = data!.url.indexOf('/protocol-deployments/');
        expect(data!.url.slice(0, at)).toBe(want.apiUrl);
        expect(data!.url.slice(at + '/protocol-deployments/'.length).split('/')[0])
          .toBe(want.deploymentPathSegment);
        const mint = mintFor(requests, (body) => body.targetKind === 'protocol-deployment');
        expect(mint?.url ?? null).toBe(want.sessionEndpoint);
        if (want.sessionEndpoint === null) expect(bearer(data)).toBeNull();
        expect(protocol.deploymentId || null).toBe(want.deploymentId);
        expect(identityOf(data)).toEqual(want.identityHeader);
      }
    });
  }
});

// ── Refusals ──────────────────────────────────────────────────────────────

const PRODUCTION_HOSTS = ['solana.arete.run', 'programs.arete.run'];
/** The resolve answer service refusals are exercised behind. */
const SAME_RELEASES = targetsFile.cases.find((testCase: any) => testCase.name === 'direct: same releases')
  .input.resolveResponse;

function isProductionRequest(url: string, installed: any): boolean {
  const parsed = new URL(url);
  // The edge rewrites installed identities itself: anything sent to it stays dev.
  if (isLoopback(parsed)) return false;
  return PRODUCTION_HOSTS.includes(parsed.hostname)
    || parsed.hostname.endsWith('.stack.arete.run')
    || (installed.protocol.deployment && url.includes(installed.protocol.deployment.deploymentId));
}

function isDevRequest(url: string): boolean {
  const parsed = new URL(url);
  return isLoopback(parsed)
    || parsed.hostname === 'devenv.arete.run'
    || parsed.hostname.endsWith('.devstack.arete.run')
    || parsed.pathname.includes('/api/dev/');
}

function expectRefusal(error: unknown, expected: any) {
  expect(error).toBeInstanceOf(AreteError);
  const refusal = error as AreteError;
  expect(refusal.code).toBe(expected.code);
  if (expected.message !== undefined) expect(refusal.message).toBe(expected.message);
  const details = (refusal.details ?? {}) as Record<string, unknown>;
  if (expected.fix !== undefined) {
    expect(details.fix).toBe(expected.fix);
    expect((refusal as DevEnvironmentError).fix).toBe(expected.fix);
  }
  if (expected.details !== undefined) {
    expect((refusal as DevEnvironmentError).refusal).toEqual(expected.details);
  }
}

/** The surface a service refusal is exercised on. */
function serviceSurface(testCase: any): string {
  const body = testCase.input.response.body;
  if (testCase.input.mode.mode === 'production') return 'protocol';
  if (body?.error && typeof body.error === 'object') return 'programReads.klend';
  return 'chain';
}

describe('dev-environment-switch v1: refusals.json', () => {
  for (const testCase of vectors('refusals.json').cases) {
    it(testCase.name, async () => {
      const { input, expect: expected } = testCase;
      const installed = targetsFile.installed[input.installed ?? 'default'];

      if (input.source === 'switch') {
        const { host } = recordingHost(input.switch);
        let error: unknown;
        try {
          resolveEnvironmentSwitch(input.switch.option ?? undefined, host);
        } catch (thrown) {
          error = thrown;
        }
        expectRefusal(error, expected);
        // The switch fails before any client exists: nothing is requested.
        return;
      }

      const net = network({
        resolveResponse: input.resolveResponse ?? SAME_RELEASES,
        edgeAnswer: input.source === 'edge' ? input.edgeAnswer : undefined,
        serviceResponse: input.source === 'service' ? input.response : undefined,
      });
      const { errors } = await exercise(input.mode, input.installed ?? 'default', net);

      const failing = expected.failingSurface
        ?? (input.source === 'service' ? serviceSurface(testCase) : 'chain');
      expectRefusal(errors[failing], expected);

      if (expected.otherSurfacesUsable) {
        for (const [surface, error] of Object.entries(errors)) {
          // A stack's stream and its queries are one stack: a refused stack
          // refuses both.
          if (surface === failing || (failing === 'stack' && surface === 'stackQuery')) continue;
          expect(error instanceof DevEnvironmentError ? `${surface}: ${error.code}` : surface).toBe(surface);
        }
      }
      if (expected.socketsOpened !== undefined) {
        expect(net.sockets.length).toBe(expected.socketsOpened);
      }
      if (expected.productionRequests !== undefined) {
        expect(net.requests.filter((request) => isProductionRequest(request.url, installed)).map((r) => r.url))
          .toEqual([]);
      }
      if (expected.devRequests !== undefined) {
        expect(net.requests.filter((request) => isDevRequest(request.url)).map((r) => r.url)).toEqual([]);
      }
    });
  }
});
