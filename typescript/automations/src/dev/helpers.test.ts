import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { ed25519 } from '@noble/curves/ed25519';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { devSimulator, devWalletsSigner, fileMemoryStore, grantDevConsent, routingSigner } from '../dev';
import { decodeBase58, encodeBase58 } from '../policy/encoding';
import type { ProtocolTransactionFormat } from '@usearete/sdk';

import type { ProtocolSigner } from '../signer/signer';

const devClient = { deploymentId: 'dep', definition: { deployment: { environment: 'dev' } } } as never;
const mainnetClient = { deploymentId: 'dep-main', definition: { deployment: {} } } as never;

function project() {
  const root = mkdtempSync(join(tmpdir(), 'dev-helpers-'));
  const wallets = join(root, '.arete', 'dev', 'wallets');
  mkdirSync(wallets, { recursive: true });
  const seed = new Uint8Array(32).fill(9);
  const publicKey = ed25519.getPublicKey(seed);
  writeFileSync(join(wallets, 'alice.json'), JSON.stringify([...seed, ...publicKey]));
  return { root, wallets, alice: encodeBase58(publicKey) };
}

function fakeSigner(kind: string, wallets: string[], formats: ProtocolTransactionFormat[] = ['v1', 'v0']): ProtocolSigner {
  return {
    kind,
    supportedTransactionFormats: formats,
    canSign: async (wallet) => wallets.includes(wallet),
    signTransactions: async () => [kind],
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('@usearete/automations/dev', () => {
  it('refuses a client that targets production, in every helper', () => {
    const { root, wallets } = project();
    expect(() => devWalletsSigner(mainnetClient, wallets)).toThrow(/devWalletsSigner runs against a dev environment only/);
    expect(() => routingSigner(mainnetClient, [fakeSigner('a', [])])).toThrow(/routingSigner runs against a dev environment only/);
    expect(() => fileMemoryStore(mainnetClient, join(root, 'store.bin'))).toThrow(/fileMemoryStore runs against a dev environment only/);
    expect(() => devSimulator(mainnetClient, { root })).toThrow(/devSimulator runs against a dev environment only/);
  });

  it('signs consent messages as a dev wallet, found by name or address', () => {
    const { wallets, alice } = project();
    const signer = devWalletsSigner(devClient, wallets);
    expect(signer.wallets()).toEqual([{ name: 'alice', address: alice }]);
    expect(signer.resolve('alice')).toBe(alice);
    expect(signer.resolve('bob')).toBeUndefined();
    const message = new TextEncoder().encode('consent');
    expect(ed25519.verify(signer.signMessage('alice', message), message, decodeBase58(alice)!)).toBe(true);
    expect(() => signer.signMessage('bob', message)).toThrow(/not a dev wallet/);
  });

  it('grants a consent the dev wallet signed', async () => {
    const { wallets, alice } = project();
    const signer = devWalletsSigner(devClient, wallets);
    const bytes = new TextEncoder().encode('opt in');
    const granted: Record<string, unknown>[] = [];
    const automations = {
      consent: {
        proofMessage: async () => ({ message: 'opt in', bytes, nonce: 'n1', issuedAt: 1 }),
        grant: async (input: Record<string, unknown>) => {
          granted.push(input);
          return { grant: {} as never, warnings: [] };
        },
      },
    };
    await grantDevConsent(automations as never, signer, { wallet: 'alice', deploymentId: 'dep', steps: ['refund'], limits: {}, expiresAt: 2 });
    expect(granted[0]).toMatchObject({ wallet: alice, nonce: 'n1', issuedAt: 1 });
    expect(ed25519.verify(granted[0]!.signature as Uint8Array, bytes, decodeBase58(alice)!)).toBe(true);
    await expect(grantDevConsent(automations as never, signer, { wallet: 'bob', deploymentId: 'dep', steps: [], limits: {}, expiresAt: 2 })).rejects.toThrow(/not a dev wallet/);
  });

  it('finds the dev wallets from the working directory', async () => {
    const { root, alice } = project();
    const cwd = vi.spyOn(process, 'cwd').mockReturnValue(join(root, '.arete'));
    try {
      expect(await devWalletsSigner(devClient).canSign(alice)).toBe(true);
    } finally {
      cwd.mockRestore();
    }
  });

  it('routes each wallet to the first signer that can sign for it', async () => {
    const routed = routingSigner(devClient, [fakeSigner('dev', ['alice']), fakeSigner('privy', ['alice', 'bob'], ['v0'])]);
    expect(routed.supportedTransactionFormats).toEqual(['v0']);
    expect(await routed.signTransactions({ wallet: 'alice' } as never)).toEqual(['dev']);
    expect(await routed.signTransactions({ wallet: 'bob' } as never)).toEqual(['privy']);
    expect(await routed.canSign('carol')).toBe(false);
    await expect(routed.signTransactions({ wallet: 'carol' } as never)).rejects.toThrow(/no signer can sign for carol/);
    expect(() => routingSigner(devClient, [fakeSigner('a', [], ['v1']), fakeSigner('b', [], ['v0'])])).toThrow(/share no transaction format/);
  });

  it('keeps the store across restarts and moves another deployment’s state aside', async () => {
    const { root } = project();
    const path = join(root, 'store.bin');
    const first = fileMemoryStore(devClient, path, { saveIntervalMs: 0 });
    await first.bindDeployment('dep');
    first.close();
    expect(existsSync(path)).toBe(true);
    await expect(fileMemoryStore(devClient, path).bindDeployment('dep')).resolves.not.toThrow();

    const archived: string[] = [];
    const other = { deploymentId: 'dep-2', definition: { deployment: { environment: 'dev' } } } as never;
    fileMemoryStore(other, path, { onArchive: (file) => archived.push(file) });
    expect(archived).toEqual([join(root, 'store-dep.bin')]);
    expect(existsSync(path)).toBe(false);
  });

  it('reports a failing periodic save instead of throwing from the timer', async () => {
    const { root } = project();
    const errors: unknown[] = [];
    const store = fileMemoryStore(devClient, join(root, 'missing', 'store.bin'), { saveIntervalMs: 10, onSaveError: (error) => errors.push(error) });
    await store.bindDeployment('dep');
    await new Promise((done) => setTimeout(done, 50));
    expect(errors.length).toBeGreaterThan(0);
    expect(() => store.close()).toThrow();
  });

  it('simulates on the RPC the dev session names, and checks the deployment', async () => {
    const { root } = project();
    const session = join(root, '.arete', 'dev', 'session.json');
    const simulator = devSimulator(devClient, { root });
    await expect(simulator.call('getSlot', [])).rejects.toThrow(/a4 dev up/);
    const calls: string[] = [];
    vi.stubGlobal('fetch', async (url: string) => {
      calls.push(url);
      return new Response(JSON.stringify({ jsonrpc: '2.0', id: 1, result: 42 }));
    });
    writeFileSync(session, JSON.stringify({ rpcUrl: 'http://127.0.0.1:9999', deploymentId: 'dep' }));
    expect(await devSimulator(devClient, { root }).call('getSlot', [])).toBe(42);
    expect(calls).toEqual(['http://127.0.0.1:9999']);
    writeFileSync(session, JSON.stringify({ rpcUrl: 'http://127.0.0.1:9999', deploymentId: 'dep-other' }));
    await expect(simulator.call('getSlot', [])).rejects.toThrow(/a4 dev sdk/);
  });
});
