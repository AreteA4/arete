// The runtime test harness: two wallets, a fake protocol, a memory store and a fake clock.

import { keypairSigner } from '../keypair';
import { type AutomationOutcome, type AutomationsOptions, createAutomations } from '../runtime/automations';
import type { StepJob } from '../runtime/types';
import type { ProtocolSigner } from '../signer/signer';
import { memoryStore } from '../store/memory';
import type { AutomationStore, ConsentGrant } from '../store/store';
import {
  DEPLOYMENT,
  fakeClock,
  type FakeProtocolOptions,
  fakeProtocol,
  feeOnlySimulator,
  policy,
  walletOf,
  walletSeed,
} from './fake-protocol';

export const ALICE_SEED = walletSeed(1);
export const ALICE = walletOf(ALICE_SEED);
export const BOB_SEED = walletSeed(2);
export const BOB = walletOf(BOB_SEED);
export const SECRET = 'whsec_test';

/** A grant written straight to the store, as if granted with a proof. Each run costs a 5000-lamport fee. */
export function storedGrant(wallet: string, now: number, overrides: Partial<ConsentGrant> = {}): ConsentGrant {
  return {
    wallet,
    deploymentId: DEPLOYMENT,
    instanceId: null,
    steps: ['deposit', 'withdraw', 'rebalance', 'claim'],
    limits: { perExecution: { SOL: '10000' }, perWindow: {} },
    params: {},
    expiresAt: now + 86_400_000,
    grantedAt: now,
    revokedAt: null,
    proof: { message: '', signature: '', nonce: '', issuedAt: now },
    ...overrides,
  };
}

/** A signer for both test wallets. */
export function walletsSigner(): ProtocolSigner {
  const alice = keypairSigner(ALICE_SEED);
  const bob = keypairSigner(BOB_SEED);
  return {
    kind: 'keypair',
    supportedTransactionFormats: ['v1', 'v0'],
    canSign: async (wallet) => wallet === ALICE || wallet === BOB,
    signTransactions: (input) => (input.wallet === ALICE ? alice : bob).signTransactions(input),
  };
}

export function harness(
  protocol: FakeProtocolOptions = {},
  overrides: Partial<AutomationsOptions> & { store?: AutomationStore } = {},
) {
  const fake = fakeProtocol(protocol);
  const clock = fakeClock();
  const store = overrides.store ?? memoryStore();
  const outcomes: AutomationOutcome[] = [];
  const refused: AutomationOutcome[] = [];
  const rejected: AutomationOutcome[] = [];
  const errors: unknown[] = [];
  const make = (worker: string) =>
    createAutomations({
      client: fake.client,
      signer: walletsSigner(),
      policy,
      simulator: feeOnlySimulator,
      store,
      cluster: 'dev',
      clock,
      worker,
      trackInterval: '1s',
      trackFor: '5s',
      onOutcome: (outcome) => void outcomes.push(outcome),
      onRefused: (outcome) => void refused.push(outcome),
      onRejected: (outcome) => void rejected.push(outcome),
      onError: (error) => void errors.push(error),
      ...overrides,
    });
  const automations = make('w1');
  const grant = (wallet: string, steps = ['deposit', 'withdraw', 'rebalance', 'claim'], instanceId: string | null = null) =>
    store.consent.put(storedGrant(wallet, clock.now(), { steps, instanceId }));
  const stepJobs = async () => (await store.jobs.list({ kind: 'step' })) as StepJob[];
  return { fake, clock, store, automations, make, outcomes, refused, rejected, errors, grant, stepJobs };
}

