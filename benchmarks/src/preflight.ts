import { Sandbox } from '@vercel/sandbox';
import { join } from 'node:path';
import { fetchAgentProfile, type AgentProfile } from './arete-setup.js';
import { agentKeys, cachedAgentKey, resultsDir, vercelCredentials } from './env.js';
import { createHarness } from './harnesses.js';
import { loadReports, quantile } from './history.js';
import { gatewayModelId, loadPricing, modelCost, sandboxCostUpperBound } from './pricing.js';
import type { RunConfig, TaskDefinition } from './types.js';

export interface PreflightCheck {
  name: string;
  ok: boolean;
  detail: string;
  /** A failed blocking check stops the sweep before any sandbox starts. */
  blocking: boolean;
}

export interface CostLine {
  task: string;
  harness: string;
  model: string;
  runs: number;
  perRunUsd?: number;
  sandboxPerRunUsd: number;
  basis: 'history' | 'reference' | 'unpriced';
}

export interface PreflightResult {
  checks: PreflightCheck[];
  cost: CostLine[];
  ok: boolean;
}

const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));

async function checkVercel(): Promise<PreflightCheck> {
  if (!('token' in vercelCredentials()) && !process.env.VERCEL_OIDC_TOKEN) {
    return { name: 'vercel sandbox', ok: false, detail: 'set VERCEL_TOKEN, VERCEL_TEAM_ID and VERCEL_PROJECT_ID (see .env.example)', blocking: true };
  }
  try {
    await Sandbox.list({ ...vercelCredentials() } as Parameters<typeof Sandbox.list>[0]);
    return { name: 'vercel sandbox', ok: true, detail: 'credentials accepted', blocking: true };
  } catch (err) {
    return { name: 'vercel sandbox', ok: false, detail: errorText(err), blocking: true };
  }
}

async function checkA4Release(version: string): Promise<PreflightCheck> {
  const url = `https://github.com/AreteA4/arete/releases/download/a4-cli-v${version}/checksums.txt`;
  try {
    const response = await fetch(url, { method: 'HEAD', redirect: 'follow', signal: AbortSignal.timeout(15_000) });
    return { name: `a4 ${version}`, ok: response.ok, detail: response.ok ? 'release published' : `HTTP ${response.status} for ${url}`, blocking: true };
  } catch (err) {
    return { name: `a4 ${version}`, ok: false, detail: errorText(err), blocking: true };
  }
}

async function errorBody(response: Response): Promise<string> {
  const text = await response.text();
  try {
    const body = JSON.parse(text) as { error?: { message?: string } | string };
    const message = typeof body.error === 'string' ? body.error : body.error?.message;
    return message ?? text.slice(0, 300);
  } catch {
    return text.slice(0, 300);
  }
}

/**
 * Prove the model is reachable with these credentials. Direct provider keys
 * are checked with a free model lookup; the gateway needs a real (16-token)
 * request, because billing problems only show up on inference.
 */
async function probeModel(config: RunConfig, harnessModel: string): Promise<PreflightCheck> {
  const name = `model ${config.model} (${config.harness}, ${config.modelAuth})`;
  try {
    let response: Response;
    if (config.modelAuth === 'ai-gateway') {
      const key = process.env.AI_GATEWAY_API_KEY || process.env.VERCEL_OIDC_TOKEN;
      const base = process.env.AI_GATEWAY_BASE_URL ?? 'https://ai-gateway.vercel.sh';
      response = await fetch(`${base}/v1/chat/completions`, {
        method: 'POST',
        headers: { Authorization: `Bearer ${key}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ model: gatewayModelId(config.model, config.harness), max_tokens: 16, messages: [{ role: 'user', content: 'Reply with ok.' }] }),
        signal: AbortSignal.timeout(60_000),
      });
    } else {
      const provider = config.harness === 'codex' ? 'openai' : config.harness === 'claude-code' ? 'anthropic' : config.model.split('/')[0];
      const id = harnessModel.replace(/^(anthropic|openai)\//, '');
      if (provider === 'anthropic') {
        if (!process.env.ANTHROPIC_API_KEY) return { name, ok: false, detail: 'ANTHROPIC_API_KEY is not set', blocking: true };
        response = await fetch(`https://api.anthropic.com/v1/models/${id}`, {
          headers: { 'x-api-key': process.env.ANTHROPIC_API_KEY, 'anthropic-version': '2023-06-01' },
          signal: AbortSignal.timeout(15_000),
        });
      } else {
        if (!process.env.OPENAI_API_KEY) return { name, ok: false, detail: 'OPENAI_API_KEY is not set', blocking: true };
        response = await fetch(`https://api.openai.com/v1/models/${id}`, {
          headers: { Authorization: `Bearer ${process.env.OPENAI_API_KEY}` },
          signal: AbortSignal.timeout(15_000),
        });
      }
    }
    return response.ok
      ? { name, ok: true, detail: 'reachable', blocking: true }
      : { name, ok: false, detail: `HTTP ${response.status}: ${await errorBody(response)}`, blocking: true };
  } catch (err) {
    return { name, ok: false, detail: errorText(err), blocking: true };
  }
}

