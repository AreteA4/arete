import { ed25519 } from '@noble/curves/ed25519';
import { describe, expect, it } from 'vitest';

import { SOL } from '../consent/consent';
import { decodeTransaction } from '../policy/decode';
import { decodeBase64, encodeBase64 } from '../policy/encoding';
import { definePolicy } from '../policy/policy';
import { PROGRAM } from '../test-support/fake-protocol';
import { ALICE, ALICE_SEED, harness, storedGrant } from '../test-support/harness';
import { type PrivyClientLike, privyPolicyFor, privySigner } from './privy';

const USDC = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v';
const AUTH_KEY = 'wallet-auth:test-authorization-key';

type Behaviour = 'sign' | 'tamper' | 'deny' | 'fail';

/** A Privy client that holds Alice's key under wallet id `w-alice`. */
function fakePrivy(behaviour: Behaviour, policyIds: string[] = ['pol-1']) {
  const calls: Array<{ walletId: string; input: unknown }> = [];
  const client: PrivyClientLike = {
    wallets: () => ({
      solana: () => ({
        async signTransaction(walletId, input) {
          calls.push({ walletId, input });
          if (behaviour === 'deny') {
            throw Object.assign(new Error('Policy violation: action not allowed by policy'), { status: 403 });
          }
          if (behaviour === 'fail') throw Object.assign(new Error('upstream timeout'), { status: 503 });
          const bytes = decodeBase64(input.transaction)!;
          // The last instruction data byte (a v0 message ends with its lookup-table count).
          if (behaviour === 'tamper') bytes[bytes.length - 2] = bytes[bytes.length - 2]! ^ 1;
          const decoded = decodeTransaction(bytes);
          bytes.set(ed25519.sign(bytes.subarray(decoded.message.start, decoded.message.end), ALICE_SEED), decoded.signatureOffset);
          return { signedTransaction: encodeBase64(bytes) };
        },
      }),
      get: async () => ({ policy_ids: policyIds }),
    }),
  };
  return { client, calls };
}

function privyHarness(behaviour: Behaviour, policyIds?: string[]) {
  const privy = fakePrivy(behaviour, policyIds);
  const signer = privySigner({
    privy: privy.client,
    walletIdFor: async (wallet) => (wallet === ALICE ? 'w-alice' : null),
    authorizationPrivateKey: AUTH_KEY,
  });
  const h = harness({}, { signer, consent: { domain: 'app.example' } });
  h.automations.on<{ wallet: string }>('deposit', (payload, ctx) => [ctx.step('deposit', { wallet: payload.wallet, instance: 'inst-1' })]);
  return { ...h, privy, signer };
}

describe('privySigner', () => {
  it('signs exact bytes through Privy, as v0, authorized by the app key', async () => {
    const h = privyHarness('sign');
    await h.store.consent.put(storedGrant(ALICE, h.clock.now()));
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'a' });
    await h.automations.drain();
    expect(h.outcomes[0]).toMatchObject({ state: 'completed' });
    expect(h.fake.prepareCalls[0]!.options?.transactionFormat).toBe('v0');
    expect(h.privy.calls[0]).toMatchObject({
      walletId: 'w-alice',
      input: { authorization_context: { authorization_private_keys: [AUTH_KEY] } },
    });
  });

  it('refuses bytes Privy changed', async () => {
    const h = privyHarness('tamper');
    await h.store.consent.put(storedGrant(ALICE, h.clock.now()));
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'a' });
    await h.automations.drain();
    expect(h.refused[0]).toMatchObject({ code: 'signer_modified' });
    expect(h.fake.submitCalls).toHaveLength(0);
  });

  it('surfaces a Privy policy refusal as provider_policy', async () => {
    const h = privyHarness('deny');
    await h.store.consent.put(storedGrant(ALICE, h.clock.now()));
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'a' });
    await h.automations.drain();
    expect(h.refused[0]).toMatchObject({ state: 'refused', code: 'provider_policy' });
    expect(h.refused[0]!.reason).toContain('Policy violation');
  });

  it('retries a Privy outage instead of refusing', async () => {
    const h = privyHarness('fail');
    await h.store.consent.put(storedGrant(ALICE, h.clock.now()));
    await h.automations.dispatch('deposit', { wallet: ALICE }, { id: 'a' });
    await h.automations.drain();
    expect(h.outcomes).toHaveLength(0);
    expect((await h.stepJobs())[0]).toMatchObject({ state: 'queued', attempts: 1 });
    expect(String(h.errors[0])).toContain('upstream timeout');
  });

  it('refuses a wallet the app has no server access to', async () => {
    const h = privyHarness('sign');
    expect(await h.signer.canSign('Unknown11111111111111111111111111111111111')).toBe(false);
    expect(await h.signer.delegationStatus!(ALICE)).toEqual({ delegated: true, providerPolicy: true });
    expect(await h.signer.delegationStatus!('Unknown11111111111111111111111111111111111')).toMatchObject({ delegated: false });
  });

  it('warns at grant time when the wallet has no Privy policy', async () => {
    const h = privyHarness('sign', []);
    const fields = {
      wallet: ALICE,
      deploymentId: 'dep-1',
      steps: ['deposit'],
      limits: { perExecution: { [SOL]: '10000' } },
      expiresAt: h.clock.now() + 86_400_000,
    };
    const proof = await h.automations.consent.proofMessage(fields);
    const { warnings } = await h.automations.consent.grant({
      ...fields,
      nonce: proof.nonce,
      issuedAt: proof.issuedAt,
      signature: ed25519.sign(proof.bytes, ALICE_SEED),
    });
    expect(warnings).toEqual([`no provider policy bounds what the privy signer may sign for ${ALICE}`]);
  });
});

