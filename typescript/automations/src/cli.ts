import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { DEFAULT_SCHEMA, migrationFiles, runtimeRoleGrants } from './postgres/migrations';

const USAGE = `arete-automations: SQL for the @usearete/automations Postgres store

Usage:
  arete-automations migrations --out <dir> [--schema <name>]
      Write the migrations as numbered SQL files for your own migration tool.
  arete-automations grants --role <name> [--schema <name>]
      Print the grants for a runtime role that reads and writes rows but cannot change the schema.

The schema defaults to ${DEFAULT_SCHEMA}.`;

function option(args: string[], name: string): string | undefined {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
}

/** Run the CLI; returns the exit code. */
export function main(args: string[], out: { log(text: string): void; error(text: string): void } = console): number {
  const [command] = args;
  const schema = option(args, '--schema') ?? DEFAULT_SCHEMA;
  try {
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
