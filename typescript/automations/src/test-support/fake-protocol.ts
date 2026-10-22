// A protocol deployment, chain and clock for runtime tests. Not part of the package.

import { ed25519 } from '@noble/curves/ed25519';
import type {
  ExecutionIntentV1,
  ProtocolEventsOptions,
  ProtocolIntentState,
  ProtocolIntentStatus,
  ProtocolPrepareOptions,
  ProtocolPrepareResult,
  ProtocolStepAvailability,
} from '@usearete/sdk';

import { decodeTransaction } from '../policy/decode';
import { decodeBase64, encodeBase58, encodeBase64 } from '../policy/encoding';
import { definePolicy } from '../policy/policy';
import type { Simulator } from '../policy/simulator';
import type { AutomationClient } from '../runtime/automations';
import type { Clock } from '../runtime/time';
import { intentFor, wire } from './wire';

export const DEPLOYMENT = 'dep-1';
export const PROGRAM = encodeBase58(new Uint8Array(32).fill(40));
export const VAULT = encodeBase58(new Uint8Array(32).fill(30));

export function walletSeed(n: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, index) => (index * 7 + n) % 256);
}

export function walletOf(seed: Uint8Array): string {
  return encodeBase58(ed25519.getPublicKey(seed));
}

export const policy = definePolicy({
  deployments: [DEPLOYMENT],
  clusters: ['devnet'],
  programs: [PROGRAM],
  maxSolPerTransaction: '0.01',
});

/** A clock whose sleeps advance time instantly. */
export function fakeClock(start = 1_700_000_000_000): Clock & { advance(ms: number): void } {
  let now = start;
  return {
    now: () => now,
    sleep: async (ms) => {
      now += ms;
      await Promise.resolve();
    },
    advance(ms) {
      now += ms;
    },
  };
}

/** A chain where each transaction costs the wallet a 5000-lamport fee and nothing else. */
export const feeOnlySimulator: Simulator = {
  async call(method, params) {
    if (method === 'getMultipleAccounts') {
      const addresses = params[0] as string[];
      return {
        context: { slot: 10 },
        value: addresses.map((_, index) =>
          index === 0 ? { lamports: 1_000_000_000, owner: '11111111111111111111111111111111', data: ['', 'base64'] } : null,
        ),
      };
    }
    const addresses = (params[1] as { accounts: { addresses: string[] } }).accounts.addresses;
    return {
      context: { slot: 11 },
      value: {
        err: null,
        logs: [],
        innerInstructions: [],
        accounts: addresses.map((_, index) =>
          index === 0 ? { lamports: 1_000_000_000 - 5000, owner: '11111111111111111111111111111111', data: ['', 'base64'] } : null,
        ),
      },
    };
  },
};

export interface IntentRecord {
  intent: ExecutionIntentV1;
  transactions: { index: number; transactionBase64: string }[];
  /** States `intents.get` returns once submitted, the last one repeating. */
  script: ProtocolIntentStatus[];
  reads: number;
  submitted: string[] | null;
}

export interface PrepareCall {
  step: string;
  input: Record<string, unknown>;
  options: ProtocolPrepareOptions | undefined;
}

export interface FakeProtocolOptions {
  /** States after submit, per prepare generation (by idempotency key suffix). */
  script?: (key: string) => ProtocolIntentStatus[];
  /** Step availability per (step, wallet). Defaults to allowed. */
  allowed?: (step: string, wallet: string) => ProtocolStepAvailability;
  /** Runs inside prepare, before it answers. */
  onPrepare?: (call: PrepareCall) => void | Promise<void>;
  /** Decide what submit does: accept, throw before accepting, or accept then throw (a lost response). */
  submitMode?: (call: number) => 'accept' | 'fail-before' | 'fail-after';
  /** Reject prepare with this code. */
  rejectPrepare?: (step: string) => string | undefined;
  /** The program the prepared transactions invoke. */
  program?: string;
}

