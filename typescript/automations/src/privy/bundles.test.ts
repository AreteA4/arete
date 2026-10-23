// Runs against the built package (CI builds before testing; skipped if
// dist/ is missing). Each entry point is its own bundle with its own copy of
// SignerRefusal, which source imports cannot show.

import { existsSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { fakeClock, fakeProtocol, feeOnlySimulator, PROGRAM } from '../test-support/fake-protocol';
import { ALICE, storedGrant } from '../test-support/harness';

const index = new URL('../../dist/index.js', import.meta.url);
const privy = new URL('../../dist/privy.js', import.meta.url);
const built = existsSync(index) && existsSync(privy);

describe.skipIf(!built)('the built entry points', () => {
  it('turns a refusal from the Privy bundle into refused(provider_policy)', async () => {
    const automationsModule = await import(index.href);
    const privyModule = await import(privy.href);
    const fake = fakeProtocol();
    const clock = fakeClock();
    const store = automationsModule.memoryStore();
    const outcomes: Array<{ state: string; code: string }> = [];
    const signer = privyModule.privySigner({
      privy: {
        wallets: () => ({
          solana: () => ({
            signTransaction: async () => {
              throw Object.assign(new Error('Policy violation'), { status: 403 });
            },
          }),
        }),
      },
      walletIdFor: async () => 'w-alice',
      authorizationPrivateKey: 'key',
    });
    const automations = automationsModule.createAutomations({
      client: fake.client,
      signer,
      policy: automationsModule.definePolicy({
        deployments: ['dep-1'],
        clusters: ['dev'],
        programs: [PROGRAM],
        maxSolPerTransaction: '0.01',
      }),
      simulator: feeOnlySimulator,
      store,
      cluster: 'dev',
      clock,
      onOutcome: (outcome: { state: string; code: string }) => void outcomes.push(outcome),
    });
    await store.consent.put(storedGrant(ALICE, clock.now()));
    automations.on('deposit', (_payload: unknown, ctx: any) => [ctx.step('deposit', { wallet: ALICE, instance: 'inst-1' })]);
    await automations.dispatch('deposit', {}, { id: 'a' });
    await automations.drain();
    expect(outcomes).toEqual([expect.objectContaining({ state: 'refused', code: 'provider_policy' })]);
  });
});
