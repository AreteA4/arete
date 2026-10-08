import { sharedAgentKey } from '../agent-key.js';
import { expandSweep } from '../config.js';
import { agentKeys, assertCredentials, resolveModelAuth, resultsDir } from '../env.js';
import { KeyPool } from '../keys.js';
import { formatPreflight, preflight } from '../preflight.js';
import { loadTask, runOne } from '../run-one.js';
import type { RunReport, TaskDefinition } from '../types.js';
import { sweepFromArgs } from './args.js';

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
  const { sweep, skipPreflight } = sweepFromArgs(process.argv.slice(2));
  assertCredentials(resolveModelAuth(sweep.modelAuth));
  const runs = expandSweep(sweep);
  const tasks = new Map<string, TaskDefinition>();
  for (const ref of new Set(sweep.tasks)) tasks.set(ref, await loadTask(ref));
  const log = (line: string) => process.stdout.write(`${line}\n`);

  if (!skipPreflight) {
    const result = await preflight(runs, tasks);
    log(`${formatPreflight(result)}\n`);
    if (!result.ok) throw new Error('preflight failed; fix the ✗ items above (or pass --skip-preflight)');
  }

  // Without configured keys, every run shares one benchmark agent.
  const keys = sweep.keyMode === 'pool' && agentKeys().length === 0 ? [await sharedAgentKey(runs[0]!, log)] : agentKeys();
  const keyPool = new KeyPool(keys);
  process.stdout.write(
    `${runs.length} run(s): ${sweep.agents.length} agent(s) × ${sweep.tasks.length} task(s) × ${sweep.repetitions} rep(s), concurrency ${sweep.concurrency}\n` +
      `results → ${resultsDir()}\n`,
  );

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
