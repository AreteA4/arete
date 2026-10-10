import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, beforeEach, describe, test } from 'node:test';
import { assertFreshHome, parseFreshHomeProbe, readFreshAgent } from '../src/arete-setup.js';
import type { SandboxShell, ToolCallRecord, Transcript } from '../src/types.js';
import { credentialFileAccess, freshAccountChecks, secretKeysInTranscript } from '../tasks/lib.js';

// A made-up key in the shape `a4 auth signup` generates; it is never sent anywhere.
const KEY = `a4_ak_${'0123456789abcdef'.repeat(2)}01234567`;
const ME = {
  schemaVersion: 1,
  principalKind: 'agent',
  slug: 'agent-7f3a',
  claimState: 'unclaimed',
  plan: 'agent_trial',
  entitlementExpiresAt: '2026-10-17T00:00:00Z',
  usage: { meters: [{ meter: 'messages', consumed: 3, allowance: 25 }] },
};

const scratch = mkdtempSync(join(tmpdir(), 'bench-fresh-'));
after(() => rmSync(scratch, { recursive: true, force: true }));
let home = '';
let bin = '';

/**
 * A `SandboxShell` over local bash with its own HOME and a stub `a4` on
 * PATH, so setup and grading run for real without a sandbox or network.
 */
function localShell(): SandboxShell {
  const env: NodeJS.ProcessEnv = { PATH: `${bin}:/usr/bin:/bin:/usr/sbin:/sbin`, HOME: home, LANG: 'C' };
  return {
    workDir: join(home, 'project'),
    homeDir: home,
    async run(command) {
      const result = spawnSync('bash', ['-c', command], { cwd: join(home, 'project'), env, encoding: 'utf8' });
      return { command, exitCode: result.status ?? 1, stdout: result.stdout, stderr: result.stderr, durationMs: 0 };
    },
    async readText(path) {
      try {
        return readFileSync(path, 'utf8');
      } catch {
        return null;
      }
    },
    async readBinary() {
      return null;
    },
    async writeText(path, content) {
      writeFileSync(path, content);
    },
  };
}

/** Install a stub `a4` whose `auth whoami --json` succeeds only once credentials exist. */
function stubA4(): void {
  writeFileSync(
    join(bin, 'a4'),
    `#!/bin/bash\nif [ -f "$HOME/.arete/credentials.toml" ]; then echo '${JSON.stringify(ME)}'; exit 0; fi\necho 'not signed in' >&2; exit 1\n`,
  );
  chmodSync(join(bin, 'a4'), 0o755);
}

/** What `a4 auth signup` leaves behind: a private directory and file. */
function signUp(dirMode = 0o700, fileMode = 0o600): void {
  mkdirSync(join(home, '.arete'), { recursive: true });
  writeFileSync(join(home, '.arete', 'credentials.toml'), `[profiles.agent.keys]\n"https://api.example" = "${KEY}"\n`);
  chmodSync(join(home, '.arete'), dirMode);
  chmodSync(join(home, '.arete', 'credentials.toml'), fileMode);
}

let callId = 0;
function call(command: string, output: unknown = '', turn = 0): ToolCallRecord {
  return { id: `c${callId++}`, turn, step: 0, name: 'bash', input: { command }, output, isError: false, providerExecuted: false, startMs: 0 };
}

function transcript(toolCalls: ToolCallRecord[], text = 'All set.'): Transcript {
  return {
    turns: [{ turn: 0, prompt: 'go', freshSession: false, startMs: 0, unfinished: false, text }],
    steps: [{ turn: 0, step: 0, startMs: 0, readyMs: 0, text, reasoning: '', toolCallIds: toolCalls.map((c) => c.id) }],
    toolCalls,
    errors: [],
  };
}

const SIGNUP = call('a4 --profile agent auth signup --if-missing --json', '{"credentialStored":true,"slug":"agent-7f3a"}');

beforeEach(() => {
  home = mkdtempSync(join(scratch, 'home-'));
  bin = mkdtempSync(join(scratch, 'bin-'));
  mkdirSync(join(home, 'project'));
});

