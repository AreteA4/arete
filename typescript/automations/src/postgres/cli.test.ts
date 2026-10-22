import { mkdtempSync, readdirSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { main } from '../cli';

function capture() {
  const lines: string[] = [];
  const errors: string[] = [];
  return { out: { log: (text: string) => lines.push(text), error: (text: string) => errors.push(text) }, lines, errors };
}

describe('arete-automations CLI', () => {
  it('writes the migrations as numbered SQL files', () => {
    const dir = join(mkdtempSync(join(tmpdir(), 'arete-cli-')), 'sql');
    const { out, lines } = capture();
    expect(main(['migrations', '--out', dir, '--schema', 'automation'], out)).toBe(0);
    expect(readdirSync(dir)).toEqual(['0001_initial.sql']);
    const sql = readFileSync(join(dir, '0001_initial.sql'), 'utf8');
    expect(sql).toContain('CREATE SCHEMA IF NOT EXISTS "automation";');
    expect(sql).toContain('CREATE TABLE "automation".jobs');
    expect(sql).toContain('INSERT INTO "automation".schema_migrations (version) VALUES (1);');
    expect(sql).not.toContain('arete_automation');
    expect(lines).toEqual([join(dir, '0001_initial.sql')]);
  });

  it('prints runtime role grants', () => {
    const { out, lines } = capture();
    expect(main(['grants', '--role', 'app_runtime'], out)).toBe(0);
    expect(lines[0]).toContain('GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA "arete_automation" TO "app_runtime";');
  });

  it('refuses bad input', () => {
    const bad = capture();
    expect(main(['migrations'], bad.out)).toBe(1);
    expect(main(['grants', '--role', 'x; DROP'], bad.out)).toBe(1);
    expect(main(['migrations', '--out', '/tmp/x', '--schema', 'Bad-Name'], bad.out)).toBe(1);
    expect(main(['migrations', '--out', '--schema', 'automation'], bad.out)).toBe(1);
    expect(main(['migrations', '--out', '/tmp/x', '--schema'], bad.out)).toBe(1);
    expect(main(['nope'], bad.out)).toBe(2);
    expect(bad.errors.some((line) => line.includes('Usage:'))).toBe(true);
  });
});
