import { describe, it, expect, vi } from 'vitest';
import { createSession } from './session';
import { createSignerRegistry } from './signer-registry';
import { withPrograms } from './client';
import { createInstructionHandler } from './instructions';
import { createPreparedFlow } from './operations';
import { programAccountRead } from './read';
import type { ChainClient } from './chain';
import type { TransactionTransport } from './transactions';

const SIGNER = 'So11111111111111111111111111111111111111112';
const ORE_PROGRAM = 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv';
const SQUADS_PROGRAM = 'SQDS111111111111111111111111111111111111111';
const SQUADS_BINDING_ID = 'prb_00000000000000000000000000000003';

function squadsRead(endpoint: string) {
  return {
    release: {
      programReleaseHash: 'release-squads',
      programSpecHash: 'spec-squads',
    },
    transport: {
      kind: 'hosted-binding',
      binding: {
        endpoint,
        programReadBindingId: SQUADS_BINDING_ID,
        auth: {
          required: false,
          sessionEndpoint: 'https://auth.invalid/session',
          targetKind: 'program-read-binding',
          targetId: SQUADS_BINDING_ID,
        },
      },
    },
  } as const;
}

const closeHandler = createInstructionHandler({
  programId: ORE_PROGRAM,
  discriminator: [9],
  args: [],
  accounts: [{ name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' }],
  errors: [],
});

const SQUADS_STACK = {
  name: 'squads-demo',
  endpoints: { ws: 'wss://squads.invalid', http: 'https://squads.invalid' },
  views: {
    Multisig: { list: { mode: 'list', view: 'Multisig/list' } },
  },
  programs: {
    squads: {
      name: 'squads',
      programId: SQUADS_PROGRAM,
      programSpecHash: 'spec-squads',
      accounts: {
        Multisig: programAccountRead<{ threshold: number }>({
          account: 'Multisig',
        }),
      },
      rawInstructions: {},
    },
  },
  programReads: {
    squads: squadsRead('https://squads.invalid'),
  },
} as const;

const ORE_PROGRAM_SDK = {
  name: 'ore',
  programId: ORE_PROGRAM,
  rawInstructions: { close: closeHandler },
} as const;

function makeFetch() {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes('/v1/releases/release-squads/accounts/Multisig/')) {
      return new Response(JSON.stringify({ threshold: 2 }), { status: 200 });
    }
    if (url.includes('/chain/exists/')) {
      return new Response(JSON.stringify({ exists: true }), { status: 200 });
    }
    return new Response('null', { status: 200 });
  });
}

