import type { HarnessKind, KeyMode, SandboxShell } from './types.js';

export const ARETE_API_URL = 'https://api.arete.run';

export interface DoctorResult {
  status: string;
  nonOk: string[];
}

export function parseJson<T = unknown>(text: string): T | undefined {
  try {
    return JSON.parse(text) as T;
  } catch {
    // Some commands print progress lines before the JSON document.
    const start = text.indexOf('{');
    const end = text.lastIndexOf('}');
    if (start >= 0 && end > start) {
      try {
        return JSON.parse(text.slice(start, end + 1)) as T;
      } catch {
        return undefined;
      }
    }
    return undefined;
  }
}

/**
 * Store a pooled agent key in the `agent` profile, exactly where
 * `a4 auth login --profile agent` would put it. Writing the file directly
 * works before `a4` is installed (onboarding runs) and keeps the key out of
 * every logged command line. `~/.arete` must be 0700 or `a4` refuses it.
 */
export async function writeAgentCredential(shell: SandboxShell, key: string): Promise<void> {
  const dir = `${shell.homeDir}/.arete`;
  await shell.run(`mkdir -p ${dir} && chmod 700 ${dir}`);
  await shell.writeText(
    `${dir}/credentials.toml`,
    `[profiles.agent.keys]\n"${ARETE_API_URL}" = "${key}"\n`,
  );
  await shell.run(`chmod 600 ${dir}/credentials.toml`);
}

/**
 * Accept the host's own trust prompt for this project, as a user would in
 * an interactive session: Claude Code loads project `.mcp.json` servers only
 * once approved, and Codex reads `.codex/config.toml` only for trusted
 * projects.
 */
export async function trustProject(shell: SandboxShell, harness: HarnessKind): Promise<void> {
  if (harness === 'claude-code') {
    await shell.run('mkdir -p .claude');
    await shell.writeText(
      `${shell.workDir}/.claude/settings.local.json`,
      `${JSON.stringify({ enableAllProjectMcpServers: true }, null, 2)}\n`,
    );
  } else if (harness === 'codex') {
    await shell.run(
      `mkdir -p ~/.codex && printf '\\n[projects."%s"]\\ntrust_level = "trusted"\\n' ${JSON.stringify(shell.workDir)} >> ~/.codex/config.toml`,
    );
  }
}

export async function installA4(shell: SandboxShell, version: string): Promise<string> {
  const install = await shell.run(`curl -fsSL https://arete.run/install.sh | sh -s -- ${version}`, {
    timeoutSeconds: 180,
  });
  if (install.exitCode !== 0) {
    throw new Error(`a4 install failed: ${install.stderr || install.stdout}`);
  }
  // `a4 self install` leaves ~/.arete at 0755, but credential writes demand
  // 0700, so a fresh install cannot sign up until this is fixed.
  await shell.run('chmod 700 ~/.arete');
  const check = await shell.run('a4 --version');
  if (check.exitCode !== 0) throw new Error(`a4 not on PATH after install: ${check.stderr}`);
  return check.stdout.trim().replace(/^a4\s+/, '');
}

export async function initProject(
  shell: SandboxShell,
  a4AgentId: string,
  skillsRef?: string,
): Promise<void> {
  const ref = skillsRef ? ` --skills-ref ${skillsRef}` : '';
  const result = await shell.run(`a4 init -y --agents ${a4AgentId}${ref} --json`, {
    timeoutSeconds: 240,
  });
  if (result.exitCode !== 0) {
    throw new Error(`a4 init failed: ${result.stderr || result.stdout}`);
  }
}

export async function ensureAgentAuth(shell: SandboxShell, mode: KeyMode, hasKey: boolean) {
  const command =
    mode === 'signup' || !hasKey
      ? 'a4 --profile agent auth signup --if-missing --json'
      : 'a4 --profile agent auth whoami --json';
  const result = await shell.run(command, { timeoutSeconds: 60 });
  if (result.exitCode !== 0) {
    throw new Error(`agent auth failed (exit ${result.exitCode}): ${result.stderr || result.stdout}`);
  }
}

export async function runDoctor(shell: SandboxShell): Promise<DoctorResult | undefined> {
  const result = await shell.run('a4 doctor --json', { timeoutSeconds: 90 });
  const parsed = parseJson<{ status?: string; checks?: Array<{ id: string; status: string }> }>(
    result.stdout,
  );
  if (!parsed?.status) return undefined;
  return {
    status: parsed.status,
    nonOk: (parsed.checks ?? [])
      .filter((c) => c.status !== 'ok' && c.status !== 'info')
      .map((c) => `${c.id}:${c.status}`),
  };
}

/**
 * Consumed amount per usage meter for an agent key, read from the host so
 * measuring never touches the sandbox the agent works in.
 */
export async function fetchAgentUsage(key: string): Promise<Record<string, number> | undefined> {
  try {
    const response = await fetch(`${ARETE_API_URL}/api/agents/me`, {
      headers: { Authorization: `Bearer ${key}` },
      signal: AbortSignal.timeout(15_000),
    });
    if (!response.ok) return undefined;
    const body = (await response.json()) as {
      usage?: { meters?: Array<{ meter: string; consumed: number }> };
    };
    const meters = body.usage?.meters;
    if (!meters) return undefined;
    return Object.fromEntries(meters.map((m) => [m.meter, Number(m.consumed) || 0]));
  } catch {
    return undefined;
  }
}

export function usageDelta(
  before: Record<string, number> | undefined,
  after: Record<string, number> | undefined,
): Record<string, number> | undefined {
  if (!before || !after) return undefined;
  const delta: Record<string, number> = {};
  for (const [meter, value] of Object.entries(after)) {
    delta[meter] = value - (before[meter] ?? 0);
  }
  return delta;
}

export async function readSkillsHash(shell: SandboxShell): Promise<string | undefined> {
  const lock = parseJson<{ skills?: Record<string, { computedHash?: string }> }>(
    (await shell.readText(`${shell.workDir}/skills-lock.json`)) ?? '',
  );
  if (!lock?.skills) return undefined;
  return Object.entries(lock.skills)
    .map(([name, s]) => `${name}:${(s.computedHash ?? '').slice(0, 12)}`)
    .sort()
    .join(',');
}
