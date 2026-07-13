import { buildProgramHookInterfaces } from './program-hooks';

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
});
