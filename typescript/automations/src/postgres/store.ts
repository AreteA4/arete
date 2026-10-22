/**
 * The Postgres store. It runs in your existing database, in its own schema,
 * through the pool (or client) you pass: it opens no connections of its
 * own and creates nothing outside its schema.
 *
 * Migrations never run at startup. Run `store.migrate()` (with a role that
 * may create objects) or apply the SQL from `migrationFiles()` with your
 * own tool; a store whose database is behind refuses to run.
 */

import {
  ACTIVE_STEP_STATES,
  type Delivery,
  type Job,
  type StepJob,
  TERMINAL_DELIVERY_STATES,
  TERMINAL_STEP_STATES,
} from '../runtime/types';
import { type AutomationStore, type ConsentGrant, type LedgerEntry, parseCursor, type SourceState, StoreDeploymentError } from '../store/store';
import { DEFAULT_SCHEMA, LATEST_SCHEMA_VERSION, MIGRATIONS, quoted, validSchema } from './migrations';

/** What the store needs from a `pg` Pool or Client (or anything with the same `query`). */
export interface PgQueryable {
  query(text: string, values?: unknown[]): Promise<{ rows: any[]; rowCount: number | null }>;
}

/** A pool: hands out clients for transactions. */
export interface PgPoolLike extends PgQueryable {
  connect(): Promise<PgQueryable & { release(error?: Error | boolean): void }>;
}

export interface PostgresStoreOptions {
  /** Defaults to `arete_automation`. */
  readonly schema?: string;
}

interface InternalOptions extends PostgresStoreOptions {
  /** Treat the executor as a client in the caller's transaction. */
  readonly client?: boolean;
}

export interface PostgresStore extends AutomationStore {
  readonly schema: string;
  /**
   * Apply pending migrations in one transaction, behind an advisory lock so
   * concurrent callers run one at a time. Needs a role that may create
   * objects in the schema.
   */
  migrate(): Promise<{ from: number; to: number }>;
  /** The schema version in the database (0 when it has never been migrated). */
  schemaVersion(): Promise<number>;
  /** Throws `SchemaVersionError` when the database is behind this package. */
  checkVersion(): Promise<void>;
  /**
   * The same store over `client`, for example to write consent inside your
   * own transaction. Multi-statement operations then use savepoints.
   */
  using(client: PgQueryable): PostgresStore;
}

export class SchemaVersionError extends Error {
  readonly found: number;
  readonly expected: number;

  constructor(found: number, expected: number, schema: string) {
    super(
      `the ${schema} schema is at version ${found}, and this package needs ${expected}; run store.migrate() or apply migrationFiles()`,
    );
    this.name = 'SchemaVersionError';
    this.found = found;
    this.expected = expected;
  }
}

const UNIQUE_VIOLATION = '23505';
const ACTIVE = `('${ACTIVE_STEP_STATES.join("', '")}')`;
const FINISHED = `('${[...TERMINAL_STEP_STATES, ...TERMINAL_DELIVERY_STATES].join("', '")}')`;

/**
 * A pool hands out clients. Clients have `connect()` too (to open their
 * connection), so a pool is recognized by its pool counters.
 */
function isPool(executor: PgQueryable): executor is PgPoolLike {
  return typeof (executor as Partial<PgPoolLike>).connect === 'function' && 'totalCount' in executor;
}

function code(error: unknown): string | undefined {
  return (error as { code?: string } | null)?.code;
}

/** JSON for a jsonb parameter: undefined fields become null so a merge can clear them. */
function json(value: unknown): string {
  return JSON.stringify(value, (_key, item) => (item === undefined ? null : typeof item === 'bigint' ? item.toString() : item));
}

/** Progress fields given as undefined or null are cleared; the rest are merged. */
function clearedKeys(progress: object): string[] {
  return Object.entries(progress)
    .filter(([, value]) => value === undefined || value === null)
    .map(([key]) => key);
}

function progressPatch(progress: object): object {
  return Object.fromEntries(Object.entries(progress).filter(([, value]) => value !== undefined && value !== null));
}

const num = (value: unknown): number => Number(value);
const numOrNull = (value: unknown): number | null => (value === null || value === undefined ? null : Number(value));

