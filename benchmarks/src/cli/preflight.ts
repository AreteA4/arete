import { expandSweep } from '../config.js';
import { resolveModelAuth } from '../env.js';
import { formatPreflight, preflight } from '../preflight.js';
import { loadTask } from '../run-one.js';
import type { TaskDefinition } from '../types.js';
import { sweepFromArgs } from './args.js';

/**
 * Check credentials, model access, the a4 release and the Arete agent, and
 * estimate what a config will cost, without starting any agent.
 */
async function main(): Promise<void> {
  const { sweep } = sweepFromArgs(process.argv.slice(2), 'configs/smoke.json');
  const runs = expandSweep(sweep);
  const tasks = new Map<string, TaskDefinition>();
  for (const ref of new Set(sweep.tasks)) tasks.set(ref, await loadTask(ref));
  process.stdout.write(`${runs.length} run(s), model auth: ${resolveModelAuth(sweep.modelAuth)}\n\n`);
  const result = await preflight(runs, tasks);
  process.stdout.write(`${formatPreflight(result)}\n`);
  if (!result.ok) process.exitCode = 1;
}

main().catch((err) => {
  process.stderr.write(`Fatal: ${err instanceof Error ? err.message : String(err)}\n`);
  process.exit(1);
});
