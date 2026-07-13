import type { AreteConfig } from './types';

import { createClientCacheKey } from './client-key';
import { initializeConnectedClient, syncClientWallets } from './wallet-sync';

describe('provider wallet helpers', () => {
  const wallet = {
    publicKey: 'wallet-111',
    async signAndSend() {
      return { signature: 'sig' };
    },
  } satisfies NonNullable<AreteConfig['wallet']>;

  const stack = {
    name: 'ore-stream',
    endpoints: {
      ws: 'wss://ore.example',
      http: 'https://ore.example',
    },
    views: {},
  } as const;

  it('builds stable client cache keys for the same stack and options', () => {
    expect(createClientCacheKey(stack)).toBe(createClientCacheKey(stack));
    expect(
      createClientCacheKey(stack, { url: 'ws://localhost:7777', httpUrl: 'http://localhost:7777' })
    ).toBe(
      createClientCacheKey(stack, { url: 'ws://localhost:7777', httpUrl: 'http://localhost:7777' })
    );
  });

  it('distinguishes cache keys by transport and attached program identity', () => {
    const programsA = { ore: { name: 'ore' } };
    const programsB = { ore: { name: 'ore' } };

    expect(createClientCacheKey(stack, { transport: 'http' })).not.toBe(createClientCacheKey(stack));
    expect(createClientCacheKey(stack, { programs: programsA as never })).not.toBe(
      createClientCacheKey(stack, { programs: programsB as never })
    );
  });

  it('syncs the latest wallet across cached clients', () => {
    const first = { setWallet: jest.fn() };
    const second = { setWallet: jest.fn() };

    syncClientWallets(
      [
        { client: first as never, disconnect: jest.fn() },
        { client: second as never, disconnect: jest.fn() },
      ],
      wallet
    );

    expect(first.setWallet).toHaveBeenCalledWith(wallet);
    expect(second.setWallet).toHaveBeenCalledWith(wallet);
  });

  it('applies the latest wallet when a client finishes connecting', () => {
    const adapter = { setConnectionState: jest.fn() };
    let onConnectionStateChange:
      | ((state: 'connected' | 'error', error?: Error) => void)
      | undefined;
    const client = {
      connectionState: 'connected',
      setWallet: jest.fn(),
      onConnectionStateChange: jest.fn((callback) => {
        onConnectionStateChange = callback;
        return jest.fn();
      }),
    };

    initializeConnectedClient(client as never, adapter as never, wallet);

    expect(client.setWallet).toHaveBeenCalledWith(wallet);
    expect(adapter.setConnectionState).toHaveBeenCalledWith('connected');

    const error = new Error('boom');
    onConnectionStateChange?.('error', error);
    expect(adapter.setConnectionState).toHaveBeenLastCalledWith('error');
  });
});
