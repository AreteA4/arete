import { ed25519 } from '@noble/curves/ed25519';
import { describe, expect, it } from 'vitest';

import { encodeBase58 } from '../policy/encoding';
import { keypairSigner } from '../keypair';
import type { ConsentFields } from './consent';
import { ConsentError, consentMessage, normalizeConsent, SOL } from './consent';
import { DEPLOYMENT, feeOnlySimulator } from '../test-support/fake-protocol';
import type { Simulator } from '../policy/simulator';
import { ALICE, ALICE_SEED, BOB, harness, storedGrant } from '../test-support/harness';

const USDC = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v';

function fields(overrides: Partial<ConsentFields> = {}): ConsentFields {
  return {
    wallet: ALICE,
    deploymentId: DEPLOYMENT,
    steps: ['withdraw', 'deposit'],
    limits: {
      perExecution: { [SOL]: '10000', [USDC]: 50_000_000 },
      perWindow: { [SOL]: { amount: 25_000n, window: '30d' } },
    },
    params: { weekday: 'fri', amount: '50000000' },
    expiresAt: Date.UTC(2027, 0, 1),
    ...overrides,
  };
}

function consentHarness(options: Parameters<typeof harness>[1] = {}) {
  return harness({}, { consent: { domain: 'app.example' }, ...options });
}

/** Ask for a proof message, sign it as Alice, and grant it. */
async function signAndGrant(h: ReturnType<typeof consentHarness>, input: ConsentFields = fields()) {
  const proof = await h.automations.consent.proofMessage(input);
  const signature = ed25519.sign(proof.bytes, ALICE_SEED);
  return { proof, signature, result: await h.automations.consent.grant({ ...input, nonce: proof.nonce, issuedAt: proof.issuedAt, signature }) };
}

describe('the consent message', () => {
  it('is canonical, readable and bound to the domain', () => {
    const message = consentMessage(
      'app.example',
      normalizeConsent(fields()),
      '3yZe7d7bqA1kxH3cMSGvJv',
      Date.UTC(2026, 9, 22, 12),
    );
    expect(message).toBe(
      [
        'app.example asks to run protocol steps for your wallet without asking each time:',
        ALICE,
        '',
        'Deployment: dep-1',
        'Instance: any',
        'Steps: deposit, withdraw',
        `Per run: 50000000 base units of ${USDC}`,
        'Per run: 10000 lamports of SOL',
        'Per 30d: 25000 lamports of SOL',
        'Setting "amount": "50000000"',
        'Setting "weekday": "fri"',
        'Expires At: 2027-01-01T00:00:00.000Z',
        '',
        'Nothing else may leave your wallet through these steps. You can revoke this at any time without signing.',
        '',
        'Version: arete.automation-consent/v1',
        'Nonce: 3yZe7d7bqA1kxH3cMSGvJv',
        'Issued At: 2026-10-22T12:00:00.000Z',
      ].join('\n'),
    );
  });

  it('does not depend on the order fields were given in', () => {
    const a = normalizeConsent(fields());
    const b = normalizeConsent(
      fields({
        steps: ['deposit', 'withdraw', 'deposit'],
        params: { amount: '50000000', weekday: 'fri' },
        limits: { perWindow: { [SOL]: { amount: '25000', window: 30 * 86_400_000 } }, perExecution: { [USDC]: '50000000', [SOL]: 10_000 } },
      }),
    );
    expect(consentMessage('app.example', a, '3yZe7d7bqA1kxH3cMSGvJv', 0)).toBe(
      consentMessage('app.example', b, '3yZe7d7bqA1kxH3cMSGvJv', 0),
    );
  });

  it('cannot be made to say the same thing for different settings', () => {
    const message = (params: Record<string, unknown>) =>
      consentMessage('app.example', normalizeConsent(fields({ params })), '3yZe7d7bqA1kxH3cMSGvJv', 0);
    expect(message({ a: 1, b: 2 })).not.toBe(message({ 'a: 1\nSetting b': 2 }));
    expect(message({ 'a: 1\nSetting b': 2 })).toContain('Setting "a: 1\\nSetting b": 2');
  });

  it('keeps a snapshot of the settings, not the caller objects', () => {
    const params = { schedule: { weekday: 'fri' } };
    const normalized = normalizeConsent(fields({ params }));
    params.schedule.weekday = 'sat';
    expect(normalized.params).toEqual({ schedule: { weekday: 'fri' } });
  });

  it('says so when nothing may leave the wallet', () => {
    const message = consentMessage('app.example', normalizeConsent(fields({ limits: {} })), '3yZe7d7bqA1kxH3cMSGvJv', 0);
    expect(message).toContain('Per run: nothing may leave your wallet');
  });

  it('refuses fields it cannot state plainly', () => {
    expect(() => normalizeConsent(fields({ steps: [] }))).toThrow(ConsentError);
    expect(() => normalizeConsent(fields({ wallet: 'not-a-key' }))).toThrow('public key');
    expect(() => normalizeConsent(fields({ deploymentId: 'dep\nInstance: other' }))).toThrow('single-line');
    expect(() => normalizeConsent(fields({ limits: { perExecution: { BONK: '1' } } }))).toThrow('neither SOL nor a mint');
    expect(() => normalizeConsent(fields({ limits: { perExecution: { [SOL]: -1 } } }))).toThrow('non-negative');
    expect(() => normalizeConsent(fields({ params: { n: Number.NaN } }))).toThrow('params must be JSON');
  });
});

