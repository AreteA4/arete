import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

import { DEFAULT_SCHEMA, migrationFiles, runtimeRoleGrants } from './postgres/migrations';
import { add, configuredEntry, isTarget } from './scaffold/add';
import { TARGETS } from './scaffold/templates';

const USAGE = `arete-automations: scaffold, deploy and run @usearete/automations

Usage:
  arete-automations add <cloudflare|vercel|node> [--signer keypair|privy] [--entry <module>] [--dir <project>]
      Write a host's entry and config (and runtime.ts, the production wiring, when missing)
      beside the project's definition, and print the steps to deploy it. Never overwrites.
  arete-automations manifest [<module>] [--dir <project>]
      Print what the definition registers: the protocol events a webhook endpoint needs,
      schedules and view sources (JSON). Needs tsx for a TypeScript module.
  arete-automations migrate [--schema <name>]
      Apply the Postgres store's migrations to DATABASE_URL (needs pg installed in the project).
  arete-automations migrations --out <dir> [--schema <name>]
      Write the migrations as numbered SQL files for your own migration tool.
  arete-automations grants --role <name> [--schema <name>]
      Print the grants for a runtime role that reads and writes rows but cannot change the schema.
  arete-automations dev
      Run the project's automations against its dev environment. \`a4 dev up\` runs it for
      [dev.automations] in arete.toml, with its settings in the environment.

The schema defaults to ${DEFAULT_SCHEMA}.`;

/** An option's value; throws when the option is given without one. */
function option(args: string[], name: string): string | undefined {
  const index = args.indexOf(name);
  if (index < 0) return undefined;
  const value = args[index + 1];
  if (value === undefined || value.startsWith('--')) throw new Error(`${name} needs a value`);
  return value;
}

type Out = { log(text: string): void; error(text: string): void };

/** `arete-automations add`. */
function addCommand(args: string[], out: Out): number {
  const target = args[1];
  if (!isTarget(target)) throw new Error(`add needs a target: ${TARGETS.join(', ')}`);
  const signer = option(args, '--signer') ?? 'keypair';
  if (signer !== 'keypair' && signer !== 'privy') throw new Error('--signer is keypair or privy');
  const root = resolve(option(args, '--dir') ?? '.');
  const entry = option(args, '--entry');
  const result = add({ root, target, signer, ...(entry ? { entry } : {}) });
  for (const file of result.written) out.log(`wrote ${file}`);
  for (const file of result.kept) out.log(`kept ${file} (it exists; compare it with the template if you need to merge)`);
  out.log('');
  out.log('Next:');
  for (const step of result.next) out.log(`  ${step}`);
  for (const note of result.notes) out.log(`\nNote: ${note}`);
  return 0;
}

/** Run the CLI; returns the exit code. */
export function main(args: string[], out: { log(text: string): void; error(text: string): void } = console): number {
  const [command] = args;
  try {
    const schema = option(args, '--schema') ?? DEFAULT_SCHEMA;
    if (command === 'migrations') {
      const dir = option(args, '--out');
      if (!dir) throw new Error('migrations needs --out <dir>');
      mkdirSync(dir, { recursive: true });
      for (const file of migrationFiles(schema)) {
        writeFileSync(join(dir, file.name), file.sql);
        out.log(join(dir, file.name));
      }
      return 0;
    }
    if (command === 'add') return addCommand(args, out);
    if (command === 'grants') {
      const role = option(args, '--role');
      if (!role) throw new Error('grants needs --role <name>');
      out.log(runtimeRoleGrants(role, schema).trimEnd());
      return 0;
    }
    out.error(USAGE);
    return command === undefined || command === '--help' || command === '-h' ? 0 : 2;
  } catch (error) {
    out.error(`arete-automations: ${(error as Error).message}`);
    return 1;
  }
}

/** `arete-automations dev`: run until SIGINT or SIGTERM. */
export async function dev(out: { log(text: string): void; error(text: string): void } = console): Promise<number> {
  const { configFromEnv, runDev } = await import('./dev/runner');
  let runner;
  try {
    runner = await runDev(configFromEnv(process.env), (line) => out.log(line));
  } catch (error) {
    out.error(`arete-automations: ${(error as Error).message}`);
    return 1;
  }
  await new Promise<void>((done) => {
    process.once('SIGINT', done);
    process.once('SIGTERM', done);
  });
  await runner.stop();
  return 0;
}

/** `arete-automations manifest`: load the definition and describe it. */
export async function manifest(args: string[], out: Out = console): Promise<number> {
  try {
    const root = resolve(option(args, '--dir') ?? '.');
    const positional = args[1] !== undefined && !args[1].startsWith('--') ? args[1] : undefined;
    const entry = positional ?? configuredEntry(root) ?? 'automations/index.ts';
    const { loadDefinition } = await import('./dev/runner');
    const { describeDefinition } = await import('./runtime/define');
    out.log(JSON.stringify(await describeDefinition(await loadDefinition(resolve(root, entry))), null, 2));
    return 0;
  } catch (error) {
    out.error(`arete-automations: ${(error as Error).message}`);
    return 1;
  }
}

/** `arete-automations migrate`: the store's migrations, on DATABASE_URL, with the project's pg. */
export async function migrate(args: string[], out: Out = console): Promise<number> {
  try {
    const url = process.env.DATABASE_URL;
    if (!url) throw new Error('set DATABASE_URL to the database to migrate (with a role that may create tables)');
    const schema = option(args, '--schema') ?? DEFAULT_SCHEMA;
    const { createRequire } = await import('node:module');
    const { pathToFileURL } = await import('node:url');
    let pgPath: string;
    try {
      pgPath = createRequire(join(process.cwd(), 'package.json')).resolve('pg');
    } catch {
      throw new Error('pg is not installed in this project: npm install pg');
    }
    const pg = (await import(pathToFileURL(pgPath).href)) as { default?: { Pool: new (config: object) => PgPool }; Pool?: new (config: object) => PgPool };
    const Pool = pg.default?.Pool ?? pg.Pool!;
    const pool = new Pool({ connectionString: url });
    try {
      const { postgresStore } = await import('./postgres/store');
      const { from, to } = await postgresStore(pool as never, { schema }).migrate();
      out.log(from === to ? `${schema} is up to date (version ${to})` : `migrated ${schema} from version ${from} to ${to}`);
    } finally {
      await pool.end();
    }
    return 0;
  } catch (error) {
    out.error(`arete-automations: ${(error as Error).message}`);
    return 1;
  }
}

interface PgPool {
  end(): Promise<void>;
}
