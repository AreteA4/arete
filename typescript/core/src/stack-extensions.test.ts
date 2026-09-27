import { describe, expect, it, vi } from 'vitest';

import {
  PROGRAM_OPERATION_EXTENSIONS,
  STACK_RUNTIME_EXTENSIONS,
  applyConnectedStackExtensions,
  createPreparedFlow,
  createPreparedInstruction,
  createPreparedTransaction,
  defineProgramExtensions,
  defineStackExtensions,
  extendProgram,
  extendPrograms,
  extendStack,
  flowOperation,
  getProgramRuntimeExtensions,
  getStackRuntimeExtensions,
  instructionOperation,
  compareProgramIdentity,
  isSameProgramSdk,
  transactionOperation,
  withProgramIdentity,
  withProgramRead,
  withPrograms,
} from './index';
import { getProgramReadDescriptor } from './program-sdk';
import type { ProgramSdkDefinition, StackDefinition } from './types';

const BASE_STACK = {
  name: 'demo',
  endpoints: { ws: 'wss://example.invalid', http: 'https://example.invalid' },
  views: {},
} as const satisfies StackDefinition;

const BASE_PROGRAM = {
  name: 'ore',
  programId: 'ore111111111111111111111111111111111111111',
  sdkDefinitionHash: 'arete:h1:sdk-definition:sha256:ore-base',
  rawInstructions: {},
} as const satisfies ProgramSdkDefinition;

describe('extendStack', () => {
  it('attaches addresses, constants, defaults, math, read, and flows', async () => {
    const extended = extendStack(BASE_STACK, {
      addresses: { vault: () => 'VaultAddr' },
      constants: { permission: { vote: 1 } },
      defaults: { member: (key: string) => ({ key }) },
      math: { add: (a: number, b: number) => a + b },
      createFlows: () => ({
        close: flowOperation(async () => createPreparedFlow({
          name: 'close',
          transactions: [{
            name: 'close',
            instructions: [{ programId: 'x', keys: [], data: new Uint8Array([]) }],
          }],
          artifacts: {},
        })),
      }),
      readArgCounts: { ping: 0 },
      createRead: () => ({ ping: () => 'pong' }),
    });

    expect(extended.addresses.vault()).toBe('VaultAddr');
    expect(extended.constants.permission.vote).toBe(1);
    expect(extended.defaults.member('x')).toEqual({ key: 'x' });
    expect(extended.math.add(1, 2)).toBe(3);
    expect(getStackRuntimeExtensions(extended)?.createRead?.(null as never)).toEqual(
      expect.objectContaining({ ping: expect.any(Function) })
    );
  });

  it('deep-merges namespaces and composes createRead', () => {
    const first = extendStack(BASE_STACK, {
      addresses: { a: 1, shared: 'base' },
      constants: { one: true },
      readArgCounts: { one: 0, shared: 0 },
      createRead: () => ({ one: 1, shared: 'base' }),
    });
    const second = extendStack(first, {
      addresses: { b: 2, shared: 'override' },
      constants: { two: true },
      readArgCounts: { two: 0, shared: 0 },
      createRead: () => ({ two: 2, shared: 'override' }),
    });
    const third = extendStack(second, {
      createFlows: () => ({}),
    });

    expect(second.addresses).toEqual({ a: 1, b: 2, shared: 'override' });
    expect(second.constants).toEqual({ one: true, two: true });
    expect(getStackRuntimeExtensions(second)?.readArgCounts).toEqual({
      one: 0,
      two: 0,
      shared: 0,
    });
    expect(getStackRuntimeExtensions(third)?.readArgCounts).toEqual({
      one: 0,
      two: 0,
      shared: 0,
    });
    expect(getStackRuntimeExtensions(second)?.createRead?.(null as never)).toEqual({
      one: 1,
      two: 2,
      shared: 'override',
    });
  });
});

