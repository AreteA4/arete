import { describe, expect, it, vi } from 'vitest';
import {
  Arete,
  createSession,
  createPreparedInstruction,
  extendProgram,
  extendStack,
  instructionOperation,
  programAccountRead,
  programQuery,
  stackQuery,
  validateProgramReadDescriptor,
  type ProgramReadDescriptor,
  type ProgramSdkDefinition,
} from './index';
import contract from './program-read-contract-v1.fixture.json';

const ALPHA_BINDING_ID = 'prb_00000000000000000000000000000001';
const BETA_BINDING_ID = 'prb_00000000000000000000000000000002';

const alphaProgram = {
  name: 'alpha',
  programId: 'Alpha1111111111111111111111111111111111111',
  programSpecHash: 'spec-alpha',
  accounts: {
    State: programAccountRead<{ value: string }>({ account: 'State' }),
  },
} as const;

const betaProgram = {
  name: 'beta',
  programId: 'Beta11111111111111111111111111111111111111',
  programSpecHash: 'spec-beta',
  accounts: {
    Vault: programAccountRead<{ value: string }>({ account: 'Vault' }),
  },
} as const;

function descriptor(
  program: 'alpha' | 'beta',
  endpoint?: string,
  bindingId?: string,
  sessionEndpoint?: string,
  releaseHash = `release-${program}`
): ProgramReadDescriptor {
  const resolvedBindingId = bindingId
    ?? (program === 'alpha' ? ALPHA_BINDING_ID : BETA_BINDING_ID);
  return {
    release: {
      programReleaseHash: releaseHash,
      programSpecHash: `spec-${program}`,
    },
    transport: endpoint
      ? {
          kind: 'hosted-binding',
          binding: {
            endpoint,
            programReadBindingId: resolvedBindingId,
            auth: {
              required: sessionEndpoint !== undefined,
              sessionEndpoint: sessionEndpoint ?? 'https://auth.example.test/session',
              targetKind: 'program-read-binding',
              targetId: resolvedBindingId,
            },
          },
        }
      : { kind: 'local-http', endpointSource: 'connect-http-url' },
  };
}

function alphaReadStack(endpoint: string, sessionEndpoint?: string) {
  return {
    name: 'alpha-program-reads',
    endpoints: { ws: '' },
    views: {},
    programs: { alpha: alphaProgram },
    programReads: {
      alpha: descriptor(
        'alpha',
        endpoint,
        undefined,
        sessionEndpoint
      ),
    },
  } as const;
}

