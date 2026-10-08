import { parseArgs } from 'node:util';
import { expandSweep, loadConfigFile, RunConfigSchema, SweepConfigSchema, type SweepConfig } from '../config.js';
import { assertCredentials, resultsDir } from '../env.js';
import { KeyPool } from '../keys.js';
import { loadTask, runOne } from '../run-one.js';
import type { RunReport, TaskDefinition } from '../types.js';

const USAGE = `Usage:
  npm run bench -- <config.json>
  npm run bench -- --task <task.ts> --harness <claude-code|codex|opencode> --model <gateway-model-id>
                   [--repetitions N] [--concurrency N] [--effort high] [--key-mode pool|signup]
                   [--model-auth ai-gateway|direct]`;

function sweepFromArgs(argv: string[]): SweepConfig {
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
      help: { type: 'boolean', short: 'h' },
    },
  });
  if (values.help) {
    process.stdout.write(`${USAGE}\n`);
    process.exit(0);
  }
  const overrides = {
    ...(values.repetitions ? { repetitions: Number(values.repetitions) } : {}),
    ...(values.concurrency ? { concurrency: Number(values.concurrency) } : {}),
    ...(values['key-mode'] ? { keyMode: values['key-mode'] } : {}),
    ...(values['model-auth'] ? { modelAuth: values['model-auth'] } : {}),
    ...(values['a4-version'] ? { a4Version: values['a4-version'] } : {}),
  };
  if (positionals[0]) {
    return SweepConfigSchema.parse({ ...loadConfigFile(positionals[0]), ...overrides });
  }
  if (!values.task || !values.harness || !values.model) throw new Error(USAGE);
  const run = RunConfigSchema.parse({
    task: values.task[0],
    harness: values.harness,
    model: values.model,
    effort: values.effort,
  });
  return SweepConfigSchema.parse({
    ...run,
    agents: [{ harness: run.harness, model: run.model, effort: run.effort }],
    tasks: values.task,
    ...overrides,
  });
}

/** Run promise-returning jobs with at most `limit` in flight. */
async function pool<T>(jobs: Array<() => Promise<T>>, limit: number): Promise<PromiseSettledResult<T>[]> {
  const results: PromiseSettledResult<T>[] = new Array(jobs.length);
  let next = 0;
  const worker = async () => {
    while (next < jobs.length) {
      const index = next++;
      try {
        results[index] = { status: 'fulfilled', value: await jobs[index]!() };
      } catch (reason) {
        results[index] = { status: 'rejected', reason };
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, jobs.length) }, worker));
  return results;
}

async function main(): Promise<void> {
  const sweep = sweepFromArgs(process.argv.slice(2));
  assertCredentials(sweep.modelAuth);
  const runs = expandSweep(sweep);
  const tasks = new Map<string, TaskDefinition>();
  for (const ref of new Set(sweep.tasks)) tasks.set(ref, await loadTask(ref));

  const keyPool = new KeyPool();
  if (sweep.keyMode === 'pool' && keyPool.size === 0) {
    process.stdout.write('note: ARETE_AGENT_KEYS is empty; runs fall back to `auth signup --if-missing` (5/hour/IP).\n');
  }
  process.stdout.write(
    `${runs.length} run(s): ${sweep.agents.length} agent(s) × ${sweep.tasks.length} task(s) × ${sweep.repetitions} rep(s), concurrency ${sweep.concurrency}\n` +
      `results → ${resultsDir()}\n`,
  );

  const log = (line: string) => process.stdout.write(`${line}\n`);
  const settled = await pool(
    runs.map((config) => () => runOne(config, tasks.get(config.task)!, { keyPool, log })),
    sweep.concurrency,
  );

  const reports = settled.flatMap((r) => (r.status === 'fulfilled' ? [r.value] : []));
  const crashed = settled.filter((r): r is PromiseRejectedResult => r.status === 'rejected');
  for (const c of crashed) process.stderr.write(`run crashed: ${String(c.reason?.stack ?? c.reason)}\n`);

  const line = (r: RunReport) =>
    `  ${r.verification.passed ? 'PASS' : 'FAIL'}  ${r.task.name.padEnd(24)} ${r.config.harness.padEnd(12)} ${(r.config.label ?? r.config.model).padEnd(32)} ${(r.timing.totalMs / 1000).toFixed(0).padStart(5)}s  $${r.cost.modelUsd.toFixed(3)}  ${r.status}`;
  process.stdout.write(`\nDone: ${reports.filter((r) => r.verification.passed).length}/${runs.length} passed\n${reports.map(line).join('\n')}\n`);
  process.stdout.write(`\nCompare: npm run compare\n`);
  if (crashed.length) process.exitCode = 1;
}

main().catch((err) => {
  process.stderr.write(`Fatal: ${err instanceof Error ? err.message : String(err)}\n`);
  process.exit(1);
});
