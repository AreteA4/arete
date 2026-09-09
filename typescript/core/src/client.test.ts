import { describe, it, expect, vi } from 'vitest';
import { parseFrame, isSnapshotFrame } from './frame';
import { gzip } from 'pako';
import { z } from 'zod';

describe('Arete SDK', () => {
  it('passes each connected client transaction transport to a shared wallet per invocation', async () => {
    const { Arete } = await import('./index');
    const transports: unknown[] = [];
    const wallet = {
      publicKey: 'wallet',
      async signAndSend(_instructions: readonly unknown[], _options?: unknown, context?: { transactionTransport?: unknown }) {
        transports.push(context?.transactionTransport);
        return { signature: `sig-${transports.length}` };
      },
    };
    const stack = (http: string) => ({
      name: http, endpoints: { ws: '', http }, views: {},
    } as const);
    const first = await Arete.connect(stack('https://first.example'), {
      transport: 'http', wallet,
    });
    const second = await Arete.connect(stack('https://second.example'), {
      transport: 'http', wallet,
    });

    await first.transaction([]);
    await second.transaction([]);
    expect(transports).toEqual([first.transactions, second.transactions]);
    expect(first.transactions).not.toBe(second.transactions);
  });

  it('should export Arete class', async () => {
    const { Arete } = await import('./index');
    expect(Arete).toBeDefined();
    expect(typeof Arete.connect).toBe('function');
    expect(typeof Arete.session).toBe('function');
  });

  it('should export ConnectionManager', async () => {
    const { ConnectionManager, isHostedAreteEndpoint } = await import('./index');
    expect(ConnectionManager).toBeDefined();
    expect(isHostedAreteEndpoint('wss://ore.stack.arete.run')).toBe(true);
  });

  it('should export MemoryAdapter', async () => {
    const { MemoryAdapter } = await import('./index');
    expect(MemoryAdapter).toBeDefined();
  });

  it('should export FrameProcessor', async () => {
    const { FrameProcessor } = await import('./index');
    expect(FrameProcessor).toBeDefined();
  });

  it('should not export EntityStore', async () => {
    const sdk = await import('./index');
    expect('EntityStore' in sdk).toBe(false);
  });

  it('waits for processed slots through the connected client after buffered storage flush', async () => {
    const { Arete } = await import('./index');
    const client = await Arete.connect({
      name: 'processed-slot-demo',
      endpoints: {
        ws: 'wss://example.invalid',
        http: 'https://example.invalid',
      },
      views: {},
    } as const, {
      autoConnect: false,
      flushIntervalMs: 100,
    });
    const processor = (client as unknown as {
      processor: {
        handleFrame(frame: unknown): void;
        flush(): void;
      };
    }).processor;
    const wait = client.waitForProcessedSlot(75, { timeoutMs: 100 });

    processor.handleFrame({
      mode: 'state',
      entity: 'Board/state',
      op: 'upsert',
      key: 'board',
      data: { round: 1 },
      seq: '75:000000000001',
    });
    expect(client.processedSlot).toBeNull();
    processor.flush();

    await expect(wait).resolves.toBe(75n);
    expect(client.processedSlot).toBe(75n);
    expect(client.store.get('Board/state', 'board')).toEqual({ round: 1 });
  });

  it('composes execution callbacks with client defaults without changing confirmation', async () => {
    const { Arete, createPreparedInstruction } = await import('./index');
    const calls: string[] = [];
    const client = await Arete.connect({
      name: 'execution-callbacks',
      endpoints: { ws: 'wss://example.invalid', http: 'https://example.invalid' },
      views: {},
    } as const, {
      autoConnect: false,
      wallet: {
        publicKey: 'wallet',
        async signAndSend() {
          return { signature: 'callback-signature', slot: 12 };
        },
      },
      execution: {
        onTransactionStart: () => { calls.push('default:start'); },
        onTransactionSuccess: () => {
          calls.push('default:success');
          throw new Error('default observer failed');
        },
        onCallbackError: () => { calls.push('default:error'); },
      },
    });
    const prepared = createPreparedInstruction({
      name: 'callback-demo',
      instruction: {
        programId: 'program',
        keys: [],
        data: new Uint8Array([1]),
      },
      artifacts: {},
    });

    const receipt = await client.execute(prepared, {
      onTransactionStart: () => { calls.push('call:start'); },
      onTransactionSuccess: () => { calls.push('call:success'); },
      onCallbackError: () => { calls.push('call:error'); },
    });

    expect(calls).toEqual([
      'default:start',
      'call:start',
      'default:success',
      'call:success',
      'default:error',
      'call:error',
    ]);
    expect(receipt).toMatchObject({
      signatures: ['callback-signature'],
      callbackErrors: [expect.objectContaining({ cause: expect.any(Error) })],
    });
  });
});

