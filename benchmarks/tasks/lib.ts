import { parseJson } from '../src/arete-setup.js';
import { classify } from '../src/classify.js';
import type { CheckResult, SandboxShell, Transcript } from '../src/types.js';

export function check(id: string, passed: boolean, detail: string, required = true): CheckResult {
  return { id, passed, detail, required };
}

/** Last stdout line that parses as a JSON object. */
export function lastJsonLine<T = Record<string, unknown>>(text: string): T | undefined {
  const lines = text.trim().split('\n').reverse();
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('{')) continue;
    try {
      return JSON.parse(trimmed) as T;
    } catch {
      // keep looking
    }
  }
  return undefined;
}

export function asNumber(value: unknown): number | undefined {
  const n = typeof value === 'number' ? value : typeof value === 'string' ? Number(value) : NaN;
  return Number.isFinite(n) ? n : undefined;
}

export async function fileExists(shell: SandboxShell, path: string): Promise<CheckResult> {
  const result = await shell.run(`test -f ${JSON.stringify(path)}`);
  return check(`file:${path}`, result.exitCode === 0, result.exitCode === 0 ? 'present' : 'missing');
}

/** `a4 doctor --json` reports a top-level `"ok"`, as agent.md requires. */
export async function doctorOk(shell: SandboxShell, required = true): Promise<CheckResult> {
  const result = await shell.run('a4 doctor --json', { timeoutSeconds: 90 });
  const parsed = parseJson<{ status?: string; checks?: Array<{ id: string; status: string }> }>(result.stdout);
  const nonOk = (parsed?.checks ?? []).filter((c) => !['ok', 'info'].includes(c.status));
  return check(
    'doctor-ok',
    parsed?.status === 'ok',
    parsed ? `status ${parsed.status}${nonOk.length ? `: ${nonOk.map((c) => `${c.id}=${c.status}`).join(', ')}` : ''}` : `no JSON (exit ${result.exitCode})`,
    required,
  );
}

export async function manifestDeclares(
  shell: SandboxShell,
  slug: string,
  required = true,
): Promise<CheckResult> {
  const toml = (await shell.readText(`${shell.workDir}/arete.toml`)) ?? '';
  const found = new RegExp(`(^|[^\\w-])${slug}([^\\w-]|$)`, 'm').test(toml);
  return check(`manifest:${slug}`, found, found ? `arete.toml declares ${slug}` : 'not declared in arete.toml', required);
}

export async function noSecretsInWorkspace(shell: SandboxShell): Promise<CheckResult> {
  const result = await shell.run(
    `grep -rIlE 'a4_(ak|sk)_[A-Za-z0-9]{16,}' --exclude-dir=node_modules --exclude-dir=.git . || true`,
  );
  const files = result.stdout.trim().split('\n').filter(Boolean);
  return check('no-secrets', files.length === 0, files.length ? `key material in ${files.join(', ')}` : 'clean');
}

/**
 * Regenerating from the lockfile reproduces the workspace exactly: generated
 * SDK output was not hand-edited and `arete.lock` matches what is installed.
 */
export async function lockReproducible(shell: SandboxShell, required = false): Promise<CheckResult> {
  if ((await shell.run('test -f arete.lock')).exitCode !== 0) {
    return check('lock-reproducible', false, 'no arete.lock', required);
  }
  const hashes = `find . \\( -name node_modules -o -name .git \\) -prune -o -type f -print0 | sort -z | xargs -0 md5sum`;
  const before = await shell.run(hashes);
  const install = await shell.run('a4 install --locked', { timeoutSeconds: 240 });
  if (install.exitCode !== 0) {
    return check('lock-reproducible', false, `a4 install --locked failed: ${(install.stderr || install.stdout).slice(0, 300)}`, required);
  }
  const after = await shell.run(hashes);
  const beforeLines = new Set(before.stdout.split('\n'));
  const changed = after.stdout
    .split('\n')
    .filter((line) => line && !beforeLines.has(line))
    .map((line) => line.split(/\s+/).pop());
  return check(
    'lock-reproducible',
    changed.length === 0,
    changed.length ? `regeneration changed ${changed.slice(0, 8).join(', ')}` : 'regenerated output identical',
    required,
  );
}

export interface OreRound {
  roundId: number;
  totalMiners: number;
  totalDeployed: number;
}

/** Current ORE round straight from the deployed view, independent of the agent's code. */
export async function oreGroundTruth(shell: SandboxShell): Promise<OreRound | undefined> {
  const result = await shell.run('a4 stream OreRound/latest --stack ore --first --no-dna', {
    timeoutSeconds: 60,
  });
  for (const line of result.stdout.split('\n')) {
    const event = parseJson<{ action?: string; data?: { payload?: { data?: Record<string, any> } } }>(line);
    const data = event?.action === 'entity_update' ? event.data?.payload?.data : undefined;
    if (!data) continue;
    const roundId = asNumber(data.id?.round_id);
    if (roundId === undefined) continue;
    return {
      roundId,
      totalMiners: asNumber(data.state?.total_miners) ?? 0,
      totalDeployed: asNumber(data.state?.total_deployed) ?? 0,
    };
  }
  return undefined;
}

/** Tool calls of one turn that touched live Arete data (MCP or `a4 stream`). */
export function liveDataCalls(transcript: Transcript, turn: number): string[] {
  return transcript.toolCalls
    .filter((t) => t.turn === turn)
    .map(classify)
    .filter((c) => (c.mcp && c.mcp.server === 'arete') || c.a4.some((a) => a.path === 'stream'))
    .map((c) => (c.mcp ? `${c.mcp.server}/${c.mcp.tool}` : 'a4 stream'));
}

/** F1 of a predicted slug set against the expected set. */
export function f1(predicted: string[], expected: string[]): { f1: number; precision: number; recall: number } {
  const p = new Set(predicted.map((s) => s.toLowerCase().trim()));
  const e = new Set(expected.map((s) => s.toLowerCase().trim()));
  const hits = [...p].filter((s) => e.has(s)).length;
  const precision = p.size ? hits / p.size : 0;
  const recall = e.size ? hits / e.size : 0;
  return { f1: precision + recall ? (2 * precision * recall) / (precision + recall) : 0, precision, recall };
}