describe('applyConnectedStackExtensions', () => {
  it('exposes addresses, constants, defaults, math, flows, and read on the connected stack', () => {
    const extended = extendStack(BASE_STACK, {
      addresses: { vault: 'V' },
      constants: { closeMemo: 'memo' },
      defaults: { retries: 3 },
      math: { double: (value: number) => value * 2 },
      readArgCounts: { echo: 0 },
      createFlows: () => ({
        close: flowOperation(async () => createPreparedFlow({
          name: 'close',
          transactions: [{
            name: 'close',
            instructions: [{ programId: 'x', keys: [], data: new Uint8Array([]) }],
          }],
          artifacts: {},
        })),
      }),
      createRead: (client) => ({ echo: () => client }),
    });

    const fakeClient = { chain: 'chain-client' };
    const connected = applyConnectedStackExtensions(fakeClient, extended);

    expect(connected.addresses).toEqual({ vault: 'V' });
    expect(connected.constants).toEqual({ closeMemo: 'memo' });
    expect(connected.defaults).toEqual({ retries: 3 });
    expect(connected.math.double(3)).toBe(6);
    expect(connected.flows.close.kind).toBe('flow');
    expect(connected.read.echo()).toBe(fakeClient);
  });
});

describe('extendProgram', () => {
  it('attaches addresses, constants, defaults, math, and operation factories', async () => {
    const extended = extendProgram(BASE_PROGRAM, {
      pdas: { vault: 'vault-pda' },
      addresses: { vault: () => 'VaultAddr' },
      constants: { AuthorityType: { MintTokens: 'AuthorityMintTokens' } },
      defaults: { closeMemo: 'prepared-close' },
      math: { double: (value: number) => value * 2 },
      createOperations() {
        return {
          instructions: {
            close: instructionOperation(async () =>
              createPreparedInstruction({
                name: 'close',
                instruction: { programId: 'ore', keys: [], data: new Uint8Array([1]) },
                artifacts: { closed: true },
              })
            ),
          },
        };
      },
    });

    expect(extended.pdas).toEqual({ vault: 'vault-pda' });
    expect((extended as ProgramSdkDefinition).sdkDefinitionHash).toBeUndefined();
    expect(extended.addresses.vault()).toBe('VaultAddr');
    expect(extended.constants).toEqual({ AuthorityType: { MintTokens: 'AuthorityMintTokens' } });
    expect(extended.defaults.closeMemo).toBe('prepared-close');
    expect(extended.math.double(3)).toBe(6);
    const operations = getProgramRuntimeExtensions(extended)?.createOperations({
      chain: null as never,
      wallet: undefined,
      program: {
        name: 'ore',
        programId: 'ore',
        schemas: {},
        pdas: {},
        accounts: {},
        queries: {},
        raw: {},
        addresses: extended.addresses,
        constants: extended.constants,
        defaults: extended.defaults,
        math: extended.math,
        instructions: {},
        transactions: {},
        flows: {},
      },
    });
    expect(operations?.instructions?.close).toBeDefined();
  });

  it('preserves transaction and flow namespaces from program extensions', () => {
    const extended = extendProgram(BASE_PROGRAM, {
      createOperations() {
        return {
          transactions: {
            closeBatch: transactionOperation(async () =>
              createPreparedTransaction({
                name: 'closeBatch',
                instructions: [{ programId: 'ore', keys: [], data: new Uint8Array([2]) }],
                artifacts: { closed: true },
              })
            ),
          },
          flows: {
            closeFlow: flowOperation(async () =>
              createPreparedFlow({
                name: 'closeFlow',
                transactions: [{
                  name: 'closeFlow',
                  instructions: [{ programId: 'ore', keys: [], data: new Uint8Array([3]) }],
                }],
                artifacts: { closed: true },
              })
            ),
          },
        };
      },
    });

    const operations = getProgramRuntimeExtensions(extended)?.createOperations({
      chain: null as never,
      wallet: undefined,
      program: {
        name: 'ore',
        programId: 'ore',
        schemas: {},
        pdas: {},
        accounts: {},
        queries: {},
        raw: {},
        addresses: {},
        constants: {},
        defaults: {},
        math: {},
        instructions: {},
        transactions: {},
        flows: {},
      },
    });

    expect(operations?.transactions?.closeBatch).toBeDefined();
    expect(operations?.flows?.closeFlow).toBeDefined();
  });

  it('deep-merges nested operation resources across extension layers', () => {
    const operation = (name: string) =>
      instructionOperation(async () =>
        createPreparedInstruction({
          name,
          instruction: {
            programId: 'ore',
            keys: [],
            data: new Uint8Array([1]),
          },
          artifacts: {},
        }),
      );
    const base = extendProgram(BASE_PROGRAM, {
      createOperations() {
        return { instructions: { position: { create: operation('create') } } };
      },
    });
    const extended = extendProgram(base, {
      createOperations() {
        return { instructions: { position: { close: operation('close') } } };
      },
    });
    const connectedProgram = {
      name: 'ore',
      programId: 'ore',
      schemas: {},
      pdas: {},
      accounts: {},
      queries: {},
      raw: {},
      addresses: {},
      constants: {},
      defaults: {},
      math: {},
      instructions: {},
      transactions: {},
      flows: {},
    };
    const operations = getProgramRuntimeExtensions(extended)?.createOperations({
      chain: null as never,
      wallet: undefined,
      program: connectedProgram as never,
    });

    expect(operations?.instructions?.position.create).toBeDefined();
    expect(operations?.instructions?.position.close).toBeDefined();
  });

  it('deep-merges program read namespaces across extension layers', () => {
    const base = extendProgram(BASE_PROGRAM, {
      createRead: () => ({
        account: { board: () => 'board', shared: () => 'base' },
      }),
    });
    const extended = extendProgram(base, {
      createRead: (context) => ({
        account: {
          miner: () => `${context.program.read.account.board()}:miner`,
          shared: () => 'extension',
        },
      }),
    });

    const context = { program: { read: {} } } as never;
    const read = getProgramRuntimeExtensions(extended)?.createRead?.(context);

    expect(read?.account.board()).toBe('board');
    expect(read?.account.miner()).toBe('board:miner');
    expect(read?.account.shared()).toBe('extension');
  });

  it('preserves an opaque bundled read descriptor through later extensions', () => {
    const descriptor = {
      release: {
        programReleaseHash: 'release-ore',
        programSpecHash: 'spec-ore',
      },
      transport: {
        kind: 'local-http',
        endpointSource: 'connect-http-url',
      },
    } as const;
    const bundled = withProgramRead(BASE_PROGRAM, descriptor);
    const extended = extendProgram(bundled, { constants: { unit: 1 } });

    expect(getProgramReadDescriptor(extended)).toBe(descriptor);
    expect(Object.keys(extended)).not.toContain('__areteProgramReadDescriptor');
  });
});