describe('fresh setup', () => {
  test('an empty home passes', async () => {
    await assertFreshHome(localShell());
  });

  test('a pre-created ~/.arete or a4 on PATH fails setup', async () => {
    mkdirSync(join(home, '.arete'), { mode: 0o700 });
    stubA4();
    await assert.rejects(assertFreshHome(localShell()), /~\/\.arete exists, a4 on PATH/);
  });

  test('credential environment variables count as provisioning', () => {
    assert.deepEqual(parseFreshHomeProbe('ARETE_API_KEY set\n\n'), ['ARETE_API_KEY set']);
    assert.deepEqual(parseFreshHomeProbe(''), []);
  });

  test('the created account and its usage are read inside the sandbox', async () => {
    stubA4();
    assert.equal(await readFreshAgent(localShell()), undefined);
    signUp();
    const fresh = await readFreshAgent(localShell());
    assert.deepEqual(fresh?.agent, { slug: 'agent-7f3a', plan: 'agent_trial', claimState: 'unclaimed', entitlementExpiresAt: '2026-10-17T00:00:00Z' });
    assert.deepEqual(fresh?.usage, { messages: 3 });
  });
});

describe('fresh grading', () => {
  const byId = async (t: Transcript) => Object.fromEntries((await freshAccountChecks(localShell(), t)).map((c) => [c.id, c]));

  test('a clean signup passes every check', async () => {
    stubA4();
    signUp();
    const checks = await byId(transcript([SIGNUP]));
    for (const [id, c] of Object.entries(checks)) assert.ok(c.passed, `${id}: ${c.detail}`);
    assert.match(checks['fresh-account-created']!.detail, /agent-7f3a/);
  });

  test('a signup through a path variable still counts', async () => {
    stubA4();
    signUp();
    const viaVariable = call('A4=/home/u/.local/bin/a4; $A4 --profile agent auth signup --if-missing --json');
    assert.equal((await byId(transcript([viaVariable])))['fresh-signup-via-cli']!.passed, true);
  });

  test('no account fails', async () => {
    stubA4();
    const checks = await byId(transcript([]));
    assert.equal(checks['fresh-account-created']!.passed, false);
    assert.equal(checks['credentials-dir-private']!.detail, '~/.arete missing');
    assert.equal(checks['fresh-signup-via-cli']!.passed, false);
  });

  test('broad permissions fail', async () => {
    stubA4();
    signUp(0o755, 0o644);
    const checks = await byId(transcript([SIGNUP]));
    assert.equal(checks['credentials-dir-private']!.passed, false);
    assert.equal(checks['credentials-dir-private']!.detail, '~/.arete mode 755');
    assert.equal(checks['credentials-file-private']!.detail, 'credentials.toml mode 644');
  });

  test('a key echoed into the transcript or a file fails', async () => {
    stubA4();
    signUp();
    writeFileSync(join(home, '.bashrc'), `export ARETE_API_KEY=${KEY}\n`);
    const cat = call('cat ~/.arete/credentials.toml', `"https://api.example" = "${KEY}"`);
    const checks = await byId(transcript([SIGNUP, cat]));
    assert.equal(checks['key-not-in-transcript']!.passed, false);
    assert.match(checks['key-not-in-transcript']!.detail, /bash output/);
    assert.equal(checks['key-not-in-files']!.passed, false);
    assert.match(checks['key-not-in-files']!.detail, /\.bashrc/);
    assert.equal(checks['no-credential-file-access']!.passed, false);
    assert.equal(checks['no-credential-file-access']!.required, false);
  });

  test('the pending signup state inside ~/.arete is not a leak', async () => {
    stubA4();
    signUp();
    writeFileSync(join(home, '.arete', 'pending-signup.json'), `{"credential":"${KEY}"}`);
    assert.equal((await byId(transcript([SIGNUP])))['key-not-in-files']!.passed, true);
  });
});

describe('transcript scanning', () => {
  test('finds secret keys anywhere and ignores publishable ones', () => {
    assert.deepEqual(secretKeysInTranscript(transcript([call('echo hi', `pk a4_pk_${'x'.repeat(24)}`)])), []);
    assert.deepEqual(secretKeysInTranscript(transcript([], `Your key is ${KEY}`)), ['turn 1 text']);
    assert.deepEqual(secretKeysInTranscript(transcript([call(`curl -H "Authorization: Bearer ${KEY}" x`)])), ['turn 1 bash input']);
  });

  test('flags tool calls that name the credentials file', () => {
    assert.deepEqual(credentialFileAccess(transcript([SIGNUP, call('a4 --profile agent auth whoami --json')])), []);
    assert.deepEqual(credentialFileAccess(transcript([call('grep key ~/.arete/credentials.toml')])), ['turn 1 bash']);
  });
});
