import { describe, expect, it, vi } from 'vitest';
import { Keypair } from '@solana/web3.js';
import { useWallet } from '@solana/wallet-adapter-react';

import { useSolanaWalletAdapter } from './react';

vi.mock('react', () => ({
  useMemo: vi.fn((factory: () => unknown) => factory()),
}));

vi.mock('@solana/wallet-adapter-react', () => ({
  useWallet: vi.fn(),
}));

const mockUseWallet = vi.mocked(useWallet);

describe('useSolanaWalletAdapter', () => {
  it('returns undefined when no wallet is connected', () => {
    mockUseWallet.mockReturnValue({ publicKey: null } as never);

    expect(useSolanaWalletAdapter()).toBeUndefined();
  });

  it('returns undefined when the wallet cannot sign transactions', () => {
    mockUseWallet.mockReturnValue({
      publicKey: Keypair.generate().publicKey,
      signTransaction: undefined,
    } as never);

    expect(useSolanaWalletAdapter()).toBeUndefined();
  });

  it('adapts a connected wallet into an Arete wallet adapter', () => {
    const publicKey = Keypair.generate().publicKey;
    const signTransaction = vi.fn(async (tx: unknown) => tx);
    mockUseWallet.mockReturnValue({
      publicKey,
      signTransaction,
      wallet: { adapter: { supportedTransactionVersions: new Set([0, 'legacy']) } },
    } as never);

    const adapter = useSolanaWalletAdapter();

    expect(adapter).toBeDefined();
    expect(adapter?.publicKey).toBe(publicKey.toBase58());
    expect(typeof adapter?.signAndSend).toBe('function');
  });

  it('defaults missing supportedTransactionVersions to null', () => {
    const publicKey = Keypair.generate().publicKey;
    mockUseWallet.mockReturnValue({
      publicKey,
      signTransaction: vi.fn(async (tx: unknown) => tx),
      wallet: undefined,
    } as never);

    expect(() => useSolanaWalletAdapter()).not.toThrow();
  });
});
