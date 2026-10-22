import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { DEFAULT_SCHEMA, migrationFiles, runtimeRoleGrants } from './postgres/migrations';

const USAGE = `arete-automations: SQL for the @usearete/automations Postgres store, and the a4 dev runner

Usage:
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
