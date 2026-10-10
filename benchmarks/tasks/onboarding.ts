import type { TaskDefinition } from '../src/types.js';
import { check, doctorOk, freshAccountChecks, liveDataCalls, noSecretsInWorkspace, oreGroundTruth } from './lib.js';

/** The exact prompt arete.run tells users to paste into their agent. */
export const ARETE_RUN_PROMPT =
  'Set up Arete in this project by following https://docs.arete.run/agent.md, then tell me what Arete can do.';

const MCP_CONFIG: Record<string, string> = {
  'claude-code': '.mcp.json',
  codex: '.codex/config.toml',
  opencode: 'opencode.json',
};

const CAPABILITY_WORDS = ['catalog', 'stream', 'view', 'sdk', 'transaction', 'instruction', 'pda', 'stack', 'program', 'wallet'];

/**
 * Onboarding track: an empty project and the arete.run prompt, verbatim.
 * Turn 2 runs in a fresh session (the "restart your agent host" step) and
 * asks a question that needs live data, proving the new skills and MCP
 * servers actually work. In `fresh` key mode the agent also has to create
 * its own agent account, and the credential handling is graded.
 */
export const task: TaskDefinition = {
  name: 'onboarding',
  description: 'Follow the arete.run setup prompt from scratch, then answer a live-data question after a host restart',
  track: 'onboarding',
  setup: 'bare',
  // Measured cost profile, used by preflight to estimate a sweep.
  estimate: {
    noCacheTokens: 30,
    cacheReadTokens: 470_609,
    cacheWriteTokens: 26_097,
    outputTokens: 3_077,
    wallMs: 78_000,
    source: 'Claude Code + Sonnet 5.5, 2026-10-08',
  },
  turns: [
    { prompt: ARETE_RUN_PROMPT },
    {
      freshSession: true,
      prompt:
        'I restarted you so the new Arete skills and MCP servers are loaded. Using Arete, what is the current ORE mining round number? ' +
        'Answer with the number and say how you got it.',
    },
  ],
  async verify({ shell, config, transcript }) {
    // First, before verification's own `a4` calls can touch ~/.arete.
    const fresh = config.keyMode === 'fresh' ? await freshAccountChecks(shell, transcript) : [];
    const version = await shell.run('a4 --version');
    const toml = await shell.run('test -f arete.toml');
    const auth = await shell.run('a4 --profile agent auth whoami --json', { timeoutSeconds: 60 });
    const mcpPath = MCP_CONFIG[config.harness]!;
    const mcp = (await shell.readText(`${shell.workDir}/${mcpPath}`)) ?? '';
    const skills = await shell.run('test -f skills-lock.json');

    const setupAnswer = (transcript.turns[0]?.text ?? '').toLowerCase();
    const mentioned = CAPABILITY_WORDS.filter((w) => setupAnswer.includes(w));
    const answer = transcript.turns[1]?.text ?? '';
    const truth = await oreGroundTruth(shell);
    const numbers = [...answer.matchAll(/\b\d{4,}\b/g)].map((m) => Number(m[0]));
    const matched = truth ? numbers.find((n) => Math.abs(n - truth.roundId) <= 5) : undefined;
    const live = liveDataCalls(transcript, 1);

    return [
      check('a4-installed', version.exitCode === 0 && version.stdout.includes(config.a4Version), version.stdout.trim() || version.stderr.trim()),
      check('project-initialized', toml.exitCode === 0, toml.exitCode === 0 ? 'arete.toml present' : 'no arete.toml'),
      await doctorOk(shell),
      check('agent-authenticated', auth.exitCode === 0, auth.exitCode === 0 ? 'agent profile verified' : `exit ${auth.exitCode}`),
      check('mcp-configured', /arete/.test(mcp), mcp ? `${mcpPath} mentions arete` : `${mcpPath} missing`, false),
      check('skills-installed', skills.exitCode === 0, skills.exitCode === 0 ? 'skills-lock.json present' : 'no skills-lock.json', false),
      check('capabilities-summarized', mentioned.length >= 4, `mentions ${mentioned.join(', ') || 'none'}`, false),
      check(
        'live-round-answered',
        matched !== undefined,
        truth ? `live ${truth.roundId}; answer numbers ${numbers.slice(0, 5).join(', ') || 'none'}` : 'could not read live ground truth',
      ),
      check('used-live-data', live.length > 0, live.length ? live.join(', ') : 'no MCP or a4 stream call in turn 2', false),
      await noSecretsInWorkspace(shell),
      ...fresh,
    ];
  },
};
