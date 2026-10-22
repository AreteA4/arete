// Runs against a real Postgres when ARETE_TEST_DATABASE_URL is set (CI sets
// it for Postgres 15, 16 and 17); skipped otherwise.

import pg from 'pg';
import { afterAll, describe, expect, it } from 'vitest';

import { runStoreConformance } from '../store/conformance';
import { fakeProtocol, feeOnlySimulator, policy, fakeClock, DEPLOYMENT } from '../test-support/fake-protocol';
import { ALICE, BOB, storedGrant, walletsSigner } from '../test-support/harness';
import { createAutomations } from '../runtime/automations';
import { LATEST_SCHEMA_VERSION, migrationFiles, runtimeRoleGrants } from './migrations';
import { postgresStore, SchemaVersionError } from './store';

const url = process.env.ARETE_TEST_DATABASE_URL;
const suite = url ? describe : describe.skip;

const pool = url ? new pg.Pool({ connectionString: url, max: 20 }) : (undefined as unknown as pg.Pool);
const schemas: string[] = [];
const roles: string[] = [];
let counter = 0;

function freshSchema(): string {
  counter += 1;
  const name = `arete_test_${process.pid}_${counter}_${Math.random().toString(36).slice(2, 8)}`;
  schemas.push(name);
  return name;
}

async function migratedStore() {
  const store = postgresStore(pool, { schema: freshSchema() });
  await store.migrate();
  return store;
}

afterAll(async () => {
  if (!url) return;
  for (const schema of schemas) await pool.query(`DROP SCHEMA IF EXISTS "${schema}" CASCADE`);
  for (const role of roles) await pool.query(`DROP ROLE IF EXISTS ${role}`);
  await pool.end();
});

if (url) runStoreConformance('postgresStore', migratedStore, { describe, it, expect });

