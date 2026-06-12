import { describe, it, expect } from 'vitest';
import { parseFrame, isSnapshotFrame } from './frame';
import { gzip } from 'pako';

describe('Arete SDK', () => {
  it('should export Arete class', async () => {
    const { Arete } = await import('./index');
    expect(Arete).toBeDefined();
    expect(typeof Arete.connect).toBe('function');
  });

  it('should export ConnectionManager', async () => {
    const { ConnectionManager } = await import('./index');
    expect(ConnectionManager).toBeDefined();
  });

  it('should export MemoryAdapter', async () => {
    const { MemoryAdapter } = await import('./index');
    expect(MemoryAdapter).toBeDefined();
  });

  it('should export FrameProcessor', async () => {
    const { FrameProcessor } = await import('./index');
    expect(FrameProcessor).toBeDefined();
  });
});

describe('Frame parsing', () => {
  it('should parse uncompressed entity frames', () => {
    const frame = {
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
      mode: 'list',
      entity: 'test/list',
      op: 'snapshot',
      data: [{ key: '1', data: { id: 1 } }],
    };
    const result = parseFrame(JSON.stringify(frame));
    expect(result.op).toBe('snapshot');
    expect(result.entity).toBe('test/list');
    expect(isSnapshotFrame(result)).toBe(true);
    if (isSnapshotFrame(result)) {
      expect(result.data).toHaveLength(1);
      expect(result.data[0].key).toBe('1');
    }
  });

  it('should decompress raw gzip binary frames', () => {
    const originalFrame = {
      mode: 'list',
      entity: 'test/list',
      op: 'snapshot',
      data: [
        { key: '1', data: { id: 1, name: 'Test Entity' } },
        { key: '2', data: { id: 2, name: 'Another Entity' } },
      ],
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
        accounts: [{ name: 'signer', isSigner: true, isWritable: true, category: 'signer' }],
        errors,
      });

    const stack = {
      name: 'demo',
      url: 'wss://example.invalid',
      views: {},
      instructions: {
        ore: { close: handler('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv') },
        entropy: { close: handler('3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X') },
      },
    } as const;

    // autoReconnect: false keeps the client fully offline.
    return Arete.connect(stack, { autoReconnect: false });
  }

  it('mirrors per-program nesting and builds through the nested path', async () => {
    const client = await makeClient();
    const wallet = {
      publicKey: SIGNER,
      async signAndSend() {
        throw new Error('not used');
      },
    };

    const ix = client.instructions.ore.close.build({}, { wallet });
    expect(ix.programId).toBe('oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv');
    expect(ix.keys[0]!.pubkey).toBe(SIGNER);

    const ix2 = client.instructions.entropy.close.build({}, { wallet });
    expect(ix2.programId).toBe('3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X');
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

    const ix = client.instructions.ore.close.build({}, { wallet });
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

    const ix = client.instructions.ore.close.build({}, { wallet });
    await expect(
      client.transaction([ix], {
        wallet,
        errors: [{ code: 6000, name: 'RightName', msg: 'from override' }],
      })
    ).rejects.toMatchObject({
      programError: { code: 6000, name: 'RightName' },
    });
  });
});