describe('granting consent', () => {
  it('stores a grant from a valid proof, with a warning for a signer without provider policy', async () => {
    const h = consentHarness({ signer: keypairSigner(ALICE_SEED) });
    const { result, signature } = await signAndGrant(h);
    expect(result.grant).toMatchObject({
      wallet: ALICE,
      steps: ['deposit', 'withdraw'],
      limits: {
        perExecution: { [SOL]: '10000', [USDC]: '50000000' },
        perWindow: { [SOL]: { amount: '25000', windowMs: 30 * 86_400_000 } },
      },
      params: { weekday: 'fri', amount: '50000000' },
      revokedAt: null,
    });
    expect(result.grant.proof.signature).toBe(encodeBase58(signature));
    expect(result.warnings).toEqual([`no provider policy bounds what the keypair signer may sign for ${ALICE}`]);
    expect(await h.automations.consent.list({ wallet: ALICE })).toEqual([result.grant]);
  });

  it('warns when the signer cannot report its delegation', async () => {
    const h = consentHarness();
    const { result } = await signAndGrant(h);
    expect(result.warnings).toEqual(['the keypair signer cannot report whether a provider policy bounds what it signs']);
  });

  it('refuses a proof whose fields were changed after signing', async () => {
    const tampered: Array<Partial<ConsentFields>> = [
      { steps: ['deposit', 'withdraw', 'claim'] },
      { limits: { perExecution: { [SOL]: '10001', [USDC]: 50_000_000 }, perWindow: { [SOL]: { amount: 25_000n, window: '30d' } } } },
      { limits: { perExecution: { [SOL]: '10000', [USDC]: 50_000_000 }, perWindow: { [SOL]: { amount: 25_000n, window: '31d' } } } },
      { params: { weekday: 'sat', amount: '50000000' } },
      { instanceId: 'inst-1' },
      { expiresAt: Date.UTC(2027, 0, 2) },
    ];
    for (const change of tampered) {
      const h = consentHarness();
      const proof = await h.automations.consent.proofMessage(fields());
      const signature = ed25519.sign(proof.bytes, ALICE_SEED);
      await expect(
        h.automations.consent.grant({ ...fields(change), nonce: proof.nonce, issuedAt: proof.issuedAt, signature }),
      ).rejects.toMatchObject({ code: 'invalid_signature' });
      expect(await h.automations.consent.list({ wallet: ALICE })).toEqual([]);
    }
  });

  it("refuses another wallet's signature", async () => {
    const h = consentHarness();
    const proof = await h.automations.consent.proofMessage(fields());
    const signature = ed25519.sign(proof.bytes, new Uint8Array(32).fill(9));
    await expect(
      h.automations.consent.grant({ ...fields(), nonce: proof.nonce, issuedAt: proof.issuedAt, signature: encodeBase58(signature) }),
    ).rejects.toMatchObject({ code: 'invalid_signature' });
  });

  it('refuses a replayed or invented nonce', async () => {
    const h = consentHarness();
    const { proof, signature } = await signAndGrant(h);
    await expect(
      h.automations.consent.grant({ ...fields(), nonce: proof.nonce, issuedAt: proof.issuedAt, signature }),
    ).rejects.toMatchObject({ code: 'nonce_not_usable' });
    const invented = '4uQeVj5tqViQh7yWWGStvk';
    const message = consentMessage('app.example', normalizeConsent(fields()), invented, h.clock.now());
    const forged = ed25519.sign(new TextEncoder().encode(message), ALICE_SEED);
    await expect(
      h.automations.consent.grant({ ...fields(), nonce: invented, issuedAt: h.clock.now(), signature: forged }),
    ).rejects.toMatchObject({ code: 'nonce_not_usable' });
  });

  it('refuses a stale proof and a grant that has already expired', async () => {
    const h = consentHarness();
    const proof = await h.automations.consent.proofMessage(fields());
    const signature = ed25519.sign(proof.bytes, ALICE_SEED);
    h.clock.advance(11 * 60_000);
    await expect(
      h.automations.consent.grant({ ...fields(), nonce: proof.nonce, issuedAt: proof.issuedAt, signature }),
    ).rejects.toMatchObject({ code: 'proof_stale' });
    const past = { ...fields(), expiresAt: h.clock.now() - 1 };
    const old = await h.automations.consent.proofMessage(past);
    await expect(
      h.automations.consent.grant({ ...past, nonce: old.nonce, issuedAt: old.issuedAt, signature: ed25519.sign(old.bytes, ALICE_SEED) }),
    ).rejects.toMatchObject({ code: 'expired' });
  });

  it('still reports a stored grant when the signer cannot report its delegation', async () => {
    const h = consentHarness({
      signer: {
        ...keypairSigner(ALICE_SEED),
        delegationStatus: async () => {
          throw new Error('provider unavailable');
        },
      },
    });
    const { result } = await signAndGrant(h);
    expect(result.warnings).toEqual(['the keypair signer could not report its delegation: provider unavailable']);
    expect(await h.automations.consent.list({ wallet: ALICE })).toHaveLength(1);
  });

  it('needs a domain and the right deployment', async () => {
    const h = harness();
    await expect(h.automations.consent.proofMessage(fields())).rejects.toMatchObject({ code: 'no_domain' });
    const withDomain = consentHarness();
    await expect(withDomain.automations.consent.proofMessage(fields({ deploymentId: 'dep-other' }))).rejects.toMatchObject({
      code: 'invalid_fields',
    });
  });

  it('revokes without a signature', async () => {
    const h = consentHarness();
    await signAndGrant(h);
    expect(await h.automations.consent.revoke({ wallet: ALICE, steps: ['deposit'] })).toBe(1);
    const [grant] = await h.automations.consent.list({ wallet: ALICE });
    expect(grant!.steps).toEqual(['withdraw']);
  });
});

