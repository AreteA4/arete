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