function describeAgent(profile: AgentProfile): string {
  const limited = (profile.usage?.meters ?? [])
    .filter((m) => typeof m.allowance === 'number')
    .map((m) => `${m.meter} ${(m.remaining ?? (m.allowance ?? 0) - m.consumed).toLocaleString('en-US')} left`);
  const trial = profile.trialRemainingSeconds ? `, trial ends in ${Math.round(profile.trialRemainingSeconds / 3600)}h` : '';
  return `${profile.slug ?? 'agent'} · ${profile.plan ?? '?'} · ${profile.claimState ?? '?'}${trial}${limited.length ? ` · ${limited.join(', ')}` : ''}`;
}

async function checkAgentKeys(runs: RunConfig[]): Promise<PreflightCheck[]> {
  const keyMode = runs[0]?.keyMode ?? 'pool';
  if (keyMode === 'signup') {
    const signups = runs.filter((r) => r.keyMode === 'signup').length;
    return [{
      name: 'arete agent',
      ok: signups <= 5,
      detail: `each run signs up its own trial agent (${signups} signups; Arete allows 5/hour/IP)`,
      blocking: false,
    }];
  }
  const keys = agentKeys();
  if (keys.length === 0) {
    const cached = cachedAgentKey();
    const profile = cached ? await fetchAgentProfile(cached) : undefined;
    if (profile) {
      return [{ name: 'arete agent', ok: !profile.usage?.exhausted, detail: `cached benchmark agent: ${describeAgent(profile)}`, blocking: Boolean(profile.usage?.exhausted) }];
    }
    return [{ name: 'arete agent', ok: true, detail: 'ARETE_AGENT_KEYS is empty: one trial agent will be signed up and shared by every run', blocking: false }];
  }
  return Promise.all(
    keys.map(async (key, i): Promise<PreflightCheck> => {
      const profile = await fetchAgentProfile(key);
      if (!profile) return { name: `arete agent key ${i + 1}`, ok: false, detail: 'rejected by Arete', blocking: true };
      const exhausted = Boolean(profile.usage?.exhausted);
      return { name: `arete agent key ${i + 1}`, ok: !exhausted, detail: `${describeAgent(profile)}${exhausted ? ' · usage exhausted' : ''}`, blocking: exhausted };
    }),
  );
}

/**
 * Estimate per-run cost from the median of earlier runs of the same task and
 * model when there are any, otherwise from the task's reference profile
 * priced at the model's AI Gateway rates.
 */
