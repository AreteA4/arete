import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { deserialize, serialize } from 'node:v8';

import { ed25519 } from '@noble/curves/ed25519';
import { verifyWebhook } from '@usearete/sdk';
import { describe, expect, it } from 'vitest';

import { decodeBase58, encodeBase58 } from '../policy/encoding';
import { defineAutomations } from '../runtime/define';
import { memoryStore, memoryStoreState } from '../store/memory';
import { fakeProtocol, PROGRAM } from '../test-support/fake-protocol';
import { ALICE, harness, storedGrant } from '../test-support/harness';
import { type PumpClient, type SignedDelivery, signDelivery, startEventPump } from './events';
import { devStackConnector, loadDefinition, projectTsImport, runDev } from './runner';
import { devWalletsSigner, readDevWallets } from './signer';

const SECRET = 'whsec_dev';
const tick = async (times = 20) => {
  for (let i = 0; i < times; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
};

function definitionFor(name = 'vault') {
  return defineAutomations(
    { name, deployment: { deploymentId: 'dep' } } as never,
    (automations) => {
      automations.on('vault.rebalanceDue', (event: { instanceId: string }, ctx) => [
        ctx.step('rebalance', { wallet: ALICE, instance: event.instanceId }),
      ]);
    },
    { policy: { programs: [PROGRAM], maxSolPerTransaction: '0.01' } },
  );
}

/** The fake client with the instance listing the pump needs. */
function pumpClient(fake: ReturnType<typeof fakeProtocol>, ids: string[]): PumpClient {
  return {
    ...fake.client,
    instances: { list: async () => ({ instances: ids.map((id) => ({ id })), nextCursor: null }) },
    events: fake.client.events!.bind(fake.client) as never,
  } as PumpClient;
}

describe('defineAutomations and use', () => {
  it('registers the definition’s handlers on the running automations', async () => {
    const h = harness();
    await h.grant(ALICE);
    await h.automations.use(definitionFor() as never);
    await h.automations.dispatch('vault.rebalanceDue', { instanceId: 'inst-1' }, { id: 't' });
    await h.automations.drain();
    expect(h.outcomes[0]).toMatchObject({ state: 'completed', step: 'rebalance' });
  });

  it('refuses a definition for another protocol than the client’s', async () => {
    const fake = fakeProtocol();
    const h = harness({}, { client: { ...fake.client, definition: { name: 'raise', deployment: {} } as never } });
    await expect(h.automations.use(definitionFor('vault') as never)).rejects.toThrow(/for protocol vault, but the client is for raise/);
  });
});

describe('the event pump', () => {
  it('forwards emissions as signed production deliveries, once each, from the first run on', async () => {
    const h = harness();
    await h.grant(ALICE);
    await h.automations.use(definitionFor() as never);
    const delivered: SignedDelivery[] = [];
    const cursors = new Map<string, string>();
    const pump = startEventPump({
      client: pumpClient(h.fake, ['inst-1']),
      secret: SECRET,
      cursors,
      // The fake's events occurred at 2026-10-22T00:00:00Z.
      since: Date.parse('2026-10-21T00:00:00Z'),
      now: () => h.clock.now(),
      listIntervalMs: 60_000,
      deliver: async (delivery) => {
        delivered.push(delivery);
        const result = await h.automations.acceptWebhook(delivery.body, delivery.headers, { secret: SECRET });
        if (result.status !== 200) throw new Error(JSON.stringify(result.body));
      },
    });
    await tick();
    h.fake.emit('vault.rebalanceDue', 7);
    h.fake.emit('vault.rebalanceDue', 7);
    h.fake.emit('vault.closed', 8);
    await tick();
    pump.stop();

    const verified = verifyWebhook(SECRET, delivered[0]!.body, delivered[0]!.headers, { now: Math.floor(h.clock.now() / 1000) });
    expect(verified.payload).toEqual({
      id: '7',
      event: 'vault.rebalanceDue',
      deploymentId: h.fake.client.deploymentId,
      instanceId: 'inst-1',
      participantId: null,
      occurredAt: '2026-10-22T00:00:00Z',
    });
    expect(cursors.get('inst-1')).toBe('cursor-8');
    expect((await h.store.jobs.list({ kind: 'delivery' })).map((job) => job.id)).toEqual([
      `webhook:${h.fake.client.deploymentId}:7`,
    ]);
    await h.automations.drain();
    expect(h.outcomes).toHaveLength(1);
  });

  it('skips history before the first run and retries a failed delivery in order', async () => {
    const fake = fakeProtocol();
    const cursors = new Map<string, string>();
    const seen: string[] = [];
    let failures = 1;
    const pump = startEventPump({
      client: pumpClient(fake, ['inst-1']),
      secret: SECRET,
      cursors,
      since: Date.parse('2026-10-23T00:00:00Z'),
      listIntervalMs: 60_000,
      sleep: async () => undefined,
      deliver: async (delivery) => {
        if (failures-- > 0) throw new Error('app down');
        seen.push(JSON.parse(delivery.body).id);
      },
    });
    await tick();
    fake.emit('vault.rebalanceDue', 1);
    await tick();
    // Before `since`: the cursor moves on, nothing is delivered.
    expect(seen).toEqual([]);
    expect(cursors.get('inst-1')).toBe('cursor-1');
    pump.stop();

    const late = fakeProtocol();
    const retried = startEventPump({
      client: pumpClient(late, ['inst-1']),
      secret: SECRET,
      cursors,
      since: 0,
      listIntervalMs: 60_000,
      sleep: async () => undefined,
      deliver: async (delivery) => {
        if (failures-- > 0) throw new Error('app down');
        seen.push(JSON.parse(delivery.body).id);
      },
    });
    failures = 1;
    await tick();
    late.emit('a', 2);
    late.emit('b', 3);
    await tick();
    retried.stop();
    expect(seen).toEqual(['2', '3']);
    expect(cursors.get('inst-1')).toBe('cursor-3');
  });

  it('signs with the production header names', () => {
    const delivery = signDelivery(SECRET, { id: 'x' }, 100);
    expect(Object.keys(delivery.headers)).toEqual([
      'content-type',
      'arete-webhook-id',
      'arete-webhook-timestamp',
      'arete-webhook-signature',
    ]);
    expect(() => verifyWebhook('other', delivery.body, delivery.headers, { now: 100 })).toThrow();
  });
});

describe('the dev store', () => {
  it('survives a restart through its saved state, with claims released', async () => {
    const store = memoryStore();
    await store.bindDeployment('dep');
    await store.consent.put(storedGrant(ALICE, 1000, { steps: ['refund'] }));
    await store.deliveries.accept({ id: 'd-1', name: 'x', source: 'dispatch', payload: {}, receivedAt: 1 }, 1);
    expect(await store.jobs.claimNext({ worker: 'w', leaseMs: 60_000, now: 1 })).not.toBeNull();

    const restored = memoryStore({ state: deserialize(serialize(memoryStoreState(store))) });
    expect(await restored.consent.list({ wallet: ALICE })).toHaveLength(1);
    const [job] = await restored.jobs.list({ kind: 'delivery' });
    expect(job).toMatchObject({ id: 'd-1', lease: null });
    await expect(restored.bindDeployment('other')).rejects.toThrow();
  });
});

describe('dev wallets', () => {
  it('signs for every keypair file, by address and by name', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'dev-wallets-'));
    const seed = new Uint8Array(32).fill(7);
    const publicKey = ed25519.getPublicKey(seed);
    writeFileSync(join(dir, 'alice.json'), JSON.stringify([...seed, ...publicKey]));
    writeFileSync(join(dir, 'notes.json'), '"not a key"');
    const address = encodeBase58(publicKey);

    const wallets = readDevWallets(dir);
    expect([...wallets.byName.keys()]).toEqual(['alice']);
    const message = new TextEncoder().encode('consent');
    expect(ed25519.verify(wallets.byName.get('alice')!.signMessage(message), message, decodeBase58(address)!)).toBe(true);

    const signer = devWalletsSigner(dir);
    expect(await signer.canSign(address)).toBe(true);
    expect(await signer.canSign(ALICE)).toBe(false);
    await expect(signer.signTransactions({ wallet: ALICE } as never)).rejects.toThrow(/not a dev wallet/);
  });
});