describe('Frame parsing', () => {
  it('should parse uncompressed entity frames', () => {
    const frame = {
      protocolVersion: 2,
      subscriptionId: 'test:all',
      mode: 'list',
      entity: 'test/list',
      op: 'upsert',
      key: '1',
      data: { id: 1 },
    };
    const result = parseFrame(JSON.stringify(frame));
    expect(result.op).toBe('upsert');
    expect(result.entity).toBe('test/list');
    expect(isSnapshotFrame(result)).toBe(false);
  });

  it('should parse uncompressed snapshot frames', () => {
    const frame = {
      protocolVersion: 2,
      subscriptionId: 'test:all',
      snapshotId: 'snapshot-1',
      authoritative: true,
      mode: 'list',
      entity: 'test/list',
      op: 'snapshot',
      key: 'requested-key',
      data: [{ key: '1', data: { id: 1 } }],
      complete: true,
    };
    const result = parseFrame(JSON.stringify(frame));
    expect(result.op).toBe('snapshot');
    expect(result.entity).toBe('test/list');
    expect(isSnapshotFrame(result)).toBe(true);
    if (isSnapshotFrame(result)) {
      expect(result.key).toBe('requested-key');
      expect(result.data).toHaveLength(1);
      expect(result.data[0].key).toBe('1');
    }
  });

  it('should decompress raw gzip binary frames', () => {
    const originalFrame = {
      protocolVersion: 2,
      subscriptionId: 'test:all',
      snapshotId: 'snapshot-2',
      authoritative: true,
      mode: 'list',
      entity: 'test/list',
      op: 'snapshot',
      data: [
        { key: '1', data: { id: 1, name: 'Test Entity' } },
        { key: '2', data: { id: 2, name: 'Another Entity' } },
      ],
      complete: true,
    };

    const jsonString = JSON.stringify(originalFrame);
    const compressed = gzip(new TextEncoder().encode(jsonString));

    expect(compressed[0]).toBe(0x1f);
    expect(compressed[1]).toBe(0x8b);

    const result = parseFrame(compressed.buffer);
    expect(result.op).toBe('snapshot');
    expect(result.entity).toBe('test/list');
    expect(isSnapshotFrame(result)).toBe(true);
    if (isSnapshotFrame(result)) {
      expect(result.key).toBeUndefined();
      expect(result.data).toHaveLength(2);
      expect(result.data[0].key).toBe('1');
      expect(result.data[0].data).toEqual({ id: 1, name: 'Test Entity' });
      expect(result.data[1].key).toBe('2');
    }
  });

});

