import { afterEach, describe, expect, it } from 'vitest';

import {
  Arete,
  createPreparedInstruction,
  createSession,
  getTransactionFailureOutcome,
  programAccountRead,
  withProgramRead,
  type BuiltInstruction,
} from '../index';
import {
  FIXTURE_WALLET_ADDRESS,
  FakeWebSocket,
  createFakeTransactionTransport,
  createFetchStub,
  createFrameHarness,
  createTransactionOutcomeFixtures,
  createWalletFixture,
  createWebSocketHarness,
  firstSignatureOfWireTransaction,
  frames,
  transactionOutcomeFixture,
  type WebSocketHarness,
} from './index';

const STACK = {
  name: 'things',
  endpoints: { ws: 'ws://arete.test', http: 'http://arete.test' },
  views: {
    Thing: { list: { mode: 'list', view: 'Thing/list' } },
  },
} as const;

const MEMO: BuiltInstruction = {
  programId: 'MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr',
  keys: [{ pubkey: FIXTURE_WALLET_ADDRESS, isSigner: true, isWritable: true }],
  data: new Uint8Array([1, 2, 3]),
};

function prepared() {
  return createPreparedInstruction({ name: 'memo', instruction: MEMO, artifacts: {} });
}

describe('createWebSocketHarness', () => {
  let harness: WebSocketHarness | undefined;
  afterEach(() => harness?.restore());

  it('drives a real client through snapshots and live frames', async () => {
    harness = createWebSocketHarness();
    const client = await Arete.connect(STACK, {
      auth: { websocketFactory: harness.websocketFactory },
      autoReconnect: false,
    });

    const updates = client.views.Thing.list.watch()[Symbol.asyncIterator]();
    const first = updates.next();
    const subscription = await harness.waitForSubscription('Thing/list');
    expect(harness.subscription('Thing/list')).toEqual(subscription);
    harness.serve(subscription, [{ key: 'a', data: { id: 'a', value: 1 } }]);
    await expect(first).resolves.toMatchObject({
      value: { type: 'upsert', key: 'a', data: { id: 'a', value: 1 } },
    });

    // A patch for a key the client never held is not an entity.
    harness.send(frames.patch(subscription, 'b', { value: 9 }));
    harness.send(frames.patch(subscription, 'a', { value: 2 }));
    expect(client.views.Thing.list.getSync()).toEqual([{ id: 'a', value: 2 }]);

    harness.send(frames.upsert(subscription, 'b', { id: 'b', value: 3 }));
    expect(client.views.Thing.list.getSync()).toHaveLength(2);

    await updates.return?.();
    client.disconnect();
  });

  it('provides the WebSocket global on runtimes without one, and removes it again', async () => {
    const globals = globalThis as { WebSocket?: unknown };
    const original = globals.WebSocket;
    delete globals.WebSocket;
    try {
      harness = createWebSocketHarness();
      expect(globals.WebSocket).toBe(FakeWebSocket);
      const client = await Arete.connect(STACK, {
        auth: { websocketFactory: harness.websocketFactory },
        autoReconnect: false,
      });
      expect(client.connectionState).toBe('connected');
      client.disconnect();
      harness.restore();
      expect(globals.WebSocket).toBeUndefined();
    } finally {
      globals.WebSocket = original;
    }
  });
});

describe('createFrameHarness', () => {
  it('processes wire frames into query snapshots', () => {
    const harness = createFrameHarness();
    const subscription = harness.subscribe('things', { view: 'Thing/list' });

    harness.process(frames.subscribed(subscription));
    harness.process(frames.snapshot(subscription, [{ key: 'a', data: { id: 'a' } }]));
    harness.process(frames.remove(subscription, 'a'));

    expect(harness.snapshot('things')).toMatchObject({ keys: [], isLoading: false });
    expect(() => harness.process({ op: 'snapshot' })).toThrow(/Invalid WebSocket protocol v2 frame/);
  });
});

