import { parseArgs } from 'node:util';
import { resolveA4Version, type FetchLatest, type ResolvedA4Version } from '../a4-version.js';
import { expandSweep, loadConfigFile, RunConfigSchema, SweepConfigSchema, type SweepConfig } from '../config.js';
import type { RunConfig } from '../types.js';

export const USAGE = `Usage:
  npm run bench -- <config.json>
  npm run bench -- --task <task.ts> --harness <claude-code|codex|opencode> --model <gateway-model-id>
                   [--repetitions N] [--concurrency N] [--effort high] [--key-mode pool|signup|fresh]
                   [--a4-version latest|<x.y.z>] [--model-auth auto|ai-gateway|direct] [--skip-preflight]
  npm run preflight -- [same arguments]   (defaults to configs/smoke.json)`;

/** Parse a config path or inline flags into a sweep, shared by `bench` and `preflight`. */
export function sweepFromArgs(argv: string[], defaultConfig?: string): { sweep: SweepConfig; skipPreflight: boolean } {
  const { values, positionals } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      task: { type: 'string', multiple: true },
      harness: { type: 'string' },
      model: { type: 'string' },
      effort: { type: 'string' },
      repetitions: { type: 'string' },
      concurrency: { type: 'string' },
      'key-mode': { type: 'string' },
      'model-auth': { type: 'string' },
      'a4-version': { type: 'string' },
      'skip-preflight': { type: 'boolean', default: false },
      help: { type: 'boolean', short: 'h' },
    },
  });
  if (values.help) {
    process.stdout.write(`${USAGE}\n`);
    process.exit(0);
  }
  const skipPreflight = values['skip-preflight'] ?? false;
  const overrides = {
    ...(values.repetitions ? { repetitions: Number(values.repetitions) } : {}),
    ...(values.concurrency ? { concurrency: Number(values.concurrency) } : {}),
    ...(values['key-mode'] ? { keyMode: values['key-mode'] } : {}),
    ...(values['model-auth'] ? { modelAuth: values['model-auth'] } : {}),
    ...(values['a4-version'] ? { a4Version: values['a4-version'] } : {}),
  };
  const configPath = positionals[0] ?? (values.task ? undefined : defaultConfig);
  if (configPath) {
    return { sweep: SweepConfigSchema.parse({ ...loadConfigFile(configPath), ...overrides }), skipPreflight };
  }
  if (!values.task || !values.harness || !values.model) throw new Error(USAGE);
  const run = RunConfigSchema.parse({
    task: values.task[0],
    harness: values.harness,
    model: values.model,
    effort: values.effort,
  });
  const sweep = SweepConfigSchema.parse({
    ...run,
    agents: [{ harness: run.harness, model: run.model, effort: run.effort }],
    tasks: values.task,
    ...overrides,
  });
  return { sweep, skipPreflight };
}

export interface PreparedSweep {
  sweep: SweepConfig;
  skipPreflight: boolean;
  /** The one `a4` release every run installs, and where it came from. */
  a4: ResolvedA4Version;
  runs: RunConfig[];
}

/** Parse arguments, resolve `a4Version` once, and expand the sweep into runs. */
export async function prepareSweep(
  argv: string[],
  defaultConfig?: string,
  fetchLatest?: FetchLatest,
): Promise<PreparedSweep> {
  const { sweep, skipPreflight } = sweepFromArgs(argv, defaultConfig);
  const a4 = await resolveA4Version(sweep.a4Version, fetchLatest);
  return { sweep, skipPreflight, a4, runs: expandSweep(sweep, a4) };
}