describe('program read transports', () => {
  it('accepts generated local HTTP descriptors unchanged', () => {
    expect(() => validateProgramReadDescriptor('alpha', descriptor('alpha'))).not.toThrow();
  });

  it('preserves typed release hashes in release-addressed paths', async () => {
    const releaseHash = 'arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ value: 'typed' })));
    const stack = {
      ...alphaReadStack('https://reads.example.test'),
      programReads: {
        alpha: descriptor('alpha', 'https://reads.example.test', undefined, undefined, releaseHash),
      },
    } as const;
    const client = await Arete.connect(stack, { fetch: fetchMock as typeof fetch });

    await client.programs.alpha.accounts.State.fetch('address');

    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `https://reads.example.test/v1/releases/${releaseHash}/accounts/State/address`
    );
    client.disconnect();
  });

  it('returns the unified raw single-account value without unwrapping it', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(contract.success.rawValue)));
    const client = await Arete.connect(alphaReadStack('https://reads.example.test'), {
      fetch: fetchMock as typeof fetch,
    });

    await expect(client.programs.alpha.accounts.State.fetch('present'))
      .resolves.toEqual(contract.success.rawValue);
  });

  it('returns null for a missing account in the unified contract', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(contract.success.missing)));
    const client = await Arete.connect(alphaReadStack('https://reads.example.test'), {
      fetch: fetchMock as typeof fetch,
    });

    await expect(client.programs.alpha.accounts.State.fetch('missing')).resolves.toBeNull();
  });

  it('translates the unified exists object to a boolean', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(contract.success.exists)));
    const client = await Arete.connect(alphaReadStack('https://reads.example.test'), {
      fetch: fetchMock as typeof fetch,
    });

    await expect(client.programs.alpha.accounts.State.exists('present')).resolves.toBe(true);
  });

  it('preserves unified batch statuses and values', async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      expect(init?.method).toBe('POST');
      expect(JSON.parse(String(init?.body))).toEqual({
        addresses: ['present', 'missing', 'broken'],
      });
      return new Response(JSON.stringify(contract.success.batch));
    });
    const client = await Arete.connect(alphaReadStack('https://reads.example.test'), {
      fetch: fetchMock as typeof fetch,
    });

    await expect(client.programs.alpha.accounts.State.fetchMany(['present', 'missing', 'broken']))
      .resolves.toEqual(contract.success.batch);
  });

  it('reads an error code from the hosted nested error body', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(contract.errors.nested), {
      status: 422,
    }));
    const client = await Arete.connect(alphaReadStack('https://reads.example.test'), {
      fetch: fetchMock as typeof fetch,
    });

    await expect(client.programs.alpha.accounts.State.fetch('broken')).rejects.toMatchObject({
      status: 422,
      serverErrorCode: 'ACCOUNT_DECODE_FAILED',
    });
  });

  it('prefers X-Error-Code over a conflicting nested body code', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(contract.errors.nested), {
      status: 422,
      headers: { 'X-Error-Code': 'ACCOUNT_OWNER_MISMATCH' },
    }));
    const client = await Arete.connect(alphaReadStack('https://reads.example.test'), {
      fetch: fetchMock as typeof fetch,
    });

    await expect(client.programs.alpha.accounts.State.fetch('broken')).rejects.toMatchObject({
      serverErrorCode: 'ACCOUNT_OWNER_MISMATCH',
    });
  });

  it.each([
    ['a non-refreshable 401', 401, contract.errors.nonRefreshable],
    ['a refreshable 5xx response', 503, contract.errors.refreshable],
  ])('does not retry %s', async (_name, status, errorBody) => {
    let readRequests = 0;
    let tokenRequests = 0;
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      if (String(input).includes('/session')) {
        tokenRequests += 1;
        return new Response(JSON.stringify({ token: 'token', scopes: ['read'] }));
      }
      readRequests += 1;
      return new Response(JSON.stringify(errorBody), { status });
    });
    const client = await Arete.connect(alphaReadStack(
      'https://reads.example.test',
      'https://auth.example.test/session'
    ), { fetch: fetchMock as typeof fetch });

    await expect(client.programs.alpha.accounts.State.fetch('address')).rejects.toBeInstanceOf(Error);
    expect(readRequests).toBe(1);
    expect(tokenRequests).toBe(1);
  });

  it('does not retry a network error', async () => {
    let readRequests = 0;
    let tokenRequests = 0;
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      if (String(input).includes('/session')) {
        tokenRequests += 1;
        return new Response(JSON.stringify({ token: 'token', scopes: ['read'] }));
      }
      readRequests += 1;
      throw new TypeError('network unavailable');
    });
    const client = await Arete.connect(alphaReadStack(
      'https://reads.example.test',
      'https://auth.example.test/session'
    ), { fetch: fetchMock as typeof fetch });

    await expect(client.programs.alpha.accounts.State.fetch('address'))
      .rejects.toThrow('network unavailable');
    expect(readRequests).toBe(1);
    expect(tokenRequests).toBe(1);
  });

  it('keeps two program origins, releases, auth targets, and batch statuses isolated', async () => {
    const tokenRequests: unknown[] = [];
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes('auth.example.test')) {
        const request = JSON.parse(String(init?.body)) as { targetId: string };
        tokenRequests.push(request);
        return new Response(JSON.stringify({
          token: `token-${request.targetId}`,
          scopes: ['read'],
        }));
      }
      const authorization = new Headers(init?.headers).get('authorization');
      if (url.includes('alpha.example.test')) {
        expect(authorization).toBe(`Bearer token-${ALPHA_BINDING_ID}`);
        if (init?.method === 'POST') {
          return new Response(JSON.stringify({
            items: [
              { address: 'alpha-1', status: 'ok', value: { value: 'alpha' } },
              { address: 'missing', status: 'missing' },
              { address: 'broken', status: 'error', error: { code: 'ACCOUNT_DECODE_FAILED' } },
            ],
          }));
        }
        return new Response(JSON.stringify({ value: 'alpha' }));
      }
      expect(authorization).toBe(`Bearer token-${BETA_BINDING_ID}`);
      return new Response(JSON.stringify({ value: 'beta' }));
    });
    const stack = {
      name: 'independent-programs',
      endpoints: { ws: '' },
      views: {},
      programs: { alpha: alphaProgram, beta: betaProgram },
      programReads: {
        alpha: descriptor(
          'alpha',
          'https://alpha.example.test/catalog/alpha/',
          ALPHA_BINDING_ID,
          'https://auth.example.test/sessions/alpha'
        ),
        beta: descriptor(
          'beta',
          'https://beta.example.test/catalog/beta',
          BETA_BINDING_ID,
          'https://auth.example.test/sessions/beta'
        ),
      },
    } as const;

    const client = await Arete.connect(stack, {
      fetch: fetchMock as typeof fetch,
      auth: { publishableKey: 'pk_test' },
    });
    await expect(client.programs.alpha.accounts.State.fetch('alpha-1'))
      .resolves.toEqual({ value: 'alpha' });
    await expect(client.programs.beta.accounts.Vault.fetch('beta-1'))
      .resolves.toEqual({ value: 'beta' });
    await expect(client.programs.alpha.accounts.State.fetchMany(['alpha-1', 'missing', 'broken']))
      .resolves.toEqual({
        items: [
          { address: 'alpha-1', status: 'ok', value: { value: 'alpha' } },
          { address: 'missing', status: 'missing' },
          { address: 'broken', status: 'error', error: { code: 'ACCOUNT_DECODE_FAILED' } },
        ],
      });

    const readUrls = fetchMock.mock.calls
      .map(([input]) => String(input))
      .filter((url) => !url.includes('auth.example.test'));
    expect(readUrls).toEqual([
      'https://alpha.example.test/catalog/alpha/v1/releases/release-alpha/accounts/State/alpha-1',
      'https://beta.example.test/catalog/beta/v1/releases/release-beta/accounts/Vault/beta-1',
      'https://alpha.example.test/catalog/alpha/v1/releases/release-alpha/accounts/State',
    ]);
    expect(tokenRequests).toEqual([
      {
        targetKind: 'program-read-binding',
        targetId: ALPHA_BINDING_ID,
        programReleaseHash: 'release-alpha',
        scopes: ['read'],
      },
      {
        targetKind: 'program-read-binding',
        targetId: BETA_BINDING_ID,
        programReleaseHash: 'release-beta',
        scopes: ['read'],
      },
    ]);
  });

  it('replaces the complete generated descriptor with a runtime override', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ value: 'runtime' })));
    const stack = {
      name: 'runtime-override',
      endpoints: { ws: '', http: 'https://stack.example.test/api' },
      views: {},
      programs: { alpha: alphaProgram },
      programReads: {
        alpha: descriptor('alpha', 'https://generated.example.test/generated'),
      },
    } as const;
    const client = await Arete.connect(stack, {
      transport: 'http',
      fetch: fetchMock as typeof fetch,
      programReads: {
        alpha: descriptor(
          'alpha',
          'https://runtime.example.test/exact/prefix/',
          undefined,
          undefined,
          'release-runtime'
        ),
      },
    });

    await client.programs.alpha.accounts.State.fetch('address');
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      'https://runtime.example.test/exact/prefix/v1/releases/release-runtime/accounts/State/address'
    );
  });

  it('lets runtime auth strategy override hosted session metadata', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url === 'https://runtime-auth.example.test/session') {
        expect(JSON.parse(String(init?.body))).toEqual({
          targetKind: 'program-read-binding',
          targetId: ALPHA_BINDING_ID,
          programReleaseHash: 'release-alpha',
          scopes: ['read'],
        });
        return new Response(JSON.stringify({ token: 'runtime-token', scopes: ['read'] }));
      }
      expect(new Headers(init?.headers).get('authorization')).toBe('Bearer runtime-token');
      return new Response(JSON.stringify({ value: 'ok' }));
    });
    const stack = {
      name: 'auth-precedence',
      endpoints: { ws: '' },
      views: {},
      programs: { alpha: alphaProgram },
      programReads: {
        alpha: descriptor(
          'alpha',
          'https://reads.example.test/prefix',
          ALPHA_BINDING_ID,
          'https://generated-auth.example.test/session'
        ),
      },
    } as const;
    const client = await Arete.connect(stack, {
      fetch: fetchMock as typeof fetch,
      auth: { tokenEndpoint: 'https://runtime-auth.example.test/session' },
    });

    await client.programs.alpha.accounts.State.fetch('address');
    expect(fetchMock.mock.calls.some(([input]) => String(input).includes('generated-auth'))).toBe(false);
  });

  it('refreshes the exact targeted token after a release read auth failure', async () => {
    const tokenRequests: unknown[] = [];
    let readRequests = 0;
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes('/session')) {
        tokenRequests.push(JSON.parse(String(init?.body)));
        return new Response(JSON.stringify({
          token: `token-${tokenRequests.length}`,
          scopes: ['read'],
        }));
      }
      readRequests += 1;
      if (new Headers(init?.headers).get('authorization') === 'Bearer token-1') {
        return new Response(JSON.stringify(contract.errors.refreshable), {
          status: 401,
        });
      }
      return new Response(JSON.stringify({ value: 'refreshed' }));
    });
    const stack = {
      name: 'targeted-refresh',
      endpoints: { ws: '' },
      views: {},
      programs: { alpha: alphaProgram },
      programReads: {
        alpha: descriptor(
          'alpha',
          'https://reads.example.test/program',
          ALPHA_BINDING_ID,
          'https://auth.example.test/session'
        ),
      },
    } as const;
    const client = await Arete.connect(stack, { fetch: fetchMock as typeof fetch });

    await expect(client.programs.alpha.accounts.State.fetch('address'))
      .resolves.toEqual({ value: 'refreshed' });
    expect(tokenRequests).toEqual([
      {
        targetKind: 'program-read-binding',
        targetId: ALPHA_BINDING_ID,
        programReleaseHash: 'release-alpha',
        scopes: ['read'],
      },
      {
        targetKind: 'program-read-binding',
        targetId: ALPHA_BINDING_ID,
        programReleaseHash: 'release-alpha',
        scopes: ['read'],
      },
    ]);
    expect(readRequests).toBe(2);
  });

  it('connects a hosted standalone program with no stack URL and fails stack surfaces at operation time', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ value: 'hosted' })));
    const stack = {
      name: 'hosted-standalone',
      endpoints: { ws: '' },
      views: {},
      queries: {
        stackOnly: stackQuery<{}, { value: string }>({ name: 'stackOnly', path: '/queries/stack-only' }),
      },
      programs: {
        alpha: {
          ...alphaProgram,
          queries: {
            stackProgramQuery: programQuery<{}, { value: string }>({
              name: 'stackProgramQuery',
              path: '/programs/alpha/queries/stack-only',
            }),
          },
        },
      },
      programReads: {
        alpha: descriptor('alpha', 'https://reads.example.test/standalone'),
      },
    } as const;

    const client = await Arete.connect(stack, { fetch: fetchMock as typeof fetch });
    await expect(client.programs.alpha.accounts.State.fetch('address'))
      .resolves.toEqual({ value: 'hosted' });
    await expect(client.chain.exists('address')).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    await expect(client.queries.stackOnly({})).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    await expect(client.programs.alpha.queries.stackProgramQuery({}))
      .rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    await expect(client.transactions.getBlockHeight())
      .rejects.toMatchObject({ code: 'INVALID_CONFIG' });
  });

  it('combines a generated local release with an explicit HTTP endpoint', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ value: 'local' })));
    const stack = {
      name: 'local-release',
      endpoints: { ws: '', http: '' },
      views: {},
      programs: { alpha: alphaProgram },
      programReads: { alpha: descriptor('alpha') },
    } as const;
    const client = await Arete.connect(stack, {
      transport: 'http',
      httpUrl: 'http://127.0.0.1:8879/local/api/',
      fetch: fetchMock as typeof fetch,
    });

    await client.programs.alpha.accounts.State.fetch('address');
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      'http://127.0.0.1:8879/local/api/v1/releases/release-alpha/accounts/State/address'
    );
  });

  it('uses a hosted binding endpoint even when httpUrl and stack HTTP are present', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ value: 'hosted' })));
    const stack = {
      name: 'hosted-endpoint-isolation',
      endpoints: { ws: '', http: 'https://stack.example.test/api' },
      views: {},
      programs: { alpha: alphaProgram },
      programReads: {
        alpha: descriptor('alpha', 'https://hosted.example.test/exact'),
      },
    } as const;
    const client = await Arete.connect(stack, {
      transport: 'http',
      httpUrl: 'https://runtime-stack.example.test/override',
      fetch: fetchMock as typeof fetch,
    });

    await client.programs.alpha.accounts.State.fetch('address');
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      'https://hosted.example.test/exact/v1/releases/release-alpha/accounts/State/address'
    );
  });

  it('never uses a WebSocket-derived HTTP endpoint for release reads', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ value: 'wrong' })));
    const stack = {
      name: 'no-ws-derived-program-http',
      endpoints: { ws: 'wss://stream.example.test/socket/prefix' },
      views: {},
      programs: { alpha: alphaProgram },
      programReads: { alpha: descriptor('alpha') },
    } as const;
    await expect(Arete.connect(stack, {
      autoConnect: false,
      fetch: fetchMock as typeof fetch,
    })).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('rejects an incomplete hosted binding during connect without falling back', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ value: 'legacy' })));
    const stack = {
      name: 'release-without-endpoint',
      endpoints: { ws: '', http: 'https://stack.example.test/api' },
      views: {},
      programs: { alpha: alphaProgram },
      programReads: {
        alpha: {
          ...descriptor('alpha', 'https://generated.example.test'),
          transport: {
            kind: 'hosted-binding',
            binding: {
              endpoint: '',
              programReadBindingId: ALPHA_BINDING_ID,
              auth: {
                targetKind: 'program-read-binding',
                targetId: ALPHA_BINDING_ID,
                sessionEndpoint: 'https://auth.example.test/session',
              },
            },
          },
        },
      },
    } as const;
    await expect(Arete.connect(stack, {
      transport: 'http',
      fetch: fetchMock as typeof fetch,
    })).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(warn).not.toHaveBeenCalledWith(expect.stringContaining('legacy enclosing-stack'));
    warn.mockRestore();
  });

  it.each([
    ['endpoint scheme', {
      endpoint: 'http://reads.example.test',
      programReadBindingId: ALPHA_BINDING_ID,
      auth: {
        targetKind: 'program-read-binding',
        targetId: ALPHA_BINDING_ID,
        sessionEndpoint: 'https://auth.example.test/session',
      },
    }],
    ['session endpoint scheme', {
      endpoint: 'https://reads.example.test',
      programReadBindingId: ALPHA_BINDING_ID,
      auth: {
        targetKind: 'program-read-binding',
        targetId: ALPHA_BINDING_ID,
        sessionEndpoint: 'http://auth.example.test/session',
      },
    }],
    ['binding ID', {
      endpoint: 'https://reads.example.test',
      programReadBindingId: 'prb_too-short',
      auth: {
        targetKind: 'program-read-binding',
        targetId: 'prb_too-short',
        sessionEndpoint: 'https://auth.example.test/session',
      },
    }],
    ['auth target kind', {
      endpoint: 'https://reads.example.test',
      programReadBindingId: ALPHA_BINDING_ID,
      auth: {
        targetKind: 'other',
        targetId: ALPHA_BINDING_ID,
        sessionEndpoint: 'https://auth.example.test/session',
      },
    }],
    ['auth target ID', {
      endpoint: 'https://reads.example.test',
      programReadBindingId: ALPHA_BINDING_ID,
      auth: {
        targetKind: 'program-read-binding',
        targetId: BETA_BINDING_ID,
        sessionEndpoint: 'https://auth.example.test/session',
      },
    }],
    ['auth metadata', {
      endpoint: 'https://reads.example.test',
      programReadBindingId: ALPHA_BINDING_ID,
      auth: {
        targetKind: 'program-read-binding',
        targetId: ALPHA_BINDING_ID,
        sessionEndpoint: '',
      },
    }],
  ])('rejects a hosted binding with invalid %s before network', async (_field, binding) => {
    const fetchMock = vi.fn();
    const stack = {
      name: 'incomplete-hosted-binding',
      endpoints: { ws: '', http: 'https://stack.example.test' },
      views: {},
      programs: { alpha: alphaProgram },
      programReads: {
        alpha: {
          release: {
            programReleaseHash: 'release-alpha',
            programSpecHash: 'spec-alpha',
          },
          transport: { kind: 'hosted-binding', binding },
        },
      },
    } as const;

    await expect(Arete.connect(stack as any, {
      transport: 'http',
      fetch: fetchMock as unknown as typeof fetch,
    })).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('rejects program account reads without a descriptor', async () => {
    const fetchMock = vi.fn();
    const legacyAlpha = {
      ...alphaProgram,
      accounts: {
        State: programAccountRead<{ value: string }>({ account: 'State' }),
      },
    } as const;
    const legacyBeta = {
      ...betaProgram,
      accounts: {
        Vault: programAccountRead<{ value: string }>({ account: 'Vault' }),
      },
    } as const;
    const client = await Arete.connect({
      name: 'legacy-programs',
      endpoints: { ws: '', http: 'https://stack.example.test/api' },
      views: {},
      programs: { alpha: legacyAlpha, beta: legacyBeta },
    }, { transport: 'http', fetch: fetchMock as unknown as typeof fetch });

    await expect(client.programs.alpha.accounts.State.fetch('address'))
      .rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    await expect(client.programs.beta.accounts.Vault.fetch('address'))
      .rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('rejects release and portable definition identity mismatches', async () => {
    const stack = {
      name: 'mismatched-release',
      endpoints: { ws: '' },
      views: {},
      programs: { alpha: alphaProgram },
      programReads: {
        alpha: {
          ...descriptor('alpha', 'https://reads.example.test'),
          release: { programReleaseHash: 'release-alpha', programSpecHash: 'wrong-spec' },
        },
      },
    } as const;

    await expect(Arete.connect(stack)).rejects.toMatchObject({
      code: 'PROGRAM_RELEASE_MISMATCH',
    });
  });

  it('requires generated programReads keys to exactly match programs', async () => {
    const stack = {
      name: 'incomplete-program-reads',
      endpoints: { ws: '', http: 'https://stack.example.test' },
      views: {},
      programs: { alpha: alphaProgram, beta: betaProgram },
      programReads: { alpha: descriptor('alpha', 'https://alpha.example.test') },
    } as const;

    await expect(Arete.connect(stack, { transport: 'http' })).rejects.toMatchObject({
      code: 'INVALID_CONFIG',
      message: expect.stringContaining('missing: beta'),
    });
  });

  it('keeps queries, chain reads, and transactions on stack HTTP', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes('/transactions/v1/block-height')) {
        return new Response(JSON.stringify({ blockHeight: '42' }));
      }
      if (url.includes('/chain/exists/')) {
        return new Response(JSON.stringify({ exists: true }));
      }
      return new Response(JSON.stringify({ value: url.includes('program.example.test') ? 'program' : 'stack' }));
    });
    const extendedAlpha = extendProgram(
      {
        ...alphaProgram,
        queries: {
          programQuery: programQuery<{}, { value: string }>({
            name: 'programQuery',
            path: '/programs/alpha/queries/stack-bound',
          }),
        },
      },
      {
        createOperations(context) {
          return {
            instructions: {
              checkChain: instructionOperation(async () => {
                const exists = await context.chain.exists('extension-address');
                return createPreparedInstruction({
                  name: 'checkChain',
                  instruction: {
                    programId: alphaProgram.programId,
                    keys: [],
                    data: new Uint8Array(),
                  },
                  artifacts: { exists },
                });
              }),
            },
          };
        },
      }
    );
    const stack = {
      name: 'surface-separation',
      endpoints: { ws: '', http: 'https://stack.example.test/api/prefix' },
      views: {},
      queries: {
        stackQuery: stackQuery<{}, { value: string }>({ name: 'stackQuery', path: '/queries/stack' }),
      },
      programs: {
        alpha: extendedAlpha,
      },
      programReads: {
        alpha: descriptor('alpha', 'https://program.example.test/read/prefix'),
      },
    } as const;
    const client = await Arete.connect(stack, {
      transport: 'http',
      fetch: fetchMock as typeof fetch,
    });

    await client.programs.alpha.accounts.State.fetch('address');
    await expect(client.programs.alpha.instructions.checkChain.prepare({}))
      .resolves.toMatchObject({ artifacts: { exists: true } });
    await client.queries.stackQuery({});
    await client.programs.alpha.queries.programQuery({});
    await client.chain.exists('address');
    await client.transactions.getBlockHeight();

    const urls = fetchMock.mock.calls.map(([input]) => String(input));
    expect(urls[0]).toContain('https://program.example.test/read/prefix/');
    expect(urls.slice(1)).toEqual([
      'https://stack.example.test/api/prefix/chain/exists/extension-address',
      'https://stack.example.test/api/prefix/queries/stack',
      'https://stack.example.test/api/prefix/programs/alpha/queries/stack-bound',
      'https://stack.example.test/api/prefix/chain/exists/address',
      'https://stack.example.test/api/prefix/transactions/v1/block-height',
    ]);
  });

  it('applies session per-program, member, and session-wide precedence to promoted programs', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ value: 'session' })));
    const stack = {
      name: 'session-stack',
      endpoints: { ws: '', http: 'https://stack.example.test' },
      views: {},
      programs: { alpha: alphaProgram },
      programReads: {
        alpha: descriptor('alpha', 'https://generated.example.test'),
      },
    } as const;
    const session = await createSession(
      { stacks: { member: stack } },
      {
        transport: 'http',
        fetch: fetchMock as typeof fetch,
        programRead: descriptor('alpha', 'https://session-wide.example.test', undefined, undefined, 'release-session'),
        programReads: {
          alpha: descriptor('alpha', 'https://session-program.example.test', undefined, undefined, 'release-session-program'),
        },
        stacks: {
          member: {
            programRead: descriptor('alpha', 'https://member.example.test', undefined, undefined, 'release-member'),
            programReads: {
              alpha: descriptor('alpha', 'https://member-program.example.test/exact', undefined, undefined, 'release-member-program'),
            },
          },
        },
      }
    );

    expect(session.programs.alpha).toBe(session.stacks.member.programs.alpha);
    await session.programs.alpha.accounts.State.fetch('address');
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      'https://member-program.example.test/exact/v1/releases/release-member-program/accounts/State/address'
    );
    session.close();
  });

  it('uses standalone session descriptors without a stack endpoint', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ value: 'standalone-session' })));
    const session = await createSession(
      {
        programs: { alpha: alphaProgram },
        programReads: {
          alpha: descriptor('alpha', 'https://standalone.example.test/programs'),
        },
      },
      { fetch: fetchMock as typeof fetch }
    );

    await expect(session.programs.alpha.accounts.State.fetch('address'))
      .resolves.toEqual({ value: 'standalone-session' });
    expect(String(fetchMock.mock.calls[0]?.[0])).toContain(
      'https://standalone.example.test/programs/v1/releases/release-alpha/'
    );
    session.close();
  });

  it('preserves attached program inference and applies its parallel session override', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ value: 'attached' })));
    const baseStack = {
      name: 'attached-program-stack',
      endpoints: { ws: '', http: 'https://stack.example.test' },
      views: {},
      programs: {},
    } as const;
    const session = await createSession(
      { stacks: { member: baseStack } },
      {
        transport: 'http',
        fetch: fetchMock as typeof fetch,
        stacks: {
          member: {
            programs: { alpha: alphaProgram },
            programReads: {
              alpha: {
                release: {
                  programReleaseHash: 'release-attached',
                  programSpecHash: 'spec-alpha',
                },
                transport: descriptor('alpha', 'https://attached.example.test/read').transport,
              },
            },
          },
        },
      }
    );

    await expect(session.programs.alpha.accounts.State.fetch('address'))
      .resolves.toEqual({ value: 'attached' });
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      'https://attached.example.test/read/v1/releases/release-attached/accounts/State/address'
    );
    session.close();
  });

  it('keeps release identity separate from portable extension composition', () => {
    const read = descriptor('alpha', 'https://reads.example.test');
    const extendedProgram = extendProgram(
      { ...alphaProgram, sdkDefinitionHash: 'sdk-definition-alpha' },
      { constants: { unit: 1 } }
    );
    const stack = {
      name: 'extension-identity',
      endpoints: { ws: '', http: 'https://stack.example.test' },
      views: {},
      programs: { alpha: extendedProgram },
      programReads: { alpha: read },
    } as const;
    const extendedStack = extendStack(stack, { constants: { network: 'test' } });

    expect(extendedProgram.programSpecHash).toBe('spec-alpha');
    expect('sdkDefinitionHash' in extendedProgram).toBe(false);
    expect('programReleaseHash' in extendedProgram).toBe(false);
    expect(extendedStack.programReads.alpha).toBe(read);

    const portable: ProgramSdkDefinition = alphaProgram;
    // @ts-expect-error release identity is deliberately absent from portable definitions
    expect(portable.programReleaseHash).toBeUndefined();
    // @ts-expect-error decoder engine identity is private release metadata
    expect(portable.decoderEngineId).toBeUndefined();
  });
});
