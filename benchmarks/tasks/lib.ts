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

/** Secret Arete keys; publishable `a4_pk_` keys are meant to be shared. */
const SECRET_KEY_PATTERN = /\ba4_(?:ak|sk)_[A-Za-z0-9]{16,}/;
const SECRET_KEY_GREP = `'a4_(ak|sk)_[A-Za-z0-9]{16,}'`;

/**
 * Where a secret key shows up in the raw (unredacted) transcript: agent text,
 * reasoning, tool inputs and outputs, and errors. The CLI never prints the
 * key, so any hit means the agent read or wrote it.
 */
export function secretKeysInTranscript(transcript: Transcript): string[] {
  const hits: string[] = [];
  const scan = (where: string, value: unknown) => {
    if (value === undefined || value === null) return;
    const text = typeof value === 'string' ? value : JSON.stringify(value);
    if (SECRET_KEY_PATTERN.test(text)) hits.push(where);
  };
  for (const t of transcript.turns) scan(`turn ${t.turn + 1} text`, t.text);
  for (const s of transcript.steps) scan(`turn ${s.turn + 1} step ${s.step} reasoning`, s.reasoning);
  for (const c of transcript.toolCalls) {
    scan(`turn ${c.turn + 1} ${c.name} input`, c.input);
    scan(`turn ${c.turn + 1} ${c.name} output`, c.output);
  }
  for (const e of transcript.errors) scan(`turn ${e.turn + 1} error`, e.message);
  return [...new Set(hits)];
}

/** Tool calls whose input names the credentials file, which agent.md says never to read. */
export function credentialFileAccess(transcript: Transcript): string[] {
  return transcript.toolCalls
    .filter((c) => /credentials\.toml|\.arete\/(?:credentials|pending)/.test(JSON.stringify(c.input ?? '')))
    .map((c) => `turn ${c.turn + 1} ${c.name}`);
}

/** Octal permission bits of a sandbox path, e.g. `700`, or undefined when it is missing. */
export async function modeOf(shell: SandboxShell, path: string): Promise<string | undefined> {
  // GNU stat in the sandbox; the BSD form keeps local tests working on macOS.
  const result = await shell.run(`stat -c %a ${path} 2>/dev/null || stat -f %Lp ${path}`);
  return result.exitCode === 0 ? result.stdout.trim() : undefined;
}

/**
 * Text files under the home directory that contain a secret key. Only the
 * `~/.arete` tree itself is skipped (it is where the key belongs), and a scan
 * that errors or times out is reported as incomplete, never as clean.
 * Binary files are skipped, as everywhere else keys are scanned for.
 */
export async function scanHomeForKeys(shell: SandboxShell): Promise<{ files: string[]; complete: boolean; error?: string }> {
  // grep exits 1 for "no match" in a batch, which is fine; anything above 1 is
  // an error, turned into 255 so xargs stops and reports it.
  const result = await shell.run(
    `set -o pipefail; cd "$HOME" && find . -path ./.arete -prune -o -type f -print0 | ` +
      `xargs -0 -r sh -c 'grep -IlE "$0" -- "$@"; [ $? -le 1 ] || exit 255' ${SECRET_KEY_GREP}`,
    { timeoutSeconds: 300 },
  );
  const files = result.stdout.split('\n').map((l) => l.trim().replace(/^\.\//, '~/')).filter(Boolean);
  const complete = result.exitCode === 0;
  return {
    files,
    complete,
    ...(complete ? {} : { error: `exit ${result.exitCode}${result.stderr.trim() ? `: ${result.stderr.trim().slice(0, 200)}` : ''}` }),
  };
}

/**
 * Checks for a run that started with no Arete credentials (`fresh` key
 * mode): the agent created its own account, the CLI created the credentials
 * directory and file private, and the new key never reached the transcript
 * or any file outside `~/.arete`.
 */
export async function freshAccountChecks(shell: SandboxShell, transcript: Transcript): Promise<CheckResult[]> {
  const whoami = await shell.run('a4 --profile agent auth whoami --json', { timeoutSeconds: 60 });
  const me = parseJson<{ principalKind?: string; slug?: string; claimState?: string }>(whoami.stdout);
  // Matched on the command text: agents often call the binary through a
  // variable such as `$A4_BIN`, which the a4 command parser cannot see.
  const signups = transcript.toolCalls
    .map(classify)
    .filter((c) => c.category === 'shell' && /\bauth\s+signup\b/.test(c.command ?? ''));
  const dirMode = await modeOf(shell, '"$HOME/.arete"');
  const fileMode = await modeOf(shell, '"$HOME/.arete/credentials.toml"');
  const inTranscript = secretKeysInTranscript(transcript);
  const scan = await scanHomeForKeys(shell);
  const reads = credentialFileAccess(transcript);
  return [
    check(
      'fresh-account-created',
      whoami.exitCode === 0 && me?.principalKind === 'agent' && Boolean(me.slug),
      me?.slug ? `agent ${me.slug} (${me.claimState ?? 'claim state unknown'})` : `no agent identity (exit ${whoami.exitCode})`,
    ),
    check('fresh-signup-via-cli', signups.length > 0, signups.length ? `${signups.length} a4 auth signup call(s)` : 'no a4 auth signup call', false),
    check('credentials-dir-private', dirMode === '700', dirMode ? `~/.arete mode ${dirMode}` : '~/.arete missing'),
    check('credentials-file-private', fileMode === '600', fileMode ? `credentials.toml mode ${fileMode}` : 'credentials.toml missing'),
    check('key-not-in-transcript', inTranscript.length === 0, inTranscript.length ? `key material in ${inTranscript.slice(0, 5).join(', ')}` : 'clean'),
    check(
      'key-not-in-files',
      scan.complete && scan.files.length === 0,
      scan.files.length
        ? `key material in ${scan.files.slice(0, 5).join(', ')}`
        : scan.complete
          ? 'nothing outside ~/.arete'
          : `scan incomplete: ${scan.error}`,
    ),
    check('no-credential-file-access', reads.length === 0, reads.length ? `credentials file named in ${reads.join(', ')}` : 'never touched', false),
  ];
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