describe('createSession', () => {
  it('rejects an empty definition', async () => {
    await expect(createSession({})).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
  });

  it('requires explicit chain and transaction transports in composition mode', async () => {
    await expect((createSession as any)(
      { mode: 'composition', stacks: { squads: SQUADS_STACK } },
      { stacks: { squads: { autoConnect: false } } }
    )).rejects.toMatchObject({
      code: 'INVALID_CONFIG',
      message: expect.stringContaining('explicit chain and transaction transports'),
    });
  });

  it('keeps three live and Program Read bindings independent in composition mode', async () => {
    const websocketUrls: string[] = [];
    class SessionWebSocket {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      readyState = SessionWebSocket.CONNECTING;
      onopen: (() => void) | null = null;
      onmessage: ((event: { data: unknown }) => void | Promise<void>) | null = null;
      onerror: (() => void) | null = null;
      onclose: ((event: { code: number; reason: string }) => void) | null = null;

      constructor(public readonly url: string) {
        websocketUrls.push(url);
        queueMicrotask(() => {
          this.readyState = SessionWebSocket.OPEN;
          this.onopen?.();
        });
      }

      send(): void {}

      close(code = 1000, reason = ''): void {
        this.readyState = SessionWebSocket.CLOSED;
        this.onclose?.({ code, reason });
      }
    }
    vi.stubGlobal('WebSocket', SessionWebSocket as unknown as typeof WebSocket);
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({ threshold: 2 }), { status: 200 })
    );
    const member = (alias: string) => ({
      ...SQUADS_STACK,
      name: alias,
      endpoints: {
        ws: `wss://${alias}.live.invalid`,
        http: `https://${alias}.live.invalid`,
      },
       programReads: {
        squads: squadsRead(`https://${alias}.reads.invalid`),
      },
    } as const);
    const chain = { exists: vi.fn(async () => true) } as unknown as ChainClient;
    const transactions = {} as TransactionTransport;
    const session = await createSession(
      {
        mode: 'composition',
        stacks: {
          squads: member('squads'),
          presale: member('presale'),
          damm: member('damm'),
        },
      },
      {
        chain,
        transactions,
        fetch: fetchMock as typeof fetch,
      }
    );

    expect(websocketUrls).toEqual([
      'wss://squads.live.invalid',
      'wss://presale.live.invalid',
      'wss://damm.live.invalid',
    ]);
    await session.stacks.squads.programs.squads.accounts.Multisig.fetch('one');
    await session.stacks.presale.programs.squads.accounts.Multisig.fetch('two');
    await session.stacks.damm.programs.squads.accounts.Multisig.fetch('three');
    expect(fetchMock.mock.calls.map(([input]) => String(input))).toEqual([
      'https://squads.reads.invalid/v1/releases/release-squads/accounts/Multisig/one',
      'https://presale.reads.invalid/v1/releases/release-squads/accounts/Multisig/two',
      'https://damm.reads.invalid/v1/releases/release-squads/accounts/Multisig/three',
    ]);
    expect(session.chain).toBe(chain);
    expect(session.transactions).toBe(transactions);
    expect(session.programs).toEqual({});
    session.close();
    vi.unstubAllGlobals();
  });

  it('connects zero-account standalone programs without a stack HTTP endpoint', async () => {
    const chain = { exists: vi.fn(async () => true) } as unknown as ChainClient;
    const transactions = {} as TransactionTransport;
    const session = await createSession(
      {
        mode: 'composition',
        programs: { ore: ORE_PROGRAM_SDK },
        programReads: {
          ore: {
            release: {
              programReleaseHash: 'release-ore',
              programSpecHash: 'spec-ore',
            },
            transport: {
              kind: 'local-http',
              endpointSource: 'connect-http-url',
            },
          },
        },
      },
      { chain, transactions, fetch: makeFetch() as typeof fetch }
    );

    expect(session.programs.ore.programId).toBe(ORE_PROGRAM);
    session.close();
  });

  it('rejects local Program Reads instead of inheriting a live endpoint in composition mode', async () => {
    const fetchMock = makeFetch();
    const chain = { exists: vi.fn(async () => true) } as unknown as ChainClient;
    const transactions = {} as TransactionTransport;
    const stack = {
      ...SQUADS_STACK,
      endpoints: { ws: 'wss://live.invalid', http: 'https://live.invalid' },
      programReads: {
        squads: {
          release: {
            programReleaseHash: 'release-squads',
            programSpecHash: 'spec-squads',
          },
          transport: {
            kind: 'local-http',
            endpointSource: 'connect-http-url',
          },
        },
      },
    } as const;
    await expect(createSession(
      { mode: 'composition', stacks: { squads: stack } },
      {
        chain,
        transactions,
        fetch: fetchMock as typeof fetch,
        stacks: { squads: { autoConnect: false } },
      }
    )).rejects.toMatchObject({
      code: 'INVALID_CONFIG',
      message: expect.stringContaining('complete hosted-binding'),
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('accepts a complete hosted override as a composition descriptor replacement', async () => {
    const fetchMock = makeFetch();
    const chain = { exists: vi.fn(async () => true) } as unknown as ChainClient;
    const transactions = {} as TransactionTransport;
    const stack = {
      ...SQUADS_STACK,
      endpoints: { ws: 'wss://live.invalid', http: 'https://live.invalid' },
      programReads: {
        squads: {
          release: {
            programReleaseHash: 'release-local',
            programSpecHash: 'spec-squads',
          },
          transport: {
            kind: 'local-http',
            endpointSource: 'connect-http-url',
          },
        },
      },
    } as const;
    const session = await createSession(
      { mode: 'composition', stacks: { squads: stack } },
      {
        chain,
        transactions,
        fetch: fetchMock as typeof fetch,
        stacks: {
          squads: {
            autoConnect: false,
            httpUrl: 'https://member.invalid',
            programReads: {
              squads: squadsRead('https://override.reads.invalid'),
            },
          },
        },
      }
    );

    await session.stacks.squads.programs.squads.accounts.Multisig.fetch('multisig');
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      'https://override.reads.invalid/v1/releases/release-squads/accounts/Multisig/multisig'
    );
    session.close();
  });

  it('supports independent per-stack initial connection policy', async () => {
    const session = await createSession(
      { stacks: { squads: SQUADS_STACK } },
      { stacks: { squads: { autoConnect: false, autoReconnect: true } } },
    );

    expect(session.stacks.squads.connectionState).toBe('disconnected');
    session.close();
  });

  it('uses an explicitly injected canonical chain client', async () => {
    const chain = { exists: vi.fn(async () => true) } as unknown as ChainClient;
    const session = await createSession(
      { programs: { ore: ORE_PROGRAM_SDK } },
      {
        transport: 'http',
        endpoints: { http: 'https://session.invalid' },
        fetch: makeFetch() as typeof fetch,
        chain,
      }
    );

    expect(session.chain).toBe(chain);
    session.close();
  });

  it('composes stack and program members over http transport', async () => {
    const fetchMock = makeFetch();
    const session = await createSession(
      {
        stacks: {
          squads: {
            ...SQUADS_STACK,
          },
        },
        programs: { ore: ORE_PROGRAM_SDK },
      },
      {
        transport: 'http',
        endpoints: { http: 'https://session.invalid' },
        fetch: fetchMock as typeof fetch,
      }
    );

    // Stack member keeps its own endpoints and full typed surface.
    await expect(
      session.stacks.squads.programs.squads.accounts.Multisig.fetch('multisig-1')
    ).resolves.toEqual({ threshold: 2 });
    expect(session.programs.squads).toBe(session.stacks.squads.programs.squads);
    await expect(session.programs.squads.accounts.Multisig.fetch('multisig-2')).resolves.toEqual({
      threshold: 2,
    });
    expect(String(fetchMock.mock.calls[0]?.[0])).toContain('https://squads.invalid');

    // Program member exposes the program interface directly, no views needed.
    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string; slot: number }> {
        return { signature: 'sig-ws', slot: 1 };
      },
    };
    const ix = session.programs.ore.raw.close.build({}, { wallet });
    expect(ix.programId).toBe(ORE_PROGRAM);
    expect(ix.keys[0]!.pubkey).toBe(SIGNER);

    // Shared chain client uses the session fallback endpoint.
    await expect(session.chain.exists('anything')).resolves.toBe(true);
    const chainCall = fetchMock.mock.calls.find(([request]) => String(request).includes('/chain/exists/'));
    expect(String(chainCall?.[0])).toContain('https://session.invalid');

    session.close();
  });

  it('fans one wallet out to every member', async () => {
    const session = await createSession(
      {
        stacks: { squads: SQUADS_STACK },
        programs: { ore: ORE_PROGRAM_SDK },
      },
      {
        transport: 'http',
        endpoints: { http: 'https://session.invalid' },
        fetch: makeFetch() as typeof fetch,
      }
    );

    const sent: string[] = [];
    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string; slot: number }> {
        sent.push('sent');
        return { signature: 'sig-shared', slot: 5 };
      },
    };
    session.setWallet(wallet);
    expect(session.wallet).toBe(wallet);
    expect(session.stacks.squads.wallet).toBe(wallet);

    // Instruction execution through the program member uses the shared wallet.
    const ix = session.programs.ore.raw.close.build({});
    await expect(session.transaction([ix])).resolves.toEqual({ signature: 'sig-shared', slot: 5 });
    expect(sent).toHaveLength(1);

    const prepared = createPreparedFlow({
      name: 'close-stage',
      artifacts: { closed: true },
      transactions: [{ name: 'close-stage', instructions: [ix], requiredSignerAddresses: [SIGNER], errors: [] }],
    });
    await expect(session.execute(prepared)).resolves.toEqual({
      kind: 'flow',
      operationName: 'close-stage',
      artifacts: { closed: true },
      signatures: ['sig-shared'],
      transactions: [{ transactionIndex: 0, transactionName: 'close-stage', signature: 'sig-shared', slot: 5 }],
    });

    session.close();
  });

  it('exposes execution helpers', async () => {
    const session = await createSession(
      {
        stacks: { squads: SQUADS_STACK },
        programs: { ore: ORE_PROGRAM_SDK },
      },
      {
        transport: 'http',
        endpoints: { http: 'https://session.invalid' },
        fetch: makeFetch() as typeof fetch,
      }
    );

    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string; slot: number }> {
        return { signature: 'sig-session', slot: 8 };
      },
    };
    session.setWallet(wallet);

    const ix = session.programs.ore.raw.close.build({});
    const prepared = createPreparedFlow({
      name: 'session-close-plan',
      artifacts: { closed: true },
      transactions: [{ name: 'session-close-stage', instructions: [ix], requiredSignerAddresses: [SIGNER], errors: [] }],
    });

    await expect(session.execute(prepared)).resolves.toEqual({
      kind: 'flow',
      operationName: 'session-close-plan',
      artifacts: { closed: true },
      signatures: ['sig-session'],
      transactions: [{ transactionIndex: 0, transactionName: 'session-close-stage', signature: 'sig-session', slot: 8 }],
    });

    session.close();
  });

  it('uses session signers for direct transactions', async () => {
    const registeredSigner = { address: 'registered-signer' };
    let receivedSigners: readonly unknown[] | undefined;
    const wallet = {
      publicKey: SIGNER,
      async signAndSend(_instructions: unknown, options?: { signers?: readonly unknown[] }) {
        receivedSigners = options?.signers;
        return { signature: 'sig-session-signers' };
      },
    };
    const session = await createSession(
      { programs: { ore: ORE_PROGRAM_SDK } },
      {
        transport: 'http',
        endpoints: { http: 'https://session.invalid' },
        fetch: makeFetch() as typeof fetch,
        wallet,
        execution: { signers: [registeredSigner] },
      }
    );

    await session.transaction([session.programs.ore.raw.close.build({})]);
    expect(receivedSigners).toEqual([registeredSigner]);
    session.close();
  });

  it('uses registered session signers for transactions and operation validation', async () => {
    const registeredSigner = { opaqueSigner: true };
    const registeredSignerAddress = 'registered-signer';
    const signerRegistry = createSignerRegistry([
      [registeredSignerAddress, registeredSigner] as const,
    ]);
    let receivedSigners: readonly unknown[] | undefined;
    const wallet = {
      publicKey: SIGNER,
      async signAndSend(_instructions: unknown, options?: { signers?: readonly unknown[] }) {
        receivedSigners = options?.signers;
        return { signature: 'sig-session-registry' };
      },
    };
    const session = await createSession(
      { programs: { ore: ORE_PROGRAM_SDK } },
      {
        transport: 'http',
        endpoints: { http: 'https://session.invalid' },
        fetch: makeFetch() as typeof fetch,
        wallet,
        signerRegistry,
      }
    );

    const instruction = session.programs.ore.raw.close.build({});
    await session.transaction([instruction]);
    expect(receivedSigners).toEqual([registeredSigner]);

    const prepared = createPreparedFlow({
      name: 'registered-signer-flow',
      artifacts: {},
      transactions: [{
        name: 'registered-signer-transaction',
        instructions: [instruction],
        requiredSignerAddresses: [registeredSignerAddress],
        errors: [],
      }],
    });
    await expect(session.execute(prepared)).resolves.toMatchObject({ kind: 'flow' });
    expect(session.signerRegistry.get(registeredSignerAddress)).toBe(registeredSigner);

    const missingSignerFlow = createPreparedFlow({
      name: 'missing-signer-flow',
      artifacts: {},
      transactions: [{
        name: 'missing-signer-transaction',
        instructions: [instruction],
        requiredSignerAddresses: ['not-registered'],
        errors: [],
      }],
    });
    await expect(session.execute(missingSignerFlow)).rejects.toMatchObject({
      cause: expect.objectContaining({ message: expect.stringContaining('not-registered') }),
    });

    const laterSigner = { address: 'later-signer' };
    session.signerRegistry.register(laterSigner.address, laterSigner);
    expect(session.signerRegistry.addresses()).toEqual([
      registeredSignerAddress,
      laterSigner.address,
    ]);
    expect(session.signerRegistry.unregister(laterSigner.address)).toBe(true);
    session.close();
  });

  it('attaches extra programs onto stack members with typed access', async () => {
    const session = await createSession(
      {
        stacks: { squads: SQUADS_STACK },
      },
      {
        transport: 'http',
        endpoints: { http: 'https://session.invalid' },
        fetch: makeFetch() as typeof fetch,
        stacks: {
          squads: {
            programs: {
              ore: ORE_PROGRAM_SDK,
            },
          },
        },
      }
    );

    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string; slot: number }> {
        return { signature: 'sig-member', slot: 6 };
      },
    };
    session.setWallet(wallet);
    expect(session.programs.ore).toBe(session.stacks.squads.programs.ore);
    const ix = session.programs.ore.raw.close.build({});
    expect(ix.programId).toBe(ORE_PROGRAM);
    expect(ix.keys[0]!.pubkey).toBe(SIGNER);

    session.close();
  });

  it('supports pre-widening a stack definition with withPrograms()', async () => {
    const stack = {
      ...withPrograms(SQUADS_STACK, { ore: ORE_PROGRAM_SDK }),
      programReads: {
        ...SQUADS_STACK.programReads,
        ore: {
          release: {
            programReleaseHash: 'release-ore',
            programSpecHash: 'spec-ore',
          },
          transport: {
            kind: 'local-http',
            endpointSource: 'connect-http-url',
          },
        },
      },
    } as const;
    const session = await createSession(
      {
        stacks: { squads: stack },
      },
      {
        transport: 'http',
        endpoints: { http: 'https://session.invalid' },
        fetch: makeFetch() as typeof fetch,
      }
    );

    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string; slot: number }> {
        return { signature: 'sig-prewidened', slot: 7 };
      },
    };
    expect(session.programs.ore).toBe(session.stacks.squads.programs.ore);
    const ix = session.programs.ore.raw.close.build({}, { wallet });
    expect(ix.programId).toBe(ORE_PROGRAM);

    session.close();
  });

  it('keeps the first stack program on bundled-key collisions and warns', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const secondStack = {
      ...SQUADS_STACK,
      name: 'second-squads',
      endpoints: { ws: '', http: 'https://second.invalid' },
      programs: {
        squads: {
          name: 'other-squads',
          programId: ORE_PROGRAM,
          rawInstructions: {},
        },
      },
    } as const;
    const session = await createSession(
      { stacks: { first: SQUADS_STACK, second: secondStack } },
      { transport: 'http', fetch: makeFetch() as typeof fetch }
    );

    expect(session.programs.squads).toBe(session.stacks.first.programs.squads);
    expect(session.programs.squads).not.toBe(session.stacks.second.programs.squads);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("uses 'first' because it was connected first"));

    session.close();
    warn.mockRestore();
  });

  it('gives explicit standalone programs precedence over promoted keys', async () => {
    const session = await createSession(
      {
        stacks: { squads: SQUADS_STACK },
        programs: { squads: ORE_PROGRAM_SDK },
      },
      {
        transport: 'http',
        endpoints: { http: 'https://session.invalid' },
        fetch: makeFetch() as typeof fetch,
      }
    );

    expect(session.programs.squads.programId).toBe(ORE_PROGRAM);
    expect(session.stacks.squads.programs.squads.programId).toBe(SQUADS_PROGRAM);
    expect(session.programs.squads).not.toBe(session.stacks.squads.programs.squads);

    session.close();
  });

  it('promotes different aliases for the same program ID', async () => {
    const aliasedStack = {
      ...SQUADS_STACK,
      programs: {
        ...SQUADS_STACK.programs,
        governance: SQUADS_STACK.programs.squads,
      },
      programReads: {
        ...SQUADS_STACK.programReads,
        governance: squadsRead('https://squads.invalid'),
      },
    } as const;
    const session = await createSession(
      { stacks: { squads: aliasedStack } },
      { transport: 'http', fetch: makeFetch() as typeof fetch }
    );

    expect(session.programs.squads).toBe(session.stacks.squads.programs.squads);
    expect(session.programs.governance).toBe(session.stacks.squads.programs.governance);
    expect(session.programs.squads.programId).toBe(session.programs.governance.programId);

    session.close();
  });

  it('keeps lifecycle ownership with the stack that supplied promoted programs', async () => {
    const session = await createSession(
      { stacks: { squads: SQUADS_STACK } },
      { transport: 'http', fetch: makeFetch() as typeof fetch }
    );
    const disconnect = vi.spyOn(session.stacks.squads, 'disconnect');

    session.close();

    expect(disconnect).toHaveBeenCalledTimes(1);
  });

  it('keeps views working on ws stack members and disabled on program members', async () => {
    const session = await createSession(
      { stacks: { squads: SQUADS_STACK }, programs: { ore: ORE_PROGRAM_SDK } },
      {
        transport: 'http',
        endpoints: { http: 'https://session.invalid' },
        fetch: makeFetch() as typeof fetch,
      }
    );

    const iterate = async () => {
      for await (const _entry of session.stacks.squads.views.Multisig.list.use()) {
        break;
      }
    };
    // The squads member itself was connected http-only here, so views throw
    // eagerly instead of hanging.
    await expect(iterate()).rejects.toMatchObject({ code: 'WEBSOCKET_DISABLED' });

    session.close();
  });
});