async function estimateCost(runs: RunConfig[], tasks: Map<string, TaskDefinition>): Promise<CostLine[]> {
  const pricing = await loadPricing();
  const history = loadReports(join(resultsDir(), 'runs')).filter((r) => r.status === 'success' || r.status === 'agent-error');
  const groups = new Map<string, RunConfig[]>();
  for (const run of runs) {
    const key = `${run.task}\u0000${run.harness}\u0000${run.model}`;
    groups.set(key, [...(groups.get(key) ?? []), run]);
  }
  const lines: CostLine[] = [];
  for (const group of groups.values()) {
    const run = group[0]!;
    const task = tasks.get(run.task);
    const past = history.filter((r) => r.task.name === task?.name && r.config.harness === run.harness && r.config.model === run.model);
    const pastCost = quantile(past.map((r) => r.cost.modelUsd), 0.5);
    const pastWall = quantile(past.map((r) => r.timing.totalMs), 0.5);
    const estimate = task?.estimate;
    const referenceCost = estimate
      ? modelCost(pricing.get(gatewayModelId(run.model, run.harness)), {
          inputTokens: estimate.noCacheTokens + estimate.cacheReadTokens + estimate.cacheWriteTokens,
          noCacheTokens: estimate.noCacheTokens,
          cacheReadTokens: estimate.cacheReadTokens,
          cacheWriteTokens: estimate.cacheWriteTokens,
          outputTokens: estimate.outputTokens,
          reasoningTokens: 0,
        })
      : undefined;
    const wallMs = pastWall ?? (estimate ? estimate.wallMs + 30_000 : 180_000);
    lines.push({
      task: task?.name ?? run.task,
      harness: run.harness,
      model: run.model,
      runs: group.length,
      perRunUsd: pastCost ?? referenceCost,
      sandboxPerRunUsd: sandboxCostUpperBound(run.sandbox.vcpus, wallMs),
      basis: pastCost !== undefined ? 'history' : referenceCost !== undefined ? 'reference' : 'unpriced',
    });
  }
  return lines;
}

export async function preflight(
  runs: RunConfig[],
  tasks: Map<string, TaskDefinition>,
  opts: { probeModels?: boolean } = {},
): Promise<PreflightResult> {
  const checks: PreflightCheck[] = [];
  const agents = new Map<string, RunConfig>();
  for (const run of runs) agents.set(`${run.harness}\u0000${run.model}\u0000${run.modelAuth}`, run);

  const probes: Array<Promise<PreflightCheck>> = [checkVercel(), checkA4Release(runs[0]?.a4Version ?? '')];
  for (const run of agents.values()) {
    let harnessModel: string;
    try {
      harnessModel = createHarness(run).model;
    } catch (err) {
      checks.push({ name: `harness ${run.harness} / ${run.model}`, ok: false, detail: errorText(err), blocking: true });
      continue;
    }
    if (run.modelAuth === 'ai-gateway' && !process.env.AI_GATEWAY_API_KEY && !process.env.VERCEL_OIDC_TOKEN) {
      checks.push({ name: `model ${run.model}`, ok: false, detail: 'AI_GATEWAY_API_KEY is not set', blocking: true });
      continue;
    }
    if (opts.probeModels !== false) probes.push(probeModel(run, harnessModel));
  }
  checks.push(...(await Promise.all(probes)), ...(await checkAgentKeys(runs)));
  const cost = await estimateCost(runs, tasks);
  return { checks, cost, ok: checks.every((c) => c.ok || !c.blocking) };
}

export function formatPreflight(result: PreflightResult): string {
  const lines = ['Preflight'];
  for (const c of result.checks) {
    lines.push(`  ${c.ok ? '✓' : c.blocking ? '✗' : '!'} ${c.name.padEnd(48)} ${c.detail}`);
  }
  const usd = (v: number) => `$${v.toFixed(2)}`;
  const model = result.cost.reduce((sum, l) => sum + (l.perRunUsd ?? 0) * l.runs, 0);
  const sandbox = result.cost.reduce((sum, l) => sum + l.sandboxPerRunUsd * l.runs, 0);
  lines.push('', 'Estimated cost (model at AI Gateway rates; sandbox is an upper bound)');
  for (const l of result.cost) {
    const per = l.perRunUsd === undefined ? 'unpriced' : `${usd(l.perRunUsd)}/run`;
    lines.push(`  ${l.task.padEnd(22)} ${l.harness.padEnd(12)} ${l.model.padEnd(30)} ${String(l.runs).padStart(3)} × ${per.padEnd(12)} (${l.basis})`);
  }
  lines.push(`  total ≈ ${usd(model)} model + ≤ ${usd(sandbox)} sandbox`);
  if (result.cost.some((l) => l.basis === 'reference')) {
    lines.push('  reference estimates come from Claude Code + Sonnet 5.5 runs; other agents can use 2–4× the tokens.');
  }
  return lines.join('\n');
}