function rowToJob(row: any): Job {
  const base = {
    id: row.id as string,
    attempts: num(row.attempts),
    notBefore: num(row.not_before),
    lease: row.lease_worker === null ? null : { worker: row.lease_worker as string, until: num(row.lease_until) },
    createdAt: num(row.created_at),
    updatedAt: num(row.updated_at),
    outcome: row.outcome ?? null,
  };
  if (row.kind === 'delivery') {
    return { ...base, kind: 'delivery', state: row.state, delivery: row.data.delivery as Delivery };
  }
  const progress = Object.fromEntries(Object.entries(row.data.progress ?? {}).filter(([, value]) => value !== null));
  return {
    ...base,
    kind: 'step',
    state: row.state,
    deliveryId: row.data.deliveryId,
    deploymentId: row.data.deploymentId,
    request: row.data.request,
    progress: { generation: 0, ...progress } as StepJob['progress'],
  };
}

function rowToGrant(row: any): ConsentGrant {
  return { ...row.grant_json, steps: row.steps, revokedAt: numOrNull(row.revoked_at) };
}

function rowToSource(row: any): SourceState {
  return {
    source: row.source,
    key: row.key,
    hash: row.hash,
    value: row.value,
    generation: num(row.generation),
    pending: row.pending ?? null,
    observed: row.observed ?? null,
    version: num(row.version),
    updatedAt: num(row.updated_at),
  };
}