describe('privyPolicyFor', () => {
  const policy = definePolicy({
    deployments: ['dep-1'],
    clusters: ['mainnet'],
    programs: [PROGRAM],
    maxSolPerTransaction: '0.01',
    maxTokenAmounts: { [USDC]: '5000000000', [PROGRAM]: '1' },
  });
  const consent = {
    wallet: ALICE,
    expiresAt: Date.UTC(2027, 0, 1),
    limits: { perExecution: { [SOL]: '20000000', [USDC]: '50000000' }, perWindow: {} },
  };

  it('allows the programs, capped direct transfers and nothing else, until the consent expires', () => {
    const { policy: privy, unenforced } = privyPolicyFor(policy, consent);
    const until = { field_source: 'system', field: 'current_unix_timestamp', operator: 'lt', value: '1798761600' };
    expect(privy).toEqual({
      version: '1.0',
      name: `Arete automations for ${ALICE}`.slice(0, 50),
      chain_type: 'solana',
      rules: [
        { name: 'Never export the key', method: 'exportPrivateKey', conditions: [], action: 'DENY' },
        {
          name: 'Programs the steps may call',
          method: 'signTransaction',
          action: 'ALLOW',
          conditions: [
            {
              field_source: 'solana_program_instruction',
              field: 'programId',
              operator: 'in',
              value: [
                'ComputeBudget111111111111111111111111111111',
                'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr',
                'Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo',
                'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL',
                PROGRAM,
              ],
            },
            until,
          ],
        },
        {
          name: 'SOL transfers up to the per-run cap',
          method: 'signTransaction',
          action: 'ALLOW',
          conditions: [
            { field_source: 'solana_system_program_instruction', field: 'instructionName', operator: 'eq', value: 'Transfer' },
            // The smaller of the policy's 0.01 SOL and the consent's 0.02 SOL.
            { field_source: 'solana_system_program_instruction', field: 'Transfer.lamports', operator: 'lte', value: '10000000' },
            until,
          ],
        },
        {
          name: 'Token accounts closed back to the wallet',
          method: 'signTransaction',
          action: 'ALLOW',
          conditions: [
            { field_source: 'solana_token_program_instruction', field: 'instructionName', operator: 'eq', value: 'CloseAccount' },
            { field_source: 'solana_token_program_instruction', field: 'CloseAccount.destination', operator: 'eq', value: ALICE },
            until,
          ],
        },
        {
          name: 'Token accounts opened for the wallet',
          method: 'signTransaction',
          action: 'ALLOW',
          conditions: [
            { field_source: 'solana_token_program_instruction', field: 'instructionName', operator: 'eq', value: 'InitializeAccount3' },
            until,
          ],
        },
        {
          // Only USDC: the other mint has a policy cap but no consent limit, so it may not leave.
          name: `Transfers of ${USDC} up to the per-run cap`,
          method: 'signTransaction',
          action: 'ALLOW',
          conditions: [
            { field_source: 'solana_token_program_instruction', field: 'instructionName', operator: 'eq', value: 'TransferChecked' },
            { field_source: 'solana_token_program_instruction', field: 'TransferChecked.mint', operator: 'eq', value: USDC },
            { field_source: 'solana_token_program_instruction', field: 'TransferChecked.amount', operator: 'lte', value: '50000000' },
            until,
          ],
        },
      ],
    });
    expect(unenforced.join('\n')).toContain('Privy checks top-level instructions only');
    expect(unenforced.join('\n')).toContain('no cumulative limits');
    expect(unenforced.join('\n')).toContain('the transfer caps apply to each instruction');
  });

  it('allows no SOL transfer when the consent sets no SOL limit', () => {
    const { policy: privy } = privyPolicyFor(policy, { ...consent, limits: { perExecution: {}, perWindow: {} } });
    const transfer = privy.rules.find((rule) => rule.name === 'SOL transfers up to the per-run cap')!;
    expect(transfer.conditions).toContainEqual({
      field_source: 'solana_system_program_instruction',
      field: 'Transfer.lamports',
      operator: 'lte',
      value: '0',
    });
  });

  it('needs a programs allow-list', () => {
    const open = definePolicy({ deployments: ['d'], maxSolPerTransaction: '0.01' });
    expect(() => privyPolicyFor(open, consent)).toThrow('programs allow-list');
  });
});
