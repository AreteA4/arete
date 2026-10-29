import { readFileSync } from 'node:fs';

import { afterEach, describe, expect, it, vi } from 'vitest';

import { Arete } from './client';
import { createSession } from './session';
import { resolveDevTargets } from './dev-targets';
import { resolveEnvironmentSwitch, systemEnvironmentSwitchHost } from './dev-environment';
import { createProtocolClient } from './protocol/client';
import type { AnyProtocolDefinition } from './protocol/types';
import type { StackDefinition } from './types';
import type { WalletAdapter } from './wallet/types';

const installed = JSON.parse(
  readFileSync(
    new URL('../../../test-vectors/dev-environment-switch/v1/targets.json', import.meta.url),
    'utf8'
  )
).installed.default;

const EDGE = 'http://127.0.0.1:8911';
const SECRET_KEY = 'a4_sk_devenvironmentunittest000000000000';

function stack(): StackDefinition {
  return {
    name: installed.stack.name,
    endpoints: installed.stack.endpoints,
    release: installed.stack.release,
    views: {},
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

function protocol(): AnyProtocolDefinition {
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

function recordingFetch() {
  const urls: string[] = [];
  const fetchImpl = (async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
    urls.push(url);
    const { hostname, pathname } = new URL(url);
    const edge = hostname === '127.0.0.1';
    if (pathname === '/ws/sessions') {
      return new Response(JSON.stringify({ token: 't', expires_at: Math.floor(Date.now() / 1000) + 3600 }));
    }
    return new Response(pathname === '/.well-known/arete-dev-edge' ? '{"edge":1}' : '{}', {
      headers: edge ? { 'X-Arete-Dev-Edge': '1' } : {},
    });
  }) as typeof fetch;
  return { urls, fetchImpl };
}

function isDevUrl(url: string): boolean {
  const { hostname } = new URL(url);
  return ['127.0.0.1', 'localhost', '[::1]'].includes(hostname)
    || hostname === 'devenv.arete.run'
    || hostname.endsWith('.devstack.arete.run')
    || url.includes('/api/dev/');
}

afterEach(() => {
  vi.unstubAllEnvs();
});

describe('production with the switch unset', () => {
  it('reads no dev input and makes no dev request, whatever else is set', async () => {
    vi.stubEnv('ARETE_DEV_URL', EDGE);
    vi.stubEnv('ARETE_DEV_ENV', 'denv_01jabcdefghjkmnpqrstvwxyz0');
    const { urls, fetchImpl } = recordingFetch();
    const client = await Arete.connect(stack(), {
      auth: { secretKey: SECRET_KEY },
      fetch: fetchImpl,
      autoConnect: false,
    });
    expect(client.environment).toBe('production');
    await client.chain.lamports(installed.programs.klend.programId).catch(() => undefined);
    await (client.programs as any).klend.accounts.Reserve.fetch(installed.programs.klend.programId)
      .catch(() => undefined);
    const protocolClient = createProtocolClient(protocol(), {
      auth: { secretKey: SECRET_KEY },
      fetch: fetchImpl,
    });
    expect(protocolClient.environment).toBe('production');
    await protocolClient.instances.list().catch(() => undefined);
    expect(urls.length).toBeGreaterThan(0);
    expect(urls.filter(isDevUrl)).toEqual([]);
  });

  it('an unknown ARETE_ENVIRONMENT is INVALID_CONFIG before any request', async () => {
    vi.stubEnv('ARETE_ENVIRONMENT', 'staging');
    const { urls, fetchImpl } = recordingFetch();
    await expect(Arete.connect(stack(), { fetch: fetchImpl, autoConnect: false }))
      .rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    expect(() => createProtocolClient(protocol(), { fetch: fetchImpl }))
      .toThrow(expect.objectContaining({ code: 'INVALID_CONFIG' }));
    expect(urls).toEqual([]);
  });
});

describe('dev mode', () => {
  it('ARETE_ENVIRONMENT=dev with ARETE_DEV_URL goes through the edge', async () => {
    vi.stubEnv('ARETE_ENVIRONMENT', 'dev');
    vi.stubEnv('ARETE_DEV_URL', EDGE);
    const { urls, fetchImpl } = recordingFetch();
    const client = await Arete.connect(stack(), { fetch: fetchImpl, autoConnect: false });
    expect(client.environment).toBe('dev');
    await client.chain.lamports(installed.programs.klend.programId).catch(() => undefined);
    expect(urls).toEqual([`${EDGE}/chain/lamports/${installed.programs.klend.programId}`]);
  });

  it('refuses url and httpUrl overrides', async () => {
    await expect(Arete.connect(stack(), {
      environment: { kind: 'dev', url: EDGE },
      url: 'ws://127.0.0.1:8878',
      autoConnect: false,
    })).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    expect(() => createProtocolClient(protocol(), {
      environment: { kind: 'dev', url: EDGE },
      deploymentId: 'dep_other',
    })).toThrow(expect.objectContaining({ code: 'INVALID_CONFIG' }));
  });

  it('a protocol before its first publish needs dev mode', () => {
    const unpublished = { ...protocol(), deployment: null } as unknown as AnyProtocolDefinition;
    expect(() => createProtocolClient(unpublished, { environment: 'production' }))
      .toThrow(expect.objectContaining({ code: 'INVALID_CONFIG' }));
    const client = createProtocolClient(unpublished, { environment: { kind: 'dev', url: EDGE } });
    expect(client.environment).toBe('dev');
    expect(client.deploymentId).toBe('');
  });

  it('a stack without a release is not loaded in an environment', async () => {
    const { fetchImpl } = recordingFetch();
    const withoutRelease = { ...stack(), release: undefined } as StackDefinition;
    // The client is still returned: only the stack's own calls fail.
    const client = await Arete.connect(withoutRelease, {
      environment: { kind: 'dev', url: EDGE },
      fetch: fetchImpl,
    });
    await expect(client.connect()).rejects.toMatchObject({ code: 'devenv-stack-not-loaded' });
    client.disconnect();
  });

  it('a session decides once and hands the decision to every member', async () => {
    const { urls, fetchImpl } = recordingFetch();
    const session = await createSession(
      { stacks: { klend: stack() }, protocols: { raise: protocol() } },
      { environment: { kind: 'dev', url: EDGE }, fetch: fetchImpl, stacks: { klend: { autoConnect: false } } }
    );
    expect(session.environment).toBe('dev');
    expect((session.stacks as any).klend.environment).toBe('dev');
    expect((session.protocols as any).raise.environment).toBe('dev');
    await session.chain.lamports(installed.programs.klend.programId).catch(() => undefined);
    expect(urls.filter((url) => !isDevUrl(url))).toEqual([]);
    session.close();
  });
});

describe('wallet sends in dev mode', () => {
  function wallet(submitsThrough?: 'context' | 'adapter') {
    const calls: unknown[] = [];
    const adapter: WalletAdapter = {
      publicKey: installed.programs.klend.programId,
      ...(submitsThrough ? { submitsThrough } : {}),
      async signAndSend(_instructions, _options, context) {
        calls.push(context?.transactionTransport);
        return { signature: 'sig', slot: 1 };
      },
    };
    return { adapter, calls };
  }

  it('refuses an adapter that sends by itself, before it is asked', async () => {
    const client = await Arete.connect(stack(), {
      environment: { kind: 'dev', url: EDGE },
      autoConnect: false,
    });
    for (const submitsThrough of [undefined, 'adapter'] as const) {
      const { adapter, calls } = wallet(submitsThrough);
      await expect(client.transaction([], { wallet: adapter }))
        .rejects.toMatchObject({ code: 'devenv-wallet-send-unsupported' });
      expect(calls).toEqual([]);
    }
  });

  it('a sign-only adapter submits through the client transport', async () => {
    const client = await Arete.connect(stack(), {
      environment: { kind: 'dev', url: EDGE },
      autoConnect: false,
    });
    const { adapter, calls } = wallet('context');
    await expect(client.transaction([], { wallet: adapter })).resolves.toEqual({ signature: 'sig', slot: 1 });
    expect(calls).toEqual([client.transactions]);
  });

  it('production asks nothing of the adapter', async () => {
    const client = await Arete.connect(stack(), { environment: 'production', autoConnect: false });
    const { adapter, calls } = wallet();
    await expect(client.transaction([], { wallet: adapter })).resolves.toEqual({ signature: 'sig', slot: 1 });
    expect(calls).toHaveLength(1);
  });
});

describe('direct dev mode with a self-hosted API', () => {
  const answer = { environment: { id: 'denv_1', name: 'my-env' } };
  const request = { chain: { solanaGatewayBindingId: 'sgb_1' } };

  it('sends an explicit key to the apiUrl the app configured', async () => {
    const calls: string[] = [];
    const fetchImpl = (async (url: string) => {
      calls.push(url);
      return new Response(JSON.stringify(answer), { status: 200 });
    }) as unknown as typeof fetch;
    await expect(resolveDevTargets({
      environment: 'my-env',
      request,
      auth: { secretKey: 'a4_sk_selfhostedapitest00000000000000000' },
      apiUrl: 'https://arete.internal.example',
      fetch: fetchImpl,
    })).resolves.toEqual(answer);
    expect(calls).toEqual(['https://arete.internal.example/api/dev/environments/my-env/resolve']);
  });

  it('never sends ARETE_API_KEY to an apiUrl set in code', async () => {
    vi.stubEnv('ARETE_API_KEY', 'a4_sk_selfhostedapitest00000000000000000');
    const fetchImpl = vi.fn() as unknown as typeof fetch;
    await expect(resolveDevTargets({
      environment: 'my-env',
      request,
      auth: undefined,
      apiUrl: 'https://arete.internal.example',
      fetch: fetchImpl,
    })).rejects.toMatchObject({ code: 'UNTRUSTED_CREDENTIAL_DESTINATION' });
    expect(fetchImpl).not.toHaveBeenCalled();
  });
});

describe('a runtime that cannot read the session file', () => {
  it('says so instead of reporting no session', () => {
    const original = (process as { getBuiltinModule?: unknown }).getBuiltinModule;
    const saved = { url: process.env.ARETE_DEV_URL, env: process.env.ARETE_DEV_ENV };
    delete process.env.ARETE_DEV_URL;
    delete process.env.ARETE_DEV_ENV;
    (process as { getBuiltinModule?: unknown }).getBuiltinModule = undefined;
    try {
      const host = systemEnvironmentSwitchHost();
      let error: unknown;
      try {
        resolveEnvironmentSwitch({ kind: 'dev' }, host);
      } catch (thrown) {
        error = thrown;
      }
      expect(error).toMatchObject({ code: 'devenv-not-configured' });
      expect((error as Error).message).toContain('cannot read .arete/dev/session.json');
    } finally {
      (process as { getBuiltinModule?: unknown }).getBuiltinModule = original;
      if (saved.url !== undefined) process.env.ARETE_DEV_URL = saved.url;
      if (saved.env !== undefined) process.env.ARETE_DEV_ENV = saved.env;
    }
  });
});
