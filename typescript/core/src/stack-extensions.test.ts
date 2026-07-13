import { describe, expect, it } from 'vitest';

import {
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
  transactionOperation,
} from './index';
import type { ProgramSdkDefinition, StackDefinition } from './types';

const BASE_STACK = {
  name: 'demo',
  endpoints: { ws: 'wss://example.invalid', http: 'https://example.invalid' },
  views: {},
} as const satisfies StackDefinition;

const BASE_PROGRAM = {
  name: 'ore',
  programId: 'ore111111111111111111111111111111111111111',
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
      createRead: () => ({ one: 1, shared: 'base' }),
    });
    const second = extendStack(first, {
      addresses: { b: 2, shared: 'override' },
      constants: { two: true },
      createRead: () => ({ two: 2, shared: 'override' }),
    });

    expect(second.addresses).toEqual({ a: 1, b: 2, shared: 'override' });
    expect(second.constants).toEqual({ one: true, two: true });
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

describe('define extensions', () => {
  it('return their extension input unchanged at runtime', () => {
    const stackInput = { addresses: { a: 1 }, createRead: () => ({ value: true }) };
    const programInput = {
      addresses: { a: 1 },
      constants: { p: true },
      math: { double: (value: number) => value * 2 },
    };
    expect(defineStackExtensions<typeof BASE_STACK>()(stackInput)).toBe(stackInput);
    expect(defineProgramExtensions<typeof BASE_PROGRAM>()(programInput)).toBe(programInput);
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