suite('postgresStore: migrations', () => {
  it('migrates into a database with unrelated tables and touches nothing outside its schema', async () => {
    const unrelated = freshSchema();
    await pool.query(`CREATE SCHEMA ${unrelated}`);
    await pool.query(`CREATE TABLE ${unrelated}.users (id serial PRIMARY KEY, email text NOT NULL)`);
    await pool.query(`INSERT INTO ${unrelated}.users (email) VALUES ('a@example.com')`);
    const store = postgresStore(pool, { schema: freshSchema() });
    const objects = async () =>
      (
        await pool.query(
          `SELECT n.nspname || '.' || c.relname || ':' || c.relkind::text AS name FROM pg_class c
           JOIN pg_namespace n ON n.oid = c.relnamespace
           WHERE n.nspname NOT IN ('pg_catalog', 'information_schema', 'pg_toast') AND n.nspname <> $1
           ORDER BY 1`,
          [store.schema],
        )
      ).rows.map((row) => row.name as string);
    const before = await objects();
    expect(await store.migrate()).toEqual({ from: 0, to: LATEST_SCHEMA_VERSION });
    expect(await objects()).toEqual(before);
    expect((await pool.query(`SELECT email FROM ${unrelated}.users`)).rows).toEqual([{ email: 'a@example.com' }]);
    expect(await store.migrate()).toEqual({ from: LATEST_SCHEMA_VERSION, to: LATEST_SCHEMA_VERSION });
  });

  it('serializes concurrent migrations with an advisory lock', async () => {
    const schema = freshSchema();
    const results = await Promise.all(Array.from({ length: 5 }, () => postgresStore(pool, { schema }).migrate()));
    expect(results.filter((result) => result.from === 0)).toHaveLength(1);
    const versions = await pool.query(`SELECT version FROM ${schema}.schema_migrations ORDER BY version`);
    expect(versions.rows.map((row) => row.version)).toEqual([LATEST_SCHEMA_VERSION]);
  });

  it('refuses to run against a database that is behind, and never migrates by itself', async () => {
    const store = postgresStore(pool, { schema: freshSchema() });
    expect(await store.schemaVersion()).toBe(0);
    await expect(store.jobs.list()).rejects.toBeInstanceOf(SchemaVersionError);
    await expect(store.deliveries.accept({ id: 'd', name: 'n', source: 'dispatch', payload: {}, receivedAt: 0 }, 0)).rejects.toThrow(
      'run store.migrate()',
    );
    expect(await store.schemaVersion()).toBe(0);
    await store.migrate();
    expect(await store.jobs.list()).toEqual([]);
  });

  it('accepts a reserved word as the schema name', async () => {
    schemas.push('user');
    await pool.query('DROP SCHEMA IF EXISTS "user" CASCADE');
    const store = postgresStore(pool, { schema: 'user' });
    await store.migrate();
    expect(await store.deliveries.accept({ id: 'd', name: 'n', source: 'dispatch', payload: {}, receivedAt: 0 }, 0)).toBe(true);
    const files = migrationFiles('select');
    expect(files[0]!.sql).toContain('CREATE TABLE "select".jobs');
  });

  it('binds a schema to one deployment across store instances', async () => {
    const store = await migratedStore();
    await store.bindDeployment('dep-a');
    await expect(postgresStore(pool, { schema: store.schema }).bindDeployment('dep-b')).rejects.toThrow('belongs to deployment dep-a');
  });

  it('exports SQL files that bring a schema to the same version', async () => {
    const schema = freshSchema();
    const files = migrationFiles(schema);
    expect(files.map((file) => file.name)).toEqual(['0001_initial.sql']);
    for (const file of files) await pool.query(file.sql);
    const store = postgresStore(pool, { schema });
    await store.checkVersion();
    expect(await store.deliveries.accept({ id: 'd', name: 'n', source: 'dispatch', payload: {}, receivedAt: 0 }, 0)).toBe(true);
  });

  it('runs with a runtime role that cannot change the schema', async () => {
    const store = await migratedStore();
    const role = `arete_rt_${process.pid}_${Math.random().toString(36).slice(2, 8)}`;
    roles.push(role);
    await pool.query(`CREATE ROLE ${role} LOGIN PASSWORD 'runtime'`);
    await pool.query(runtimeRoleGrants(role, store.schema));
    const runtimeUrl = new URL(url!);
    runtimeUrl.username = role;
    runtimeUrl.password = 'runtime';
    const runtimePool = new pg.Pool({ connectionString: runtimeUrl.toString(), max: 2 });
    try {
      const runtime = postgresStore(runtimePool, { schema: store.schema });
      expect(await runtime.deliveries.accept({ id: 'd', name: 'n', source: 'dispatch', payload: {}, receivedAt: 0 }, 0)).toBe(true);
      expect((await runtime.jobs.claimNext({ worker: 'w', leaseMs: 1000, now: 0 }))?.id).toBe('d');
      await expect(runtimePool.query(`CREATE TABLE ${store.schema}.sneaky (id int)`)).rejects.toMatchObject({ code: '42501' });
      await expect(runtimePool.query(`ALTER TABLE ${store.schema}.jobs ADD COLUMN sneaky int`)).rejects.toMatchObject({ code: '42501' });
      await expect(runtime.migrate()).rejects.toMatchObject({ code: '42501' });
    } finally {
      await runtimePool.end();
    }
  });
});

