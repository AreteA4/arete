import { sharedAgentKey } from '../agent-key.js';
import { agentKeys, assertCredentials, resolveModelAuth, resultsDir } from '../env.js';
import { KeyPool } from '../keys.js';
import { formatPreflight, preflight } from '../preflight.js';
import { print } from '../report.js';
import { loadTask, prewarmTemplate, runOne } from '../run-one.js';
import type { RunReport, TaskDefinition } from '../types.js';
import { prepareSweep } from './args.js';

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
  const { sweep, skipPreflight, a4, runs } = await prepareSweep(process.argv.slice(2));
  assertCredentials(resolveModelAuth(sweep.modelAuth));
  const tasks = new Map<string, TaskDefinition>();
  for (const ref of new Set(sweep.tasks)) tasks.set(ref, await loadTask(ref));
  const log = (line: string) => print(`${line}\n`);

  if (!skipPreflight) {
    const result = await preflight(runs, tasks, { a4 });
    log(`${formatPreflight(result)}\n`);
    if (!result.ok) throw new Error('preflight failed; fix the ✗ items above (or pass --skip-preflight)');
  }

  // Without configured keys, every run shares one benchmark agent. Fresh
  // runs get no key at all: the agent must create its own account.
  const keys =
    sweep.keyMode === 'fresh'
      ? []
      : sweep.keyMode === 'pool' && agentKeys().length === 0
        ? [await sharedAgentKey(runs[0]!, log)]
        : agentKeys();
  const keyPool = new KeyPool(keys);
  print(
    `${runs.length} run(s): ${sweep.agents.length} agent(s) × ${sweep.tasks.length} task(s) × ${sweep.repetitions} rep(s), concurrency ${sweep.concurrency}\n` +
      `a4 ${a4.version} (${a4.source === 'latest' ? 'latest release' : 'pinned'}), key mode ${sweep.keyMode}\n` +
      `results → ${resultsDir()}\n`,
  );

  // One image build per harness up front; concurrent first builds would race.
  const firstPerHarness = [...new Map(runs.map((r) => [r.harness, r])).values()];
  log(`preparing sandbox images: ${firstPerHarness.map((r) => r.harness).join(', ')}`);
  await Promise.all(firstPerHarness.map((config) => prewarmTemplate(config)));

  const settled = await pool(
    runs.map((config) => () => runOne(config, tasks.get(config.task)!, { keyPool, log })),
    sweep.concurrency,
  );

  const reports = settled.flatMap((r) => (r.status === 'fulfilled' ? [r.value] : []));
  const crashed = settled.filter((r): r is PromiseRejectedResult => r.status === 'rejected');
  for (const c of crashed) print(`run crashed: ${String(c.reason?.stack ?? c.reason)}\n`, process.stderr);

  const line = (r: RunReport) =>
    `  ${r.verification.passed ? 'PASS' : 'FAIL'}  ${r.task.name.padEnd(24)} ${r.config.harness.padEnd(12)} ${(r.config.label ?? r.config.model).padEnd(32)} ${(r.timing.totalMs / 1000).toFixed(0).padStart(5)}s  $${r.cost.modelUsd.toFixed(3)}  ${r.status}`;
  print(`\nDone: ${reports.filter((r) => r.verification.passed).length}/${runs.length} passed\n${reports.map(line).join('\n')}\n`);
  print(`\nCompare: npm run compare\n`);
  // runOne() returns setup and infrastructure failures as reports, so they
  // fail the sweep here; agent failures are results, not errors.
  const broken = reports.filter((r) => r.status === 'infra-error' || r.status === 'setup-error');
  if (crashed.length || broken.length) process.exitCode = 1;
}

main().catch((err) => {
  print(`Fatal: ${err instanceof Error ? err.message : String(err)}\n`, process.stderr);
  process.exit(1);
});