describe('createFakeTransactionTransport', () => {
  it('records inspection without a send, and echoes the submitted signature', async () => {
    const relay = createFakeTransactionTransport();
    await relay.getLatestBlockhash();
    await relay.getFeeForMessage('message');
    await relay.simulateTransaction('transaction');
    expect(relay.calls).toEqual(['latest-blockhash', 'fee', 'simulate']);
    expect(relay.sent).toEqual([]);

    const signature = new Uint8Array(64).fill(3);
    const wire = btoa(String.fromCharCode(1, ...signature, 0, 0, 0));
    expect(firstSignatureOfWireTransaction(wire)).toBeDefined();
    await expect(relay.sendTransaction(wire)).resolves.toEqual({
      signature: firstSignatureOfWireTransaction(wire),
    });
    expect(relay.calls.at(-1)).toBe('send');
  });

  it('can fail or stall after recording the dispatch', async () => {
    const failure = new Error('relay down');
    const failing = createFakeTransactionTransport({ sendError: failure });
    await expect(failing.sendTransaction('AA==')).rejects.toBe(failure);
    expect(failing.sent).toEqual(['AA==']);

    const stalled = createFakeTransactionTransport({ stallSend: true });
    const outcome = await Promise.race([
      stalled.sendTransaction('AA==').then(() => 'answered'),
      new Promise((resolve) => setTimeout(() => resolve('stalled'), 10)),
    ]);
    expect(outcome).toBe('stalled');
    expect(stalled.calls).toEqual(['send']);
  });
});

describe('createWalletFixture', () => {
  it('executes, inspects and fails through the real client', async () => {
    const outcomes = createTransactionOutcomeFixtures();
    const wallet = createWalletFixture({
      responses: [{ signature: 'sig-1', slot: 7 }, outcomes['submitted-unknown:send']],
    });
    const client = await Arete.connect(STACK, { transport: 'http', wallet });

    await expect(client.inspectOperation(prepared())).resolves.toMatchObject({
      transaction: { feeLamports: 5_000 },
    });
    expect(wallet.inspected).toHaveLength(1);
    expect(wallet.sent).toHaveLength(0);

    await expect(client.execute(prepared())).resolves.toMatchObject({
      transaction: { signature: 'sig-1' },
    });
    const failure = await client.execute(prepared()).catch((error: unknown) => error);
    expect(getTransactionFailureOutcome(failure)).toMatchObject({
      status: 'submitted-unknown',
      phase: 'send',
    });
    expect(wallet.sent).toHaveLength(2);
    client.disconnect();
  });

  it('can omit the inspection capability', () => {
    expect(createWalletFixture({ inspection: false }).inspectTransaction).toBeUndefined();
  });
});

describe('transaction outcome fixtures', () => {
  it('covers every status and phase of the outcome model', () => {
    const outcomes = createTransactionOutcomeFixtures();
    expect(Object.values(outcomes).map(({ status, phase }) => `${status}:${phase}`)).toEqual([
      'confirmed:confirmation',
      'not-submitted:build',
      'not-submitted:wallet',
      'not-submitted:send',
      'submitted-unknown:send',
      'submitted-unknown:confirmation',
      'chain-failed:confirmation',
      'chain-failed:chain',
    ]);
    expect(transactionOutcomeFixture('confirmed', { slot: 1 })).toMatchObject({ slot: 1 });
  });
});

describe('createFetchStub', () => {
  it('routes program reads and records every request', async () => {
    const fetch = createFetchStub([
      { match: '/accounts/Multisig/', reply: { threshold: 2n } },
    ]);
    const program = withProgramRead(
      {
        name: 'squads',
        programId: 'SQDS111111111111111111111111111111111111111',
        programSpecHash: 'spec-squads',
        accounts: { Multisig: programAccountRead<{ threshold: string }>({ account: 'Multisig' }) },
      },
      {
        release: { programReleaseHash: 'release-squads', programSpecHash: 'spec-squads' },
        transport: { kind: 'local-http', endpointSource: 'connect-http-url' },
      },
    );
    const session = await createSession(
      { programs: { squads: program } },
      { fetch, endpoints: { http: 'http://arete.test' } },
    );

    await expect(session.programs.squads.accounts.Multisig.fetch('address'))
      .resolves.toEqual({ threshold: '2' });
    expect(fetch.calls.map(({ method, url }) => `${method} ${url}`)).toEqual([
      'GET http://arete.test/v1/releases/release-squads/accounts/Multisig/address',
    ]);
    await expect(fetch('http://arete.test/unrouted')).resolves.toMatchObject({ status: 404 });
    session.close();
  });
});
