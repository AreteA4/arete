/**
 * Versioned SQL migrations for the Postgres store. Every object lives in
 * one schema (`arete_automation` by default); nothing outside it is
 * created, altered or read. Times are milliseconds since the epoch.
 */

export interface Migration {
  readonly version: number;
  readonly name: string;
  /** The SQL, for the given schema. */
  sql(schema: string): string;
}

const ACTIVE_STEP_STATES = "('running', 'signed', 'submitted', 'submitted_unknown')";

export const MIGRATIONS: readonly Migration[] = [
  {
    version: 1,
    name: 'initial',
    sql: (s) => `
CREATE TABLE ${s}.deliveries (
  id text PRIMARY KEY,
  accepted_at bigint NOT NULL
);

CREATE TABLE ${s}.jobs (
  id text PRIMARY KEY,
  seq bigserial NOT NULL,
  kind text NOT NULL CHECK (kind IN ('delivery', 'step')),
  state text NOT NULL,
  wallet text,
  deployment_id text,
  step text,
  data jsonb NOT NULL,
  outcome jsonb,
  attempts integer NOT NULL DEFAULT 0,
  not_before bigint NOT NULL,
  lease_worker text,
  lease_until bigint,
  created_at bigint NOT NULL,
  updated_at bigint NOT NULL,
  CHECK (kind = 'delivery' OR wallet IS NOT NULL)
);

-- At most one active step job per wallet: the store's per-wallet serialization.
CREATE UNIQUE INDEX jobs_one_active_step_per_wallet ON ${s}.jobs (wallet)
  WHERE kind = 'step' AND state IN ${ACTIVE_STEP_STATES};

CREATE INDEX jobs_claimable ON ${s}.jobs (not_before, seq)
  WHERE state IN ('queued', 'running', 'signed', 'submitted', 'submitted_unknown');

CREATE INDEX jobs_completed_steps ON ${s}.jobs (wallet, deployment_id, step, updated_at)
  WHERE kind = 'step' AND state = 'completed';

CREATE INDEX jobs_finished ON ${s}.jobs (updated_at)
  WHERE state IN ('done', 'failed', 'completed', 'chain_failed', 'expired', 'skipped', 'rejected', 'refused');

CREATE TABLE ${s}.consents (
  wallet text NOT NULL,
  deployment_id text NOT NULL,
  -- '' for a grant that covers every instance.
  instance_id text NOT NULL,
  steps text[] NOT NULL,
  expires_at bigint NOT NULL,
  revoked_at bigint,
  grant_json jsonb NOT NULL,
  PRIMARY KEY (wallet, deployment_id, instance_id)
);

CREATE INDEX consents_active ON ${s}.consents (deployment_id, expires_at) WHERE revoked_at IS NULL;

CREATE TABLE ${s}.consent_nonces (
  nonce text PRIMARY KEY,
  expires_at bigint NOT NULL,
  used_at bigint
);

CREATE TABLE ${s}.ledger (
  id bigserial PRIMARY KEY,
  job_id text NOT NULL,
  wallet text NOT NULL,
  deployment_id text NOT NULL,
  asset text NOT NULL,
  amount numeric(39, 0) NOT NULL CHECK (amount >= 0),
  recorded_at bigint NOT NULL,
  voided_at bigint
);

CREATE INDEX ledger_window ON ${s}.ledger (wallet, deployment_id, asset, recorded_at) WHERE voided_at IS NULL;
CREATE INDEX ledger_job ON ${s}.ledger (job_id);

CREATE TABLE ${s}.source_state (
  source text NOT NULL,
  key text NOT NULL,
  hash text NOT NULL,
  value jsonb,
  generation integer NOT NULL,
  pending jsonb,
  version integer NOT NULL,
  updated_at bigint NOT NULL,
  PRIMARY KEY (source, key)
);

CREATE TABLE ${s}.source_cursors (
  source text PRIMARY KEY,
  cursor text NOT NULL,
  updated_at bigint NOT NULL
);

CREATE TABLE ${s}.source_leases (
  source text PRIMARY KEY,
  holder text NOT NULL,
  until bigint NOT NULL
);
`,
  },
];

export const LATEST_SCHEMA_VERSION = MIGRATIONS[MIGRATIONS.length - 1]!.version;
export const DEFAULT_SCHEMA = 'arete_automation';

/** A schema name used unquoted in SQL: lower case, digits and underscores. */
export function validSchema(schema: string): string {
  if (!/^[a-z_][a-z0-9_]{0,62}$/.test(schema)) {
    throw new TypeError(`'${schema}' is not a valid schema name (lower case letters, digits and underscores)`);
  }
  return schema;
}

/**
 * The migrations as files for your own migration tool (Flyway, Prisma,
 * Drizzle, Knex, Atlas): `0001_initial.sql` and so on. Each creates the
 * schema if needed and records its version, as `migrate()` does.
 */
export function migrationFiles(schema: string = DEFAULT_SCHEMA): Array<{ name: string; sql: string }> {
  const s = validSchema(schema);
  return MIGRATIONS.map((migration) => ({
    name: `${String(migration.version).padStart(4, '0')}_${migration.name}.sql`,
    sql: [
      `-- @usearete/automations migration ${migration.version}: ${migration.name}`,
      `CREATE SCHEMA IF NOT EXISTS ${s};`,
      `CREATE TABLE IF NOT EXISTS ${s}.schema_migrations (version integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now());`,
      migration.sql(s).trim(),
      `INSERT INTO ${s}.schema_migrations (version) VALUES (${migration.version});`,
      '',
    ].join('\n'),
  }));
}

/**
 * Grants for the runtime role: read and write rows, no DDL. Run as the role
 * that owns the schema, after migrating.
 */
export function runtimeRoleGrants(role: string, schema: string = DEFAULT_SCHEMA): string {
  const s = validSchema(schema);
  if (!/^[a-z_][a-z0-9_]{0,62}$/.test(role)) throw new TypeError(`'${role}' is not a valid role name`);
  return [
    `GRANT USAGE ON SCHEMA ${s} TO ${role};`,
    `GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA ${s} TO ${role};`,
    `GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA ${s} TO ${role};`,
    `REVOKE CREATE ON SCHEMA ${s} FROM ${role};`,
    '',
  ].join('\n');
}
