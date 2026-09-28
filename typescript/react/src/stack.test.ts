import { buildDisconnectedProgramHooks, buildProgramHookInterfaces } from './program-hooks';

function createMutationResult() {
  return {
    submit: jest.fn(),
    status: 'idle' as const,
    error: null,
    signatures: [],
    signature: null,
    isLoading: false,
    reset: jest.fn(),
  };
}

describe('program hook parity', () => {
  it('preserves program namespaces and recursively wraps operations', async () => {
    const createMutationHook = jest.fn(() => createMutationResult());
    const rawInstruction = {
      build: jest.fn(() => ({
        programId: 'ore-program',
        keys: [],
        data: new Uint8Array([1]),
      })),
    };
    const semanticInstruction = {
      prepare: jest.fn(async () => ({
        kind: 'instruction',
        name: 'close',
        instruction: { programId: 'ore-program', keys: [], data: new Uint8Array([2]) },
        transaction: { name: 'close', instructions: [{ programId: 'ore-program', keys: [], data: new Uint8Array([2]) }], requiredSignerAddresses: [], errors: [] },
        plan: { name: 'close', artifacts: { ok: true }, transactions: [{ name: 'close', instructions: [{ programId: 'ore-program', keys: [], data: new Uint8Array([2]) }], requiredSignerAddresses: [], errors: [] }] },
        artifacts: { ok: true },
      })),
    };
    const addresses = { vault: (owner: string) => `vault:${owner}` };
    const constants = { CloseMode: { Immediate: 0 } };
    const defaults = { closeMemo: 'prepared-close' };
    const read = { balance: jest.fn(async () => 42n) };
    const client = {
      transaction: jest.fn(async () => ({ signature: 'raw-signature', slot: 11 })),
      execute: jest.fn(async () => ({ kind: 'instruction', operationName: 'close', artifacts: { ok: true }, signatures: ['semantic-signature'], transaction: { transactionIndex: 0, transactionName: 'close', signature: 'semantic-signature', slot: 22 } })),
    };

    const programs = buildProgramHookInterfaces({
      ore: {
        name: 'ore',
        programId: 'ore-program',
        schemas: {},
        pdas: {},
        accounts: {},
        queries: {},
        addresses,
        constants,
        defaults,
        read,
        raw: { close: rawInstruction },
        instructions: {
          admin: {
            close: semanticInstruction,
          },
        },
        transactions: {},
        flows: {},
      },
    } as never, client as never, createMutationHook as never);

    expect(programs.ore.addresses).toBe(addresses);
    expect(programs.ore.constants).toBe(constants);
    expect(programs.ore.defaults).toBe(defaults);
    expect(programs.ore.read).toBe(read);

    expect(programs.ore.raw.close.build).toBe(rawInstruction.build);
    await programs.ore.raw.close.execute({});
    expect(client.transaction).toHaveBeenCalled();

    await expect(programs.ore.instructions.admin.close.prepare({})).resolves.toMatchObject({ kind: 'instruction' });
    await programs.ore.instructions.admin.close.execute({});
    expect(client.execute).toHaveBeenCalled();

    programs.ore.instructions.admin.close.useMutation();
    expect(createMutationHook).toHaveBeenCalled();
  });

  it('returns an empty map when no programs are attached', () => {
    expect(buildProgramHookInterfaces(undefined, null, jest.fn() as never)).toEqual({});
  });

  it('injects the default stream reconciliation when the client can report processed slots', () => {
    const createMutationHook = jest.fn(() => createMutationResult());
    const client = {
      transaction: jest.fn(async () => ({})),
      execute: jest.fn(async () => ({})),
      waitForProcessedSlot: jest.fn(async () => 0n),
    };
    const programs = {
      ore: {
        name: 'ore',
        programId: 'ore-program',
        schemas: {},
        pdas: {},
        accounts: {},
        queries: {},
        raw: { close: { build: jest.fn() } },
        instructions: {},
        transactions: {},
        flows: {},
      },
    };

    const connected = buildProgramHookInterfaces(programs as never, client as never, createMutationHook as never);
    connected.ore.raw.close.useMutation();
    const injected = createMutationHook.mock.calls[0]?.[1] as { reconcile?: unknown };
    expect(injected?.reconcile).toEqual(expect.any(Function));
    expect((injected?.reconcile as { areteDefaultReconciliation?: unknown }).areteDefaultReconciliation).toBe(true);

    createMutationHook.mockClear();
    const disconnected = buildProgramHookInterfaces(programs as never, null, createMutationHook as never);
    disconnected.ore.raw.close.useMutation();
    expect(createMutationHook.mock.calls[0]?.[1]).toBeUndefined();
  });

  it('lets hook-level reconcile options override the injected default', () => {
    const createMutationHook = jest.fn(() => createMutationResult());
    const client = {
      transaction: jest.fn(async () => ({})),
      execute: jest.fn(async () => ({})),
      waitForProcessedSlot: jest.fn(async () => 0n),
    };
    const programs = {
      ore: {
        name: 'ore',
        programId: 'ore-program',
        schemas: {},
        pdas: {},
        accounts: {},
        queries: {},
        raw: { close: { build: jest.fn() } },
        instructions: {},
        transactions: {},
        flows: {},
      },
    };

    const connected = buildProgramHookInterfaces(programs as never, client as never, createMutationHook as never);

    connected.ore.raw.close.useMutation({ reconcile: false });
    expect((createMutationHook.mock.calls[0]?.[1] as { reconcile?: unknown }).reconcile).toBe(false);

    const custom = jest.fn();
    connected.ore.raw.close.useMutation({ reconcile: custom });
    expect((createMutationHook.mock.calls[1]?.[1] as { reconcile?: unknown }).reconcile).toBe(custom);

    connected.ore.raw.close.useMutation({ reconcile: { timeoutMs: 5_000 } });
    const shorthand = (createMutationHook.mock.calls[2]?.[1] as { reconcile?: { areteDefaultReconciliation?: unknown } }).reconcile;
    expect(shorthand?.areteDefaultReconciliation).toBe(true);
  });

  it('does not inject stream reconciliation for HTTP-only clients', () => {
    const createMutationHook = jest.fn(() => createMutationResult());
    const client = {
      transaction: jest.fn(async () => ({})),
      execute: jest.fn(async () => ({})),
      waitForProcessedSlot: jest.fn(async () => 0n),
    };
    const programs = {
      ore: {
        name: 'ore',
        programId: 'ore-program',
        schemas: {},
        pdas: {},
        accounts: {},
        queries: {},
        raw: { close: { build: jest.fn() } },
        instructions: {},
        transactions: {},
        flows: {},
      },
    };

    const connected = buildProgramHookInterfaces(
      programs as never,
      client as never,
      createMutationHook as never,
      { defaultReconciliation: false },
    );
    connected.ore.raw.close.useMutation();

    expect(createMutationHook.mock.calls[0]?.[1]).toBeUndefined();
  });
});