export function postgresStore(executor: PgQueryable, options: PostgresStoreOptions = {}): PostgresStore {
  const internal = options as InternalOptions;
  const name = validSchema(options.schema ?? DEFAULT_SCHEMA);
  const s = quoted(name);
  let checked: Promise<void> | null = null;

  /** Run `fn` atomically: a transaction on a pool, a savepoint on a client. */
  const atomic = async <T>(fn: (db: PgQueryable) => Promise<T>): Promise<T> => {
    if (!internal.client && isPool(executor)) {
      const client = await executor.connect();
      try {
        await client.query('BEGIN');
        const result = await fn(client);
        await client.query('COMMIT');
        return result;
      } catch (error) {
        await client.query('ROLLBACK').catch(() => undefined);
        throw error;
      } finally {
        client.release();
      }
    }
    const savepoint = `arete_${Math.random().toString(36).slice(2, 10)}`;
    await executor.query(`SAVEPOINT ${savepoint}`);
    try {
      const result = await fn(executor);
      await executor.query(`RELEASE SAVEPOINT ${savepoint}`);
      return result;
    } catch (error) {
      await executor.query(`ROLLBACK TO SAVEPOINT ${savepoint}`).catch(() => undefined);
      throw error;
    }
  };

  const schemaVersion = async (): Promise<number> => {
    // Look the table up first: querying a missing one would raise an error
    // that leaves a caller's transaction aborted.
    const { rows: found } = await executor.query('SELECT to_regclass($1) IS NOT NULL AS present', [`${s}.schema_migrations`]);
    if (!found[0]?.present) return 0;
    const { rows } = await executor.query(`SELECT coalesce(max(version), 0) AS version FROM ${s}.schema_migrations`);
    return num(rows[0]?.version ?? 0);
  };

  const checkVersion = async () => {
    const found = await schemaVersion();
    if (found < LATEST_SCHEMA_VERSION) throw new SchemaVersionError(found, LATEST_SCHEMA_VERSION, name);
  };

  /** Every operation waits for one successful version check. */
  const ready = async () => {
    checked ??= checkVersion().catch((error) => {
      checked = null;
      throw error;
    });
    await checked;
  };

  const acceptIn = async (db: PgQueryable, delivery: Delivery, now: number): Promise<boolean> => {
    const inserted = await db.query(
      `INSERT INTO ${s}.deliveries (id, accepted_at) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING`,
      [delivery.id, now],
    );
    if (inserted.rowCount !== 1) return false;
    // The delivery id may have been pruned while its job remains: a duplicate.
    const job = await db.query(
      `INSERT INTO ${s}.jobs (id, kind, state, data, not_before, created_at, updated_at)
       VALUES ($1, 'delivery', 'queued', $2::jsonb, $3, $3, $3)
       ON CONFLICT (id) DO NOTHING`,
      [delivery.id, json({ delivery }), now],
    );
    return job.rowCount === 1;
  };

  const claimOnce = async (db: PgQueryable, { worker, leaseMs, now }: { worker: string; leaseMs: number; now: number }) => {
    const { rows } = await db.query(
      `WITH candidate AS (
         SELECT j.id FROM ${s}.jobs j
         WHERE j.not_before <= $1
           AND (j.lease_until IS NULL OR j.lease_until <= $1)
           AND (
             (j.kind = 'delivery' AND j.state IN ('queued', 'running'))
             OR (j.kind = 'step' AND j.state IN ${ACTIVE})
             OR (j.kind = 'step' AND j.state = 'queued' AND NOT EXISTS (
                   SELECT 1 FROM ${s}.jobs a
                   WHERE a.kind = 'step' AND a.wallet = j.wallet AND a.id <> j.id AND a.state IN ${ACTIVE}))
           )
         ORDER BY j.not_before, j.seq
         LIMIT 1
         FOR UPDATE SKIP LOCKED
       )
       UPDATE ${s}.jobs j
       SET state = CASE WHEN j.state = 'queued' THEN 'running' ELSE j.state END,
           lease_worker = $2, lease_until = $3, updated_at = $1
       FROM candidate WHERE j.id = candidate.id
       RETURNING j.*`,
      [now, worker, now + leaseMs],
    );
    return rows[0] ? rowToJob(rows[0]) : null;
  };

  const store: PostgresStore = {
    schema: name,
    schemaVersion,
    checkVersion,
    using: (client) => postgresStore(client, { schema: name, client: true } as PostgresStoreOptions),

    async bindDeployment(deploymentId) {
      await ready();
      await executor.query(`INSERT INTO ${s}.meta (key, value) VALUES ('deployment', $1) ON CONFLICT (key) DO NOTHING`, [
        deploymentId,
      ]);
      const { rows } = await executor.query(`SELECT value FROM ${s}.meta WHERE key = 'deployment'`);
      const bound = rows[0]?.value as string;
      if (bound !== deploymentId) throw new StoreDeploymentError(bound, deploymentId);
    },

    async migrate() {
      return atomic(async (db) => {
        await db.query('SELECT pg_advisory_xact_lock(hashtext($1))', [`@usearete/automations:${name}`]);
        await db.query(`CREATE SCHEMA IF NOT EXISTS ${s}`);
        await db.query(
          `CREATE TABLE IF NOT EXISTS ${s}.schema_migrations (version integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())`,
        );
        const { rows } = await db.query(`SELECT coalesce(max(version), 0) AS version FROM ${s}.schema_migrations`);
        const from = num(rows[0].version);
        for (const migration of MIGRATIONS) {
          if (migration.version <= from) continue;
          await db.query(migration.sql(s));
          await db.query(`INSERT INTO ${s}.schema_migrations (version) VALUES ($1)`, [migration.version]);
        }
        checked = null;
        return { from, to: Math.max(from, LATEST_SCHEMA_VERSION) };
      });
    },

    deliveries: {
      async accept(delivery, now) {
        await ready();
        return atomic((db) => acceptIn(db, delivery, now));
      },
      async prune(before) {
        await ready();
        const result = await executor.query(`DELETE FROM ${s}.deliveries WHERE accepted_at < $1`, [before]);
        return result.rowCount ?? 0;
      },
    },

    jobs: {
      async get(id) {
        await ready();
        const { rows } = await executor.query(`SELECT * FROM ${s}.jobs WHERE id = $1`, [id]);
        return rows[0] ? rowToJob(rows[0]) : null;
      },
      async claimNext(input) {
        await ready();
        // A concurrent claim can start another job of the same wallet; the
        // unique index refuses the second, which then looks again.
        for (let attempt = 0; attempt < 10; attempt += 1) {
          try {
            // In its own transaction (a savepoint inside a caller's), so a lost
            // race never leaves the caller's transaction aborted.
            return await atomic((db) => claimOnce(db, input));
          } catch (error) {
            if (code(error) !== UNIQUE_VIOLATION) throw error;
          }
        }
        return null;
      },
      async transition(id, input) {
        await ready();
        try {
          return await atomic(async (db) => {
            const result = await db.query(
              `UPDATE ${s}.jobs SET
                 state = $4,
                 updated_at = $5,
                 lease_worker = CASE WHEN $6::bigint IS NULL THEN NULL ELSE $3 END,
                 lease_until = $6::bigint,
                 not_before = coalesce($7::bigint, not_before),
                 attempts = coalesce($8::integer, attempts),
                 outcome = CASE WHEN $9::boolean THEN $10::jsonb ELSE outcome END,
                 data = CASE WHEN $11::jsonb IS NULL THEN data
                   ELSE jsonb_set(data, '{progress}', (coalesce(data->'progress', '{}'::jsonb) || $11::jsonb) - $12::text[]) END
               WHERE id = $1 AND state = $2 AND lease_worker = $3`,
              [
                id,
                input.from,
                input.worker,
                input.to,
                input.now,
                input.keepLease?.until ?? null,
                input.notBefore ?? null,
                input.attempts ?? null,
                input.outcome !== undefined,
                input.outcome === undefined || input.outcome === null ? null : json(input.outcome),
                input.progress ? json(progressPatch(input.progress)) : null,
                input.progress ? clearedKeys(input.progress) : [],
              ],
            );
            if (result.rowCount !== 1) return false;
            if (input.ledger?.void) {
              await db.query(
                `UPDATE ${s}.ledger SET voided_at = $2
                 WHERE job_id = $1 AND voided_at IS NULL AND NOT (asset = ANY ($3::text[]))`,
                [id, input.now, [...(input.ledger.keepAssets ?? [])]],
              );
            }
            for (const entry of input.ledger?.record ?? []) {
              await db.query(
                `INSERT INTO ${s}.ledger (job_id, wallet, deployment_id, asset, amount, recorded_at)
                 VALUES ($1, $2, $3, $4, $5::numeric, $6)`,
                [id, entry.wallet, entry.deploymentId, entry.asset, entry.amount, input.now],
              );
            }
            return true;
          });
        } catch (error) {
          // Another job of the wallet is active.
          if (code(error) === UNIQUE_VIOLATION) return false;
          throw error;
        }
      },
      async renew(id, worker, until) {
        await ready();
        const result = await executor.query(
          `UPDATE ${s}.jobs SET lease_until = $3 WHERE id = $1 AND lease_worker = $2`,
          [id, worker, until],
        );
        return result.rowCount === 1;
      },
      async completeDelivery(id, worker, steps, now) {
        await ready();
        return atomic(async (db) => {
          const done = await db.query(
            `UPDATE ${s}.jobs SET state = 'done', lease_worker = NULL, lease_until = NULL, updated_at = $3, outcome = NULL
             WHERE id = $1 AND kind = 'delivery' AND state = 'running' AND lease_worker = $2`,
            [id, worker, now],
          );
          if (done.rowCount !== 1) return false;
          for (const step of steps) {
            await db.query(
              `INSERT INTO ${s}.jobs (id, kind, state, wallet, deployment_id, step, data, not_before, created_at, updated_at)
               VALUES ($1, 'step', 'queued', $2, $3, $4, $5::jsonb, $6, $6, $6)
               ON CONFLICT (id) DO NOTHING`,
              [
                step.id,
                step.request.wallet,
                step.deploymentId,
                step.request.step,
                json({ deliveryId: id, deploymentId: step.deploymentId, request: step.request, progress: { generation: 0 } }),
                now,
              ],
            );
          }
          return true;
        });
      },
      async lastCompleted({ wallet, deploymentId, step }) {
        await ready();
        const { rows } = await executor.query(
          `SELECT max(updated_at) AS at FROM ${s}.jobs
           WHERE kind = 'step' AND state = 'completed' AND wallet = $1 AND deployment_id = $2 AND step = $3`,
          [wallet, deploymentId, step],
        );
        return numOrNull(rows[0]?.at);
      },
      async list(filter = {}) {
        await ready();
        const { rows } = await executor.query(
          `SELECT * FROM ${s}.jobs WHERE ($1::text IS NULL OR kind = $1) AND ($2::text IS NULL OR state = $2) ORDER BY seq`,
          [filter.kind ?? null, filter.state ?? null],
        );
        return rows.map(rowToJob);
      },
      async prune(before) {
        await ready();
        const result = await executor.query(`DELETE FROM ${s}.jobs WHERE state IN ${FINISHED} AND updated_at < $1`, [before]);
        return result.rowCount ?? 0;
      },
    },

    consent: {
      async put(grant) {
        await ready();
        await executor.query(
          `INSERT INTO ${s}.consents (wallet, deployment_id, instance_id, steps, expires_at, revoked_at, grant_json)
           VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb)
           ON CONFLICT (wallet, deployment_id, instance_id) DO UPDATE SET
             steps = excluded.steps, expires_at = excluded.expires_at,
             revoked_at = excluded.revoked_at, grant_json = excluded.grant_json`,
          [
            grant.wallet,
            grant.deploymentId,
            grant.instanceId ?? '',
            [...grant.steps],
            grant.expiresAt,
            grant.revokedAt,
            json(grant),
          ],
        );
      },
      async find({ wallet, deploymentId, step, instanceId, now }) {
        await ready();
        const { rows } = await executor.query(
          `SELECT * FROM ${s}.consents
           WHERE wallet = $1 AND deployment_id = $2 AND (instance_id = '' OR instance_id = $3)
             AND $4 = ANY (steps) AND revoked_at IS NULL AND expires_at > $5
           ORDER BY instance_id <> '' DESC
           LIMIT 1`,
          [wallet, deploymentId, instanceId ?? '', step, now],
        );
        return rows[0] ? rowToGrant(rows[0]) : null;
      },
      async revoke({ wallet, deploymentId, steps, instanceId, now }) {
        await ready();
        return atomic(async (db) => {
          const { rows } = await db.query(
            `SELECT * FROM ${s}.consents
             WHERE wallet = $1 AND deployment_id = $2 AND revoked_at IS NULL AND ($3::text IS NULL OR instance_id = $3)
             FOR UPDATE`,
            [wallet, deploymentId, instanceId === undefined ? null : instanceId],
          );
          let changed = 0;
          for (const row of rows) {
            const current: string[] = row.steps;
            if (steps && !current.some((step) => steps.includes(step))) continue;
            const remaining = steps ? current.filter((step) => !steps.includes(step)) : [];
            const grant = { ...row.grant_json, steps: remaining.length > 0 ? remaining : current };
            await db.query(
              `UPDATE ${s}.consents SET steps = $4, revoked_at = $5, grant_json = $6::jsonb
               WHERE wallet = $1 AND deployment_id = $2 AND instance_id = $3`,
              [
                row.wallet,
                row.deployment_id,
                row.instance_id,
                remaining.length > 0 ? remaining : current,
                remaining.length > 0 ? null : now,
                json(grant),
              ],
            );
            changed += 1;
          }
          return changed;
        });
      },
      async list({ wallet }) {
        await ready();
        const { rows } = await executor.query(
          `SELECT * FROM ${s}.consents WHERE wallet = $1 ORDER BY deployment_id, instance_id`,
          [wallet],
        );
        return rows.map(rowToGrant);
      },
      async active({ deploymentId, step, now }) {
        await ready();
        const { rows } = await executor.query(
          `SELECT * FROM ${s}.consents
           WHERE deployment_id = $1 AND revoked_at IS NULL AND expires_at > $2 AND ($3::text IS NULL OR $3 = ANY (steps))
           ORDER BY wallet, instance_id`,
          [deploymentId, now, step ?? null],
        );
        return rows.map(rowToGrant);
      },
      async issueNonce(nonce, expiresAt) {
        await ready();
        await executor.query(
          `INSERT INTO ${s}.consent_nonces (nonce, expires_at) VALUES ($1, $2) ON CONFLICT (nonce) DO NOTHING`,
          [nonce, expiresAt],
        );
      },
      async useNonce(nonce, now) {
        await ready();
        const result = await executor.query(
          `UPDATE ${s}.consent_nonces SET used_at = $2 WHERE nonce = $1 AND used_at IS NULL AND expires_at > $2`,
          [nonce, now],
        );
        return result.rowCount === 1;
      },
    },

    sources: {
      async get(source, key) {
        await ready();
        const { rows } = await executor.query(`SELECT * FROM ${s}.source_state WHERE source = $1 AND key = $2`, [source, key]);
        return rows[0] ? rowToSource(rows[0]) : null;
      },
      async advance({ source, key, expectedVersion, next, deliver, now }) {
        await ready();
        try {
          return await atomic(async (db) => {
            let row: any;
            if (expectedVersion === null) {
              const inserted = await db.query(
                `INSERT INTO ${s}.source_state (source, key, hash, value, generation, pending, observed, version, updated_at)
                 VALUES ($1, $2, $3, $4::jsonb, 1, $5::jsonb, $7, 1, $6)
                 ON CONFLICT (source, key) DO NOTHING
                 RETURNING *`,
                [source, key, next.hash, json(next.value), next.pending ? json(next.pending) : null, now, next.observed ?? null],
              );
              row = inserted.rows[0];
            } else {
              const updated = await db.query(
                `UPDATE ${s}.source_state SET
                   generation = generation + CASE WHEN hash = $3 THEN 0 ELSE 1 END,
                   hash = $3, value = $4::jsonb, pending = $5::jsonb, observed = $8, version = version + 1, updated_at = $6
                 WHERE source = $1 AND key = $2 AND version = $7
                 RETURNING *`,
                [source, key, next.hash, json(next.value), next.pending ? json(next.pending) : null, now, expectedVersion, next.observed ?? null],
              );
              row = updated.rows[0];
            }
            if (!row) return null;
            const state = rowToSource(row);
            if (deliver) {
              await acceptIn(
                db,
                {
                  id: `view:${encodeURIComponent(source)}:${encodeURIComponent(key)}:${state.generation}`,
                  name: deliver.name,
                  source: 'view',
                  payload: deliver.payload,
                  receivedAt: now,
                },
                now,
              );
            }
            return state;
          });
        } catch (error) {
          if (code(error) === UNIQUE_VIOLATION) return null;
          throw error;
        }
      },
      async cursor(source) {
        await ready();
        const { rows } = await executor.query(`SELECT cursor FROM ${s}.source_cursors WHERE source = $1`, [source]);
        return rows[0]?.cursor ?? null;
      },
      async putCursor(source, cursor, now) {
        await ready();
        const order = parseCursor(cursor);
        if (!order) throw new TypeError(`'${cursor}' is not an {epoch}:{offset} cursor`);
        // Only forward, atomically: a slow replica never rewinds the source.
        await executor.query(
          `INSERT INTO ${s}.source_cursors (source, cursor, updated_at) VALUES ($1, $2, $3)
           ON CONFLICT (source) DO UPDATE SET cursor = excluded.cursor, updated_at = excluded.updated_at
           WHERE ${s}.source_cursors.cursor !~ '^[0-9]+:[0-9]+$'
              OR (split_part(${s}.source_cursors.cursor, ':', 1)::numeric, split_part(${s}.source_cursors.cursor, ':', 2)::numeric)
                 < ($4::numeric, $5::numeric)`,
          [source, cursor, now, order[0].toString(), order[1].toString()],
        );
      },
      async acquireLease(source, holder, until, now) {
        await ready();
        const { rows } = await executor.query(
          `INSERT INTO ${s}.source_leases (source, holder, until) VALUES ($1, $2, $3)
           ON CONFLICT (source) DO UPDATE SET holder = excluded.holder, until = excluded.until
             WHERE ${s}.source_leases.holder = excluded.holder OR ${s}.source_leases.until <= $4
           RETURNING holder`,
          [source, holder, until, now],
        );
        return rows.length === 1;
      },
      async releaseLease(source, holder) {
        await ready();
        await executor.query(`DELETE FROM ${s}.source_leases WHERE source = $1 AND holder = $2`, [source, holder]);
      },
    },

    ledger: {
      async spent({ wallet, deploymentId, asset, since }) {
        await ready();
        const { rows } = await executor.query(
          `SELECT coalesce(sum(amount), 0)::text AS total FROM ${s}.ledger
           WHERE wallet = $1 AND deployment_id = $2 AND asset = $3 AND voided_at IS NULL AND recorded_at >= $4`,
          [wallet, deploymentId, asset, since],
        );
        return BigInt(rows[0]?.total ?? '0');
      },
      async list({ wallet }) {
        await ready();
        const { rows } = await executor.query(`SELECT * FROM ${s}.ledger WHERE wallet = $1 ORDER BY id`, [wallet]);
        return rows.map(
          (row): LedgerEntry => ({
            jobId: row.job_id,
            wallet: row.wallet,
            deploymentId: row.deployment_id,
            asset: row.asset,
            amount: String(row.amount),
            recordedAt: num(row.recorded_at),
            voidedAt: numOrNull(row.voided_at),
          }),
        );
      },
    },
  };
  return store;
}