export function fakeProtocol(options: FakeProtocolOptions = {}) {
  const intents = new Map<string, IntentRecord>();
  const byKey = new Map<string, string>();
  const prepareCalls: PrepareCall[] = [];
  const submitCalls: { executionId: string; signed: string[] }[] = [];
  const allowedCalls: { instance: string; wallet: string }[] = [];
  const eventListeners: ProtocolEventsOptions['onEvent'][] = [];
  const terminal = new Set<string>();
  const instanceReads: string[] = [];
  let executions = 0;

  const stepClient = (step: string) => ({
    async prepare(input: Record<string, unknown>, prepareOptions?: ProtocolPrepareOptions): Promise<ProtocolPrepareResult> {
      const call = { step, input, options: prepareOptions };
      prepareCalls.push(call);
      await options.onPrepare?.(call);
      const rejected = options.rejectPrepare?.(step);
      if (rejected) return { ok: false, rejection: { rejected, step } };
      const key = prepareOptions?.idempotencyKey ?? `key-${prepareCalls.length}`;
      const existing = byKey.get(key);
      if (existing) {
        const record = intents.get(existing)!;
        return { ok: true, intent: record.intent, transactions: record.transactions };
      }
      const wallet = String(input.wallet);
      const format = prepareOptions?.transactionFormat ?? 'v1';
      const keyBytes = [...new TextEncoder().encode(key)].slice(-24);
      const message = {
        header: [1, 0, 1] as [number, number, number],
        keys: [wallet, VAULT, options.program ?? PROGRAM],
        instructions: [{ program: 2, accounts: [0, 1], data: [1, ...keyBytes] }],
      };
      const bytes = wire(message, format);
      executions += 1;
      const executionId = `exec-${executions}`;
      const intent = intentFor(wallet, format, [bytes], { executionId, step });
      const record: IntentRecord = {
        intent,
        transactions: [{ index: 0, transactionBase64: encodeBase64(bytes) }],
        script: options.script?.(key) ?? ['submitting', 'completed'],
        reads: 0,
        submitted: null,
      };
      intents.set(executionId, record);
      byKey.set(key, executionId);
      return { ok: true, intent, transactions: record.transactions };
    },
  });

  const stateOf = (record: IntentRecord): ProtocolIntentState => {
    let state: ProtocolIntentStatus = 'prepared';
    if (record.submitted) {
      state = record.script[Math.min(record.reads, record.script.length - 1)]!;
      record.reads += 1;
    }
    const signature =
      record.submitted && state !== 'expired'
        ? encodeBase58(decodeTransaction(decodeBase64(record.submitted[0]!)!).signatures[0]!)
        : null;
    return {
      executionId: record.intent.executionId,
      state,
      transactions: [{ index: 0, signature, state, ...(state === 'chain_failed' ? { error: 'custom program error: 0x1' } : {}) }],
    };
  };

  const client: AutomationClient & { steps: Record<string, ReturnType<typeof stepClient>> } = {
    deploymentId: DEPLOYMENT,
    steps: new Proxy({} as Record<string, ReturnType<typeof stepClient>>, {
      get: (_target, name) => (typeof name === 'string' ? stepClient(name) : undefined),
    }),
    async allowedSteps({ instance, wallet }) {
      allowedCalls.push({ instance, wallet });
      const steps = ['deposit', 'withdraw', 'rebalance', 'claim'];
      return steps.map((step) => options.allowed?.(step, wallet) ?? { step, allowed: true });
    },
    async submit(intent, signed) {
      const executionId = typeof intent === 'string' ? intent : intent.executionId;
      const record = intents.get(executionId);
      if (!record) throw new Error(`no intent ${executionId}`);
      const mode = options.submitMode?.(submitCalls.length + 1) ?? 'accept';
      if (mode === 'fail-before') {
        submitCalls.push({ executionId, signed: [] });
        throw new Error('connection reset before the API answered');
      }
      submitCalls.push({ executionId, signed: signed.map(String) });
      if (!record.submitted) record.submitted = signed.map(String);
      if (mode === 'fail-after') throw new Error('connection reset after the API accepted');
      return stateOf(record);
    },
    intents: {
      async get(executionId) {
        const record = intents.get(executionId);
        if (!record) throw new Error(`no intent ${executionId}`);
        return stateOf(record);
      },
    },
    instances: {
      async get(id: string) {
        instanceReads.push(id);
        return { terminal: terminal.has(id) };
      },
    },
    events(_instanceId, eventOptions) {
      eventListeners.push(eventOptions.onEvent);
      return { lastEventId: undefined, closed: false, close() {} };
    },
  };

  return {
    client,
    prepareCalls,
    submitCalls,
    allowedCalls,
    intents,
    instanceReads,
    /** Make an instance terminal. */
    endInstance(id: string) {
      terminal.add(id);
    },
    /** Emit a stream event to every follower. */
    emit(event: string, id: number, instanceId = 'inst-1', participantId: string | null = null) {
      for (const listener of eventListeners) {
        listener({
          type: 'emit',
          id: `cursor-${id}`,
          event,
          data: { id, event, reaction: 'r', instanceId, participantId, transitionId: null, at: '2026-10-22T00:00:00Z' },
        } as never);
      }
    },
  };
}