describe('extendPrograms', () => {
  it('extends only targeted program entries', () => {
    const extended = extendPrograms(
      {
        ore: BASE_PROGRAM,
        entropy: { ...BASE_PROGRAM, name: 'entropy', programId: 'entropy1111111111111111111111111111111111' },
      },
      {
        ore: {
          addresses: { vault: 'vault' },
          math: { double: (value: number) => value * 2 },
        },
      }
    );

    expect(extended.ore.addresses).toEqual({ vault: 'vault' });
    expect(extended.ore.math.double(3)).toBe(6);
    expect(extended.entropy.name).toBe('entropy');
  });
});

describe('spread-safe runtime extensions', () => {
  const descriptor = {
    release: { programReleaseHash: 'release-ore', programSpecHash: 'spec-ore' },
    transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
  } as const;

  it('keeps read, flows and readArgCounts when a stack is spread', () => {
    const extended = extendStack(BASE_STACK, {
      readArgCounts: { ping: 0 },
      createRead: () => ({ ping: () => 'pong' }),
      createFlows: () => ({
        close: flowOperation(async () => createPreparedFlow({
          name: 'close',
          transactions: [{
            name: 'close',
            instructions: [{ programId: 'x', keys: [], data: new Uint8Array([]) }],
          }],
          artifacts: {},
        })),
      }),
    });
    const spread = { ...extended, name: 'renamed' };

    expect(getStackRuntimeExtensions(spread)).toBe(getStackRuntimeExtensions(extended));
    expect(getStackRuntimeExtensions(spread)?.readArgCounts).toEqual({ ping: 0 });
    const client = applyConnectedStackExtensions({}, spread) as {
      read?: { ping(): string };
      flows?: { close?: unknown };
    };
    expect(client.read?.ping()).toBe('pong');
    expect(client.flows?.close).toBeDefined();

    // Invisible to key enumeration and serialization.
    expect(Object.keys(extended)).toEqual(Object.keys(BASE_STACK));
    expect(JSON.parse(JSON.stringify(extended))).toEqual(JSON.parse(JSON.stringify(BASE_STACK)));
  });

  it('keeps operations and the read descriptor when a program is spread', () => {
    const program = withProgramRead(
      extendProgram(BASE_PROGRAM, {
        createOperations: () => ({ instructions: { ping: 'op' as never } }),
      }),
      descriptor,
    );
    const spread = { ...program };

    expect(getProgramReadDescriptor(spread)).toBe(descriptor);
    expect(getProgramRuntimeExtensions(spread)).toBe(getProgramRuntimeExtensions(program));
    expect(getProgramRuntimeExtensions(spread)?.createOperations?.({
      chain: null as never,
      wallet: undefined,
      program: {} as never,
    })?.instructions).toEqual({ ping: 'op' });
    expect(Object.getOwnPropertySymbols(spread)).toContain(PROGRAM_OPERATION_EXTENSIONS);
    expect(Object.keys(spread)).not.toContain('__areteProgramOperationExtensions');
    expect(JSON.stringify(spread)).not.toContain('areteProgram');
  });

  it('keeps operations written by a generated object literal when spread', () => {
    const generated = {
      ...BASE_PROGRAM,
      [PROGRAM_OPERATION_EXTENSIONS]: {
        createOperations: () => ({ instructions: { ping: 'op' as never } }),
      },
    };
    const spread = { ...generated };

    expect(getProgramRuntimeExtensions(spread)?.createOperations).toBe(
      generated[PROGRAM_OPERATION_EXTENSIONS].createOperations
    );
    expect(Object.keys(generated)).toEqual(Object.keys(BASE_PROGRAM));
  });

  it('uses registry symbols so separate module copies interoperate', () => {
    expect(STACK_RUNTIME_EXTENSIONS).toBe(Symbol.for('@usearete/sdk/stack-runtime-extensions'));
    expect(PROGRAM_OPERATION_EXTENSIONS).toBe(Symbol.for('@usearete/sdk/program-runtime-extensions'));
  });
});