describe('the dev stack connector', () => {
  it('waits for the environment’s stack instead of reading published data', async () => {
    let session: Record<string, unknown> | null = { deploymentId: 'dep' };
    const lines: string[] = [];
    const connect = devStackConnector(() => session as never, 'key', (line) => lines.push(line), 2000);
    const stack = { name: 'vault', endpoints: {} } as never;
    setTimeout(() => {
      session = { deploymentId: 'dep', stack: 'other', stackWsUrl: 'ws://127.0.0.1:1', sessionEndpoint: 'http://127.0.0.1:1' };
    }, 600);
    // The environment's stack is another one: published data, said once.
    await connect(stack).catch(() => undefined);
    expect(lines).toEqual([
      "waiting for the dev environment's stack before connecting vault",
      "stack vault is not in the dev environment; its sources read the published stack's data",
    ]);
  });
});

describe('loading the entry', () => {
  it("uses the tsx of the entry's project, which this package cannot see under pnpm", async () => {
    const project = mkdtempSync(join(tmpdir(), 'dev-tsx-'));
    const api = join(project, 'node_modules', 'tsx', 'dist', 'esm', 'api');
    mkdirSync(api, { recursive: true });
    writeFileSync(
      join(project, 'node_modules', 'tsx', 'package.json'),
      JSON.stringify({ name: 'tsx', exports: { './package.json': './package.json', './esm/api': { import: { default: './dist/esm/api/index.mjs' } } } }),
    );
    // A stand-in tsImport that marks what it loaded, shaped as tsx returns a
    // CommonJS entry (a project without "type": "module").
    writeFileSync(
      join(api, 'index.mjs'),
      "export const tsImport = async (url) => ({ default: { default: { kind: 'automations', protocol: { name: 'via-tsx', url }, setup() {} } } });\n",
    );
    const entry = join(project, 'automations.ts');
    writeFileSync(entry, 'export default {};\n');
    expect((await loadDefinition(entry)).protocol).toMatchObject({ name: 'via-tsx' });
    expect(await projectTsImport(join(mkdtempSync(join(tmpdir(), 'dev-none-')), 'a.ts'))).toBeUndefined();
  });

  it('takes the default export and refuses a module without a definition', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'dev-entry-'));
    const good = join(dir, 'good.mjs');
    writeFileSync(good, "export default { kind: 'automations', protocol: { name: 'p' }, setup() {} };\n");
    const bad = join(dir, 'bad.mjs');
    writeFileSync(bad, 'export const nothing = 1;\n');
    expect((await loadDefinition(good)).protocol).toEqual({ name: 'p' });
    await expect(loadDefinition(bad)).rejects.toThrow(/default export/);
  });
});

