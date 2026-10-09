import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, describe, test } from 'node:test';
import type { TextStreamPart, ToolSet } from 'ai';
import { readCodexRollouts } from '../src/native-usage.js';
import { Recorder } from '../src/recorder.js';
import { print, RunDir } from '../src/report.js';

// Console output reaches public CI logs, so these guard the order of
// redaction and shortening as well as redaction itself.
const KEY = `a4_ak_${'Z'.repeat(40)}`;
process.env.ARETE_AGENT_KEYS = KEY;
/** Any run of the key's body long enough to matter, e.g. a key cut short. */
const FRAGMENT = 'a4_ak_ZZZZ';

const scratch = mkdtempSync(join(tmpdir(), 'bench-redaction-'));
after(() => rmSync(scratch, { recursive: true, force: true }));

async function progressLines(parts: object[]): Promise<string[]> {
  const lines: string[] = [];
  const recorder = new Recorder((line) => lines.push(line));
  recorder.beginTurn(1, 'go', true);
  async function* stream() {
    yield* parts as unknown as TextStreamPart<ToolSet>[];
  }
  await recorder.consume(stream());
  return lines;
}

describe('recorder progress lines', () => {
  test('redact a key that crosses the 160-character preview cutoff', async () => {
    // The key starts at 150, so shortening first would keep 10 characters of
    // it: too few for the key pattern, enough to leak a prefix.
    const command = `${'x'.repeat(150)}${KEY} && npx tsx src/main.ts`;
    const lines = await progressLines([
      { type: 'start-step' },
      { type: 'tool-call', toolCallId: 't1', toolName: 'Bash', input: { command } },
    ]);
    const line = lines.find((l) => l.startsWith('Bash: '));
    assert.ok(line, `no tool-call line in ${JSON.stringify(lines)}`);
    assert.ok(!line.includes(FRAGMENT), line);
  });

  test('redact agent text and errors', async () => {
    const lines = await progressLines([
      { type: 'start-step' },
      { type: 'text-start', id: 'm1' },
      { type: 'text-delta', id: 'm1', text: `Using key ${KEY}` },
      { type: 'text-end', id: 'm1' },
      { type: 'error', error: new Error(`auth failed for ${KEY}`) },
    ]);
    const text = lines.find((l) => l.startsWith('text: '));
    const error = lines.find((l) => l.startsWith('error: '));
    assert.equal(text, 'text: Using key [REDACTED]');
    assert.ok(error?.includes('[REDACTED]'), String(error));
    for (const line of lines) assert.ok(!line.includes(FRAGMENT), line);
  });
});

describe('print', () => {
  test('redacts known secrets and unknown agent keys', () => {
    let out = '';
    const stream = { write: (chunk: string) => ((out += chunk), true) } as unknown as NodeJS.WriteStream;
    const unknownKey = `a4_sk_${'Q'.repeat(24)}`;
    print(`configured ${KEY}, unlisted ${unknownKey}\n`, stream);
    assert.equal(out, 'configured [REDACTED], unlisted [REDACTED_A4_KEY]\n');
  });
});

describe('downloaded sandbox trees', () => {
  test('redactTree rewrites real files but never follows links', () => {
    const host = join(scratch, 'host');
    const run = join(scratch, 'run');
    mkdirSync(host, { recursive: true });
    mkdirSync(join(run, 'workspace', 'src'), { recursive: true });
    writeFileSync(join(host, '.env'), `KEY=${KEY}\n`);
    writeFileSync(join(run, 'workspace', 'src', 'main.ts'), `const key = '${KEY}';\n`);
    // Relative links resolve against the host once the archive is extracted.
    symlinkSync('../../../host/.env', join(run, 'workspace', 'src', 'env-link'));
    symlinkSync(host, join(run, 'workspace', 'host-dir'));

    new RunDir(run).redactTree('workspace');

    assert.equal(readFileSync(join(host, '.env'), 'utf8'), `KEY=${KEY}\n`);
    assert.equal(readFileSync(join(run, 'workspace', 'src', 'main.ts'), 'utf8'), "const key = '[REDACTED]';\n");
  });

  test('Codex rollouts behind links are not read', () => {
    const outside = join(scratch, 'outside');
    const native = join(scratch, 'native');
    mkdirSync(outside, { recursive: true });
    mkdirSync(join(native, '.codex', 'sessions'), { recursive: true });
    writeFileSync(join(outside, 'rollout-1.jsonl'), '{"type":"event_msg","payload":{"type":"token_count"}}\n');
    symlinkSync(outside, join(native, '.codex', 'sessions', 'linked'));
    symlinkSync(join(outside, 'rollout-1.jsonl'), join(native, '.codex', 'sessions', 'rollout-2.jsonl'));

    assert.deepEqual(readCodexRollouts(native), []);
  });
});
