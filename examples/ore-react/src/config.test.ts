import { describe, expect, it } from 'vitest';
import { resolveConfig } from './config';

describe('endpoint safety', () => {
  it('uses the generated production Arete pair without a public RPC by default', () => {
    const config = resolveConfig({});
    expect(config.areteWsUrl).toBe('wss://ore.stack.arete.run');
    expect(config.areteHttpUrl).toBe('https://ore.stack.arete.run');
    expect(config.transactionTransport).toBe('auto');
    expect(config.solanaRpcUrl).toBeUndefined();
    expect(config.publishableKey).toBeUndefined();
  });

  it('requires paired overrides unless derivation is explicit', () => {
    expect(() => resolveConfig({ VITE_ARETE_WS_URL: 'wss://example.test' })).toThrow(/requires/);
    expect(() => resolveConfig({ VITE_ARETE_HTTP_URL: 'https://example.test' })).toThrow(/requires/);
    expect(resolveConfig({
      VITE_ARETE_WS_URL: 'wss://example.test',
      VITE_DERIVE_ARETE_HTTP: 'true',
    }).areteHttpUrl).toBe('https://example.test');
    expect(resolveConfig({
      VITE_ARETE_WS_URL: 'wss://stream.example.test',
      VITE_ARETE_HTTP_URL: 'https://read.example.test',
    }).areteHttpUrl).toBe('https://read.example.test');
  });

  it('rejects explicitly non-mainnet Solana endpoints', () => {
    const direct = { VITE_TRANSACTION_TRANSPORT: 'direct' } as const;
    expect(() => resolveConfig({ ...direct, VITE_SOLANA_RPC_URL: 'https://api.devnet.solana.com' })).toThrow(/mainnet/);
    expect(() => resolveConfig({ ...direct, VITE_SOLANA_RPC_URL: 'http://127.0.0.1:8899' })).toThrow(/mainnet/);
    expect(() => resolveConfig({ ...direct, VITE_SOLANA_RPC_URL: 'https://rpc.example.com?cluster=testnet' })).toThrow(/mainnet/);
  });

  it('requires an explicit RPC only in direct mode', () => {
    expect(() => resolveConfig({ VITE_TRANSACTION_TRANSPORT: 'direct' })).toThrow(/required/);
    expect(resolveConfig({
      VITE_TRANSACTION_TRANSPORT: 'direct',
      VITE_SOLANA_RPC_URL: 'https://mainnet.rpc.example.com',
    }).solanaRpcUrl).toBe('https://mainnet.rpc.example.com');
  });
});