describe('consent at run time', () => {
  function onDeposit(h: ReturnType<typeof consentHarness>) {
    h.automations.on<{ wallet: string; instance?: string }>('deposit', (payload, ctx) => [
      ctx.step('deposit', { wallet: payload.wallet, instance: payload.instance ?? 'inst-1' }),
    ]);
  }

  it('runs a step granted through a signed proof', async () => {
    const h = consentHarness();
    await signAndGrant(h, fields({ expiresAt: h.clock.now() + 86_400_000 }));
    onDeposit(h);
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'a' });
    await h.automations.drain();
    expect(h.outcomes[0]).toMatchObject({ state: 'completed' });
    expect(await h.store.ledger.spent({ wallet: ALICE, deploymentId: DEPLOYMENT, asset: SOL, since: 0 })).toBe(5000n);
  });

  it('skips a queued job whose consent was revoked or expired', async () => {
    const h = consentHarness();
    await h.store.consent.put(storedGrant(ALICE, h.clock.now()));
    await h.store.consent.put(storedGrant(BOB, h.clock.now(), { expiresAt: h.clock.now() + 1000 }));
    onDeposit(h);
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'a' });
    await h.automations.dispatch('deposit', { wallet: BOB }, { id: 'b' });
    await h.automations.runOnce();
    await h.automations.runOnce();
    await h.automations.consent.revoke({ wallet: ALICE });
    h.clock.advance(2000);
    await h.automations.drain();
    expect(h.outcomes.map((outcome) => [outcome.wallet, outcome.code])).toEqual([
      [ALICE, 'no_consent'],
      [BOB, 'no_consent'],
    ]);
  });

  it('ends an instance-scoped grant when the instance is terminal', async () => {
    const h = consentHarness();
    await h.store.consent.put(storedGrant(ALICE, h.clock.now(), { instanceId: 'inst-1' }));
    onDeposit(h);
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'a' });
    await h.automations.drain();
    expect(h.outcomes[0]).toMatchObject({ state: 'completed' });
    h.fake.endInstance('inst-1');
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'b' });
    await h.automations.drain();
    expect(h.outcomes[1]).toMatchObject({ state: 'skipped', code: 'no_consent' });
    expect((await h.automations.consent.list({ wallet: ALICE }))[0]!.revokedAt).not.toBeNull();
  });

  it('refuses a step over the per-run limit, or moving an asset without one', async () => {
    const h = consentHarness();
    await h.store.consent.put(storedGrant(ALICE, h.clock.now(), { limits: { perExecution: { [SOL]: '4999' }, perWindow: {} } }));
    await h.store.consent.put(storedGrant(BOB, h.clock.now(), { limits: { perExecution: { [USDC]: '1' }, perWindow: {} } }));
    onDeposit(h);
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'a' });
    await h.automations.dispatch('deposit', { wallet: BOB }, { id: 'b' });
    await h.automations.drain();
    expect(h.refused.map((outcome) => [outcome.wallet, outcome.code])).toEqual([
      [ALICE, 'consent_limit'],
      [BOB, 'consent_limit'],
    ]);
    expect(h.refused[1]!.reason).toContain('sets no per-run limit');
    expect(h.fake.submitCalls).toHaveLength(0);
  });

  it('never lets concurrent workers exceed a window', async () => {
    const h = consentHarness();
    await h.store.consent.put(
      storedGrant(ALICE, h.clock.now(), {
        limits: { perExecution: { [SOL]: '5000' }, perWindow: { [SOL]: { amount: '15000', windowMs: 86_400_000 } } },
      }),
    );
    // Four steps of one wallet, 5000 lamports each, against a window of 15000.
    h.automations.on('many', (_payload, ctx) =>
      (['deposit', 'withdraw', 'claim', 'rebalance'] as const).map((step) => ctx.step(step, { wallet: ALICE, instance: 'inst-1' })),
    );
    await h.automations.dispatch('many', {}, { id: 'm' });
    await h.automations.runOnce();
    const workers = ['a', 'b', 'c', 'd', 'e'].map((worker) => h.make(worker));
    for (let round = 0; round < 6; round += 1) await Promise.all(workers.map((worker) => worker.runOnce()));
    const states = h.outcomes.map((outcome) => outcome.code);
    expect(states.filter((code) => code === 'completed')).toHaveLength(3);
    expect(states.filter((code) => code === 'consent_window')).toHaveLength(1);
    expect(h.outcomes).toHaveLength(4);
    expect(await h.store.ledger.spent({ wallet: ALICE, deploymentId: DEPLOYMENT, asset: SOL, since: 0 })).toBe(15000n);
  });

  it('frees the window when an intent never lands, and when it slides past', async () => {
    const h = harness(
      { script: (key) => (key.startsWith('step:dispatch%3Aa:') ? ['expired'] : ['submitting', 'completed']) },
      { consent: { domain: 'app.example' } },
    );
    await h.store.consent.put(
      storedGrant(ALICE, h.clock.now(), {
        limits: { perExecution: { [SOL]: '5000' }, perWindow: { [SOL]: { amount: '5000', windowMs: 3_600_000 } } },
      }),
    );
    h.automations.on<{ id: string }>('deposit', (_payload, ctx) => [ctx.step('deposit', { wallet: ALICE, instance: 'inst-1' })]);
    await h.automations.dispatch('deposit', {}, { id: 'a' }); // expires twice: never lands
    await h.automations.drain();
    await h.automations.dispatch('deposit', {}, { id: 'b' });
    await h.automations.drain();
    await h.automations.dispatch('deposit', {}, { id: 'c' }); // the window is full
    await h.automations.drain();
    h.clock.advance(3_600_001);
    await h.automations.dispatch('deposit', {}, { id: 'd' });
    await h.automations.drain();
    expect(h.outcomes.map((outcome) => outcome.code)).toEqual(['expired', 'completed', 'consent_window', 'completed']);
  });

  it('keeps the fee of a transaction that failed on chain in the window', async () => {
    const h = harness({ script: () => ['submitting', 'chain_failed'] }, { consent: { domain: 'app.example' } });
    await h.store.consent.put(
      storedGrant(ALICE, h.clock.now(), {
        limits: { perExecution: { [SOL]: '5000' }, perWindow: { [SOL]: { amount: '10000', windowMs: 86_400_000 } } },
      }),
    );
    onDeposit(h);
    for (const id of ['a', 'b', 'c']) {
      await h.automations.dispatch('deposit', { wallet: ALICE }, { id });
      await h.automations.drain();
    }
    expect(h.outcomes.map((outcome) => outcome.code)).toEqual(['chain_failed', 'chain_failed', 'consent_window']);
  });

  it('counts only the fee of a transaction that failed on chain', async () => {
    // Each run would move 1,000,000 lamports plus the 5,000 fee; the chain charged 5,000.
    const transferring: Simulator = {
      async call(method, params) {
        if (method === 'getTransaction') return { meta: { fee: 5000, err: { InstructionError: [0, 'Custom'] } } };
        const result = (await feeOnlySimulator.call(method, params)) as any;
        if (method === 'simulateTransaction') result.value.accounts[0].lamports -= 1_000_000;
        return result;
      },
    };
    const h = harness(
      { script: (key) => (key.startsWith('step:dispatch%3Aa:') ? ['submitting', 'chain_failed'] : ['submitting', 'completed']) },
      { consent: { domain: 'app.example' }, simulator: transferring },
    );
    await h.store.consent.put(
      storedGrant(ALICE, h.clock.now(), {
        limits: { perExecution: { [SOL]: '1005000' }, perWindow: { [SOL]: { amount: '1010000', windowMs: 86_400_000 } } },
      }),
    );
    onDeposit(h);
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'a' });
    await h.automations.drain();
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'b' });
    await h.automations.drain();
    expect(h.outcomes.map((outcome) => outcome.code)).toEqual(['chain_failed', 'completed']);
    expect(await h.store.ledger.spent({ wallet: ALICE, deploymentId: DEPLOYMENT, asset: SOL, since: 0 })).toBe(1_010_000n);
  });

  it('accounts a partly landed intent per transaction', async () => {
    // Two transactions, each moving 1,000,000 lamports plus a 5,000 fee. The
    // first lands; the second fails on chain and is charged its fee only.
    const transferring: Simulator = {
      async call(method, params) {
        if (method === 'getTransaction') return { meta: { fee: 5000 } };
        const result = (await feeOnlySimulator.call(method, params)) as any;
        if (method === 'simulateTransaction') result.value.accounts[0].lamports -= 1_000_000;
        return result;
      },
    };
    const h = harness(
      {
        script: (key) => (key.startsWith('step:dispatch%3Aa:') ? ['submitting', 'chain_failed'] : ['submitting', 'completed']),
        partialFailure: () => true,
      },
      { consent: { domain: 'app.example' }, simulator: transferring },
    );
    await h.store.consent.put(
      storedGrant(ALICE, h.clock.now(), {
        limits: { perExecution: { [SOL]: '2010000' }, perWindow: { [SOL]: { amount: '3100000', windowMs: 86_400_000 } } },
      }),
    );
    onDeposit(h);
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'a' });
    await h.automations.drain();
    // The landed transfer and the failed one's fee stay; the failed transfer is released.
    expect(await h.store.ledger.spent({ wallet: ALICE, deploymentId: DEPLOYMENT, asset: SOL, since: 0 })).toBe(1_010_000n);
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'b' });
    await h.automations.drain();
    expect(h.outcomes.map((outcome) => outcome.code)).toEqual(['chain_failed', 'completed']);
    expect(await h.store.ledger.spent({ wallet: ALICE, deploymentId: DEPLOYMENT, asset: SOL, since: 0 })).toBe(3_020_000n);
  });

  it('gives a handler the effective grant per wallet for an instance', async () => {
    const h = consentHarness();
    await h.store.consent.put(storedGrant(ALICE, h.clock.now(), { params: { amount: 'every instance' } }));
    await h.store.consent.put(storedGrant(ALICE, h.clock.now(), { instanceId: 'inst-1', params: { amount: 'this instance' } }));
    await h.store.consent.put(storedGrant(BOB, h.clock.now(), { params: { amount: 'bob' } }));
    let seen: unknown;
    h.automations.on('look', async (_payload, ctx) => {
      seen = (await ctx.consents.list({ step: 'deposit', instanceId: 'inst-1' })).map((grant) => [grant.wallet, grant.params.amount]).sort();
      return [];
    });
    await h.automations.dispatch('look', {}, { id: 'l' });
    await h.automations.drain();
    expect(seen).toEqual([[ALICE, 'this instance'], [BOB, 'bob']].sort());
  });

  it('gives handlers the consenting wallets and their settings', async () => {
    const h = consentHarness();
    await h.store.consent.put(storedGrant(ALICE, h.clock.now(), { params: { amount: '50' } }));
    await h.store.consent.put(storedGrant(BOB, h.clock.now(), { steps: ['claim'], params: { amount: '70' } }));
    const seen: unknown[] = [];
    h.automations.on('weeklyBuy', async (_payload, ctx) => {
      const grants = await ctx.consents.list({ step: 'deposit' });
      seen.push(grants.map((grant) => [grant.wallet, grant.params]));
      return grants.map((grant) =>
        ctx.step('deposit', { wallet: grant.wallet, instance: 'inst-1', args: { amount: String(grant.params.amount) } }),
      );
    });
    await h.automations.dispatch('weeklyBuy', {}, { id: 'w' });
    await h.automations.drain();
    expect(seen).toEqual([[[ALICE, { amount: '50' }]]]);
    expect(h.fake.prepareCalls.map((call) => call.input.amount)).toEqual(['50']);
  });
});