describe('program identity', () => {
  const descriptor = {
    release: { programReleaseHash: 'release-ore', programSpecHash: 'spec-ore' },
    transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
  } as const;
  const RELEASE = 'arete:registry-package-release:v2:ore';
  const SPEC = { ...BASE_PROGRAM, programSpecHash: 'spec-ore' };
  const RELEASED = { ...SPEC, packageReleaseHash: RELEASE };

  it('drops packageReleaseHash when a program is changed outside its generated SDK', () => {
    const extended = extendProgram(RELEASED, { constants: { unit: 1 } });
    const bundled = withProgramRead(RELEASED, descriptor);
    const programs = extendPrograms({ ore: RELEASED, other: RELEASED }, { ore: { math: { one: () => 1 } } });

    for (const program of [extended, bundled, programs.ore]) {
      expect('packageReleaseHash' in program).toBe(false);
      expect(program.programSpecHash).toBe('spec-ore');
    }
    // An entry the extension does not target is untouched.
    expect(programs.other).toBe(RELEASED);
    // sdkDefinitionHash describes generated content only, so extension drops it.
    expect('sdkDefinitionHash' in extended).toBe(false);
  });

  it('keeps packageReleaseHash where programs are only carried, not changed', () => {
    const stack = extendStack({ ...BASE_STACK, programs: { ore: RELEASED } }, { addresses: { vault: 'V' } });
    const attached = withPrograms(BASE_STACK, { ore: RELEASED });

    expect(stack.programs.ore).toBe(RELEASED);
    expect(attached.programs.ore).toBe(RELEASED);
  });

  it('stamps identity last, keeping extensions and the read descriptor', () => {
    const generated = withProgramIdentity(
      withProgramRead(
        extendProgram(SPEC, {
          constants: { unit: 1 },
          createOperations: () => ({ instructions: { ping: 'op' as never } }),
        }),
        descriptor,
      ),
      { packageReleaseHash: RELEASE },
    );

    expect(generated.packageReleaseHash).toBe(RELEASE);
    expect(generated.constants).toEqual({ unit: 1 });
    expect(getProgramRuntimeExtensions(generated)?.createOperations).toBeTypeOf('function');
    expect(getProgramReadDescriptor(generated)).toBe(descriptor);
    expect(isSameProgramSdk(generated, RELEASED)).toBe(true);
    // A spread keeps every part of the identity-stamped program.
    expect(isSameProgramSdk({ ...generated }, RELEASED)).toBe(true);
    expect(getProgramReadDescriptor({ ...generated })).toBe(descriptor);
    // An absent identity removes it.
    expect('packageReleaseHash' in withProgramIdentity(generated, {})).toBe(false);
  });

  it('compares by package release, then by program spec, never by name', () => {
    expect(compareProgramIdentity(BASE_PROGRAM, BASE_PROGRAM)).toBe('same');
    expect(compareProgramIdentity(RELEASED, { ...RELEASED, name: 'renamed' })).toBe('same');
    expect(compareProgramIdentity(RELEASED, { ...RELEASED, packageReleaseHash: 'other' })).toBe('different');
    // At least one side unknown: the program spec decides between a match that
    // cannot be proven and a conflict.
    expect(compareProgramIdentity(RELEASED, SPEC)).toBe('unproven');
    expect(compareProgramIdentity(SPEC, { ...SPEC })).toBe('unproven');
    expect(compareProgramIdentity(SPEC, { ...SPEC, programSpecHash: 'spec-other' })).toBe('different');
    expect(compareProgramIdentity(BASE_PROGRAM, { ...BASE_PROGRAM })).toBe('different');

    expect(isSameProgramSdk(RELEASED, { ...RELEASED })).toBe(true);
    expect(isSameProgramSdk(SPEC, { ...SPEC })).toBe(false);
  });

  it('lets a user-extended copy of a stack program replace it, with one warning', () => {
    const stack = { ...BASE_STACK, programs: { ore: RELEASED } };
    const userOre = extendProgram(RELEASED, {
      createOperations: () => ({ instructions: { mine: 'op' as never } }),
    });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      expect(withPrograms(stack, { ore: userOre }).programs.ore).toBe(userOre);
      expect(withPrograms(stack, { ore: userOre }).programs.ore).toBe(userOre);
      expect(warn).toHaveBeenCalledTimes(1);
      expect(String(warn.mock.calls[0]?.[0])).toMatch(/could not be proven identical/);
    } finally {
      warn.mockRestore();
    }
  });
});