describe('runDev', () => {
  it('runs the entry against the dev deployment: forwarded events, dev-wallet consent, saved state', async () => {
    const root = mkdtempSync(join(tmpdir(), 'dev-project-'));
    const devDir = join(root, '.arete', 'dev');
    mkdirSync(join(devDir, 'wallets'), { recursive: true });
    const seed = new Uint8Array(32).fill(9);
    const alice = encodeBase58(ed25519.getPublicKey(seed));
    writeFileSync(join(devDir, 'wallets', 'alice.json'), JSON.stringify([...seed, ...ed25519.getPublicKey(seed)]));
    const fake = fakeProtocol();
    const deploymentId = fake.client.deploymentId;
    writeFileSync(
      join(root, 'automations.mjs'),
      `export default {
        kind: 'automations',
        protocol: { name: 'vault', deployment: { deploymentId: '${deploymentId}', environment: 'dev' } },
        setup(automations) {
          automations.on('vault.rebalanceDue', (event) => { globalThis.__devSeen.push(event.id); return []; });
        },
        policy: { programs: ['${PROGRAM}'], maxSolPerTransaction: '0.01' },
      };\n`,
    );
    // The fake's events are dated 2026-10-22: count them as after the first run.
    mkdirSync(join(devDir, 'automations'), { recursive: true });
    writeFileSync(join(devDir, 'automations', 'since'), '0');
    const seen: string[] = [];
    (globalThis as unknown as { __devSeen: string[] }).__devSeen = seen;
    const lines: string[] = [];
    const client = {
      ...fake.client,
      definition: { name: 'vault', deployment: { deploymentId, environment: 'dev' } },
      instances: { list: async () => ({ instances: [{ id: 'inst-1' }], nextCursor: null }) },
    };
    const started = runDev(
      { root, entry: 'automations.mjs', apiKey: 'a4_sk_test', webhookSecret: SECRET, port: 0 },
      (line) => lines.push(line),
      { createClient: () => client },
    );
    // It waits for the protocol to deploy.
    await tick();
    writeFileSync(
      join(devDir, 'session.json'),
      JSON.stringify({ apiUrl: 'http://127.0.0.1:1', sessionEndpoint: 'http://127.0.0.1:1/ws/sessions', rpcUrl: 'http://127.0.0.1:1', deploymentId }),
    );
    const runner = await started;
    expect(lines[0]).toMatch(/waiting for the protocol to deploy/);

    await tick();
    fake.emit('vault.rebalanceDue', 41);
    for (let wait = 0; wait < 300 && seen.length === 0; wait += 1) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    expect(seen, lines.join('\n')).toEqual(['41']);

    const { port, token } = JSON.parse(readFileSync(join(devDir, 'automations', 'runner.json'), 'utf8')) as {
      port: number;
      token: string;
    };
    expect(port).toBe(runner.port);
    const control = (body: unknown, headers: Record<string, string> = {}) =>
      fetch(`http://127.0.0.1:${port}/consent`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', authorization: `Bearer ${token}`, ...headers },
        body: JSON.stringify(body),
      });
    const consent = { wallet: 'alice', steps: ['rebalance'], limits: { perExecution: { SOL: '5000' } } };
    // Only `a4 dev consent`: the token, JSON, and no browser origin.
    expect((await control(consent, { authorization: 'Bearer wrong' })).status).toBe(401);
    expect((await control(consent, { origin: 'https://evil.example' })).status).toBe(403);
    expect((await control(consent, { 'content-type': 'text/plain' })).status).toBe(415);

    const response = await control(consent);
    const body = (await response.json()) as { grant: { wallet: string; steps: string[] } };
    expect(response.status).toBe(200);
    expect(body.grant).toMatchObject({ wallet: alice, steps: ['rebalance'] });
    expect((await control({ wallet: 'bob', steps: ['rebalance'] })).status).toBe(400);
    // Revoking one instance leaves the deployment-wide grant.
    const scoped = (await (await control({ wallet: 'alice', steps: [], instanceId: 'inst-9', revoke: true })).json()) as {
      revoked: number;
    };
    expect(scoped.revoked).toBe(0);

    await runner.stop();
    const saved = deserialize(readFileSync(join(devDir, 'automations', 'store.bin'))) as { grants: { wallet: string }[] };
    expect(saved.grants.map((grant) => grant.wallet)).toEqual([alice]);
    expect(JSON.parse(readFileSync(join(devDir, 'automations', 'cursors.json'), 'utf8'))).toEqual({ 'inst-1': 'cursor-41' });
  });

  it('refuses a client generated for another deployment than the session’s', async () => {
    const root = mkdtempSync(join(tmpdir(), 'dev-project-'));
    mkdirSync(join(root, '.arete', 'dev'), { recursive: true });
    writeFileSync(join(root, '.arete', 'dev', 'session.json'), JSON.stringify({ rpcUrl: 'http://127.0.0.1:1', deploymentId: 'dep-new' }));
    writeFileSync(join(root, 'a.mjs'), "export default { kind: 'automations', protocol: { name: 'p', deployment: {} }, setup() {} };\n");
    await expect(
      runDev({ root, entry: 'a.mjs', apiKey: 'k', webhookSecret: SECRET, port: 0 }, () => undefined, {
        createClient: () => ({ deploymentId: 'dep-old', definition: { name: 'p', deployment: { deploymentId: 'dep-old', environment: 'dev' } } }),
      }),
    ).rejects.toThrow(/names deployment dep-old, but the dev environment runs dep-new; run `a4 dev sdk`/);
  });

  it('refuses the published client: a4 dev never targets mainnet', async () => {
    const root = mkdtempSync(join(tmpdir(), 'dev-project-'));
    mkdirSync(join(root, '.arete', 'dev'), { recursive: true });
    writeFileSync(join(root, '.arete', 'dev', 'session.json'), JSON.stringify({ rpcUrl: 'http://127.0.0.1:1', deploymentId: 'dep' }));
    writeFileSync(join(root, 'a.mjs'), "export default { kind: 'automations', protocol: { name: 'p', deployment: {} }, setup() {} };\n");
    await expect(
      runDev({ root, entry: 'a.mjs', apiKey: 'k', webhookSecret: SECRET, port: 0 }, () => undefined, {
        createClient: () => ({ deploymentId: 'dep', definition: { name: 'p', deployment: { deploymentId: 'dep' } } }),
      }),
    ).rejects.toThrow(/targets mainnet/);
  });
});