describe('Arete instructions (namespaced stacks)', () => {
  const SIGNER = 'So11111111111111111111111111111111111111112';

  async function makeClient(errors: { code: number; name: string; msg: string }[] = []) {
    const { Arete, createInstructionHandler } = await import('./index');
    const handler = (programId: string) =>
      createInstructionHandler({
        programId,
        discriminator: [9],
        args: [],
        accounts: [{ name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' }],
        errors,
      });

    const stack = {
      name: 'demo',
      endpoints: {
        ws: 'wss://example.invalid',
        http: 'https://example.invalid',
      },
      views: {},
      programs: {
        ore: {
          name: 'ore',
          programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
          rawInstructions: { close: handler('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv') },
        },
        entropy: {
          name: 'entropy',
          programId: '3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X',
          rawInstructions: { close: handler('3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X') },
        },
      },
    } as const;

    // autoConnect: false keeps the client fully offline.
    return Arete.connect(stack, { autoConnect: false });
  }

  it('mirrors per-program nesting and builds through the raw path', async () => {
    const client = await makeClient();
    const wallet = {
      publicKey: SIGNER,
      async signAndSend() {
        throw new Error('not used');
      },
    };

    const ix = client.programs.ore.raw.close.build({}, { wallet });
    expect(ix.programId).toBe('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv');
    expect(ix.keys[0]!.pubkey).toBe(SIGNER);

    const ix2 = client.programs.entropy.raw.close.build({}, { wallet });
    expect(ix2.programId).toBe('3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X');
  });

  it('exposes attached programs through client.programs', async () => {
    const { Arete, createInstructionHandler, extendProgram } = await import('./index');
    const handler = createInstructionHandler({
      programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
      discriminator: [7],
      args: [],
      accounts: [{ name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' }],
      errors: [],
    });
    const attachedProgram = extendProgram(
      {
        name: 'attached',
        programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
        rawInstructions: { transfer: handler },
      } as const,
      {
        addresses: { vault: () => 'VaultAddr' },
        constants: { AuthorityType: { MintTokens: 'AuthorityMintTokens' } },
      }
    );
    const client = await Arete.connect(
      {
        name: 'attached-demo',
        endpoints: {
          ws: 'wss://example.invalid',
          http: 'https://example.invalid',
        },
        views: {},
        programs: {},
      } as const,
      {
        autoConnect: false,
        programs: {
          attached: attachedProgram,
        },
      }
    );
    const wallet = {
      publicKey: SIGNER,
      async signAndSend() {
        throw new Error('not used');
      },
    };

    const ix = client.programs.attached.raw.transfer.build({}, { wallet });
    expect(ix.programId).toBe('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
    expect(ix.keys[0]!.pubkey).toBe(SIGNER);
    expect(client.programs.attached.addresses.vault()).toBe('VaultAddr');
    expect(client.programs.attached.constants.AuthorityType.MintTokens).toBe('AuthorityMintTokens');
  });

  it('prefers stack-defined programs over attached programs with the same key', async () => {
    const { Arete, createInstructionHandler } = await import('./index');
    const stackHandler = createInstructionHandler({
      programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
      discriminator: [9],
      args: [],
      accounts: [{ name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' }],
      errors: [],
    });
    const attachedHandler = createInstructionHandler({
      programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
      discriminator: [7],
      args: [],
      accounts: [{ name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' }],
      errors: [],
    });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    try {
      const client = await Arete.connect(
        {
          name: 'collision-demo',
          endpoints: {
            ws: 'wss://example.invalid',
            http: 'https://example.invalid',
          },
          views: {},
          programs: {
            ore: {
              name: 'ore',
              programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
              rawInstructions: { close: stackHandler },
            },
          },
        } as const,
        {
          autoConnect: false,
          programs: {
            ore: {
              name: 'ore-attached',
              programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
              rawInstructions: { close: attachedHandler },
            },
          },
        }
      );
      const wallet = {
        publicKey: SIGNER,
        async signAndSend() {
          throw new Error('not used');
        },
      };

      const ix = client.programs.ore.raw.close.build({}, { wallet });
      expect(ix.programId).toBe('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv');
      expect(warn).toHaveBeenCalledWith(
        "Ignoring attached program 'ore' for stack 'collision-demo' because the stack already defines that key"
      );
    } finally {
      warn.mockRestore();
    }
  });

  it('parses attached-program errors in transaction() from aggregated handler metadata', async () => {
    const { Arete, InstructionError, createInstructionHandler } = await import('./index');
    const handler = createInstructionHandler({
      programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
      discriminator: [7],
      args: [],
      accounts: [{ name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' }],
      errors: [{ code: 7000, name: 'AttachedProgramError', msg: 'attached failure' }],
    });
    const client = await Arete.connect(
      {
        name: 'attached-errors-demo',
        endpoints: {
          ws: 'wss://example.invalid',
          http: 'https://example.invalid',
        },
        views: {},
        programs: {},
      } as const,
      {
        autoConnect: false,
        programs: {
          attached: {
            name: 'attached',
            programId: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
            rawInstructions: { transfer: handler },
          },
        },
      }
    );
    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string }> {
        throw { InstructionError: [0, { Custom: 7000 }] };
      },
    };

    const ix = client.programs.attached.raw.transfer.build({}, { wallet });
    await expect(client.transaction([ix], { wallet })).rejects.toMatchObject({
      name: 'InstructionError',
      programError: { code: 7000, name: 'AttachedProgramError' },
    });
    await expect(client.transaction([ix], { wallet })).rejects.toBeInstanceOf(InstructionError);
  });

  it('parses program errors in transaction() from aggregated handler metadata', async () => {
    const { InstructionError } = await import('./index');
    const client = await makeClient([
      { code: 6000, name: 'SlippageExceeded', msg: 'Slippage tolerance exceeded' },
    ]);
    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string }> {
        throw { InstructionError: [0, { Custom: 6000 }] };
      },
    };

    const ix = client.programs.ore.raw.close.build({}, { wallet });
    await expect(client.transaction([ix], { wallet })).rejects.toMatchObject({
      name: 'InstructionError',
      programError: { code: 6000, name: 'SlippageExceeded' },
    });
    await expect(client.transaction([ix], { wallet })).rejects.toBeInstanceOf(InstructionError);
  });

  it('prefers explicit errors over aggregated metadata in transaction()', async () => {
    const client = await makeClient([
      { code: 6000, name: 'WrongName', msg: 'from aggregate' },
    ]);
    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string }> {
        throw { InstructionError: [0, { Custom: 6000 }] };
      },
    };

    const ix = client.programs.ore.raw.close.build({}, { wallet });
    await expect(
      client.transaction([ix], {
        wallet,
        errors: [{ code: 6000, name: 'RightName', msg: 'from override' }],
      })
    ).rejects.toMatchObject({
      programError: { code: 6000, name: 'RightName' },
    });
  });

  it('executes prepared flows through the client instance', async () => {
    const { createPreparedFlow } = await import('./index');
    const client = await makeClient();
    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string; slot: number }> {
        return { signature: 'sig-plan', slot: 77 };
      },
    };
    client.setWallet(wallet);

    const ix = client.programs.ore.raw.close.build({}, { wallet });
    const plan = createPreparedFlow({
      name: 'close-plan',
      artifacts: { closed: true },
      transactions: [{ name: 'close-stage', instructions: [ix], requiredSignerAddresses: [SIGNER], errors: [] }],
    });

    await expect(client.execute(plan)).resolves.toEqual({
      kind: 'flow',
      operationName: 'close-plan',
      artifacts: { closed: true },
      signatures: ['sig-plan'],
      transactions: [{ transactionIndex: 0, transactionName: 'close-stage', signature: 'sig-plan', slot: 77 }],
    });
  });

  it('inspects prepared operations without signing and rejects flows', async () => {
    const { createPreparedFlow, createPreparedInstruction } = await import('./index');
    const client = await makeClient();
    const signAndSend = vi.fn();
    const inspectTransaction = vi.fn(async () => ({
      feeLamports: 5000,
      contextSlot: 88,
      logs: [],
    }));
    const wallet = { publicKey: SIGNER, signAndSend, inspectTransaction };
    client.setWallet(wallet);
    const ix = client.programs.ore.raw.close.build({}, { wallet });
    const prepared = createPreparedInstruction({
      name: 'inspect-close',
      instruction: ix,
      artifacts: { close: true },
    });

    await expect(client.inspectOperation(prepared)).resolves.toMatchObject({
      description: { kind: 'instruction', name: 'inspect-close' },
      transaction: { feeLamports: 5000, contextSlot: 88 },
      programError: null,
    });
    expect(signAndSend).not.toHaveBeenCalled();
    expect(inspectTransaction).toHaveBeenCalledTimes(1);

    const flow = createPreparedFlow({
      name: 'inspect-flow',
      artifacts: undefined,
      transactions: [{ name: 'one-stage', instructions: [ix] }],
    });
    await expect(client.inspectOperation(flow)).rejects.toThrow(
      "Cannot inspect flow 'inspect-flow': flow inspection is not supported"
    );
    expect(inspectTransaction).toHaveBeenCalledTimes(1);
  });

  it('attaches stack extensions to the connected client without extra binding', async () => {
    const { Arete, createInstructionHandler, createPreparedFlow, extendStack, flowOperation } = await import('./index');
    const handler = createInstructionHandler({
      programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
      discriminator: [9],
      args: [],
      accounts: [{ name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' }],
      errors: [],
    });

    let plan: ReturnType<typeof createPreparedFlow>;

    const stack = extendStack(
      {
        name: 'extended-demo',
        endpoints: {
          ws: 'wss://example.invalid',
          http: 'https://example.invalid',
        },
        views: {},
        programs: {
          ore: {
            name: 'ore',
            programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
            rawInstructions: { close: handler },
          },
        },
      } as const,
      {
        addresses: {
          vault(owner: string) {
            return `vault:${owner}`;
          },
        },
        constants: {
          closeMemo: 'prepared-close',
        },
        createFlows() {
          return {
            close: flowOperation(async () => plan),
          };
        },
        readArgCounts: { closeMemo: 0 },
        createRead(client) {
          return {
            closeMemo() {
              return (client as { constants: { closeMemo: string } }).constants.closeMemo;
            },
          };
        },
      }
    );

    const client = await Arete.connect(stack, { autoConnect: false });
    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string; slot: number }> {
        return { signature: 'sig-extended', slot: 99 };
      },
    };
    client.setWallet(wallet);

    const ix = client.programs.ore.raw.close.build({}, { wallet });
    plan = createPreparedFlow({
      name: 'extended-close-plan',
      artifacts: { closed: true },
      transactions: [
        {
          name: 'extended-close-stage',
          instructions: [ix],
          requiredSignerAddresses: [SIGNER],
          errors: [],
        },
      ],
    });

    expect(client.addresses.vault('alice')).toBe('vault:alice');
    expect(client.constants.closeMemo).toBe('prepared-close');
    await expect(client.flows.close.prepare({})).resolves.toBe(plan);
    expect(client.read.closeMemo()).toBe('prepared-close');
    await expect(client.execute(plan)).resolves.toEqual({
      kind: 'flow',
      operationName: 'extended-close-plan',
      artifacts: { closed: true },
      signatures: ['sig-extended'],
      transactions: [{ transactionIndex: 0, transactionName: 'extended-close-stage', signature: 'sig-extended', slot: 99 }],
    });
  });

  it('preserves standalone program extension namespaces on connected clients', async () => {
    const { Arete, createInstructionHandler, createPreparedInstruction, extendProgram, instructionOperation } = await import('./index');
    const rawHandler = createInstructionHandler({
      programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
      discriminator: [9],
      args: [],
      accounts: [{ name: 'signer', isSigner: true, isWritable: true, category: 'signer', signerKind: 'wallet' }],
      errors: [],
    });
    const stack = {
      name: 'program-extensions-demo',
      endpoints: {
        ws: 'wss://example.invalid',
        http: 'https://example.invalid',
      },
      views: {},
      programs: {
        ore: extendProgram(
          {
            name: 'ore',
            programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
            rawInstructions: { close: rawHandler },
          },
          {
            raw: { close: rawHandler },
            addresses: { vault: (owner: string) => `vault:${owner}` },
            defaults: { closeMemo: 'prepared-close' },
            math: { double: (value: number) => value * 2 },
            createOperations() {
              return {
                instructions: {
                  close: instructionOperation(async () =>
                    createPreparedInstruction({
                      name: 'close',
                      instruction: {
                        programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
                        keys: [],
                        data: new Uint8Array([1]),
                      },
                      artifacts: { closed: true },
                    })
                  ),
                },
              };
            },
          }
        ),
      },
    } as const;

    const client = await Arete.connect(stack, { autoConnect: false });

    const wallet = {
      publicKey: SIGNER,
      async signAndSend(): Promise<{ signature: string; slot: number }> {
        return { signature: 'sig-program-extension', slot: 101 };
      },
    };
    client.setWallet(wallet);

    expect(client.programs.ore.addresses.vault('alice')).toBe('vault:alice');
    expect(client.programs.ore.defaults.closeMemo).toBe('prepared-close');
    expect(client.programs.ore.math.double(3)).toBe(6);
    const prepared = await client.programs.ore.instructions.close.prepare({});
    expect(prepared.kind).toBe('instruction');
    await expect(client.execute(prepared)).resolves.toEqual({
      kind: 'instruction',
      operationName: 'close',
      artifacts: { closed: true },
      signatures: ['sig-program-extension'],
      transaction: { transactionIndex: 0, transactionName: 'close', signature: 'sig-program-extension', slot: 101 },
    });
  });

  it('provides program extension callbacks with the connected program interface', async () => {
    const {
      Arete,
      createInstructionHandler,
      createPreparedInstruction,
      defineProgramExtensions,
      extendProgram,
      instructionOperation,
    } = await import('./index');
    const rawHandler = createInstructionHandler({
      programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
      discriminator: [9],
      args: [],
      accounts: [],
      errors: [],
    });
    const base = extendProgram(
      {
        name: 'ore',
        programId: 'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv',
        rawInstructions: { close: rawHandler },
      },
      {
        createOperations() {
          return {
            instructions: {
              close: instructionOperation(async () =>
                createPreparedInstruction({
                  name: 'close',
                  instruction: { programId: 'ore', keys: [], data: new Uint8Array([1]) },
                  artifacts: { source: 'base' as const },
                })
              ),
            },
          };
        },
      }
    );
    const extensions = defineProgramExtensions<typeof base>()({
      addresses: { vault: (owner: string) => `vault:${owner}` },
      defaults: { operationName: 'close-via-context' as const },
      math: { double: (value: number) => value * 2 },
      createRead(context) {
        return {
          identity: () => context.program.programId,
        };
      },
      createOperations(context) {
        context.program.instructions.close.kind satisfies 'instruction';
        context.program.addresses.vault('typecheck');
        context.program.defaults.operationName satisfies 'close-via-context';
        context.program.math.double(3) satisfies number;
        context.program.read.identity() satisfies string | undefined;
        context.program.raw.close.build;
        // @ts-expect-error operations being created are available after the factory returns
        context.program.instructions.closeViaContext;
        return {
          instructions: {
            closeViaContext: instructionOperation(async () => {
              const prepared = await context.program.instructions.close.prepare({});
              return createPreparedInstruction({
                name: context.program.defaults.operationName,
                instruction: prepared.instruction,
                artifacts: {
                  source: prepared.artifacts.source,
                  vault: context.program.addresses.vault('alice'),
                },
              });
            }),
          },
        };
      },
    });
    const stack = {
      name: 'program-extension-context-demo',
      endpoints: { ws: 'wss://example.invalid', http: 'https://example.invalid' },
      views: {},
      programs: { ore: extendProgram(base, extensions) },
    } as const;

    const client = await Arete.connect(stack, { autoConnect: false });
    expect(client.programs.ore.read.identity()).toBe(
      'oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv'
    );
    const prepared = await client.programs.ore.instructions.closeViaContext.prepare({});

    expect(prepared.artifacts).toEqual({ source: 'base', vault: 'vault:alice' });
  });

  it('exposes program account reads, stack queries, and chain helpers through HTTP surfaces', async () => {
    const originalFetch = globalThis.fetch;
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith('/v1/releases/release-squads/accounts/Multisig/multisig-1')) {
        return new Response(JSON.stringify({ threshold: 2, transactionIndex: '41' }), { status: 200 });
      }
      if (url.endsWith('/v1/releases/release-squads/accounts/Multisig') && init?.method === 'POST') {
        return new Response(JSON.stringify({
          items: [
            { address: 'multisig-1', status: 'ok', value: { threshold: 2, transaction_index: '41' } },
            { address: 'missing', status: 'missing' },
            { address: 'broken', status: 'error', error: { code: 'ACCOUNT_DECODE_FAILED' } },
          ],
        }), { status: 200 });
      }
      if (url.endsWith('/programs/squads/queries/findThreshold')) {
        return new Response(JSON.stringify({ transaction_index: '99' }), { status: 200 });
      }
      if (url.endsWith('/queries/currentMultisig')) {
        return new Response(JSON.stringify({ multisig_key: 'multisig-1' }), { status: 200 });
      }
      if (url.endsWith('/chain/exists/multisig-1')) {
        return new Response(JSON.stringify({ exists: true }), { status: 200 });
      }
      return new Response(JSON.stringify({ init }), { status: 200 });
    });
    globalThis.fetch = fetchMock as typeof fetch;

    try {
      const { Arete, programAccountRead, programQuery, stackQuery } = await import('./index');
      const bigintSchema = z
        .union([z.bigint(), z.string(), z.number().int()])
        .transform((value) => BigInt(value));
      const stack = {
        name: 'reads-demo',
        endpoints: {
          ws: 'wss://example.invalid',
          http: 'https://example.invalid',
        },
        views: {},
        queries: {
          currentMultisig: stackQuery<{ owner: string }, { multisigKey: string }>({
            name: 'currentMultisig',
            path: '/queries/currentMultisig',
            schema: z
              .object({ multisig_key: z.string() })
              .transform((value) => ({ multisigKey: value.multisig_key })),
          }),
        },
        programs: {
          squads: {
            name: 'squads',
            programId: 'SQDS111111111111111111111111111111111111111',
            accounts: {
              Multisig: programAccountRead<{ threshold: number; transactionIndex: bigint }>({
                account: 'Multisig',
                schema: z
                  .object({
                    threshold: z.number(),
                    transaction_index: bigintSchema,
                  })
                  .transform((value) => ({
                    threshold: value.threshold,
                    transactionIndex: value.transaction_index,
                  })),
              }),
            },
            queries: {
              findThreshold: programQuery<{ owner: string }, { transactionIndex: bigint }>({
                name: 'findThreshold',
                path: '/programs/squads/queries/findThreshold',
                schema: z
                  .object({ transaction_index: bigintSchema })
                  .transform((value) => ({ transactionIndex: value.transaction_index })),
              }),
            },
            rawInstructions: {},
          },
        },
        programReads: {
          squads: {
            release: {
              programReleaseHash: 'release-squads',
              programSpecHash: 'spec-squads',
            },
            transport: {
              kind: 'hosted-binding',
              binding: {
                endpoint: 'https://example.invalid',
                programReadBindingId: 'prb_00000000000000000000000000000003',
                auth: {
                  required: true,
                  sessionEndpoint: 'https://auth.example.invalid/session',
                  targetKind: 'program-read-binding',
                  targetId: 'prb_00000000000000000000000000000003',
                },
              },
            },
          },
        },
      } as const;

      const client = await Arete.connect(stack, {
        autoConnect: false,
        auth: {
          getToken: async () => ({ token: 'session-token', expiresAt: Math.floor(Date.now() / 1000) + 300 }),
        },
      });
      await expect(client.programs.squads.accounts.Multisig.fetch('multisig-1')).resolves.toEqual({
        threshold: 2,
        transactionIndex: BigInt('41'),
      });
      await expect(client.programs.squads.queries.findThreshold({ owner: 'owner-1' })).resolves.toEqual({
        transactionIndex: BigInt('99'),
      });
      await expect(client.queries.currentMultisig({ owner: 'owner-1' })).resolves.toEqual({
        multisigKey: 'multisig-1',
      });
      await expect(client.chain.exists('multisig-1')).resolves.toBe(true);
      await expect(
        client.programs.squads.accounts.Multisig.fetchMany(['multisig-1', 'missing', 'broken'])
      ).resolves.toEqual({
        items: [
          { address: 'multisig-1', status: 'ok', value: { threshold: 2, transactionIndex: BigInt('41') } },
          { address: 'missing', status: 'missing' },
          { address: 'broken', status: 'error', error: { code: 'ACCOUNT_DECODE_FAILED' } },
        ],
      });

      expect(fetchMock).toHaveBeenCalledWith('https://example.invalid/v1/releases/release-squads/accounts/Multisig/multisig-1', {
        method: 'GET',
        headers: expect.any(Headers),
        body: undefined,
      });
      expect(fetchMock).toHaveBeenCalledWith('https://example.invalid/programs/squads/queries/findThreshold', {
        method: 'POST',
        headers: expect.any(Headers),
        body: JSON.stringify({ owner: 'owner-1' }),
      });
      expect(fetchMock).toHaveBeenCalledWith('https://example.invalid/queries/currentMultisig', {
        method: 'POST',
        headers: expect.any(Headers),
        body: JSON.stringify({ owner: 'owner-1' }),
      });
      const firstHeaders = fetchMock.mock.calls[0]?.[1]?.headers as Headers;
      const secondHeaders = fetchMock.mock.calls[1]?.[1]?.headers as Headers;
      const thirdHeaders = fetchMock.mock.calls[2]?.[1]?.headers as Headers;
      expect(firstHeaders.get('authorization')).toBe('Bearer session-token');
      expect(secondHeaders.get('authorization')).toBe('Bearer session-token');
      expect(thirdHeaders.get('authorization')).toBe('Bearer session-token');
      expect(secondHeaders.get('content-type')).toBe('application/json');
      expect(thirdHeaders.get('content-type')).toBe('application/json');
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  it('allows explicit HTTP-only clients', async () => {
    const originalFetch = globalThis.fetch;
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ exists: false }), { status: 200 }));
    globalThis.fetch = fetchMock as typeof fetch;

    try {
      const { Arete } = await import('./index');
      const stack = {
        name: 'http-only',
        endpoints: {
          ws: '',
          http: 'https://example.invalid',
        },
        views: {},
        programs: {},
      } as const;

      const client = await Arete.connect(stack, { transport: 'http' });
      await expect(client.chain.exists('unused')).resolves.toBe(false);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  it('separates initial connection from automatic reconnection policy', async () => {
    const { Arete } = await import('./index');
    const stack = {
      name: 'manual-connect',
      endpoints: {
        ws: 'wss://example.invalid',
        http: 'https://example.invalid',
      },
      views: {},
    } as const;

    const client = await Arete.connect(stack, {
      autoConnect: false,
      autoReconnect: true,
    });

    expect(client.connectionState).toBe('disconnected');
    client.disconnect();
  });

  it('does not infer HTTP transport from disabled initial connection', async () => {
    const { Arete } = await import('./index');
    const stack = {
      name: 'missing-websocket',
      endpoints: { ws: '', http: 'https://example.invalid' },
      views: {},
    } as const;

    await expect(Arete.connect(stack, { autoConnect: false })).rejects.toMatchObject({
      code: 'INVALID_CONFIG',
    });
  });

  it('throws structured HTTP read errors without conflating schema failures', async () => {
    const { Arete, ReadRequestError, stackQuery } = await import('./index');
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const path = new URL(String(input)).pathname;
      if (path === '/queries/failing') {
        return new Response(JSON.stringify({ error: 'read failed', code: 'read-denied' }), {
          status: 403,
          headers: { 'X-Error-Code': 'read-denied' },
        });
      }
      return new Response(JSON.stringify({ unexpected: true }), { status: 200 });
    });
    const stack = {
      name: 'read-errors',
      endpoints: { ws: '', http: 'https://example.invalid' },
      views: {},
      queries: {
        failing: stackQuery({ name: 'failing', path: '/queries/failing' }),
        invalid: stackQuery({
          name: 'invalid',
          path: '/queries/invalid',
          schema: z.object({ value: z.string() }),
        }),
      },
      programs: {},
    } as const;
    const client = await Arete.connect(stack, {
      transport: 'http',
      fetch: fetchMock as typeof fetch,
    });

    const requestError = await client.queries.failing({}).catch((error: unknown) => error);
    expect(requestError).toBeInstanceOf(ReadRequestError);
    expect(requestError).toMatchObject({
      status: 403,
      path: '/queries/failing',
      body: JSON.stringify({ error: 'read failed', code: 'read-denied' }),
      serverErrorCode: 'read-denied',
    });

    const schemaError = await client.queries.invalid({}).catch((error: unknown) => error);
    expect(schemaError).not.toBeInstanceOf(ReadRequestError);
    expect(schemaError).toEqual(expect.objectContaining({
      message: "Query 'invalid' failed schema validation",
    }));
  });
});