describe('define extensions', () => {
  it('return their extension input unchanged at runtime', () => {
    const stackInput = {
      addresses: { a: 1 },
      readArgCounts: { value: 0 },
      createRead: () => ({ value: true }),
    };
    const programInput = {
      addresses: { a: 1 },
      constants: { p: true },
      math: { double: (value: number) => value * 2 },
    };
    expect(defineStackExtensions<typeof BASE_STACK>()(stackInput)).toBe(stackInput);
    expect(defineProgramExtensions<typeof BASE_PROGRAM>()(programInput)).toBe(programInput);
  });

  it('requires argument metadata for every stack read', () => {
    const define = defineStackExtensions<typeof BASE_STACK>();

    // @ts-expect-error createRead requires static argument-count metadata.
    define({ createRead: () => ({ ping: () => 'pong' }) });
    // @ts-expect-error the metadata must cover every returned read.
    define({ readArgCounts: {}, createRead: () => ({ ping: () => 'pong' }) });
    // @ts-expect-error fixed read arity must match the function signature.
    define({ readArgCounts: { ping: 0 }, createRead: () => ({ ping: (_value: string) => 'pong' }) });
    // @ts-expect-error optional read arity must declare required and total counts.
    define({
      readArgCounts: { ping: 1 },
      createRead: () => ({ ping: (_value: string, _limit?: number) => 'pong' }),
    });
  });

  it('rejects operations placed in the wrong cardinality namespace', () => {
    defineProgramExtensions<typeof BASE_PROGRAM>()({
      createOperations() {
        return {
          instructions: {
            // @ts-expect-error flows cannot be exposed as instructions
            invalidFlow: flowOperation(async () => createPreparedFlow({
              name: 'invalid',
              transactions: [{
                name: 'invalid',
                instructions: [{ programId: 'ore', keys: [], data: new Uint8Array([1]) }],
              }],
              artifacts: {},
            })),
          },
        };
      },
    });

    defineStackExtensions<typeof BASE_STACK>()({
      createFlows() {
        return {
          // @ts-expect-error instructions cannot be exposed as stack flows
          invalidInstruction: instructionOperation(async () => createPreparedInstruction({
            name: 'invalid',
            instruction: { programId: 'ore', keys: [], data: new Uint8Array([1]) },
            artifacts: {},
          })),
          // @ts-expect-error transactions cannot be exposed as stack flows
          invalidTransaction: transactionOperation(async () => createPreparedTransaction({
            name: 'invalid',
            instructions: [{ programId: 'ore', keys: [], data: new Uint8Array([1]) }],
            artifacts: {},
          })),
        };
      },
    });
  });
});