describe('disconnected program hooks', () => {
  it('resolves any namespace path and throws not-connected on direct calls', () => {
    const programs = buildDisconnectedProgramHooks() as Record<string, Record<string, Record<string, Record<string, {
      prepare: () => unknown;
      execute: () => unknown;
      build: () => unknown;
    }>>>>;

    const deploy = programs.ore.transactions.mining.deployWithCheckpoint;
    expect(deploy).toBeDefined();
    expect(() => deploy.prepare()).toThrow('Arete client is not connected');
    expect(() => deploy.execute()).toThrow('Arete client is not connected');
    expect(() => deploy.build()).toThrow('Arete client is not connected');
  });

  it('serves the static program surface from the definitions', () => {
    const ore = {
      name: 'ore',
      programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
      schemas: {},
      rawInstructions: {},
      pdas: { board: { seeds: [] } },
      addresses: { board: () => 'board-address' },
      constants: { squares: 25 },
      math: { double: (value: number) => value * 2 },
    };
    const programs = buildDisconnectedProgramHooks({ ore } as never) as unknown as Record<string, {
      programId: string;
      pdas: Record<string, unknown>;
      addresses: { board: () => string };
      constants: { squares: number };
      defaults: Record<string, unknown>;
      math: { double: (value: number) => number };
      transactions: Record<string, Record<string, { prepare: () => unknown }>>;
    }>;

    expect(programs.ore.programId).toBe(ore.programId);
    expect(programs.ore.pdas).toBe(ore.pdas);
    expect(programs.ore.addresses.board()).toBe('board-address');
    expect(programs.ore.constants.squares).toBe(25);
    expect(programs.ore.defaults).toEqual({});
    expect(programs.ore.math.double(3)).toBe(6);
    expect(() => programs.ore.transactions.mining.deployWithCheckpoint.prepare())
      .toThrow('Arete client is not connected');
    expect(() => programs.other.transactions.any.thing.prepare())
      .toThrow('Arete client is not connected');
  });

  it('resolves attached programs by the connected client rules', () => {
    const program = (name: string, identity: Record<string, string>) => ({
      name,
      programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
      schemas: {},
      rawInstructions: {},
      ...identity,
    });
    const stackOre = program('ore', { programSpecHash: 'spec', packageReleaseHash: 'release' });
    const resolve = (attached: Record<string, unknown>) =>
      buildDisconnectedProgramHooks({ ore: stackOre } as never, attached as never) as unknown as Record<
        string,
        { name: string }
      >;

    // The same program SDK keeps the stack's definition, as the client does.
    expect(resolve({ ore: program('ore-attached', { programSpecHash: 'spec', packageReleaseHash: 'release' }) }).ore.name)
      .toBe('ore');
    // An unproven match (no package release on one side) uses the attached one.
    expect(resolve({ ore: program('ore-local', { programSpecHash: 'spec' }) }).ore.name).toBe('ore-local');
    // Programs the stack does not provide are added next to its own.
    const added = resolve({ token: program('token', { programSpecHash: 'token-spec' }) });
    expect([added.ore.name, added.token.name]).toEqual(['ore', 'token']);
  });
});