describe('Arete transport: http', () => {
  const HTTP_STACK = {
    name: 'http-demo',
    endpoints: {
      ws: 'wss://example.invalid',
      http: 'https://example.invalid',
    },
    views: {
      Thing: {
        list: { mode: 'list', view: 'Thing/list' },
      },
    },
    programs: {},
  } as const;

  it('serves point reads and chain reads without opening a socket', async () => {
    const { Arete } = await import('./index');
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith('/chain/exists/addr-1')) {
        return new Response(JSON.stringify({ exists: true }), { status: 200 });
      }
      if (url.endsWith('/chain/native-balance')) {
        expect(JSON.parse(String(init?.body))).toEqual({
          address: 'addr-1',
          minContextSlot: '9007199254740993',
        });
        return new Response(
          JSON.stringify({
            lamports: '9007199254740995',
            contextSlot: '9007199254740997',
          }),
          { status: 200 }
        );
      }
      if (url.endsWith('/chain/rent-exemption/82')) {
        return new Response(JSON.stringify({ lamports: 1461600 }), { status: 200 });
      }
      if (url.includes('/chain/accounts/addr-1')) {
        return new Response(
          JSON.stringify({
            address: 'addr-1',
            ownerProgram: 'owner-program',
            lamports: '5',
            executable: false,
            data: Buffer.from([1, 2, 3]).toString('base64'),
          }),
          { status: 200 }
        );
      }
      return new Response('null', { status: 200 });
    });

    const client = await Arete.connect(HTTP_STACK, { transport: 'http', fetch: fetchMock as typeof fetch });
    expect(client.isConnected()).toBe(false);
    await expect(client.chain.exists('addr-1')).resolves.toBe(true);
    await expect(
      client.chain.nativeBalance('addr-1', { minContextSlot: 9_007_199_254_740_993n })
    ).resolves.toEqual({
      lamports: 9_007_199_254_740_995n,
      contextSlot: 9_007_199_254_740_997n,
    });
    await expect(client.chain.minimumBalanceForRentExemption(82)).resolves.toBe(1461600);

    const account = await client.chain.account('addr-1');
    expect(account).toMatchObject({ address: 'addr-1', ownerProgram: 'owner-program', lamports: 5n });
    expect(Array.from(account!.data)).toEqual([1, 2, 3]);
  });

  it('uses structured errors for failed chain reads', async () => {
    const { Arete, ReadRequestError } = await import('./index');
    const fetchMock = vi.fn(async () => new Response(
      JSON.stringify({ error: 'temporarily unavailable', code: 'chain-unavailable' }),
      { status: 503 }
    ));
    const client = await Arete.connect(HTTP_STACK, {
      transport: 'http',
      fetch: fetchMock as typeof fetch,
    });

    const error = await client.chain.exists('addr-1').catch((cause: unknown) => cause);
    expect(error).toBeInstanceOf(ReadRequestError);
    expect(error).toMatchObject({
      status: 503,
      path: '/chain/exists/addr-1',
      serverErrorCode: 'chain-unavailable',
    });
  });

  it('rejects connect() and view subscriptions with WEBSOCKET_DISABLED', async () => {
    const { Arete } = await import('./index');
    const client = await Arete.connect(HTTP_STACK, {
      transport: 'http',
      fetch: vi.fn(async () => new Response('null', { status: 200 })) as typeof fetch,
    });

    await expect(client.connect()).rejects.toMatchObject({ code: 'WEBSOCKET_DISABLED' });

    const iterate = async () => {
      for await (const _entry of client.views.Thing.list.use()) {
        break;
      }
    };
    await expect(iterate()).rejects.toMatchObject({ code: 'WEBSOCKET_DISABLED' });
  });

  it('requires an HTTP endpoint in http transport mode', async () => {
    const { Arete } = await import('./index');
    const stack = {
      name: 'no-http',
      endpoints: { ws: '' },
      views: {},
      programs: {},
    } as const;

    await expect(Arete.connect(stack, { transport: 'http' })).rejects.toMatchObject({
      code: 'INVALID_CONFIG',
    });
  });

  it('uses generated HTTP metadata without deriving from an unrelated WebSocket endpoint', async () => {
    const { Arete } = await import('./index');
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ exists: true }), { status: 200 }));
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const stack = {
      name: 'independent-endpoints',
      endpoints: {
        ws: 'wss://stream.example.test/ws/v2?tenant=endpoint',
        http: 'https://reads.unrelated.test/api/arete/v3',
      },
      views: {},
      programs: {},
    } as const;

    const client = await Arete.connect(stack, { transport: 'http', fetch: fetchMock as typeof fetch });
    await expect(client.chain.exists('x')).resolves.toBe(true);
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      'https://reads.unrelated.test/api/arete/v3/chain/exists/x'
    );
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });

  it('prefers an explicit runtime HTTP endpoint over generated HTTP metadata', async () => {
    const { Arete } = await import('./index');
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ exists: true }), { status: 200 }));
    const stack = {
      name: 'runtime-http',
      endpoints: {
        ws: 'wss://stream.example.test/socket',
        http: 'https://generated.example.test/reads',
      },
      views: {},
      programs: {},
    } as const;

    const client = await Arete.connect(stack, {
      transport: 'http',
      httpUrl: 'https://runtime.unrelated.test/custom/prefix',
      fetch: fetchMock as typeof fetch,
    });
    await expect(client.chain.exists('x')).resolves.toBe(true);
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      'https://runtime.unrelated.test/custom/prefix/chain/exists/x'
    );
  });

  it('accepts explicit runtime endpoints for an endpointless local definition', async () => {
    const { Arete } = await import('./index');
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ exists: true }), { status: 200 }));
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const stack = {
      name: 'local-endpointless',
      endpoints: { ws: '', http: '' },
      views: {},
      programs: {},
    } as const;

    const client = await Arete.connect(stack, {
      url: 'ws://127.0.0.1:8878/socket',
      httpUrl: 'http://127.0.0.1:8879/local/api',
      autoConnect: false,
      fetch: fetchMock as typeof fetch,
    });
    await expect(client.chain.exists('local-account')).resolves.toBe(true);
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      'http://127.0.0.1:8879/local/api/chain/exists/local-account'
    );
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });

  it('does not derive when generated endpoint metadata explicitly omits HTTP', async () => {
    const { Arete } = await import('./index');
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const stack = {
      name: 'authoritative-no-http',
      endpoints: { ws: 'wss://stream.example.test/socket', http: '' },
      views: {},
      programs: {},
    } as const;

    await expect(Arete.connect(stack, { transport: 'http' })).rejects.toMatchObject({
      code: 'INVALID_CONFIG',
    });
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });

  it('does not derive stack HTTP from a WebSocket endpoint', async () => {
    const { Arete } = await import('./index');
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const stack = {
      name: 'legacy-derive-http',
      endpoints: { ws: 'wss://legacy.example.test/socket' },
      views: {},
      programs: {},
    } as const;

    await expect(Arete.connect(stack, { transport: 'http' })).rejects.toMatchObject({
      code: 'INVALID_CONFIG',
    });
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });
});