suite('postgresStore: transactions and concurrency', () => {
  it('runs multi-statement operations inside the caller transaction', async () => {
    const store = await migratedStore();
    await store.consent.put(storedGrant(ALICE, 0));
    await store.deliveries.accept({ id: 'd', name: 'n', source: 'dispatch', payload: {}, receivedAt: 0 }, 0);
    const client = await pool.connect();
    try {
      await client.query('BEGIN');
      const inTransaction = store.using(client);
      expect(await inTransaction.consent.revoke({ wallet: ALICE, deploymentId: DEPLOYMENT, now: 1 })).toBe(1);
      expect((await inTransaction.jobs.claimNext({ worker: 'w', leaseMs: 1000, now: 1 }))?.id).toBe('d');
      expect(await inTransaction.jobs.completeDelivery('d', 'w', [], 2)).toBe(true);
      // A client passed straight in is used the same way, not connected again.
      const direct = postgresStore(client, { schema: store.schema });
      expect(await direct.consent.revoke({ wallet: ALICE, deploymentId: DEPLOYMENT, now: 3 })).toBe(0);
      await client.query('ROLLBACK');
    } finally {
      client.release();
    }
    expect((await store.consent.list({ wallet: ALICE }))[0]!.revokedAt).toBeNull();
    expect((await store.jobs.get('d'))?.state).toBe('queued');
  });

  it("keeps the caller's transaction usable after a lost claim race", async () => {
    const store = await migratedStore();
    await store.deliveries.accept({ id: 'd', name: 'n', source: 'dispatch', payload: {}, receivedAt: 0 }, 0);
    await store.jobs.claimNext({ worker: 'seed', leaseMs: 1000, now: 0 });
    await store.jobs.completeDelivery(
      'd',
      'seed',
      ['a', 'b'].map((step) => ({ id: `d:${step}:${ALICE}`, deploymentId: DEPLOYMENT, request: { step, wallet: ALICE, args: {} } })),
      0,
    );
    const first = await pool.connect();
    const second = await pool.connect();
    try {
      await first.query('BEGIN');
      await second.query('BEGIN');
      expect((await store.using(first).jobs.claimNext({ worker: 'w1', leaseMs: 60_000, now: 1 }))?.id).toBe(`d:a:${ALICE}`);
      // The second transaction cannot see the first claim yet, races into the unique index, and must lose cleanly.
      const racing = store.using(second).jobs.claimNext({ worker: 'w2', leaseMs: 60_000, now: 1 });
      await new Promise((resolve) => setTimeout(resolve, 100));
      await first.query('COMMIT');
      expect(await racing).toBeNull();
      expect((await second.query('SELECT 1 AS ok')).rows).toEqual([{ ok: 1 }]);
      await second.query('COMMIT');
    } finally {
      first.release();
      second.release();
    }
  });

  it('round-trips a saved intent exactly, nested nulls included', async () => {
    const store = await migratedStore();
    await store.deliveries.accept({ id: 'd', name: 'n', source: 'dispatch', payload: {}, receivedAt: 0 }, 0);
    await store.jobs.claimNext({ worker: 'seed', leaseMs: 1000, now: 0 });
    await store.jobs.completeDelivery('d', 'seed', [{ id: 's', deploymentId: DEPLOYMENT, request: { step: 'x', wallet: ALICE, args: {} } }], 0);
    await store.jobs.claimNext({ worker: 'w', leaseMs: 60_000, now: 1 });
    const intent = {
      executionId: 'e',
      participantId: null,
      fees: null,
      signers: [{ class: 'wallet', pubkey: ALICE, name: null }],
      transactions: [{ index: 0, messageSha256: 'ab', signers: [ALICE] }],
    };
    await store.jobs.transition('s', {
      worker: 'w',
      from: 'running',
      to: 'signed',
      now: 2,
      keepLease: { until: 60_000 },
      progress: { intent: intent as never, signed: ['AAAA'], signatures: [null] },
    });
    let job = (await store.jobs.get('s')) as unknown as { progress: Record<string, unknown> };
    expect(job.progress).toEqual({ generation: 0, intent, signed: ['AAAA'], signatures: [null] });
    // Clearing fields removes just those fields.
    await store.jobs.transition('s', {
      worker: 'w',
      from: 'signed',
      to: 'running',
      now: 3,
      keepLease: { until: 60_000 },
      progress: { generation: 1, intent: undefined, signed: undefined },
    });
    job = (await store.jobs.get('s')) as unknown as { progress: Record<string, unknown> };
    expect(job.progress).toEqual({ generation: 1, signatures: [null] });
  });

  it("checks the version without aborting a caller's transaction", async () => {
    const store = postgresStore(pool, { schema: freshSchema() });
    const client = await pool.connect();
    try {
      await client.query('BEGIN');
      expect(await store.using(client).schemaVersion()).toBe(0);
      await store.using(client).migrate();
      expect(await store.using(client).schemaVersion()).toBe(LATEST_SCHEMA_VERSION);
      await client.query('COMMIT');
    } finally {
      client.release();
    }
    expect(await store.schemaVersion()).toBe(LATEST_SCHEMA_VERSION);
  });

  it('writes inside the caller transaction and rolls back with it', async () => {
    const store = await migratedStore();
    const client = await pool.connect();
    try {
      await client.query('BEGIN');
      await store.using(client).consent.put(storedGrant(ALICE, 0));
      expect(await store.using(client).consent.list({ wallet: ALICE })).toHaveLength(1);
      await client.query('ROLLBACK');
    } finally {
      client.release();
    }
    expect(await store.consent.list({ wallet: ALICE })).toEqual([]);
  });

  it('never starts two jobs of one wallet across parallel connections', async () => {
    // Racing claims are what the unique index exists for; repeat to make the race happen.
    for (let trial = 0; trial < 8; trial += 1) {
      const store = await migratedStore();
      await store.deliveries.accept({ id: 'd', name: 'n', source: 'dispatch', payload: {}, receivedAt: 0 }, 0);
      await store.jobs.claimNext({ worker: 'seed', leaseMs: 1000, now: 0 });
      const steps = [ALICE, BOB].flatMap((wallet) =>
        Array.from({ length: 12 }, (_, index) => ({
          id: `d:step${index}:${wallet}`,
          deploymentId: DEPLOYMENT,
          request: { step: `step${index}`, wallet, args: {} },
        })),
      );
      await store.jobs.completeDelivery('d', 'seed', steps, 0);
      const claims = await Promise.all(
        Array.from({ length: 18 }, (_, index) => store.jobs.claimNext({ worker: `w${index}`, leaseMs: 60_000, now: 1 })),
      );
      const claimed = claims.filter((job) => job !== null);
      const active = await pool.query(
        `SELECT wallet, count(*)::int AS n FROM ${store.schema}.jobs WHERE kind = 'step' AND state <> 'queued' GROUP BY wallet`,
      );
      expect(active.rows.every((row) => row.n === 1)).toBe(true);
      expect(claimed).toHaveLength(2);
      expect(new Set(claimed.map((job) => (job!.kind === 'step' ? job!.request.wallet : '')))).toEqual(new Set([ALICE, BOB]));
    }
  });

  it('runs automations end to end, with a window cap held under parallel workers', async () => {
    const store = await migratedStore();
    const fake = fakeProtocol();
    const clock = fakeClock();
    await store.consent.put(
      storedGrant(ALICE, clock.now(), {
        limits: { perExecution: { SOL: '5000' }, perWindow: { SOL: { amount: '15000', windowMs: 86_400_000 } } },
      }),
    );
    const workers = Array.from({ length: 4 }, (_, index) =>
      createAutomations({
        client: fake.client,
        signer: walletsSigner(),
        policy,
        simulator: feeOnlySimulator,
        store,
        cluster: 'devnet',
        clock,
        worker: `w${index}`,
        trackInterval: '1s',
      }).on('many', (_payload, ctx) =>
        (['deposit', 'withdraw', 'claim', 'rebalance'] as const).map((step) => ctx.step(step, { wallet: ALICE, instance: 'inst-1' })),
      ),
    );
    await workers[0]!.dispatch('many', {}, { id: 'm' });
    for (let round = 0; round < 8; round += 1) await Promise.all(workers.map((worker) => worker.runOnce()));
    const states = (await store.jobs.list({ kind: 'step' })).map((job) => [job.state, job.outcome?.code]);
    expect(states.filter(([state]) => state === 'completed')).toHaveLength(3);
    expect(states.filter(([, outcome]) => outcome === 'consent_window')).toHaveLength(1);
    expect(await store.ledger.spent({ wallet: ALICE, deploymentId: DEPLOYMENT, asset: 'SOL', since: 0 })).toBe(15000n);
    expect(fake.submitCalls).toHaveLength(3);
  });
});
