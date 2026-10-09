import type { TaskDefinition } from '../src/types.js';
import {
  asNumber,
  check,
  doctorOk,
  fileExists,
  lastJsonLine,
  lockReproducible,
  manifestDeclares,
  noSecretsInWorkspace,
  oreGroundTruth,
} from './lib.js';

const SCRIPT = 'src/ore-round.ts';

/**
 * Build track: a project already set up with `a4 init`. The prompt states
 * only the intent, so the agent has to discover the ORE stack, install it and
 * build against the generated SDK.
 */
export const task: TaskDefinition = {
  name: 'ore-live-round',
  description: 'Discover and install the ORE stack, then print the live round from a TypeScript script',
  track: 'build',
  setup: 'initialized',
  // Measured cost profile, used by preflight to estimate a sweep.
  estimate: {
    noCacheTokens: 40,
    cacheReadTokens: 893_679,
    cacheWriteTokens: 44_417,
    outputTokens: 4_830,
    wallMs: 132_000,
    source: 'Claude Code + Sonnet 5.5, 2026-10-08',
  },
  turns: [
    {
      prompt:
        `Write a TypeScript script at \`${SCRIPT}\` that prints live data about the current ORE mining round on Solana, using Arete for the data. ` +
        'It must print exactly one line of JSON to stdout — `{"roundId": <number>, "totalDeployedSol": <number>, "totalMiners": <number>}` — ' +
        `and then exit with code 0 within 60 seconds. Make sure \`npx tsx ${SCRIPT}\` works from the project root, and run it once to confirm.`,
    },
  ],
  async verify({ shell }) {
    const checks = [await fileExists(shell, SCRIPT)];
    // Resolve tsx first so a cold npx download does not count against the
    // script's 60 seconds; the shell's timeout then enforces the prompt's limit.
    await shell.run('npx -y tsx --version', { timeoutSeconds: 120 });
    const run = await shell.run(`npx -y tsx ${SCRIPT}`, { timeoutSeconds: 60 });
    const timedOut = run.exitCode === 124 || run.exitCode === 137;
    const truth = await oreGroundTruth(shell);
    const output = lastJsonLine(run.stdout);
    const stdoutLines = run.stdout.trim().split('\n').filter((line) => line.trim());
    const roundId = asNumber(output?.roundId);
    const totalMiners = asNumber(output?.totalMiners);
    checks.push(
      check(
        'script-runs',
        run.exitCode === 0 && output !== undefined,
        timedOut
          ? 'did not exit within 60 seconds'
          : run.exitCode === 0
            ? output ? 'printed JSON' : `no JSON line in: ${run.stdout.slice(-300)}`
            : `exit ${run.exitCode}: ${(run.stderr || run.stdout).slice(-400)}`,
      ),
      check(
        'single-json-line',
        stdoutLines.length === 1 && output !== undefined,
        `${stdoutLines.length} stdout line(s)${stdoutLines.length > 1 ? `: ${stdoutLines.slice(0, 3).join(' | ').slice(0, 300)}` : ''}`,
      ),
      check(
        'output-shape',
        roundId !== undefined && totalMiners !== undefined && asNumber(output?.totalDeployedSol) !== undefined,
        output ? JSON.stringify(output).slice(0, 200) : 'no output',
      ),
    );
    if (truth && roundId !== undefined) {
      checks.push(
        check('round-matches-live', Math.abs(roundId - truth.roundId) <= 3, `script ${roundId}, live ${truth.roundId}`),
      );
      if (totalMiners !== undefined && roundId === truth.roundId) {
        const tolerance = Math.max(25, truth.totalMiners * 0.5);
        checks.push(
          check('miners-plausible', Math.abs(totalMiners - truth.totalMiners) <= tolerance, `script ${totalMiners}, live ${truth.totalMiners}`, false),
        );
      }
    } else {
      checks.push(check('round-matches-live', false, truth ? 'script printed no roundId' : 'could not read live ground truth'));
    }
    checks.push(
      await manifestDeclares(shell, 'ore'),
      await noSecretsInWorkspace(shell),
      await doctorOk(shell, false),
      await lockReproducible(shell, false),
    );
    return checks;
  },
};