describe('transaction version capability', () => {
  const stack = {
    name: 'v1-capability',
    endpoints: { ws: '', http: 'https://stack.example' },
    views: {},
  } as const;
  const instruction = { programId: 'program', keys: [], data: new Uint8Array() };

  it('transaction_v1 request is rejected before any adapter call', async () => {
    const { Arete, TransactionOptionsError } = await import('./index');
    const signAndSend = vi.fn(async () => ({ signature: 'never' }));
    const inspectTransaction = vi.fn(async () => ({ feeLamports: 1 }));
    const client = await Arete.connect(stack, {
      transport: 'http',
      wallet: { publicKey: 'wallet', signAndSend, inspectTransaction },
    });

    const error = await client.transaction([instruction], {
      send: { transactionVersion: 1 },
    }).catch((cause) => cause);

    expect(error).toBeInstanceOf(TransactionOptionsError);
    expect(error).toMatchObject({
      code: 'unsupported_transaction_version',
      requestedVersion: 1,
      supportedVersions: undefined,
    });
    expect(signAndSend).not.toHaveBeenCalled();
  });

  it('lets an adapter with no declared capability keep serving ordinary sends', async () => {
    const signAndSend = vi.fn(async () => ({ signature: 'sig' }));
    const { Arete } = await import('./index');
    const client = await Arete.connect(stack, {
      transport: 'http',
      wallet: { publicKey: 'wallet', signAndSend },
    });

    await expect(client.transaction([instruction], {
      send: { transactionVersion: 0, resources: { computeUnitLimit: 200_000 } },
    })).resolves.toEqual({ signature: 'sig', slot: undefined });
    await expect(client.transaction([instruction], {
      send: { transactionVersion: 'legacy' },
    })).resolves.toMatchObject({ signature: 'sig' });
  });

  it('merges execution defaults into per-call resources key by key', async () => {
    const { Arete, createPreparedInstruction } = await import('./index');
    const signAndSend = vi.fn(async () => ({ signature: 'sig' }));
    const client = await Arete.connect(stack, {
      transport: 'http',
      wallet: { publicKey: 'wallet', signAndSend },
      execution: {
        send: {
          confirmationLevel: 'finalized',
          resources: { computeUnitLimit: 200_000, computeUnitPriceMicroLamports: 1_000n },
        },
      },
    });

    await client.execute(
      createPreparedInstruction({ name: 'budgeted', instruction, artifacts: undefined }),
      {
        send: {
          resources: {
            computeUnitPriceMicroLamports: 9_000_000_000_000_000_001n,
            heapSize: '262144',
          },
        },
      }
    );

    // A per-call fee override must not discard the configured compute budget.
    expect(signAndSend).toHaveBeenCalledWith([instruction], expect.objectContaining({
      confirmationLevel: 'finalized',
      resources: {
        computeUnitLimit: 200_000,
        computeUnitPriceMicroLamports: 9_000_000_000_000_000_001n,
        heapSize: '262144',
      },
    }), expect.anything());
  });

  // The two fee fields are one slot. Inheriting a v0 default fee alongside a V1
  // per-call fee left the call permanently rejected as mutually exclusive, so a
  // configured fee model could never be replaced per call.
  it('v1_contract replaces the inherited fee model when a per-call fee overrides it', async () => {
    // Dynamic import matches every other test here: each one re-imports the
    // module so client state cannot leak between cases.
    const { Arete, createPreparedInstruction } = await import('./index');
    const signAndSend = vi.fn(async () => ({ signature: 'sig' }));
    const client = await Arete.connect(stack, {
      transport: 'http',
      wallet: {
        publicKey: 'wallet',
        signAndSend,
        supportedTransactionVersions: [0, 1],
      },
      execution: {
        send: {
          resources: { computeUnitLimit: 200_000, computeUnitPriceMicroLamports: 1_000n },
        },
      },
    });

    await client.execute(
      createPreparedInstruction({ name: 'budgeted', instruction, artifacts: undefined }),
      { send: { transactionVersion: 1, resources: { priorityFeeLamports: 50_000n } } }
    );

    expect(signAndSend).toHaveBeenCalledWith([instruction], expect.objectContaining({
      transactionVersion: 1,
      resources: {
        computeUnitLimit: 200_000,
        priorityFeeLamports: 50_000n,
      },
    }), expect.anything());
  });

  it('rejects a V1 inspection before the adapter is asked to simulate', async () => {
    const { Arete, createPreparedInstruction, TransactionOptionsError } = await import('./index');
    const inspectTransaction = vi.fn(async () => ({ feeLamports: 1 }));
    const signAndSend = vi.fn();
    const client = await Arete.connect(stack, {
      transport: 'http',
      wallet: {
        publicKey: 'wallet',
        signAndSend,
        inspectTransaction,
        supportedTransactionVersions: [0],
      },
    });
    const prepared = createPreparedInstruction({
      name: 'inspect-v1', instruction, artifacts: undefined,
    });

    await expect(client.inspectOperation(prepared, { inspect: { transactionVersion: 1 } }))
      .rejects.toBeInstanceOf(TransactionOptionsError);
    expect(inspectTransaction).not.toHaveBeenCalled();
    expect(signAndSend).not.toHaveBeenCalled();
  });
});
