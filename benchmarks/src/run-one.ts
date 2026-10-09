import { HarnessAgent, type HarnessAgentSession } from '@ai-sdk/harness/agent';
import type { TextStreamPart, ToolSet } from 'ai';
import { randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import {
  ensureAgentAuth,
  fetchAgentUsage,
  initProject,
  installA4,
  readSkillsHash,
  runDoctor,
  trustProject,
  usageDelta,
  writeAgentCredential,
} from './arete-setup.js';
import { classify } from './classify.js';
import { BENCH_ROOT, resultsDir } from './env.js';
import { detectFriction } from './friction.js';
import { backfillCodexUsage } from './native-usage.js';
import { createHarness, NATIVE_LOG_DIRS } from './harnesses.js';
import type { KeyPool } from './keys.js';
import { summarizeTiming, summarizeTokens, summarizeTools } from './metrics.js';
import { gatewayModelId, loadPricing, modelCost, sandboxCostUpperBound } from './pricing.js';
import { errorText, Recorder, turnErrorStatus } from './recorder.js';
import { formatSummary, RunDir } from './report.js';
import { createRunSandbox, createShell, destroySandbox, resolveHomeDir } from './sandbox.js';
import { renderTranscript } from './transcript.js';
import type {
  AreteState,
  CheckResult,
  PhaseTimings,
  RunConfig,
  RunReport,
  SandboxShell,
  TaskDefinition,
} from './types.js';
import { diffWorkspace, downloadDirs, snapshotWorkspace, WORKSPACE_EXCLUDES, type WorkspaceEntry } from './workspace.js';

/** Project directory inside the sandbox, relative to its default working directory. */
const RUN_WORKDIR = 'project';

const UNATTENDED_NOTE =
  'You are running unattended in a fresh Linux sandbox as part of an automated evaluation. ' +
  'No human is available to answer questions or approve actions: when a decision is needed, ' +
  'make a reasonable choice, state it, and continue until the task is complete.';

export async function loadTask(taskRef: string): Promise<TaskDefinition> {
  const mod = (await import(resolve(BENCH_ROOT, 'tasks', taskRef))) as { task?: TaskDefinition };
  if (!mod.task) throw new Error(`tasks/${taskRef} does not export \`task\``);
  return mod.task;
}

function packageVersion(name: string): string {
  try {
    const pkg = JSON.parse(
      readFileSync(resolve(BENCH_ROOT, 'node_modules', name, 'package.json'), 'utf8'),
    ) as { version: string };
    return pkg.version;
  } catch {
    return 'unknown';
  }
}

function runIdFor(config: RunConfig, task: TaskDefinition, startedAt: Date): string {
  const stamp = startedAt.toISOString().replace(/[-:]/g, '').replace(/\.\d+Z$/, 'Z');
  const model = (config.label ?? config.model.split('/').pop() ?? config.model).replace(/[^\w.-]+/g, '-');
  return `${stamp}_${task.name}_${config.harness}_${model}_${randomBytes(2).toString('hex')}`;
}

function nativeIds(state: unknown): Record<string, string> {
  const ids: Record<string, string> = {};
  const data = (state as { data?: Record<string, unknown> } | undefined)?.data ?? {};
  for (const [key, value] of Object.entries(data)) {
    if (typeof value === 'string' && /id$/i.test(key)) ids[key] = value;
  }
  return ids;
}

/**
 * Build (or reuse) a harness's cached sandbox image. Concurrent runs that
 * all find the image missing would race to build the same named template,
 * so sweeps call this once per harness before starting runs.
 */
export async function prewarmTemplate(config: RunConfig): Promise<void> {
  const agent = new HarnessAgent({ harness: createHarness(config).adapter, sandboxConfig: { workDir: RUN_WORKDIR } });
  const sandbox = await createRunSandbox(config, await agent.getSandboxTemplate(), `prewarm-${config.harness}`);
  await destroySandbox(sandbox);
}

export interface RunOptions {
  keyPool: KeyPool;
  /** Progress sink; lines are already prefixed with the run label. */
  log?: (line: string) => void;
}

/**
 * Run one task with one harness/model in a fresh sandbox and write the run
 * directory: report, transcript, event log, native session logs, final
 * workspace and setup log.
 */
export async function runOne(config: RunConfig, task: TaskDefinition, opts: RunOptions): Promise<RunReport> {
  const startedAt = new Date();
  const runId = runIdFor(config, task, startedAt);
  const dir = new RunDir(join(resultsDir(), 'runs', startedAt.toISOString().slice(0, 10), runId));
  const t0 = performance.now();
  const runLog: string[] = [];
  const label = `[${task.name} · ${config.harness} · ${config.label ?? config.model}]`;
  const log = (line: string) => {
    runLog.push(`${((performance.now() - t0) / 1000).toFixed(1).padStart(7)}s ${line}`);
    opts.log?.(`${label} ${line}`);
  };
  const harnessDiagnostics: Array<{ t: number; message?: string }> = [];

  const lease = opts.keyPool.lease();
  const harness = createHarness(config);
  const pricing = (await loadPricing()).get(gatewayModelId(config.model, config.harness));
  const phases: PhaseTimings = { totalMs: 0, sandboxMs: 0, setupMs: 0, agentMs: 0, verifyMs: 0, collectMs: 0 };
  const arete: AreteState = { usageAttribution: 'unknown' };
  const checks: CheckResult[] = [];
  const recorder = new Recorder((line) => log(`  ${line}`));
  const nativeSessionIds: Record<string, string> = {};
  let status: RunReport['status'] = 'success';
  let error: string | undefined;
  let shell: SandboxShell | undefined;
  let before: WorkspaceEntry[] = [];
  let workspace: RunReport['workspace'];
  let usageBefore: Record<string, number> | undefined;
  let phase = 'setup';

  log(`run ${runId}`);
  const sandboxStart = performance.now();
  let sandboxSession: Awaited<ReturnType<typeof createRunSandbox>> | undefined;

  const agent = new HarnessAgent({
    harness: harness.adapter,
    model: harness.model,
    instructions: [UNATTENDED_NOTE, config.instructions].filter(Boolean).join('\n\n'),
    ...(harness.inactiveTools ? { inactiveTools: harness.inactiveTools as never } : {}),
    debug: { enabled: true, level: 'info' },
    onLog: (diagnostic) => harnessDiagnostics.push({ t: Math.round(performance.now() - t0), ...diagnostic }),
    sandboxConfig: {
      workDir: RUN_WORKDIR,
      // Runs once per sandbox, before the harness runtime starts, so the
      // runtime sees the MCP servers, skills and instructions `a4 init` wrote.
      onSession: async ({ sessionWorkDir }) => {
        if (shell || !sandboxSession) return;
        phases.sandboxMs = Math.round(performance.now() - sandboxStart);
        const setupStart = performance.now();
        const homeDir = await resolveHomeDir(sandboxSession);
        shell = createShell(sandboxSession, sessionWorkDir, homeDir, (line) => log(`[${phase}] ${line}`));
        await trustProject(shell, config.harness);
        if (lease.key && config.keyMode === 'pool') await writeAgentCredential(shell, lease.key);
        if (task.setup === 'initialized') {
          arete.a4Version = await installA4(shell, config.a4Version);
          await initProject(shell, harness.a4AgentId, config.skillsRef);
          await ensureAgentAuth(shell, config.keyMode, Boolean(lease.key));
          arete.doctorBefore = await runDoctor(shell);
          arete.skillsHash = await readSkillsHash(shell);
          if (arete.doctorBefore?.status === 'fail') {
            throw new Error(`doctor failed before the agent started: ${arete.doctorBefore.nonOk.join(', ')}`);
          }
        }
        before = await snapshotWorkspace(shell);
        phases.setupMs = Math.round(performance.now() - setupStart);
        log(`setup done in ${(phases.setupMs / 1000).toFixed(1)}s${arete.doctorBefore ? ` (doctor ${arete.doctorBefore.status})` : ''}`);
      },
    },
  });

  let session: HarnessAgentSession | undefined;
  try {
    sandboxSession = await createRunSandbox(config, await agent.getSandboxTemplate(), runId);
    log(`sandbox ${sandboxSession.id} ready`);
    try {
      session = await agent.createSession({ sandboxSession });
    } catch (err) {
      status = shell ? 'setup-error' : 'infra-error';
      throw err;
    }
    if (lease.key && config.keyMode === 'pool') usageBefore = await fetchAgentUsage(lease.key);

    // ---- agent turns -------------------------------------------------------
    const agentStart = performance.now();
    for (const [index, turn] of task.turns.entries()) {
      if (index > 0 && turn.freshSession) {
        Object.assign(nativeSessionIds, nativeIds(await session.stop().catch(() => undefined)));
        session = await agent.createSession({ sandboxSession });
        log('restarted harness session');
      }
      recorder.beginTurn(index, turn.prompt, Boolean(turn.freshSession));
      log(`turn ${index + 1}: ${turn.prompt.slice(0, 100)}`);
      try {
        const result = await agent.stream({
          session,
          prompt: turn.prompt,
          abortSignal: AbortSignal.timeout(config.turnTimeoutMinutes * 60_000),
        });
        await recorder.consume(result.fullStream as AsyncIterable<TextStreamPart<ToolSet>>);
        const providerMetadata = await Promise.resolve(result.providerMetadata).catch(() => undefined);
        const streamError = recorder.transcript.errors.find((e) => e.turn === index)?.message;
        recorder.endTurn({ unfinished: session.hasUnfinishedTurn(), providerMetadata });
        if (streamError) {
          // An `error` part ends the turn without throwing. If the model did no
          // work it is a configuration or provider failure, not the agent's.
          status = turnErrorStatus(recorder.transcript, index);
          error = streamError;
          log(`turn ${index + 1} errored: ${streamError}`);
          break;
        }
      } catch (err) {
        recorder.endTurn({ unfinished: session.hasUnfinishedTurn(), error: errorText(err) });
        status = 'agent-error';
        error = errorText(err);
        log(`turn ${index + 1} failed: ${error}`);
        break;
      }
    }
    phases.agentMs = Math.round(performance.now() - agentStart);
    Object.assign(nativeSessionIds, nativeIds(await session.stop().catch(() => undefined)));
    session = undefined;

    // ---- verification ------------------------------------------------------
    const verifyStart = performance.now();
    phase = 'verify';
    if (shell) {
      const finalText = recorder.transcript.turns.map((t) => t.text).join('\n\n');
      try {
        checks.push(...(await task.verify({ shell, config, transcript: recorder.transcript, finalText })));
      } catch (err) {
        checks.push({ id: 'verifier', passed: false, required: true, detail: `verifier crashed: ${errorText(err)}` });
      }
      arete.doctorAfter = await runDoctor(shell);
    }
    phases.verifyMs = Math.round(performance.now() - verifyStart);
  } catch (err) {
    if (status === 'success') status = 'infra-error';
    error ??= errorText(err);
    log(`error: ${error}`);
  } finally {
    // ---- collect artifacts -------------------------------------------------
    const collectStart = performance.now();
    phase = 'collect';
    if (session) await session.destroy().catch(() => {});
    if (shell) {
      try {
        workspace = diffWorkspace(before, await snapshotWorkspace(shell));
        await downloadDirs(shell, shell.workDir, ['.'], join(dir.path, 'workspace'), WORKSPACE_EXCLUDES);
        dir.redactTree('workspace');
        await downloadDirs(
          shell,
          shell.homeDir,
          [...NATIVE_LOG_DIRS[config.harness], '.ai-sdk-harness/.agent-runs'],
          join(dir.path, 'native'),
        );
        dir.redactTree('native');
      } catch (err) {
        log(`artifact collection failed: ${errorText(err)}`);
      }
    }
    if (lease.key && usageBefore) {
      arete.usageDelta = usageDelta(usageBefore, await fetchAgentUsage(lease.key));
      arete.usageAttribution = lease.attribution();
    }
    lease.release();
    if (sandboxSession) await destroySandbox(sandboxSession).catch(() => {});
    phases.collectMs = Math.round(performance.now() - collectStart);
    phases.totalMs = Math.round(performance.now() - t0);
  }

  // ---- report --------------------------------------------------------------
  const transcript = recorder.transcript;
  // Codex reports usage per turn only; its own rollout log has every call.
  const codex = config.harness === 'codex' ? backfillCodexUsage(transcript, join(dir.path, 'native')) : undefined;
  const calls = transcript.toolCalls.map(classify);
  const tokens = summarizeTokens(transcript, codex?.contextWindow ?? pricing?.contextWindow);
  const cost = modelCost(pricing, tokens);
  const harnessReportedUsd = transcript.turns
    .map((t) => (t.providerMetadata as Record<string, { costUsd?: number }> | undefined)?.['claude-code']?.costUsd)
    .filter((v): v is number => typeof v === 'number')
    .reduce<number | undefined>((sum, v) => (sum ?? 0) + v, undefined);
  const required = checks.filter((c) => c.required);
  const report: RunReport = {
    schemaVersion: 2,
    runId,
    startedAt: startedAt.toISOString(),
    config,
    task: { name: task.name, track: task.track, setup: task.setup, grading: task.grading ?? 1 },
    versions: {
      '@ai-sdk/harness': packageVersion('@ai-sdk/harness'),
      [`@ai-sdk/harness-${config.harness}`]: packageVersion(`@ai-sdk/harness-${config.harness}`),
      '@ai-sdk/sandbox-vercel': packageVersion('@ai-sdk/sandbox-vercel'),
      a4: arete.a4Version ?? config.a4Version,
    },
    status,
    ...(error ? { error } : {}),
    nativeSessionIds,
    timing: summarizeTiming(phases, transcript, calls),
    tokens,
    cost: {
      modelUsd: cost ?? 0,
      source: cost === undefined ? 'unpriced' : 'gateway-pricing',
      ...(harnessReportedUsd !== undefined ? { harnessReportedUsd } : {}),
      sandboxUsdUpperBound: sandboxCostUpperBound(config.sandbox.vcpus, phases.totalMs),
    },
    tools: summarizeTools(calls),
    steps: transcript.steps.length,
    arete,
    harness: {
      apiRetries: harnessDiagnostics.filter((d) => /API retry/i.test(d.message ?? '')).length,
      rateLimitRetries: harnessDiagnostics.filter((d) => /API retry/i.test(d.message ?? '') && /429|rate.?limit/i.test(d.message ?? '')).length,
    },
    ...(workspace ? { workspace } : {}),
    friction: detectFriction(transcript, calls),
    verification: {
      passed: status === 'success' && required.length > 0 && required.every((c) => c.passed),
      score: checks.length ? checks.filter((c) => c.passed).length / checks.length : 0,
      checks,
    },
    finalText: transcript.turns.map((t) => t.text).join('\n\n'),
  };

  dir.writeJson('report.json', report);
  dir.writeJson('transcript.json', transcript);
  dir.writeText('transcript.md', `# ${runId}\n\n${renderTranscript(transcript, calls)}\n`);
  dir.writeJsonl('events.jsonl', recorder.events);
  dir.writeJsonl('harness-diagnostics.jsonl', harnessDiagnostics);
  dir.writeText('run.log', `${runLog.join('\n')}\n`);
  const summary = formatSummary(report);
  dir.writeText('summary.txt', `${summary}\n`);
  opts.log?.(`\n${summary}\n  → ${dir.path}\n`);
  return report;
}
